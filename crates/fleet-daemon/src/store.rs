//! SQLite persistence: current agents plus an append-only, trimmed event log with global `seq`.
use fleet_api::{
    Agent, AgentId, Event, EventKind, GithubAuth, LogEntry, Project, ProjectState, RepoSource,
    Snapshot, Status, StatusTimes, StopCause,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

pub const DEFAULT_RETAINED_EVENTS: u64 = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no such agent: {0:?}")]
    NotFound(AgentId),
}

type Result<T> = std::result::Result<T, StoreError>;

/// A repo the daemon remembers. `location` is what the user gave (path or URL); `base` is the checkout worktrees hang off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRow {
    pub name: String,
    pub source: RepoSource,
    pub location: String,
    pub base: PathBuf,
    /// None until a persona adopts the repo.
    pub persona: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersonaRow {
    pub id: String,
    pub name: String,
    pub host: String,
    pub commit_name: String,
    pub commit_email: String,
    pub github_auth: GithubAuth,
    pub ready: bool,
    pub created_at: i64,
}

// sqlx only takes static SQL, so the column list is spliced in at compile time.
macro_rules! select_repos {
    ($tail:literal) => {
        concat!(
            "SELECT persona, name, source, location, base FROM repos ",
            $tail
        )
    };
}

/// The host of an http(s) URL, lowercased, without port or `www.`. scp-like (`git@host:x`) and `ssh://` remotes
/// give None: a persona's host rule only covers URLs its own login can serve.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority.rsplit('@').next()?;
    let host = hostport.split(':').next()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    (!host.is_empty()).then(|| host.to_string())
}

fn auth_str(a: GithubAuth) -> &'static str {
    match a {
        GithubAuth::Gh => "gh",
        GithubAuth::Pat => "pat",
    }
}

fn persona_row(r: &sqlx::sqlite::SqliteRow) -> Result<PersonaRow> {
    let github_auth = match r.get::<&str, _>("github_auth") {
        "gh" => GithubAuth::Gh,
        "pat" => GithubAuth::Pat,
        other => {
            return Err(sqlx::Error::Decode(format!("unknown github auth {other:?}").into()).into())
        }
    };
    Ok(PersonaRow {
        id: r.get("id"),
        name: r.get("name"),
        host: r.get("host"),
        commit_name: r.get("commit_name"),
        commit_email: r.get("commit_email"),
        github_auth,
        ready: r.get::<i64, _>("ready") != 0,
        created_at: r.get("created_at"),
    })
}

fn source_str(s: RepoSource) -> &'static str {
    match s {
        RepoSource::Path => "path",
        RepoSource::Url => "url",
    }
}

