//! Products, their releases and repos: the rules behind the product routes.
use crate::agents::Clock;
use crate::internal::{Internal, InternalError};
use crate::personas::persona_id;
use crate::repos::{RepoError, Repos};
use crate::store::{ProductRow, ReleaseRow, RepoRow, Store, StoreError};
use fleet_api::{
    parse_date, AgentId, NewProduct, NewRelease, ProductUpdate, ReleaseUpdate, Task, TaskState,
};
use std::sync::Arc;

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
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Store(StoreError),
    #[error(transparent)]
    Internal(#[from] InternalError),
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
}

impl Products {
    pub fn new(
        store: Arc<Store>,
        repos: Arc<Repos>,
        clock: Arc<dyn Clock>,
        internal: Arc<Internal>,
    ) -> Products {
        Products {
            store,
            repos,
            clock,
            internal,
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
            created_at: self.clock.now(),
        };
        self.store.insert_product(&row).await?;
        // Not undone if it fails: the first task of the product makes it again.
        self.internal.ensure(&row.id).await?;
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
        if let Some(persona) = u.default_persona {
            if let Some(p) = &persona {
                self.known_persona(p).await?;
            }
            row.default_persona = persona;
        }
        self.store.update_product(&row).await?;
        Ok(row)
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        self.known(id).await?;
        match self.store.delete_product_if_unused(id).await? {
            0 => Ok(self.internal.delete(id).await?),
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
