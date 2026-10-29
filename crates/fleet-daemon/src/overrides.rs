//! A product's sparse overrides of the global settings, and how a value is resolved through them. Rows are stored
//! as section/key/value text; this module is the one place that knows what each section accepts.
use crate::config::{GateBuild, LimitValues, PrSettings};
use crate::fleet::defs::{valid_model, Defs};
use fleet_api::{
    DefKind, EffectiveSettings, EffectiveValue, Effort, PermissionMode, ProductOverride,
    SettingSource,
};

pub const SECTIONS: [&str; 5] = ["models", "modes", "efforts", "limits", "pr"];
pub const PR_KEYS: [&str; 5] = ["style", "attribution", "draft", "labels", "gate_build"];
/// The `LimitValues` fields, in the order `resolve_limits` reports their sources.
pub const LIMIT_KEYS: [&str; 4] = [
    "max_breakdown_versions",
    "max_steps",
    "max_parallel_specialists",
    "max_pr_attempts",
];

/// The definition names an entry may refer to.
struct Targets {
    /// What a `models`, `modes` or `efforts` key may name.
    models: Vec<String>,
    /// What `pr.style` may name.
    pr_styles: Vec<String>,
}

fn targets(defs: &Defs) -> Targets {
    let named = |kinds: &[DefKind]| {
        defs.list()
            .into_iter()
            .filter(|d| kinds.contains(&d.kind))
            .map(|d| d.name)
            .collect()
    };
    Targets {
        models: named(&[DefKind::Orchestrator, DefKind::Specialist]),
        pr_styles: named(&[DefKind::PrStyle]),
    }
}

fn positive(value: &str) -> Option<u32> {
    value.parse::<u32>().ok().filter(|n| *n > 0)
}