fn repo_row(r: &sqlx::sqlite::SqliteRow) -> Result<RepoRow> {
    let source = match r.get::<&str, _>("source") {
        "url" => RepoSource::Url,
        "path" => RepoSource::Path,
        other => {
            return Err(sqlx::Error::Decode(format!("unknown repo source {other:?}").into()).into())
        }
    };
    Ok(RepoRow {
        name: r.get("name"),
        source,
        location: r.get("location"),
        base: PathBuf::from(r.get::<String, _>("base")),
        persona: r.get("persona"),
    })
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub struct Store {
    pool: SqlitePool,
    retain: u64,
    // sqlite takes a write lock per transaction; serialising here avoids SQLITE_BUSY upgrades.
    write_lock: Mutex<()>,
    // Sent after commit, still under the write lock, so subscribers see events in `seq` order.
    live: broadcast::Sender<Event>,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Store> {
        Store::open_with(path, DEFAULT_RETAINED_EVENTS).await
    }

    pub async fn open_with(path: &Path, retain: u64) -> Result<Store> {
        let in_memory = path == Path::new(":memory:");
        let opts = if in_memory {
            SqliteConnectOptions::from_str("sqlite::memory:")?
        } else {
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
                // WAL needs shared memory, which is unsafe on NFS homes; the rollback journal is fine for one writer.
                .journal_mode(SqliteJournalMode::Delete)
        }
        .busy_timeout(Duration::from_secs(5));
        // A private in-memory database exists per connection, so it must stay one long-lived connection.
        let pool = SqlitePoolOptions::new()
            .max_connections(if in_memory { 1 } else { 4 })
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await?;
        sqlx::raw_sql(include_str!("../migrations/001_init.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../migrations/002_repos.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../migrations/003_projects.sql"))
            .execute(&pool)
            .await?;
        // 004 rebuilds `repos`, so unlike the others it must run exactly once.
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        if version < 4 {
            let mut tx = pool.begin().await?;
            sqlx::raw_sql(include_str!("../migrations/004_personas.sql"))
                .execute(&mut *tx)
                .await?;
            // PRAGMA takes no bind parameters; the value is a constant.
            sqlx::raw_sql("PRAGMA user_version = 4")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(Store {
            pool,
            retain,
            write_lock: Mutex::new(()),
            live: broadcast::channel(1024).0,
        })
    }

    pub async fn repos(&self) -> Result<Vec<RepoRow>> {
        let rows = sqlx::query(select_repos!("ORDER BY added_at, name"))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(repo_row).collect()
    }

    pub async fn repos_for(&self, persona: &str) -> Result<Vec<RepoRow>> {
        let rows = sqlx::query(select_repos!(
            "WHERE COALESCE(persona, '') = ? ORDER BY added_at, name"
        ))
        .bind(persona)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(repo_row).collect()
    }

    pub async fn repo(&self, persona: &str, name: &str) -> Result<Option<RepoRow>> {
        let row = sqlx::query(select_repos!(
            "WHERE COALESCE(persona, '') = ? AND name = ?"
        ))
        .bind(persona)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(repo_row).transpose()
    }

    /// The row at a path. A URL can be remembered by several personas, so look one up with `repo_at`.
    pub async fn repo_by_location(&self, location: &str) -> Result<Option<RepoRow>> {
        let row = sqlx::query(select_repos!("WHERE location = ?"))
            .bind(location)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(repo_row).transpose()
    }

    pub async fn repo_at(&self, persona: &str, location: &str) -> Result<Option<RepoRow>> {
        let row = sqlx::query(select_repos!(
            "WHERE COALESCE(persona, '') = ? AND location = ?"
        ))
        .bind(persona)
        .bind(location)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(repo_row).transpose()
    }

    /// Gives an unadopted repo to `persona` under `name`; false when it has an owner by now.
    pub async fn claim_repo(&self, location: &str, persona: &str, name: &str) -> Result<bool> {
        let _w = self.write_lock.lock().await;
        let n = sqlx::query(
            "UPDATE repos SET persona = ?, name = ? WHERE location = ? AND persona IS NULL",
        )
        .bind(persona)
        .bind(name)
        .bind(location)
        .execute(&self.pool)
        .await?
        .rows_affected();
        Ok(n > 0)
    }

    /// Fails if the location is already remembered by the persona (a path: by anyone), or the name is already taken
    /// within the persona.
    pub async fn insert_repo(&self, r: &RepoRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO repos (persona, name, source, location, base, added_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.persona)
        .bind(&r.name)
        .bind(source_str(r.source))
        .bind(&r.location)
        .bind(r.base.to_string_lossy().as_ref())
        .bind(now_secs())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn personas(&self) -> Result<Vec<PersonaRow>> {
        let rows = sqlx::query("SELECT * FROM personas ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(persona_row).collect()
    }

    pub async fn persona(&self, id: &str) -> Result<Option<PersonaRow>> {
        let row = sqlx::query("SELECT * FROM personas WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(persona_row).transpose()
    }

    /// Fails if the id is taken.
    pub async fn insert_persona(&self, p: &PersonaRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO personas (id, name, host, commit_name, commit_email, github_auth, ready, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(&p.host)
        .bind(&p.commit_name)
        .bind(&p.commit_email)
        .bind(auth_str(p.github_auth))
        .bind(p.ready)
        .bind(p.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Writes the editable fields; `host`, `ready` and `created_at` are not changed here.
    pub async fn update_persona(&self, p: &PersonaRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "UPDATE personas SET name = ?, commit_name = ?, commit_email = ?, github_auth = ? WHERE id = ?",
        )
        .bind(&p.name)
        .bind(&p.commit_name)
        .bind(&p.commit_email)
        .bind(auth_str(p.github_auth))
        .bind(&p.id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_persona_ready(&self, id: &str) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("UPDATE personas SET ready = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_persona(&self, id: &str) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("DELETE FROM personas WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Counts and deletes in one transaction, so a project cannot appear between the two. Returns how many
    /// projects block the delete; 0 means the persona is gone.
    pub async fn delete_persona_if_unused(&self, id: &str) -> Result<u32> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM projects WHERE json_extract(data, '$.persona') = ? AND json_extract(data, '$.state') IN ('running', 'stopped')",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if n == 0 {
            sqlx::query("DELETE FROM personas WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            // Its repos go too: a path it held would stay claimed by no one, and a persona later made with the
            // same name (so the same id) would inherit them.
            sqlx::query("DELETE FROM repos WHERE persona = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(n as u32)
    }

    /// Marks the persona ready and hands it what predates personas, in one transaction that first checks the row
    /// still exists: a persona deleted while its CLIs were being asked must not take the orphans with it.
    /// `paths` are the unadopted path repos whose origin the persona may reach (see `adopt_orphans`).
    /// None when the persona is gone.
    pub async fn ready_and_adopt(&self, id: &str, paths: &[String]) -> Result<Option<(u32, u32)>> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let Some(host) = sqlx::query_scalar::<_, String>("SELECT host FROM personas WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(None);
        };
        sqlx::query("UPDATE personas SET ready = 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let (counts, events) = self.adopt_in(&mut tx, id, &host, paths).await?;
        tx.commit().await?;
        for ev in events {
            let _ = self.live.send(ev);
        }
        Ok(Some(counts))
    }

    /// Whether any project record, archived ones included, belongs to the persona.
    pub async fn persona_has_records(&self, id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM projects WHERE json_extract(data, '$.persona') = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Project records that still hold something: running or stopped, not archived.
    pub async fn persona_project_count(&self, id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM projects WHERE json_extract(data, '$.persona') = ? AND json_extract(data, '$.state') IN ('running', 'stopped')",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
    }

    /// Hands what predates personas to `id`: every project record and agent with no persona (archived records
    /// included, but not an archived orphan's record), the URL repos with no persona on `host`, and the path repos
    /// with no persona at `paths`. The store cannot read a checkout's origin, so the caller picks the paths: those
    /// whose origin is on the persona's host, or that have none. Returns (project records, repos) adopted.
    pub async fn adopt_orphans(
        &self,
        id: &str,
        host: &str,
        paths: &[String],
    ) -> Result<(u32, u32)> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let (counts, events) = self.adopt_in(&mut tx, id, host, paths).await?;
        tx.commit().await?;
        for ev in events {
            let _ = self.live.send(ev);
        }
        Ok(counts)
    }

    async fn adopt_in(
        &self,
        tx: &mut sqlx::SqliteConnection,
        id: &str,
        host: &str,
        paths: &[String],
    ) -> Result<((u32, u32), Vec<Event>)> {
        // An archived orphan's record is the only one no agent ever ran in (agents_used 0). It is persona-less by
        // design, not from before personas, so it is not handed to whichever persona logs in next.
        let projects = sqlx::query(
            "UPDATE projects SET data = json_set(data, '$.persona', ?) WHERE COALESCE(json_extract(data, '$.persona'), '') = '' AND json_extract(data, '$.agents_used') > 0",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let adopted_ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM agents WHERE COALESCE(json_extract(data, '$.persona'), '') = ''",
        )
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE agents SET data = json_set(data, '$.persona', ?) WHERE COALESCE(json_extract(data, '$.persona'), '') = ''",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        let orphans = sqlx::query("SELECT name, source, location FROM repos WHERE persona IS NULL")
            .fetch_all(&mut *tx)
            .await?;
        let mut repos = 0;
        for r in &orphans {
            let location: &str = r.get("location");
            let adopt = match r.get::<&str, _>("source") {
                "url" => url_host(location).as_deref() == Some(host),
                // A path not passed (origin on another host) waits for its own persona: it is claimed when typed
                // under it, or adopted when that persona becomes ready.
                _ => paths.iter().any(|p| p == location),
            };
            if adopt {
                // A name already taken in the persona stays unadopted rather than failing the whole migration.
                repos += sqlx::query(
                    "UPDATE OR IGNORE repos SET persona = ? WHERE persona IS NULL AND location = ?",
                )
                .bind(id)
                .bind(location)
                .execute(&mut *tx)
                .await?
                .rows_affected() as u32;
            }
        }
        // The stream has no "agent updated" event and StatusChanged carries no persona, so a live client learns of
        // the adoption by seeing the card leave and come back. It happens once per daemon, on the first login.
        let mut events = Vec::new();
        for aid in &adopted_ids {
            let data: String = sqlx::query_scalar("SELECT data FROM agents WHERE id = ?")
                .bind(aid)
                .fetch_one(&mut *tx)
                .await?;
            let agent: Agent = serde_json::from_str(&data)?;
            events.push(
                self.append(
                    tx,
                    EventKind::AgentRemoved {
                        id: agent.id.clone(),
                    },
                )
                .await?,
            );
            events.push(self.append(tx, EventKind::AgentCreated { agent }).await?);
        }
        Ok(((projects as u32, repos), events))
    }

    /// Imports v0.1 `[[repos]]` as persona-less path rows. OR IGNORE covers both the name and the unique location,
    /// so re-running on every start is a no-op and never overrides a repo the user renamed.
    pub async fn seed_repos(&self, seed: &[crate::config::Repo]) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        for r in seed {
            let loc = r.path.to_string_lossy();
            sqlx::query("INSERT OR IGNORE INTO repos (persona, name, source, location, base, added_at) VALUES (NULL, ?, 'path', ?, ?, ?)")
                .bind(&r.name)
                .bind(loc.as_ref())
                .bind(loc.as_ref())
                .bind(now_secs())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Committed events as they happen. A receiver that falls behind gets `Lagged` and must resync from a snapshot.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.live.subscribe()
    }

    pub async fn insert_agent(&self, a: &Agent, token: &str) -> Result<Event> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO agents (id, token, data) VALUES (?, ?, ?)")
            .bind(&a.id.0)
            .bind(token)
            .bind(serde_json::to_string(a)?)
            .execute(&mut *tx)
            .await?;
        let repos = match a
            .workspaces
            .iter()
            .map(|w| w.repo.as_str())
            .collect::<Vec<_>>()
        {
            r if r.is_empty() => "no repos".to_string(),
            r => r.join(", "),
        };
        let mut project = project_of(a, ProjectState::Running, Some(a.status_since));
        project.log.push(LogEntry {
            at: a.status_since,
            text: format!("started with {repos}"),
        });
        save_project(&mut tx, &project).await?;
        let ev = self
            .append(&mut tx, EventKind::AgentCreated { agent: a.clone() })
            .await?;
        tx.commit().await?;
        let _ = self.live.send(ev.clone());
        Ok(ev)
    }

    /// Applies `f` and returns the events it produced: a `StatusChanged` when a board-visible field
    /// changed and a `WorkspacesChanged` when the workspaces did. Any other change is stored silently.
    pub async fn update_agent(
        &self,
        id: &AgentId,
        f: impl FnOnce(&mut Agent),
    ) -> Result<Vec<Event>> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT data FROM agents WHERE id = ?")
            .bind(&id.0)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        let old: Agent = serde_json::from_str(row.get::<&str, _>("data"))?;
        let mut new = old.clone();
        f(&mut new);
        if new == old {
            return Ok(vec![]);
        }
        sqlx::query("UPDATE agents SET data = ? WHERE id = ?")
            .bind(serde_json::to_string(&new)?)
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        if let Some(mut p) = load_project(&mut tx, id).await? {
            let before = p.clone();
            if old.status != new.status {
                let span = (new.status_since - old.status_since).max(0);
                match old.status {
                    Status::Queued => p.times.queued += span,
                    Status::Working => p.times.working += span,
                    Status::NeedsYou => p.times.needs_you += span,
                    Status::Review => p.times.review += span,
                    Status::Talking | Status::Stopped => {}
                }
                if new.status == Status::Stopped {
                    p.state = ProjectState::Stopped;
                    p.stop_cause = new.stop_cause;
                    p.stopped_at = Some(new.status_since);
                    // A stop the owner asked for is logged by the caller, which knows more than the store.
                    let text = match new.stop_cause {
                        Some(StopCause::Exited) => Some("agent exited".to_string()),
                        Some(StopCause::Crashed) => Some(new.activity.clone()),
                        _ => None,
                    };
                    if let Some(text) = text {
                        p.log.push(LogEntry {
                            at: new.status_since,
                            text,
                        });
                    }
                }
            }
            if old.workspaces != new.workspaces {
                p.repos = new.workspaces.clone();
            }
            if p != before {
                save_project(&mut tx, &p).await?;
            }
        }
        let visible = |a: &Agent| {
            (
                a.status,
                a.activity.clone(),
                a.status_since,
                a.has_changes,
                a.paused,
                a.blocked_reason,
                a.repo_request.clone(),
                a.stop_cause,
                a.disk_bytes,
            )
        };
        let mut events = vec![];
        if visible(&old) != visible(&new) {
            let kind = EventKind::StatusChanged {
                id: id.clone(),
                status: new.status,
                activity: new.activity.clone(),
                since: new.status_since,
                has_changes: new.has_changes,
                paused: new.paused,
                blocked_reason: new.blocked_reason,
                repo_request: new.repo_request.clone(),
                stop_cause: new.stop_cause,
                disk_bytes: new.disk_bytes,
            };
            events.push(self.append(&mut tx, kind).await?);
        }
        if old.workspaces != new.workspaces {
            let kind = EventKind::WorkspacesChanged {
                id: id.clone(),
                workspaces: new.workspaces.clone(),
            };
            events.push(self.append(&mut tx, kind).await?);
        }
        tx.commit().await?;
        for ev in &events {
            let _ = self.live.send(ev.clone());
        }
        Ok(events)
    }

    /// Every project record, archived ones included, oldest first; records made on upgrade have no start time and lead.
    pub async fn projects(&self) -> Result<Vec<Project>> {
        let rows = sqlx::query("SELECT data FROM projects ORDER BY rowid")
            .fetch_all(&self.pool)
            .await?;
        let mut all = rows
            .iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("data")))
            .collect::<std::result::Result<Vec<Project>, _>>()?;
        all.sort_by_key(|p| p.created_at);
        Ok(all)
    }

    pub async fn project(&self, id: &AgentId) -> Result<Option<Project>> {
        let mut conn = self.pool.acquire().await?;
        load_project(&mut conn, id).await
    }

    /// Edits the record in place. Stored silently: no event, the board does not show projects live.
    pub async fn update_project(&self, id: &AgentId, f: impl FnOnce(&mut Project)) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let mut p = load_project(&mut tx, id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        f(&mut p);
        save_project(&mut tx, &p).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn project_log(&self, id: &AgentId, at: i64, text: &str) -> Result<()> {
        self.update_project(id, |p| {
            p.log.push(LogEntry {
                at,
                text: text.to_string(),
            })
        })
        .await
    }

    /// Gives every agent that predates project records one, so the Projects view lists them. Returns how many
    /// it made; the start time is unknown, so `created_at` stays empty.
    pub async fn backfill_projects(&self) -> Result<usize> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT a.data FROM agents a LEFT JOIN projects p ON p.id = a.id WHERE p.id IS NULL ORDER BY a.rowid",
        )
        .fetch_all(&mut *tx)
        .await?;
        for r in &rows {
            let a: Agent = serde_json::from_str(r.get::<&str, _>("data"))?;
            let stopped = a.status == Status::Stopped;
            let mut p = project_of(
                &a,
                if stopped {
                    ProjectState::Stopped
                } else {
                    ProjectState::Running
                },
                None,
            );
            p.stop_cause = a.stop_cause;
            p.stopped_at = stopped.then_some(a.status_since);
            p.log.push(LogEntry {
                at: now_secs(),
                text: "record created on upgrade".into(),
            });
            save_project(&mut tx, &p).await?;
        }
        tx.commit().await?;
        Ok(rows.len())
    }

    pub async fn remove_agent(&self, id: &AgentId) -> Result<Event> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let gone = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        if gone.rows_affected() == 0 {
            return Err(StoreError::NotFound(id.clone()));
        }
        let ev = self
            .append(&mut tx, EventKind::AgentRemoved { id: id.clone() })
            .await?;
        tx.commit().await?;
        let _ = self.live.send(ev.clone());
        Ok(ev)
    }

    /// Writes a project's record as archived and removes its card in one transaction, so the card never leaves
    /// the board without the record saying why. A record with no card left (an earlier half-written archive) is
    /// still written.
    pub async fn archive_project(&self, id: &AgentId, f: impl FnOnce(&mut Project)) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let mut p = load_project(&mut tx, id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        f(&mut p);
        save_project(&mut tx, &p).await?;
        let gone = sqlx::query("DELETE FROM agents WHERE id = ?")
            .bind(&id.0)
            .execute(&mut *tx)
            .await?;
        let ev = if gone.rows_affected() > 0 {
            Some(
                self.append(&mut tx, EventKind::AgentRemoved { id: id.clone() })
                    .await?,
            )
        } else {
            None
        };
        tx.commit().await?;
        if let Some(ev) = ev {
            let _ = self.live.send(ev);
        }
        Ok(())
    }

    /// Stores a record for something that had none (an archived orphan). An id already taken is an error, never
    /// an overwrite: records are never replaced.
    pub async fn insert_project(&self, p: &Project) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("INSERT INTO projects (id, data) VALUES (?, ?)")
            .bind(&p.id.0)
            .bind(serde_json::to_string(p)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn snapshot(&self) -> Result<Snapshot> {
        let mut tx = self.pool.begin().await?;
        let seq = last_seq(&mut tx).await?;
        let rows = sqlx::query("SELECT data FROM agents ORDER BY rowid")
            .fetch_all(&mut *tx)
            .await?;
        let agents = rows
            .iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("data")))
            .collect::<std::result::Result<Vec<Agent>, _>>()?;
        Ok(Snapshot { seq, agents })
    }

    /// Events after `seq`, gap-free; `None` when the log no longer reaches back that far or `seq`
    /// is from the future (database was reset), meaning the client must take a fresh snapshot.
    pub async fn events_since(&self, seq: u64) -> Result<Option<Vec<Event>>> {
        let mut tx = self.pool.begin().await?;
        let last = last_seq(&mut tx).await?;
        if seq > last {
            return Ok(None);
        }
        if seq == last {
            return Ok(Some(vec![]));
        }
        let oldest: i64 = sqlx::query("SELECT MIN(seq) AS m FROM events")
            .fetch_one(&mut *tx)
            .await?
            .get("m");
        if (oldest as u64) > seq + 1 {
            return Ok(None);
        }
        let rows = sqlx::query("SELECT seq, data FROM events WHERE seq > ? ORDER BY seq")
            .bind(seq as i64)
            .fetch_all(&mut *tx)
            .await?;
        let events = rows
            .iter()
            .map(|r| {
                Ok(Event {
                    seq: r.get::<i64, _>("seq") as u64,
                    kind: serde_json::from_str(r.get::<&str, _>("data"))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(events))
    }

    pub async fn agent_token(&self, id: &AgentId) -> Result<Option<String>> {
        let row = sqlx::query("SELECT token FROM agents WHERE id = ?")
            .bind(&id.0)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("token")))
    }

    async fn append(&self, tx: &mut sqlx::SqliteConnection, kind: EventKind) -> Result<Event> {
        let seq = sqlx::query("INSERT INTO events (data) VALUES (?)")
            .bind(serde_json::to_string(&kind)?)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid() as u64;
        sqlx::query("DELETE FROM events WHERE seq <= ?")
            .bind(seq.saturating_sub(self.retain) as i64)
            .execute(&mut *tx)
            .await?;
        Ok(Event { seq, kind })
    }
}

async fn last_seq(tx: &mut sqlx::SqliteConnection) -> Result<u64> {
    let row = sqlx::query("SELECT seq FROM sqlite_sequence WHERE name = 'events'")
        .fetch_optional(&mut *tx)
        .await?;
    Ok(row.map(|r| r.get::<i64, _>("seq") as u64).unwrap_or(0))
}

fn project_of(a: &Agent, state: ProjectState, created_at: Option<i64>) -> Project {
    Project {
        id: a.id.clone(),
        name: std::path::Path::new(&a.workdir)
            .file_name()
            .map_or_else(|| a.workdir.clone(), |n| n.to_string_lossy().into_owned()),
        task: a.task.clone(),
        workdir: a.workdir.clone(),
        repos: a.workspaces.clone(),
        state,
        stop_cause: None,
        created_at,
        stopped_at: None,
        archived_at: None,
        agents_used: 1,
        times: StatusTimes::default(),
        log: vec![],
        footprint: None,
        last_screen: None,
        archive: None,
        persona: a.persona.clone(),
        breakdowns: vec![],
        gate_runs: vec![],
        inbox: vec![],
        kind: fleet_api::AgentKind::Solo,
        lead: None,
        role: None,
        roster: vec![],
        test_command: None,
    }
}

async fn load_project(conn: &mut sqlx::SqliteConnection, id: &AgentId) -> Result<Option<Project>> {
    let row = sqlx::query("SELECT data FROM projects WHERE id = ?")
        .bind(&id.0)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row
        .map(|r| serde_json::from_str(r.get::<&str, _>("data")))
        .transpose()?)
}

async fn save_project(conn: &mut sqlx::SqliteConnection, p: &Project) -> Result<()> {
    sqlx::query("INSERT INTO projects (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data")
        .bind(&p.id.0)
        .bind(serde_json::to_string(p)?)
        .execute(&mut *conn)
        .await?;
    Ok(())
}
