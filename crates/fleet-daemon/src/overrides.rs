//! A product's sparse overrides of the global settings, and how a value is resolved through them. Rows are stored
//! as section/key/value text; this module is the one place that knows what each section accepts.
use crate::config::LimitValues;
use crate::fleet::defs::{valid_model, Defs};
use fleet_api::{DefKind, EffectiveSettings, EffectiveValue, ProductOverride, SettingSource};

pub const SECTIONS: [&str; 2] = ["models", "limits"];
/// The `LimitValues` fields, in the order `resolve_limits` reports their sources.
pub const LIMIT_KEYS: [&str; 3] = [
    "max_breakdown_versions",
    "max_steps",
    "max_parallel_specialists",
];

/// Orchestrator and specialist names: what a `models` key may name.
fn model_targets(defs: &Defs) -> Vec<String> {
    defs.list()
        .into_iter()
        .filter(|d| matches!(d.kind, DefKind::Orchestrator | DefKind::Specialist))
        .map(|d| d.name)
        .collect()
}

fn positive(value: &str) -> Option<u32> {
    value.parse::<u32>().ok().filter(|n| *n > 0)
}

/// Why one entry cannot apply, or None. `targets` are the definition names a `models` key may name.
fn entry_error(e: &ProductOverride, targets: &[String]) -> Option<String> {
    let name = format!("{}.{}", e.section, e.key);
    match e.section.as_str() {
        "models" => {
            if !targets.contains(&e.key) {
                Some(format!(
                    "{name}: no orchestrator or specialist named `{}`",
                    e.key
                ))
            } else if !valid_model(&e.value) {
                Some(format!(
                    "{name}: model must be a name such as sonnet or claude-sonnet-5-5"
                ))
            } else {
                None
            }
        }
        "limits" => {
            if !LIMIT_KEYS.contains(&e.key.as_str()) {
                Some(format!(
                    "{name}: unknown limit (one of {})",
                    LIMIT_KEYS.join(", ")
                ))
            } else if positive(&e.value).is_none() {
                Some(format!("{name}: must be a whole number above 0"))
            } else {
                None
            }
        }
        _ => Some(format!(
            "{name}: unknown section `{}` (one of {})",
            e.section,
            SECTIONS.join(", ")
        )),
    }
}

/// The first entry that cannot apply, named in the message; a section/key given twice counts too.
pub fn validate(entries: &[ProductOverride], defs: &Defs) -> Result<(), String> {
    let targets = model_targets(defs);
    for (i, e) in entries.iter().enumerate() {
        if let Some(msg) = entry_error(e, &targets) {
            return Err(msg);
        }
        if entries[..i]
            .iter()
            .any(|p| p.section == e.section && p.key == e.key)
        {
            return Err(format!("{}.{}: given twice", e.section, e.key));
        }
    }
    Ok(())
}

/// `entries` with `error` set on each one `validate` would refuse, for a read. A stored entry goes stale when the
/// definition it names is deleted, so it is listed, not hidden.
pub fn annotate(entries: Vec<ProductOverride>, defs: &Defs) -> Vec<ProductOverride> {
    let targets = model_targets(defs);
    entries
        .into_iter()
        .map(|mut e| {
            e.error = entry_error(&e, &targets);
            e
        })
        .collect()
}

fn entry<'a>(entries: &'a [ProductOverride], section: &str, key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|e| e.section == section && e.key == key)
        .map(|e| e.value.as_str())
}

/// The limits that apply to the product, and where each of the three came from. A global value equal to the
/// built-in default reads as `Default`: the config cannot tell "left out" from "set to the default".
pub fn resolve_limits(
    global: &LimitValues,
    entries: &[ProductOverride],
) -> (LimitValues, [SettingSource; 3]) {
    let defaults = LimitValues::default();
    let pick = |key: &str, global: u32, default: u32| match entry(entries, "limits", key)
        .and_then(positive)
    {
        Some(n) => (n, SettingSource::Product),
        None if global != default => (global, SettingSource::Global),
        None => (global, SettingSource::Default),
    };
    let (versions, s0) = pick(
        LIMIT_KEYS[0],
        global.max_breakdown_versions,
        defaults.max_breakdown_versions,
    );
    let (steps, s1) = pick(LIMIT_KEYS[1], global.max_steps, defaults.max_steps);
    let (parallel, s2) = pick(
        LIMIT_KEYS[2],
        global.max_parallel_specialists,
        defaults.max_parallel_specialists,
    );
    (
        LimitValues {
            max_breakdown_versions: versions,
            max_steps: steps,
            max_parallel_specialists: parallel,
        },
        [s0, s1, s2],
    )
}

/// The model a definition's agents run on: the product's override, else the definition's own, else None (Claude's
/// default).
pub fn resolve_model(
    name: &str,
    def_model: Option<&str>,
    entries: &[ProductOverride],
) -> Option<String> {
    entry(entries, "models", name)
        .filter(|m| valid_model(m))
        .or(def_model)
        .map(str::to_string)
}

/// Every limit, then one `models` row per orchestrator and specialist definition. An override naming a
/// definition that is gone has no row: there is nothing for it to apply to.
pub fn effective(
    global: &LimitValues,
    defs: &Defs,
    entries: &[ProductOverride],
) -> EffectiveSettings {
    let (limits, sources) = resolve_limits(global, entries);
    let numbers = [
        limits.max_breakdown_versions,
        limits.max_steps,
        limits.max_parallel_specialists,
    ];
    let mut values: Vec<EffectiveValue> = LIMIT_KEYS
        .iter()
        .zip(numbers)
        .zip(sources)
        .map(|((key, n), source)| EffectiveValue {
            section: "limits".into(),
            key: (*key).into(),
            value: Some(n.to_string()),
            source,
        })
        .collect();
    for d in defs
        .list()
        .into_iter()
        .filter(|d| matches!(d.kind, DefKind::Orchestrator | DefKind::Specialist))
    {
        let value = resolve_model(&d.name, d.model.as_deref(), entries);
        let source = if entry(entries, "models", &d.name).is_some_and(valid_model) {
            SettingSource::Product
        } else if d.model.is_some() {
            SettingSource::Definition
        } else {
            SettingSource::Default
        };
        values.push(EffectiveValue {
            section: "models".into(),
            key: d.name,
            value,
            source,
        });
    }
    EffectiveSettings { values }
}