/// Why one entry cannot apply, or None.
fn entry_error(e: &ProductOverride, known: &Targets) -> Option<String> {
    let name = format!("{}.{}", e.section, e.key);
    match e.section.as_str() {
        "models" => {
            if !known.models.contains(&e.key) {
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
        "modes" | "efforts" if !known.models.contains(&e.key) => Some(format!(
            "{name}: no orchestrator or specialist named `{}`",
            e.key
        )),
        "modes" => e
            .value
            .parse::<PermissionMode>()
            .err()
            .map(|err| format!("{name}: {err}")),
        "efforts" => e
            .value
            .parse::<Effort>()
            .err()
            .map(|err| format!("{name}: {err}")),
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
        "pr" => match e.key.as_str() {
            "style" if !known.pr_styles.contains(&e.value) => {
                Some(format!("{name}: no PR style named `{}`", e.value))
            }
            "draft" if !matches!(e.value.as_str(), "true" | "false") => {
                Some(format!("{name}: must be true or false"))
            }
            "attribution" if e.value.contains(['\n', '\r']) => {
                Some(format!("{name}: must be one line"))
            }
            "gate_build" if e.value.parse::<GateBuild>().is_err() => {
                Some(format!("{name}: must be shared or clean"))
            }
            "style" | "draft" | "attribution" | "labels" | "gate_build" => None,
            _ => Some(format!(
                "{name}: unknown key (one of {})",
                PR_KEYS.join(", ")
            )),
        },
        _ => Some(format!(
            "{name}: unknown section `{}` (one of {})",
            e.section,
            SECTIONS.join(", ")
        )),
    }
}

/// The first entry that cannot apply, named in the message; a section/key given twice counts too.
pub fn validate(entries: &[ProductOverride], defs: &Defs) -> Result<(), String> {
    let known = targets(defs);
    for (i, e) in entries.iter().enumerate() {
        if let Some(msg) = entry_error(e, &known) {
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
    let known = targets(defs);
    entries
        .into_iter()
        .map(|mut e| {
            e.error = entry_error(&e, &known);
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

/// The limits that apply to the product, and where each of the four came from. A global value equal to the
/// built-in default reads as `Default`: the config cannot tell "left out" from "set to the default".
pub fn resolve_limits(
    global: &LimitValues,
    entries: &[ProductOverride],
) -> (LimitValues, [SettingSource; 4]) {
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
    let (pr_attempts, s3) = pick(
        LIMIT_KEYS[3],
        global.max_pr_attempts,
        defaults.max_pr_attempts,
    );
    (
        LimitValues {
            max_breakdown_versions: versions,
            max_steps: steps,
            max_parallel_specialists: parallel,
            max_pr_attempts: pr_attempts,
        },
        [s0, s1, s2, s3],
    )
}

/// The PR settings that apply to the product, and where the style came from. An entry that does not parse is
/// ignored, as in `resolve_limits`.
pub fn resolve_pr(global: &PrSettings, entries: &[ProductOverride]) -> (PrSettings, SettingSource) {
    let style = entry(entries, "pr", "style").filter(|n| !n.trim().is_empty());
    let source = match style {
        Some(_) => SettingSource::Product,
        None if global.style != PrSettings::default().style => SettingSource::Global,
        None => SettingSource::Default,
    };
    let pr = PrSettings {
        style: style.map_or_else(|| global.style.clone(), str::to_string),
        attribution: entry(entries, "pr", "attribution")
            .map_or_else(|| global.attribution.clone(), str::to_string),
        draft: match entry(entries, "pr", "draft") {
            Some("true") => true,
            Some("false") => false,
            _ => global.draft,
        },
        labels: entry(entries, "pr", "labels").map_or_else(
            || global.labels.clone(),
            |v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect()
            },
        ),
        gate_build: entry(entries, "pr", "gate_build")
            .and_then(|v| v.parse().ok())
            .unwrap_or(global.gate_build),
    };
    (pr, source)
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

/// The permission mode a definition's agents run in: the product's override, else the definition's own, else the
/// global default. An entry that does not parse is ignored, as in `resolve_model`.
pub fn resolve_mode(
    name: &str,
    def: Option<PermissionMode>,
    global: PermissionMode,
    entries: &[ProductOverride],
) -> (PermissionMode, SettingSource) {
    match entry(entries, "modes", name).and_then(|v| v.parse().ok()) {
        Some(mode) => (mode, SettingSource::Product),
        None => def.map_or((global, SettingSource::Global), |m| {
            (m, SettingSource::Definition)
        }),
    }
}

/// The effort a definition's agents run at; resolved like `resolve_mode`.
pub fn resolve_effort(
    name: &str,
    def: Option<Effort>,
    global: Effort,
    entries: &[ProductOverride],
) -> (Effort, SettingSource) {
    match entry(entries, "efforts", name).and_then(|v| v.parse().ok()) {
        Some(effort) => (effort, SettingSource::Product),
        None => def.map_or((global, SettingSource::Global), |e| {
            (e, SettingSource::Definition)
        }),
    }
}

/// Every limit, then `models`, `modes` and `efforts` rows per orchestrator and specialist definition. An override
/// naming a definition that is gone has no row: there is nothing for it to apply to.
pub fn effective(
    global: &LimitValues,
    global_mode: PermissionMode,
    global_effort: Effort,
    defs: &Defs,
    entries: &[ProductOverride],
) -> EffectiveSettings {
    let (limits, sources) = resolve_limits(global, entries);
    let numbers = [
        limits.max_breakdown_versions,
        limits.max_steps,
        limits.max_parallel_specialists,
        limits.max_pr_attempts,
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
            key: d.name.clone(),
            value,
            source,
        });
        let (mode, source) = resolve_mode(&d.name, d.mode, global_mode, entries);
        values.push(EffectiveValue {
            section: "modes".into(),
            key: d.name.clone(),
            value: Some(mode.as_flag().into()),
            source,
        });
        let (effort, source) = resolve_effort(&d.name, d.effort, global_effort, entries);
        values.push(EffectiveValue {
            section: "efforts".into(),
            key: d.name,
            value: Some(effort.as_flag().into()),
            source,
        });
    }
    EffectiveSettings { values }
}
