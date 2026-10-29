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
    /// The git remote of the product's internal repo, credentials redacted; None keeps the repo local-only.
    #[serde(default)]
    pub internal_url: Option<String>,
    pub repos: Vec<RepoInfo>,
    pub releases: Vec<Release>,
    /// How many task records belong to the product, archived ones included.
    pub tasks: u32,
    /// Settings this product sets for itself; every other setting follows the global one.
    #[serde(default)]
    pub overrides: Vec<ProductOverride>,
}

/// One setting a product overrides: `models.<definition>` or `limits.<max_*>`, with the value as text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ProductOverride {
    pub section: String,
    pub key: String,
    pub value: String,
    /// Why the entry no longer applies (its definition is gone, say). Filled on reads, ignored on PUT.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub error: Option<String>,
}

/// The whole override list of a product: it replaces what is stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct OverridesPut {
    pub overrides: Vec<ProductOverride>,
}

/// Which layer a setting's effective value came from, nearest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SettingSource {
    Product,
    Global,
    Definition,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct EffectiveValue {
    pub section: String,
    pub key: String,
    /// None for a model nothing names: the agent runs on Claude's own default.
    pub value: Option<String>,
    pub source: SettingSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct EffectiveSettings {
    pub values: Vec<EffectiveValue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ProductList {
    pub products: Vec<Product>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct NewProduct {
    pub name: String,
    #[serde(default)]
    pub default_persona: Option<String>,
    /// Needs `default_persona`: the clone runs under that persona's login.
    #[serde(default)]
    pub internal_url: Option<String>,
}

/// An absent field is left alone. `default_persona: null` and `internal_url: null` clear them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ProductUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "some_even_if_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    #[ts(type = "string | null", optional)]
    pub default_persona: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "some_even_if_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[schema(value_type = Option<String>)]
    #[ts(type = "string | null", optional)]
    pub internal_url: Option<Option<String>>,
}

/// Tells an explicit `null` (`Some(None)`) from a missing field (`None`), which serde's default does not.
fn some_even_if_null<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct NewRelease {
    pub version: String,
    pub start: String,
    pub target: String,
}

/// An absent field is left alone; `released` stamps or clears `released_at`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct ReleaseUpdate {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub released: Option<bool>,
}

/// Adds a repo to a product, or fills in the path or URL a repo lacks. At least one of the two.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct RepoUpsert {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// None moves the task to its product's backlog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, TS)]
#[ts(export)]
pub struct SetTaskRelease {
    #[serde(default)]
    #[ts(type = "number | null")]
    pub release: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_update_tells_null_from_absent_across_the_wire() {
        let absent: ProductUpdate = serde_json::from_str(r#"{"name":"x"}"#).unwrap();
        assert_eq!(absent.default_persona, None);
        let null: ProductUpdate = serde_json::from_str(r#"{"default_persona":null}"#).unwrap();
        assert_eq!(null.default_persona, Some(None));
        let round: ProductUpdate =
            serde_json::from_str(&serde_json::to_string(&absent).unwrap()).unwrap();
        assert_eq!(round, absent);
    }
}
