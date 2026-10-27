//! SQLite persistence: current agents plus an append-only, trimmed event log with global `seq`.
use fleet_api::{
    task_id, task_outcome, Agent, AgentId, AgentKind, Approval, BlockedReason, Event, EventKind,
    FleetEvent, FleetEventKind, GithubAuth, LogEntry, ProductOverride, Snapshot, Status,
    StatusTimes, StepState, StopCause, Task, TaskOutcome, TaskState,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Connection, Row, SqlitePool};
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
    #[error("no such persona: {0}")]
    PersonaGone(String),
    #[error("no such product: {0}")]
    ProductGone(String),
    #[error("release {release} is not a release of {product}")]
    ReleaseGone { release: i64, product: String },
    /// The task's PRs merged or it was marked done: it stays in the release it finished in.
    #[error("a finished task stays in the release it finished in")]
    Finished,
    /// Tasks join a release only until it is marked released; the text is its version.
    #[error("release {0} is already released")]
    Released(String),
    /// A unique name or version already taken; the text names it.
    #[error("{0} is already taken")]
    Conflict(String),
}

type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductRow {
    pub id: String,
    pub name: String,
    pub default_persona: Option<String>,
    pub created_at: i64,
}

/// A version of a product. Dates are `YYYY-MM-DD`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseRow {
    pub id: i64,
    pub product: String,
    pub version: String,
    pub start: String,
    pub target: String,
    pub released_at: Option<i64>,
}

/// A product's repo: a local main checkout, a clone URL, or both (never neither). Each persona clones a URL repo
/// into its own folder (`Repos::clone_dir`), so the row names no clone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRow {
    pub product: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub url: Option<String>,
    /// Specialist names the repo offers an orchestrator.
    pub roster: Vec<String>,
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

/// Leftovers from before products: every entry under `<state_dir>/repos/` and under `tasks_root`. Listed once,
/// when the fresh start runs, for the user to delete; nothing deletes them.
pub fn leftovers(state_dir: &Path, tasks_root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in [state_dir.join("repos"), tasks_root.to_path_buf()] {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            found.extend(entries.flatten().map(|e| e.path()));
        }
    }
    found
}

/// A unique constraint the write broke, as `Conflict(what)`; anything else as it is.
fn conflict(e: sqlx::Error, what: impl FnOnce() -> String) -> StoreError {
    match &e {
        sqlx::Error::Database(d) if d.is_unique_violation() => StoreError::Conflict(what()),
        _ => e.into(),
    }
}

fn product_row(r: &sqlx::sqlite::SqliteRow) -> ProductRow {
    ProductRow {
        id: r.get("id"),
        name: r.get("name"),
        default_persona: r.get("default_persona"),
        created_at: r.get("created_at"),
    }
}

