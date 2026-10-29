//! Products, their releases and repos: the rules behind the product routes.
use crate::agents::Clock;
use crate::internal::{Internal, InternalError};
use crate::personas::{persona_id, PersonaEnv, PersonaError, Personas};
use crate::repos::{check_clone_url, check_host, RepoError, Repos};
use crate::store::{ProductRow, ReleaseRow, RepoRow, Store, StoreError};
use fleet_api::{
    parse_date, AgentId, NewProduct, NewRelease, ProductUpdate, ReleaseUpdate, Task, TaskState,
};
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Debug, thiserror::Error)]
pub enum ProductError {
    #[error("no such product: {0}")]
    NotFound(String),
    #[error("no such release: {0}")]
    NoRelease(i64),
    #[error("no repo named `{0}`")]
    NoRepo(String),
    #[error("no such task")]
    NoTask,
    #[error("the product needs a name")]
    BadName,
    #[error("no such persona: {0}")]
    NoPersona(String),
    #[error("the release needs a version")]
    BadVersion,
    #[error("{0:?} is not a date; use YYYY-MM-DD")]
    BadDate(String),
    #[error("the target date is before the start date")]
    TargetBeforeStart,
    #[error("release {release} is not a release of {product}")]
    OtherProductRelease { release: i64, product: String },
    #[error("the product still has {0} task record(s), archived ones included")]
    HasTasks(u32),
    #[error("a running task still uses {0}")]
    RepoInUse(String),
    #[error("{0} is already taken")]
    Conflict(String),
    #[error("a finished task stays in the release it finished in")]
    Finished,
    #[error("release {0} is already released")]
    Released(String),
    #[error("an internal repo URL needs a default persona")]
    InternalNeedsPersona,
    #[error("tasks still have notes in the internal repo: {}", .0.join(", "))]
    InternalInUse(Vec<String>),
    #[error(transparent)]
    Internal(#[from] InternalError),
    #[error(transparent)]
    Persona(#[from] PersonaError),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Store(StoreError),
}

impl From<StoreError> for ProductError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Conflict(what) => ProductError::Conflict(what),
            StoreError::ReleaseGone { release, product } => {
                ProductError::OtherProductRelease { release, product }
            }
            StoreError::NotFound(_) => ProductError::NoTask,
            StoreError::Finished => ProductError::Finished,
            StoreError::Released(v) => ProductError::Released(v),
            e => ProductError::Store(e),
        }
    }
}

type Result<T> = std::result::Result<T, ProductError>;

pub struct Products {
    store: Arc<Store>,
    repos: Arc<Repos>,
    clock: Arc<dyn Clock>,
    internal: Arc<Internal>,
    personas: Arc<Personas>,
}

impl Products {
    pub fn new(
        store: Arc<Store>,
        repos: Arc<Repos>,
        clock: Arc<dyn Clock>,
        internal: Arc<Internal>,
        personas: Arc<Personas>,
    ) -> Products {
        Products {
            store,
            repos,
            clock,
            internal,
            personas,
        }
    }

    async fn known(&self, id: &str) -> Result<ProductRow> {
        self.store
            .product(id)
            .await?
            .ok_or_else(|| ProductError::NotFound(id.to_string()))
    }

    async fn known_persona(&self, id: &str) -> Result<()> {
        match self.store.persona(id).await? {
            Some(_) => Ok(()),
            None => Err(ProductError::NoPersona(id.to_string())),
        }
    }

    /// Everything that can refuse `url` short of the clone itself; the env is the persona's login for the clone.
    async fn check_internal_url(
        &self,
        product: &str,
        persona: &str,
        url: &str,
    ) -> Result<PersonaEnv> {
        check_clone_url(url)?;
        let env = self.personas.gh_env(persona).await?;
        check_host(url, &env)?;
        self.check_internal_free(product).await?;
        Ok(env)
    }

