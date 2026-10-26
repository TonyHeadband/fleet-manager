use crate::RepoInfo;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utoipa::ToSchema;

/// A version of a product with a calendar span. Dates are `YYYY-MM-DD`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Release {
    #[ts(type = "number")]
    pub id: i64,
    pub product: String,
    pub version: String,
    pub start: String,
    pub target: String,
    /// When the user marked it released; None while it is open.
    #[ts(type = "number | null")]
    pub released_at: Option<i64>,
}

/// Something long-running that holds releases, repos and tasks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct Product {
    pub id: String,
    pub name: String,
    /// The persona New task preselects.
    pub default_persona: Option<String>,
    pub repos: Vec<RepoInfo>,
    pub releases: Vec<Release>,
    /// How many task records belong to the product, archived ones included.
    pub tasks: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ProductList {
    pub products: Vec<Product>,
}
