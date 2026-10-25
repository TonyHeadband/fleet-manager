//! SQLite persistence: current agents plus an append-only, trimmed event log with global `seq`.
use fleet_api::{Agent, AgentId, Event, EventKind, RepoSource, Snapshot};
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