fn release_row(r: &sqlx::sqlite::SqliteRow) -> ReleaseRow {
    ReleaseRow {
        id: r.get("id"),
        product: r.get("product"),
        version: r.get("version"),
        start: r.get("start"),
        target: r.get("target"),
        released_at: r.get("released_at"),
    }
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

fn repo_row(r: &sqlx::sqlite::SqliteRow) -> Result<RepoRow> {
    Ok(RepoRow {
        product: r.get("product"),
        name: r.get("name"),
        path: r.get::<Option<String>, _>("path").map(PathBuf::from),
        url: r.get("url"),
        roster: serde_json::from_str(r.get::<&str, _>("roster"))?,
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
    // True when this open ran the fresh start (007).
    reset: bool,
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
        // One connection for every migration: a connection prepares DDL against the schema it last loaded, so an
        // ALTER on one that has not seen the previous ALTER (from another pooled connection) names a stale column.
        let mut conn = pool.acquire().await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *conn)
            .await?;
        sqlx::raw_sql(include_str!("../migrations/001_init.sql"))
            .execute(&mut *conn)
            .await?;
        // 002 and 003 make tables that 007 drops; run again on a newer database they would bring them back.
        if version < 7 {
            sqlx::raw_sql(include_str!("../migrations/002_repos.sql"))
                .execute(&mut *conn)
                .await?;
            sqlx::raw_sql(include_str!("../migrations/003_projects.sql"))
                .execute(&mut *conn)
                .await?;
        }
        // 004 to 008 alter or drop tables, so unlike the others they must run exactly once.
        // PRAGMA takes no bind parameters, so each step carries its own constant.
        let steps: [(i64, &'static str, &'static str); 5] = [
            (
                4,
                include_str!("../migrations/004_personas.sql"),
                "PRAGMA user_version = 4",
            ),
            (
                5,
                include_str!("../migrations/005_fleet.sql"),
                "PRAGMA user_version = 5",
            ),
            (
                6,
                include_str!("../migrations/006_workflow.sql"),
                "PRAGMA user_version = 6",
            ),
            (
                7,
                include_str!("../migrations/007_products.sql"),
                "PRAGMA user_version = 7",
            ),
            (
                8,
                include_str!("../migrations/008_product_overrides.sql"),
                "PRAGMA user_version = 8",
            ),
        ];
        for (_, sql, pragma) in steps.into_iter().filter(|(to, ..)| version < *to) {
            let mut tx = conn.begin().await?;
            sqlx::raw_sql(sql).execute(&mut *tx).await?;
            sqlx::raw_sql(pragma).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        drop(conn);
        Ok(Store {
            pool,
            retain,
            write_lock: Mutex::new(()),
            live: broadcast::channel(1024).0,
            reset: version < 7,
        })
    }

    /// True when this open dropped the old tables (the fresh start of products), so the caller lists what the old
    /// rows left on disk.
    pub fn was_reset(&self) -> bool {
        self.reset
    }

    pub async fn products(&self) -> Result<Vec<ProductRow>> {
        let rows = sqlx::query("SELECT * FROM products ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(product_row).collect())
    }

    pub async fn product(&self, id: &str) -> Result<Option<ProductRow>> {
        let row = sqlx::query("SELECT * FROM products WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(product_row))
    }

    /// `Conflict` when the id or the name is taken.
    pub async fn insert_product(&self, p: &ProductRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO products (id, name, default_persona, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.name)
        .bind(&p.default_persona)
        .bind(p.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| conflict(e, || format!("product name {:?}", p.name)))?;
        Ok(())
    }

    /// Writes the name and the default persona; `Conflict` when the name is another product's.
    pub async fn update_product(&self, p: &ProductRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("UPDATE products SET name = ?, default_persona = ? WHERE id = ?")
            .bind(&p.name)
            .bind(&p.default_persona)
            .bind(&p.id)
            .execute(&self.pool)
            .await
            .map_err(|e| conflict(e, || format!("product name {:?}", p.name)))?;
        Ok(())
    }

    /// Counts the product's task records, archived ones included, and deletes it with its repos and releases only
    /// when there are none, in one transaction so a task cannot appear between the two. Returns the count.
    pub async fn delete_product_if_unused(&self, id: &str) -> Result<u32> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tasks WHERE json_extract(data, '$.product') = ?",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if n == 0 {
            for sql in [
                "DELETE FROM repos WHERE product = ?",
                "DELETE FROM releases WHERE product = ?",
                "DELETE FROM products WHERE id = ?",
            ] {
                sqlx::query(sql).bind(id).execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        Ok(n as u32)
    }

    /// How many task records each product has, archived ones included.
    pub async fn product_task_counts(&self) -> Result<std::collections::HashMap<String, u32>> {
        let rows = sqlx::query(
            "SELECT json_extract(data, '$.product') AS product, COUNT(*) AS n FROM tasks GROUP BY 1",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let product: Option<String> = r.get("product");
                Some((product?, r.get::<i64, _>("n") as u32))
            })
            .collect())
    }

    pub async fn product_overrides(&self, product: &str) -> Result<Vec<ProductOverride>> {
        let rows = sqlx::query(
            "SELECT section, key, value FROM product_overrides WHERE product = ? ORDER BY section, key",
        )
        .bind(product)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| ProductOverride {
                section: r.get("section"),
                key: r.get("key"),
                value: r.get("value"),
                error: None,
            })
            .collect())
    }

    /// Replaces the product's whole list in one transaction, so a reader never sees half of it.
    pub async fn set_product_overrides(
        &self,
        product: &str,
        entries: &[ProductOverride],
    ) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let known: Option<i64> = sqlx::query_scalar("SELECT 1 FROM products WHERE id = ?")
            .bind(product)
            .fetch_optional(&mut *tx)
            .await?;
        if known.is_none() {
            return Err(StoreError::ProductGone(product.to_string()));
        }
        sqlx::query("DELETE FROM product_overrides WHERE product = ?")
            .bind(product)
            .execute(&mut *tx)
            .await?;
        for e in entries {
            sqlx::query(
                "INSERT INTO product_overrides (product, section, key, value) VALUES (?, ?, ?, ?)",
            )
            .bind(product)
            .bind(&e.section)
            .bind(&e.key)
            .bind(&e.value)
            .execute(&mut *tx)
            .await
            .map_err(|err| conflict(err, || format!("{}.{}", e.section, e.key)))?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn releases(&self, product: &str) -> Result<Vec<ReleaseRow>> {
        let rows = sqlx::query("SELECT * FROM releases WHERE product = ? ORDER BY start, id")
            .bind(product)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(release_row).collect())
    }

    pub async fn release(&self, id: i64) -> Result<Option<ReleaseRow>> {
        let row = sqlx::query("SELECT * FROM releases WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.as_ref().map(release_row))
    }

    /// `r.id` is ignored; returns the new one. `Conflict` when the product has the version already.
    pub async fn insert_release(&self, r: &ReleaseRow) -> Result<i64> {
        let _w = self.write_lock.lock().await;
        let id = sqlx::query(
            "INSERT INTO releases (product, version, start, target, released_at, created_at) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&r.product)
        .bind(&r.version)
        .bind(&r.start)
        .bind(&r.target)
        .bind(r.released_at)
        .bind(now_secs())
        .execute(&self.pool)
        .await
        .map_err(|e| conflict(e, || format!("version {:?}", r.version)))?
        .last_insert_rowid();
        Ok(id)
    }

    /// Writes the version, dates and release time; the product never changes.
    pub async fn update_release(&self, r: &ReleaseRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "UPDATE releases SET version = ?, start = ?, target = ?, released_at = ? WHERE id = ?",
        )
        .bind(&r.version)
        .bind(&r.start)
        .bind(&r.target)
        .bind(r.released_at)
        .bind(r.id)
        .execute(&self.pool)
        .await
        .map_err(|e| conflict(e, || format!("version {:?}", r.version)))?;
        Ok(())
    }

    /// Deletes the release and moves its tasks to the product's backlog, in one transaction.
    pub async fn delete_release(&self, id: i64) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE tasks SET data = json_set(data, '$.release', NULL) WHERE json_extract(data, '$.release') = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM releases WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn repos(&self, product: &str) -> Result<Vec<RepoRow>> {
        let rows = sqlx::query("SELECT * FROM repos WHERE product = ? ORDER BY added_at, name")
            .bind(product)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(repo_row).collect()
    }

    pub async fn repo(&self, product: &str, name: &str) -> Result<Option<RepoRow>> {
        let row = sqlx::query("SELECT * FROM repos WHERE product = ? AND name = ?")
            .bind(product)
            .bind(name)
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(repo_row).transpose()
    }

    /// Adds the repo, or replaces the path, URL and roster of the product's repo of that name. A row with neither
    /// a path nor a URL is refused by the table.
    pub async fn upsert_repo(&self, r: &RepoRow) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query(
            "INSERT INTO repos (product, name, path, url, roster, added_at) VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (product, name) DO UPDATE SET path = excluded.path, url = excluded.url, roster = excluded.roster",
        )
        .bind(&r.product)
        .bind(&r.name)
        .bind(r.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
        .bind(&r.url)
        .bind(serde_json::to_string(&r.roster)?)
        .bind(now_secs())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_repo(&self, product: &str, name: &str) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("DELETE FROM repos WHERE product = ? AND name = ?")
            .bind(product)
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Stores the specialists the repo offers; the next orchestrated task to start reads them from here.
    pub async fn set_repo_roster(
        &self,
        product: &str,
        name: &str,
        roster: &[String],
    ) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("UPDATE repos SET roster = ? WHERE product = ? AND name = ?")
            .bind(serde_json::to_string(&crate::fleet::unique_roster(roster))?)
            .bind(product)
            .bind(name)
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

    /// False when the persona is gone (deleted while its CLIs were being asked).
    pub async fn set_persona_ready(&self, id: &str) -> Result<bool> {
        let _w = self.write_lock.lock().await;
        let n = sqlx::query("UPDATE personas SET ready = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(n > 0)
    }

    pub async fn delete_persona(&self, id: &str) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("DELETE FROM personas WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Counts and deletes in one transaction, so a task cannot appear between the two. Returns how many
    /// tasks block the delete; 0 means the persona is gone.
    pub async fn delete_persona_if_unused(&self, id: &str) -> Result<u32> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tasks WHERE json_extract(data, '$.persona') = ? AND json_extract(data, '$.state') IN ('running', 'stopped')",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if n == 0 {
            sqlx::query("DELETE FROM personas WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        Ok(n as u32)
    }

    /// Whether any task record, archived ones included, belongs to the persona.
    pub async fn persona_has_records(&self, id: &str) -> Result<bool> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tasks WHERE json_extract(data, '$.persona') = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n > 0)
    }

    /// Task records that still hold something: running or stopped, not archived.
    pub async fn persona_task_count(&self, id: &str) -> Result<u32> {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM tasks WHERE json_extract(data, '$.persona') = ? AND json_extract(data, '$.state') IN ('running', 'stopped')",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(n as u32)
    }

    /// Committed events as they happen. A receiver that falls behind gets `Lagged` and must resync from a snapshot.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.live.subscribe()
    }

    pub async fn insert_agent(&self, a: &Agent, token: &str) -> Result<Event> {
        self.insert_agent_checked(a, token, false, None).await
    }

    /// `insert_agent`, failing with `PersonaGone` when `a.persona` has no row. Checked in the insert's transaction,
    /// under the write lock `delete_persona_if_unused` also takes: either the delete sees this task, or this
    /// insert sees the persona gone.
    pub async fn insert_agent_under_persona(&self, a: &Agent, token: &str) -> Result<Event> {
        self.insert_agent_checked(a, token, true, None).await
    }

    /// `insert_agent_under_persona` for the first agent of a new task, which also starts in `release`. Fails with
    /// `ProductGone` when `a.product` has no row, with `ReleaseGone` when `release` is not one of its releases and
    /// with `Released` when it is already released,
    /// both checked in the insert's transaction under the write lock `delete_product_if_unused` and
    /// `delete_release` take: either the delete sees this task, or this insert sees the product or release gone.
    pub async fn insert_new_task_agent(
        &self,
        a: &Agent,
        token: &str,
        release: Option<i64>,
    ) -> Result<Event> {
        self.insert_agent_checked(a, token, true, Some(release))
            .await
    }

    async fn insert_agent_checked(
        &self,
        a: &Agent,
        token: &str,
        require_persona: bool,
        new_task: Option<Option<i64>>,
    ) -> Result<Event> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        if require_persona {
            let found: Option<i64> = sqlx::query_scalar("SELECT 1 FROM personas WHERE id = ?")
                .bind(&a.persona)
                .fetch_optional(&mut *tx)
                .await?;
            if found.is_none() {
                return Err(StoreError::PersonaGone(a.persona.clone()));
            }
        }
        if let Some(release) = new_task {
            let product: Option<i64> = sqlx::query_scalar("SELECT 1 FROM products WHERE id = ?")
                .bind(&a.product)
                .fetch_optional(&mut *tx)
                .await?;
            if product.is_none() {
                return Err(StoreError::ProductGone(a.product.clone()));
            }
            if let Some(release) = release {
                open_release(&mut tx, release, &a.product).await?;
            }
        }
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
        let joined = match &a.task {
            Some(p) => load_task(&mut tx, p).await?,
            None => None,
        };
        let task = match joined {
            Some(mut p) => {
                p.agents_used += 1;
                let text = if a.kind == AgentKind::Orchestrator {
                    p.lead = Some(a.id.clone());
                    // Every agent had ended: the record runs again with this one.
                    if p.state == TaskState::Stopped {
                        p.state = TaskState::Running;
                        p.stop_cause = None;
                        p.stopped_at = None;
                        "orchestrator restarted; task running again".to_string()
                    } else {
                        "orchestrator restarted".to_string()
                    }
                } else {
                    format!(
                        "{} started for step {}",
                        a.role.as_deref().unwrap_or("specialist"),
                        a.step.unwrap_or(0)
                    )
                };
                p.log.push(LogEntry {
                    at: a.status_since,
                    text,
                });
                p
            }
            None => {
                let mut p = task_of(a, TaskState::Running, Some(a.status_since));
                p.release = new_task.flatten();
                p.log.push(LogEntry {
                    at: a.status_since,
                    text: format!("started with {repos}"),
                });
                p
            }
        };
        save_task(&mut tx, &task).await?;
        let mut shown = a.clone();
        board_view(&mut shown, &task);
        let ev = self
            .append(&mut tx, EventKind::AgentCreated { agent: shown })
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
        let pid = task_id(&new).clone();
        if let Some(mut p) = load_task(&mut tx, &pid).await? {
            let before = p.clone();
            if old.status != new.status {
                // A fleet's clock is its orchestrator's; counting every agent would multiply the time.
                if p.lead.as_ref().unwrap_or(&p.id) == id {
                    let span = (new.status_since - old.status_since).max(0);
                    match old.status {
                        Status::Queued => p.times.queued += span,
                        Status::Working => p.times.working += span,
                        Status::NeedsYou => p.times.needs_you += span,
                        Status::Review => p.times.review += span,
                        Status::Talking | Status::Stopped => {}
                    }
                }
                if new.kind == AgentKind::Specialist {
                    if let Some(ev) = fleet_event(&new) {
                        if ev.kind == FleetEventKind::Failed {
                            // Every version that carries the step running, not only the approved one: a newer
                            // proposal copied its progress and would bring it back as Running once approved.
                            for step in p
                                .breakdowns
                                .iter_mut()
                                .flat_map(|b| b.steps.iter_mut())
                                .filter(|s| s.n == ev.step && s.state == StepState::Running)
                            {
                                step.state = StepState::Failed;
                            }
                        }
                        p.inbox.push(ev);
                    }
                }
                let others_alive: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM agents WHERE id != ? AND (id = ? OR json_extract(data, '$.task') = ?) AND json_extract(data, '$.status') != 'stopped'",
                )
                .bind(&id.0)
                .bind(&pid.0)
                .bind(&pid.0)
                .fetch_one(&mut *tx)
                .await?;
                if new.status == Status::Stopped && others_alive == 0 {
                    p.state = TaskState::Stopped;
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
            // The record's repos are its lead's; a specialist's step worktree is not one of them.
            if old.workspaces != new.workspaces && p.lead.as_ref().unwrap_or(&p.id) == id {
                p.repos = new.workspaces.clone();
            }
            if p != before {
                save_task(&mut tx, &p).await?;
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
            // As the board shows it, or a hook from a lead whose breakdown waits would flip the card back.
            let shown = viewed(&mut tx, new.clone()).await?;
            events.push(self.append(&mut tx, status_changed(&shown)).await?);
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

    /// The agents of a task (its orchestrator and fleet), oldest first; a solo task's is its one agent.
    pub async fn task_agents(&self, task: &AgentId) -> Result<Vec<Agent>> {
        let rows = sqlx::query(
            "SELECT data FROM agents WHERE id = ? OR json_extract(data, '$.task') = ? ORDER BY rowid",
        )
        .bind(&task.0)
        .bind(&task.0)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok(serde_json::from_str(r.get::<&str, _>("data"))?))
            .collect()
    }

    /// The fleet events queued for the orchestrator, left in place until `ack_inbox`, so a delivery that never
    /// completes loses nothing.
    pub async fn peek_inbox(&self, task: &AgentId) -> Result<Vec<FleetEvent>> {
        Ok(self.task(task).await?.map(|p| p.inbox).unwrap_or_default())
    }

    /// Drops the first `n` queued events in one write; events queued since the peek stay.
    pub async fn ack_inbox(&self, task: &AgentId, n: usize) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let Some(mut p) = load_task(&mut tx, task).await? else {
            return Ok(());
        };
        if n == 0 || p.inbox.is_empty() {
            return Ok(());
        }
        p.inbox.drain(..n.min(p.inbox.len()));
        save_task(&mut tx, &p).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Every task record, archived ones included, oldest first; records made on upgrade have no start time and lead.
    pub async fn tasks(&self) -> Result<Vec<Task>> {
        let rows = sqlx::query("SELECT data FROM tasks ORDER BY rowid")
            .fetch_all(&self.pool)
            .await?;
        let mut all = rows
            .iter()
            .map(|r| serde_json::from_str(r.get::<&str, _>("data")))
            .collect::<std::result::Result<Vec<Task>, _>>()?;
        all.sort_by_key(|p| p.created_at);
        Ok(all)
    }

    pub async fn task(&self, id: &AgentId) -> Result<Option<Task>> {
        let mut conn = self.pool.acquire().await?;
        load_task(&mut conn, id).await
    }

    /// Edits the record in place. Stored silently: no event, the board does not show tasks live.
    pub async fn update_task(&self, id: &AgentId, f: impl FnOnce(&mut Task)) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let mut p = load_task(&mut tx, id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        f(&mut p);
        save_task(&mut tx, &p).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Moves the task into `release`, or to the backlog with None, and returns it. The release is checked in the
    /// transaction, under the write lock `delete_release` takes, so it cannot be deleted in between: `ReleaseGone`
    /// when it is not one of the task's product, `Released` when it is already released.
    pub async fn set_task_release(&self, id: &AgentId, release: Option<i64>) -> Result<Task> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let mut p = load_task(&mut tx, id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        let finished = matches!(
            task_outcome(&p),
            TaskOutcome::Merged | TaskOutcome::DoneByHand
        );
        if finished && p.release != release {
            return Err(StoreError::Finished);
        }
        if let Some(release) = release.filter(|r| p.release != Some(*r)) {
            open_release(&mut tx, release, &p.product).await?;
        }
        p.release = release;
        save_task(&mut tx, &p).await?;
        tx.commit().await?;
        Ok(p)
    }

    pub async fn task_log(&self, id: &AgentId, at: i64, text: &str) -> Result<()> {
        self.update_task(id, |p| {
            p.log.push(LogEntry {
                at,
                text: text.to_string(),
            })
        })
        .await
    }

    /// Gives every agent that predates task records one, so the Tasks view lists them. Returns how many
    /// it made; the start time is unknown, so `created_at` stays empty.
    pub async fn backfill_tasks(&self) -> Result<usize> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT a.data FROM agents a LEFT JOIN tasks p ON p.id = a.id WHERE p.id IS NULL ORDER BY a.rowid",
        )
        .fetch_all(&mut *tx)
        .await?;
        for r in &rows {
            let a: Agent = serde_json::from_str(r.get::<&str, _>("data"))?;
            let stopped = a.status == Status::Stopped;
            let mut p = task_of(
                &a,
                if stopped {
                    TaskState::Stopped
                } else {
                    TaskState::Running
                },
                None,
            );
            p.stop_cause = a.stop_cause;
            p.stopped_at = stopped.then_some(a.status_since);
            p.log.push(LogEntry {
                at: now_secs(),
                text: "record created on upgrade".into(),
            });
            save_task(&mut tx, &p).await?;
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

    /// Writes a task's record as archived and removes its cards (its fleet's too) in one transaction, so no card
    /// leaves the board without the record saying why. A record with no card left (an earlier half-written archive)
    /// is still written.
    pub async fn archive_task(&self, id: &AgentId, f: impl FnOnce(&mut Task)) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let mut p = load_task(&mut tx, id)
            .await?
            .ok_or_else(|| StoreError::NotFound(id.clone()))?;
        f(&mut p);
        save_task(&mut tx, &p).await?;
        let cards: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM agents WHERE id = ? OR json_extract(data, '$.task') = ? ORDER BY rowid",
        )
        .bind(&id.0)
        .bind(&id.0)
        .fetch_all(&mut *tx)
        .await?;
        let mut events = Vec::with_capacity(cards.len());
        for card in cards {
            sqlx::query("DELETE FROM agents WHERE id = ?")
                .bind(&card)
                .execute(&mut *tx)
                .await?;
            events.push(
                self.append(&mut tx, EventKind::AgentRemoved { id: AgentId(card) })
                    .await?,
            );
        }
        tx.commit().await?;
        for ev in events {
            let _ = self.live.send(ev);
        }
        Ok(())
    }

    /// Stores a record for something that had none (an archived orphan). An id already taken is an error, never
    /// an overwrite: records are never replaced.
    pub async fn insert_task(&self, p: &Task) -> Result<()> {
        let _w = self.write_lock.lock().await;
        sqlx::query("INSERT INTO tasks (id, data) VALUES (?, ?)")
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

    /// `snapshot` as the board shows it: every agent through `board_view`. For the clients only; the daemon's own
    /// decisions read the stored status.
    pub async fn board(&self) -> Result<Snapshot> {
        let mut tx = self.pool.begin().await?;
        let seq = last_seq(&mut tx).await?;
        let rows = sqlx::query("SELECT data FROM agents ORDER BY rowid")
            .fetch_all(&mut *tx)
            .await?;
        let mut agents = Vec::with_capacity(rows.len());
        for r in &rows {
            agents.push(viewed(&mut tx, serde_json::from_str(r.get::<&str, _>("data"))?).await?);
        }
        Ok(Snapshot { seq, agents })
    }

    /// `a` as the board shows it, for a response that carries an agent outside the snapshot and the stream.
    pub async fn view(&self, a: Agent) -> Result<Agent> {
        let mut conn = self.pool.acquire().await?;
        viewed(&mut conn, a).await
    }

    /// Logs and sends a `StatusChanged` carrying the agent as the board shows it now: for a change to what the board
    /// derives (a decided breakdown) that no write to the agent made. An agent gone since is not an error.
    pub async fn announce(&self, id: &AgentId) -> Result<()> {
        let _w = self.write_lock.lock().await;
        let mut tx = self.pool.begin().await?;
        let Some(row) = sqlx::query("SELECT data FROM agents WHERE id = ?")
            .bind(&id.0)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(());
        };
        let shown = viewed(&mut tx, serde_json::from_str(row.get::<&str, _>("data"))?).await?;
        let ev = self.append(&mut tx, status_changed(&shown)).await?;
        tx.commit().await?;
        let _ = self.live.send(ev);
        Ok(())
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

/// What the orchestrator should hear about a specialist whose status just changed, if anything.
fn fleet_event(a: &Agent) -> Option<FleetEvent> {
    let (kind, detail) = match a.status {
        Status::NeedsYou => (FleetEventKind::NeedsYou, a.activity.clone()),
        Status::Review => (FleetEventKind::IdleWithoutFinish, String::new()),
        Status::Stopped if a.stop_cause == Some(StopCause::Finished) => {
            (FleetEventKind::Finished, String::new())
        }
        Status::Stopped => {
            let cause = match a.stop_cause {
                Some(StopCause::Crashed) => "crashed",
                Some(StopCause::Exited) => "exited",
                _ => "stopped",
            };
            (FleetEventKind::Failed, cause.to_string())
        }
        Status::Queued | Status::Working | Status::Talking => return None,
    };
    Some(FleetEvent {
        at: a.status_since,
        step: a.step.unwrap_or(0),
        agent: a.id.clone(),
        kind,
        detail,
    })
}

fn task_of(a: &Agent, state: TaskState, created_at: Option<i64>) -> Task {
    Task {
        id: a.id.clone(),
        product: a.product.clone(),
        release: None,
        name: std::path::Path::new(&a.workdir)
            .file_name()
            .map_or_else(|| a.workdir.clone(), |n| n.to_string_lossy().into_owned()),
        goal: a.goal.clone(),
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
        prs: vec![],
        done_at: None,
        done_by_hand: false,
        inbox: vec![],
        kind: fleet_api::AgentKind::Solo,
        lead: None,
        role: None,
        roster: vec![],
    }
}

/// What the board shows for `a`: a running task lead whose task's last breakdown version waits for the owner
/// is on Needs you for it, whatever its session reports. Derived, never stored, so the hooks of a lead that took over
/// from the one that proposed cannot clear it. A prompt of the lead's own (stored Needs you) keeps the card, as in
/// `ingest`: the owner has to answer it in the session, and the breakdown is still in the side panel.
pub fn board_view(a: &mut Agent, p: &Task) {
    if a.kind != AgentKind::Orchestrator
        || matches!(a.status, Status::Stopped | Status::NeedsYou)
        || p.lead.as_ref().unwrap_or(&p.id) != &a.id
    {
        return;
    }
    if p.breakdowns
        .last()
        .is_some_and(|v| v.approval == Approval::Proposed)
    {
        a.status = Status::NeedsYou;
        a.blocked_reason = Some(BlockedReason::Breakdown);
    }
}

/// `board_view` with the agent's task read on `conn`, so a write sees its own transaction.
async fn viewed(conn: &mut sqlx::SqliteConnection, mut a: Agent) -> Result<Agent> {
    if a.kind == AgentKind::Orchestrator {
        if let Some(p) = load_task(conn, task_id(&a)).await? {
            board_view(&mut a, &p);
        }
    }
    Ok(a)
}

fn status_changed(a: &Agent) -> EventKind {
    EventKind::StatusChanged {
        id: a.id.clone(),
        status: a.status,
        activity: a.activity.clone(),
        since: a.status_since,
        has_changes: a.has_changes,
        paused: a.paused,
        blocked_reason: a.blocked_reason,
        repo_request: a.repo_request.clone(),
        stop_cause: a.stop_cause,
        disk_bytes: a.disk_bytes,
    }
}

async fn load_task(conn: &mut sqlx::SqliteConnection, id: &AgentId) -> Result<Option<Task>> {
    let row = sqlx::query("SELECT data FROM tasks WHERE id = ?")
        .bind(&id.0)
        .fetch_optional(&mut *conn)
        .await?;
    Ok(row
        .map(|r| serde_json::from_str(r.get::<&str, _>("data")))
        .transpose()?)
}

async fn save_task(conn: &mut sqlx::SqliteConnection, p: &Task) -> Result<()> {
    sqlx::query("INSERT INTO tasks (id, data) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data")
        .bind(&p.id.0)
        .bind(serde_json::to_string(p)?)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Checks, in the caller's transaction, that `release` is one of `product`'s and not released yet.
async fn open_release(tx: &mut sqlx::SqliteConnection, release: i64, product: &str) -> Result<()> {
    let row: Option<(String, String, Option<i64>)> =
        sqlx::query_as("SELECT product, version, released_at FROM releases WHERE id = ?")
            .bind(release)
            .fetch_optional(&mut *tx)
            .await?;
    match row {
        Some((theirs, _, _)) if theirs != product => Err(StoreError::ReleaseGone {
            release,
            product: product.to_string(),
        }),
        None => Err(StoreError::ReleaseGone {
            release,
            product: product.to_string(),
        }),
        Some((_, version, Some(_))) => Err(StoreError::Released(version)),
        Some(_) => Ok(()),
    }
}
