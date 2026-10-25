//! SQLite persistence: current agents plus an append-only, trimmed event log with global `seq`.
use fleet_api::{
    Agent, AgentId, Event, EventKind, LogEntry, Project, ProjectState, RepoSource, Snapshot,
    Status, StatusTimes, StopCause,
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
    })
}

fn now_secs() -> i64 {
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
        Ok(Store {
            pool,
            retain,
            write_lock: Mutex::new(()),
            live: broadcast::channel(1024).0,
        })
    }

    pub async fn repos(&self) -> Result<Vec<RepoRow>> {
        let rows =
            sqlx::query("SELECT name, source, location, base FROM repos ORDER BY added_at, name")
                .fetch_all(&self.pool)
                .await?;
        rows.iter().map(repo_row).collect()
    }

    pub async fn repo(&self, name: &str) -> Result<Option<RepoRow>> {
        let row = sqlx::query("SELECT name, source, location, base FROM repos WHERE name = ?")
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(repo_row).transpose()
    }

    pub async fn repo_by_location(&self, location: &str) -> Result<Option<RepoRow>> {
        let row = sqlx::query("SELECT name, source, location, base FROM repos WHERE location = ?")
            .bind(location)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(repo_row).transpose()
    }

    /// Fails if the name or the location is already remembered.
    pub async fn insert_repo(&self, r: &RepoRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO repos (name, source, location, base, added_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&r.name)
        .bind(source_str(r.source))
        .bind(&r.location)
        .bind(r.base.to_string_lossy().as_ref())
        .bind(now_secs())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Imports v0.1 `[[repos]]` as path rows. OR IGNORE covers both the name key and the unique location,
    /// so re-running on every start is a no-op and never overrides a repo the user renamed.
    pub async fn seed_repos(&self, seed: &[crate::config::Repo]) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        for r in seed {
            let loc = r.path.to_string_lossy();
            sqlx::query("INSERT OR IGNORE INTO repos (name, source, location, base, added_at) VALUES (?, 'path', ?, ?, ?)")
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