    /// Clones `url` into the product's internal repo; nothing changes when it fails.
    async fn connect_internal(&self, product: &str, url: &str, env: &PersonaEnv) -> Result<()> {
        if let Some(aside) = self
            .internal
            .connect(product, url, &env.vars, self.clock.now())
            .await?
        {
            info!(product, aside = %aside.display(), "moved the local internal repo aside");
        }
        Ok(())
    }

    /// The URL can change only while no live task keeps notes on a `task/<name>` branch: the clone replaces them.
    async fn check_internal_free(&self, product: &str) -> Result<()> {
        let mut holding = Vec::new();
        for t in self.store.tasks().await? {
            if t.product == product
                && t.state != TaskState::Archived
                && self
                    .internal
                    .branch_exists(product, &Internal::task_branch(&t.name))
                    .await?
            {
                holding.push(t.name);
            }
        }
        if holding.is_empty() {
            Ok(())
        } else {
            Err(ProductError::InternalInUse(holding))
        }
    }

    /// The id is the name's slug, suffixed `-2`, `-3`... while another product has it.
    pub async fn create(&self, req: NewProduct) -> Result<ProductRow> {
        let name = req.name.trim();
        if name.is_empty() {
            return Err(ProductError::BadName);
        }
        if let Some(p) = &req.default_persona {
            self.known_persona(p).await?;
        }
        let taken: Vec<String> = self
            .store
            .products()
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let base = persona_id(name);
        let id = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|id| !taken.contains(id))
            .expect("an endless range has a free id");
        let row = ProductRow {
            id,
            name: name.to_string(),
            default_persona: req.default_persona,
            internal_url: req.internal_url,
            created_at: self.clock.now(),
        };
        if let Some(url) = &row.internal_url {
            let persona = row
                .default_persona
                .as_deref()
                .ok_or(ProductError::InternalNeedsPersona)?;
            let env = self.check_internal_url(&row.id, persona, url).await?;
            self.connect_internal(&row.id, url, &env).await?;
            if let Err(e) = self.store.insert_product(&row).await {
                if let Err(e) = self.internal.delete(&row.id).await {
                    warn!(product = %row.id, error = %e, "cannot remove the clone of a product that was not stored");
                }
                return Err(e.into());
            }
            return Ok(row);
        }
        self.store.insert_product(&row).await?;
        // The row is stored, so an error here would make a retry create a second product. The first task of the
        // product makes the repo again.
        if let Err(e) = self.internal.ensure(&row.id).await {
            warn!(product = %row.id, error = %e, "cannot create the product's internal repo");
        }
        Ok(row)
    }

    pub async fn update(&self, id: &str, u: ProductUpdate) -> Result<ProductRow> {
        let mut row = self.known(id).await?;
        if let Some(name) = u.name {
            let name = name.trim();
            if name.is_empty() {
                return Err(ProductError::BadName);
            }
            row.name = name.to_string();
        }
        let old_persona = row.default_persona.clone();
        if let Some(persona) = u.default_persona {
            if let Some(p) = &persona {
                self.known_persona(p).await?;
            }
            row.default_persona = persona;
        }
        let old_url = row.internal_url.clone();
        if let Some(url) = u.internal_url {
            row.internal_url = url;
        }
        if row.internal_url.is_some() && row.default_persona.is_none() {
            return Err(ProductError::InternalNeedsPersona);
        }
        if self
            .store
            .products()
            .await?
            .iter()
            .any(|p| p.id != row.id && p.name == row.name)
        {
            return Err(ProductError::Conflict(format!(
                "product name {:?}",
                row.name
            )));
        }
        // The connect or disconnect is not undone when the store then fails, so it comes after every refusal.
        if row.internal_url != old_url {
            match (&row.internal_url, &row.default_persona) {
                (Some(url), Some(persona)) => {
                    let env = self.check_internal_url(&row.id, persona, url).await?;
                    self.connect_internal(&row.id, url, &env).await?;
                }
                _ => {
                    self.check_internal_free(&row.id).await?;
                    self.internal.disconnect(&row.id).await?;
                }
            }
        } else if row.default_persona != old_persona {
            // Every push goes out under the default persona, so the new one must log in to the URL's host.
            if let (Some(url), Some(persona)) = (&row.internal_url, &row.default_persona) {
                check_host(url, &self.personas.gh_env(persona).await?)?;
            }
        }
        self.store.update_product(&row).await?;
        Ok(row)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.known(id).await?;
        match self.store.delete_product_if_unused(id).await? {
            0 => {
                // The row is gone, so an error here would leave a retry with a 404 and the repo orphaned anyway.
                if let Err(e) = self.internal.delete(id).await {
                    warn!(product = id, error = %e, "cannot delete the product's internal repo");
                }
                Ok(())
            }
            n => Err(ProductError::HasTasks(n)),
        }
    }

    pub async fn upsert_repo(
        &self,
        product: &str,
        name: &str,
        path: Option<&str>,
        url: Option<&str>,
    ) -> Result<RepoRow> {
        self.known(product).await?;
        Ok(self.repos.upsert(product, name, path, url).await?)
    }

    pub async fn delete_repo(&self, product: &str, name: &str) -> Result<()> {
        self.known(product).await?;
        if self.store.repo(product, name).await?.is_none() {
            return Err(ProductError::NoRepo(name.to_string()));
        }
        let in_use = self.store.tasks().await?.iter().any(|t| {
            t.state == TaskState::Running
                && t.product == product
                && t.repos.iter().any(|w| w.repo == name)
        });
        if in_use {
            return Err(ProductError::RepoInUse(name.to_string()));
        }
        self.store.delete_repo(product, name).await?;
        Ok(())
    }

    pub async fn create_release(&self, product: &str, req: NewRelease) -> Result<ReleaseRow> {
        self.known(product).await?;
        let version = req.version.trim();
        if version.is_empty() {
            return Err(ProductError::BadVersion);
        }
        check_span(&req.start, &req.target)?;
        let mut row = ReleaseRow {
            id: 0,
            product: product.to_string(),
            version: version.to_string(),
            start: req.start,
            target: req.target,
            released_at: None,
        };
        row.id = self.store.insert_release(&row).await?;
        Ok(row)
    }

    /// `released: true` stamps the daemon's clock, unless the release already carries a stamp; `false` clears it.
    pub async fn update_release(&self, id: i64, u: ReleaseUpdate) -> Result<ReleaseRow> {
        let mut row = self
            .store
            .release(id)
            .await?
            .ok_or(ProductError::NoRelease(id))?;
        if let Some(v) = u.version {
            let v = v.trim();
            if v.is_empty() {
                return Err(ProductError::BadVersion);
            }
            row.version = v.to_string();
        }
        row.start = u.start.unwrap_or(row.start);
        row.target = u.target.unwrap_or(row.target);
        check_span(&row.start, &row.target)?;
        match u.released {
            Some(true) => row.released_at = row.released_at.or(Some(self.clock.now())),
            Some(false) => row.released_at = None,
            None => {}
        }
        self.store.update_release(&row).await?;
        Ok(row)
    }

    pub async fn delete_release(&self, id: i64) -> Result<()> {
        if self.store.release(id).await?.is_none() {
            return Err(ProductError::NoRelease(id));
        }
        self.store.delete_release(id).await?;
        Ok(())
    }

    pub async fn set_task_release(&self, task: &AgentId, release: Option<i64>) -> Result<Task> {
        Ok(self.store.set_task_release(task, release).await?)
    }
}

fn check_span(start: &str, target: &str) -> Result<()> {
    let day = |s: &str| parse_date(s).ok_or_else(|| ProductError::BadDate(s.to_string()));
    if day(target)? < day(start)? {
        return Err(ProductError::TargetBeforeStart);
    }
    Ok(())
}
