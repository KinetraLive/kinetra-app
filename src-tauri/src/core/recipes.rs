//! Recipes: built-in manifests (`presets/recipes/<id>/recipe.json`, embedded like presets) that
//! install a source, templates (from presets), layers on a screen and flows into a project,
//! recorded in `<project>/recipes/<instanceId>.json` (`{ schemaVersion: 2, ...RecipeInstance }`
//! with `flowIds`, atomic writes, part of the backup config set). A manifest describes its
//! automation as rules ("When … then …"); installing converts each into a flow
//! (`flows::rule_to_flow`: deployed, disarmed as the manifest says, `origin` = the instance).
//! `schemaVersion: 1` instance files (with `rules`) are migrated to flows on open
//! (`core::flows`).
//!
//! # Manifest
//! ```json
//! { "schemaVersion": 1, "id", "name", "description", "panel": "slideshow|schedule|countdown",
//!   "version": 1,
//!   "source": { "kind": "schedule", "name", "sample": "around-now|single|empty",
//!               "ripple"?, "labels"? }
//!           | { "kind": "text", "name", "columns": [SourceColumn], "rows": [Fields],
//!               "auto"?: AutoAdvance },                      (optional)
//!   "screen": { "name", "width", "height" },                  (defaults for a new screen)
//!   "layers": [{ "name", "preset", "rect"?, "sizing"?, "fields"?, "vars"?, "bindings"? }],
//!   "rules": [{ "name", "enabled"?: false, "when": Trigger, "then": [RuleAction] }] }
//! ```
//! Placeholders in bindings, triggers and actions: `$source` (the created source), `$screen`
//! (the target screen), `$layer:<i>` and `$template:<i>` (the layer at index `i` of `layers`,
//! 0-based, and its template). Layers sharing a preset share one created template, named
//! `<instance name> · <template name>`. On an existing screen of another size the layer boxes
//! are scaled from the manifest's screen size. A layer without `rect` is placed like a layer added
//! in the editor (`screens::place_layer`: the preset's design box and sizing, or the full screen).
//! A layer with a `rect` keeps it and gets `sizing` only when the manifest sets one (absent =
//! `reflow`).
//!
//! Sample schedules (relative to the install time, local, whole minutes):
//! - `around-now`: six items of 30 min: two finished, one NOW, UP NEXT in ~15 min, two later;
//! - `single`: one item ("Doors open") starting in 30 min, 60 min long;
//! - `empty`: no items.
//!
//! Sample images: files in `<id>/assets/` (embedded with the manifest) are copied into the
//! project's `assets/` on install (`assets::add_bytes`: the import's checks, atomic, an own write
//! the watcher ignores). A taken name is never overwritten: the file gets `-2`, `-3`, … and the
//! sample rows' `/assets/<name>` cells are rewritten to match. Every `/assets/<name>` the
//! manifest's rows name must be bundled.
//!
//! Removing an instance deletes its file; with `deleteCreated` also its flows, its layers (on-air
//! ones play out), its templates (unless another layer still uses them) and its source. The screen
//! stays. Without it, its flows stay as own flows (`origin` cleared). The sample images always
//! stay in `assets/` (they may be used elsewhere; the instance doesn't record them); only a
//! failed install removes the ones it copied.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::assets;
use crate::core::flows;
use crate::core::fsx;
use crate::core::live::is_reserved_screen;
use crate::core::localtime;
use crate::core::macros::MACRO_NAME_MAX;
use crate::core::model::{
    AutoAdvance, Flow, InstallRecipe, ItemState, NewScreen, RecipeInfo, RecipeInstance,
    RecipePanel, RecipeScreen, Rect, Rule, RuleAction, ScheduleItem, ScheduleLabels, ScreenLayer,
    ScreenLayerKind, SourceBinding, SourceColumn, SourceKind, SourceKindName, SourceStep,
    TemplateDesign, TemplateSource, Trigger, FLOWS_MAX, RULES_MAX, RULE_ACTIONS_MAX,
};
use crate::core::persist::{self, PersistError, Recovered, Store};
use crate::core::protocol::{ContentSizing, Fields};
use crate::core::screens::{self, VALUE_MAX};
use crate::core::sources;
use crate::core::state::AppState;
use crate::core::templates::{self, is_id, is_key};
use crate::error::{AppError, AppResult};

pub const RECIPES_DIR: &str = "recipes";
/// Longest `forMs` of a take, and the largest `beforeNext` minutes.
pub const TAKE_FOR_MAX_MS: u32 = 3_600_000;
pub const BEFORE_NEXT_MAX_MIN: u32 = 24 * 60;

#[derive(RustEmbed)]
#[folder = "../presets/recipes/"]
#[allow_missing = true]
struct RecipeFiles;

/// Gallery order; unknown recipes follow, by id.
pub const ORDER: &[&str] = &[
    "event-schedule",
    "sponsor-slideshow",
    "sponsor-wall",
    "countdown-to-doors",
];

// ---- Manifests -------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default = "one")]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub panel: RecipePanel,
    #[serde(default = "one")]
    pub version: u32,
    #[serde(default)]
    pub source: Option<ManifestSource>,
    pub screen: ManifestScreen,
    #[serde(default)]
    pub layers: Vec<ManifestLayer>,
    #[serde(default)]
    pub rules: Vec<ManifestRule>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ManifestSource {
    Schedule {
        name: String,
        #[serde(default)]
        sample: Option<String>,
        #[serde(default)]
        ripple: bool,
        #[serde(default)]
        labels: Option<ScheduleLabels>,
    },
    Text {
        name: String,
        columns: Vec<SourceColumn>,
        #[serde(default)]
        rows: Vec<Fields>,
        #[serde(default)]
        auto: Option<AutoAdvance>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestScreen {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestLayer {
    pub name: String,
    pub preset: String,
    /// Absent: placed from the preset's design box (see the module docs).
    #[serde(default)]
    pub rect: Option<Rect>,
    #[serde(default)]
    pub sizing: Option<ContentSizing>,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default)]
    pub vars: Fields,
    #[serde(default)]
    pub bindings: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRule {
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    pub when: Value,
    pub then: Vec<Value>,
}

/// The ids of the embedded recipes, in gallery order.
pub fn ids() -> Vec<String> {
    let mut ids: Vec<String> = RecipeFiles::iter()
        .filter_map(|p| {
            let p = p.replace('\\', "/");
            let (id, file) = p.split_once('/')?;
            (file == "recipe.json").then(|| id.to_string())
        })
        .collect();
    ids.sort_by_key(|id| {
        (
            ORDER.iter().position(|o| o == id).unwrap_or(ORDER.len()),
            id.clone(),
        )
    });
    ids.dedup();
    ids
}

/// The sample files bundled in `<id>/assets/` (name, bytes), by name.
fn bundled_assets(id: &str) -> Vec<(String, Cow<'static, [u8]>)> {
    let prefix = format!("{id}/assets/");
    let mut out: Vec<(String, Cow<'static, [u8]>)> = RecipeFiles::iter()
        .filter_map(|p| {
            let name = p.replace('\\', "/").strip_prefix(&prefix)?.to_string();
            let file = RecipeFiles::get(&p)?;
            Some((name, file.data))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn bad(id: &str, msg: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("Recipe {id} is not valid: {msg}"))
}

/// Loads and validates a manifest (placeholders included: every `$layer:<i>` / `$template:<i>`
/// must name a layer, every preset must exist).
pub fn load(id: &str) -> AppResult<Manifest> {
    if !is_id(id) {
        return Err(AppError::NotFound(format!("Recipe not found: {id}")));
    }
    let Some(file) = RecipeFiles::get(&format!("{id}/recipe.json")) else {
        return Err(AppError::NotFound(format!("Recipe not found: {id}")));
    };
    let m: Manifest = serde_json::from_slice(&file.data).map_err(|e| bad(id, e))?;
    if m.schema_version != 1 {
        return Err(bad(
            id,
            format!("schema {} is not supported", m.schema_version),
        ));
    }
    if m.id != id {
        return Err(bad(id, format!("its id is \"{}\"", m.id)));
    }
    if m.name.trim().is_empty() {
        return Err(bad(id, "it has no name"));
    }
    screens::validate_name(&m.screen.name).map_err(|e| bad(id, e))?;
    screens::validate_size(m.screen.width, m.screen.height).map_err(|e| bad(id, e))?;
    if m.layers.is_empty() || m.layers.len() > crate::core::model::SCREEN_LAYERS_MAX {
        return Err(bad(id, "it needs 1 to 64 layers"));
    }
    for l in &m.layers {
        crate::core::presets::load(&l.preset).map_err(|e| bad(id, e))?;
        if let Some(r) = &l.rect {
            screens::validate_rect(r, m.screen.width, m.screen.height)
                .map_err(|e| bad(id, format!("layer \"{}\": {e}", l.name)))?;
        }
    }
    // Every bundled file is an image the import would accept.
    let bundled = bundled_assets(id);
    for (n, bytes) in &bundled {
        let ext = assets::asset_ext(n).unwrap_or("");
        if !assets::is_asset_name(n)
            || bytes.len() as u64 > crate::core::model::ASSET_MAX_BYTES
            || !assets::sniff(ext, &bytes[..bytes.len().min(512)])
        {
            return Err(bad(id, format!("assets/{n} is not a valid image")));
        }
    }
    if let Some(s) = &m.source {
        match s {
            ManifestSource::Schedule { sample, .. } => {
                if !matches!(
                    sample.as_deref(),
                    None | Some("around-now" | "single" | "empty")
                ) {
                    return Err(bad(id, "unknown sample (around-now, single or empty)"));
                }
            }
            ManifestSource::Text {
                name,
                columns,
                rows,
                auto,
            } => {
                // Every asset the rows name is bundled.
                let mut named = BTreeSet::new();
                for v in rows.iter().flat_map(|r| r.values()) {
                    assets::mentioned(v, &mut named);
                }
                if let Some(n) = named.iter().find(|n| !bundled.iter().any(|(b, _)| b == *n)) {
                    return Err(bad(id, format!("assets/{n} is not bundled")));
                }
                let doc = crate::core::model::SourceDoc {
                    id: "check".into(),
                    meta: crate::core::model::SourceMeta {
                        name: name.clone(),
                        kind: SourceKind::Text {
                            columns: columns.clone(),
                            rows: rows.clone(),
                            index: 0,
                            auto: *auto,
                        },
                        updated_at: String::new(),
                    },
                };
                sources::validate(doc).map_err(|e| bad(id, e))?;
            }
        }
    }
    // A dry run of the placeholders with dummy ids.
    let ctx = Placeholders {
        source: Some("s".into()),
        screen: "x".into(),
        layers: (0..m.layers.len()).map(|i| format!("l{i}")).collect(),
        templates: (0..m.layers.len()).map(|i| format!("t{i}")).collect(),
    };
    for l in &m.layers {
        resolve_bindings(&l.bindings, &ctx).map_err(|e| bad(id, e))?;
    }
    let rules = resolve_rules(&m.rules, &ctx).map_err(|e| bad(id, e))?;
    let rules = validate_rules(rules).map_err(|e| bad(id, e))?;
    for r in &rules {
        let flow = flows::rule_to_flow(r, "check".into(), None);
        flows::validate_flow(&flow).map_err(|e| bad(id, format!("rule \"{}\": {e}", r.name)))?;
    }
    Ok(m)
}

/// The recipes with valid manifests, in gallery order (invalid ones are logged and left out).
pub fn list() -> Vec<RecipeInfo> {
    ids()
        .into_iter()
        .filter_map(|id| match load(&id) {
            Ok(m) => Some(info(&m)),
            Err(e) => {
                tracing::warn!("{e}");
                None
            }
        })
        .collect()
}

fn info(m: &Manifest) -> RecipeInfo {
    let mut templates: Vec<String> = Vec::new();
    for l in &m.layers {
        if !templates.contains(&l.preset) {
            templates.push(l.preset.clone());
        }
    }
    RecipeInfo {
        id: m.id.clone(),
        name: m.name.trim().to_string(),
        description: m.description.clone(),
        panel: m.panel,
        version: m.version,
        templates,
    }
}

// ---- Placeholders ----------------------------------------------------------------------------

struct Placeholders {
    source: Option<String>,
    screen: String,
    layers: Vec<String>,
    templates: Vec<String>,
}

fn resolve_str(s: &str, ctx: &Placeholders) -> Result<Option<String>, String> {
    let Some(rest) = s.strip_prefix('$') else {
        return Ok(None);
    };
    let pick = |list: &[String], i: &str, what: &str| {
        i.parse::<usize>()
            .ok()
            .and_then(|i| list.get(i).cloned())
            .ok_or_else(|| format!("{s}: there is no {what} {i}"))
    };
    let v = match rest {
        "source" => ctx
            .source
            .clone()
            .ok_or_else(|| "$source used without a source".to_string())?,
        "screen" => ctx.screen.clone(),
        _ => {
            if let Some(i) = rest.strip_prefix("layer:") {
                pick(&ctx.layers, i, "layer")?
            } else if let Some(i) = rest.strip_prefix("template:") {
                pick(&ctx.templates, i, "layer")?
            } else {
                return Err(format!("unknown placeholder {s}"));
            }
        }
    };
    Ok(Some(v))
}

fn resolve_value(v: &mut Value, ctx: &Placeholders) -> Result<(), String> {
    match v {
        Value::String(s) => {
            if let Some(r) = resolve_str(s, ctx)? {
                *s = r;
            }
        }
        Value::Array(a) => {
            for x in a {
                resolve_value(x, ctx)?;
            }
        }
        Value::Object(o) => {
            for (_, x) in o.iter_mut() {
                resolve_value(x, ctx)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve_bindings(
    b: &BTreeMap<String, Value>,
    ctx: &Placeholders,
) -> Result<BTreeMap<String, SourceBinding>, String> {
    b.iter()
        .map(|(k, v)| {
            let mut v = v.clone();
            resolve_value(&mut v, ctx)?;
            let b: SourceBinding =
                serde_json::from_value(v).map_err(|e| format!("binding \"{k}\": {e}"))?;
            Ok((k.clone(), b))
        })
        .collect()
}

fn resolve_rules(rules: &[ManifestRule], ctx: &Placeholders) -> Result<Vec<Rule>, String> {
    rules
        .iter()
        .map(|r| {
            let mut when = r.when.clone();
            resolve_value(&mut when, ctx)?;
            let when: Trigger = serde_json::from_value(when)
                .map_err(|e| format!("rule \"{}\": trigger: {e}", r.name))?;
            let then = r
                .then
                .iter()
                .map(|a| {
                    let mut a = a.clone();
                    resolve_value(&mut a, ctx)?;
                    serde_json::from_value::<RuleAction>(a)
                        .map_err(|e| format!("rule \"{}\": action: {e}", r.name))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Rule {
                id: templates::random_id(),
                name: r.name.clone(),
                enabled: r.enabled,
                when,
                then,
                last_fired_at: None,
            })
        })
        .collect()
}

// ---- Rules validation ------------------------------------------------------------------------

fn check_id(what: &str, id: &str) -> Result<(), String> {
    if is_id(id) {
        Ok(())
    } else {
        Err(format!("invalid {what} id: {id}"))
    }
}

/// Checks a rule list (shapes and limits; targets may be missing, they are skipped when a rule
/// fires). Names are trimmed; a rule without an id gets one.
pub fn validate_rules(mut rules: Vec<Rule>) -> Result<Vec<Rule>, String> {
    if rules.len() > RULES_MAX {
        return Err(format!("At most {RULES_MAX} rules"));
    }
    let mut ids = HashSet::new();
    for r in &mut rules {
        if r.id.is_empty() {
            r.id = templates::random_id();
        }
        check_id("rule", &r.id)?;
        if !ids.insert(r.id.clone()) {
            return Err(format!("Duplicate rule id: {}", r.id));
        }
        r.name = r.name.trim().to_string();
        let n = r.name.chars().count();
        if n == 0 || n > MACRO_NAME_MAX {
            return Err(format!(
                "Rule names must be 1 to {MACRO_NAME_MAX} characters"
            ));
        }
        let at = |e: String| format!("Rule \"{}\": {e}", r.name);
        match &r.when {
            Trigger::ItemStatus {
                source_id, item_id, ..
            } => {
                check_id("source", source_id).map_err(at)?;
                if let Some(i) = item_id {
                    check_id("item", i).map_err(at)?;
                }
            }
            Trigger::BeforeNext { source_id, minutes } => {
                check_id("source", source_id).map_err(at)?;
                if *minutes > BEFORE_NEXT_MAX_MIN {
                    return Err(at(format!("at most {BEFORE_NEXT_MAX_MIN} minutes before")));
                }
            }
            Trigger::TimeOfDay { at: t, days } => {
                if localtime::parse_hhmm(t).is_none() {
                    return Err(at(format!("\"{t}\" is not a time like 09:30")));
                }
                let mut seen = HashSet::new();
                if days.len() > 7 || days.iter().any(|d| *d > 6 || !seen.insert(*d)) {
                    return Err(at("days are 0 (Monday) to 6 (Sunday), each once".into()));
                }
            }
            Trigger::TimerZero {
                screen_id,
                layer_id,
                key,
            } => {
                check_id("screen", screen_id).map_err(at)?;
                check_id("layer", layer_id).map_err(at)?;
                if !is_key(key) {
                    return Err(at(format!("invalid field key: {key}")));
                }
            }
            Trigger::Layer {
                screen_id,
                layer_id,
                ..
            } => {
                check_id("screen", screen_id).map_err(at)?;
                check_id("layer", layer_id).map_err(at)?;
            }
            Trigger::SlideshowWrap { source_id } => check_id("source", source_id).map_err(at)?,
        }
        if r.then.is_empty() || r.then.len() > RULE_ACTIONS_MAX {
            return Err(at(format!("1 to {RULE_ACTIONS_MAX} actions")));
        }
        for a in &r.then {
            match a {
                RuleAction::Take {
                    screen_id,
                    layer_id,
                    for_ms,
                } => {
                    check_id("screen", screen_id).map_err(at)?;
                    check_id("layer", layer_id).map_err(at)?;
                    if for_ms.is_some_and(|ms| ms == 0 || ms > TAKE_FOR_MAX_MS) {
                        return Err(at(format!(
                            "\"take for\" must be 1 ms to {} minutes",
                            TAKE_FOR_MAX_MS / 60_000
                        )));
                    }
                }
                RuleAction::Clear {
                    screen_id,
                    layer_id,
                } => {
                    check_id("screen", screen_id).map_err(at)?;
                    check_id("layer", layer_id).map_err(at)?;
                }
                RuleAction::RunMacro { macro_id } => check_id("macro", macro_id).map_err(at)?,
                RuleAction::StepSource { source_id, step } => {
                    check_id("source", source_id).map_err(at)?;
                    let _: &SourceStep = step;
                }
                RuleAction::PauseSource { source_id, .. } => {
                    check_id("source", source_id).map_err(at)?
                }
                RuleAction::SetFields {
                    screen_id,
                    layer_id,
                    fields,
                } => {
                    check_id("screen", screen_id).map_err(at)?;
                    check_id("layer", layer_id).map_err(at)?;
                    for (k, v) in fields {
                        if !is_key(k) || v.len() > VALUE_MAX {
                            return Err(at(format!("invalid field \"{k}\"")));
                        }
                    }
                }
            }
        }
    }
    Ok(rules)
}

// ---- Instance files --------------------------------------------------------------------------

// Here rather than a derive so `model.rs` (mirrored in TS) stays untouched.
impl Default for RecipeInstance {
    fn default() -> Self {
        Self {
            id: String::new(),
            recipe_id: String::new(),
            name: String::new(),
            panel: RecipePanel::Slideshow,
            source_id: None,
            screen_id: String::new(),
            layer_ids: Vec::new(),
            template_ids: Vec::new(),
            rules: Vec::new(),
            flow_ids: Vec::new(),
            created_at: String::new(),
        }
    }
}

impl Store for RecipeInstance {
    /// Unused: instance files are named after their id.
    const FILE: &'static str = "recipe.json";
    /// 2: `flowIds` instead of `rules` (a v1 file still reads; `core::flows` migrates it).
    const VERSION: u32 = 2;
}

#[derive(Serialize)]
struct Versioned<'a> {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(flatten)]
    inst: &'a RecipeInstance,
}

pub fn recipes_root(project: &Path) -> PathBuf {
    project.join(RECIPES_DIR)
}

fn instance_path(project: &Path, id: &str) -> AppResult<PathBuf> {
    if !is_id(id) {
        return Err(AppError::Invalid(format!("Invalid recipe id: {id}")));
    }
    Ok(recipes_root(project).join(format!("{id}.json")))
}

/// Writes an instance atomically. A file from a newer Kinetra is never overwritten.
pub fn write_instance(project: &Path, inst: &RecipeInstance) -> AppResult<()> {
    let path = instance_path(project, &inst.id)?;
    if let Ok(bytes) = fs::read(&path) {
        if let Err(PersistError::NewerVersion { found, supported }) =
            persist::decode::<RecipeInstance>(&path, &bytes)
        {
            return Err(AppError::NewerVersion { found, supported });
        }
    }
    let root = recipes_root(project);
    fs::create_dir_all(&root)?;
    fsx::write_json_atomic(
        &path,
        &Versioned {
            schema_version: RecipeInstance::VERSION,
            inst,
        },
    )?;
    Ok(())
}

pub fn delete_instance(project: &Path, id: &str) -> AppResult<()> {
    let path = instance_path(project, id)?;
    match fsx::remove_file_own(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Every instance of the project, by creation time. Corrupt files are set aside (reported);
/// newer or invalid ones are logged and skipped.
pub fn load_all(project: &Path) -> (Vec<RecipeInstance>, Vec<Recovered>) {
    let mut list = Vec::new();
    let mut recovered = Vec::new();
    let root = recipes_root(project);
    let Ok(entries) = fs::read_dir(&root) else {
        return (list, recovered);
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".json").filter(|id| is_id(id)) else {
            continue;
        };
        let path = entry.path();
        let Ok(bytes) = fs::read(&path) else { continue };
        match persist::decode::<RecipeInstance>(&path, &bytes) {
            Ok(mut inst) => {
                inst.id = id.to_string();
                list.push(inst);
            }
            Err(e @ PersistError::Corrupt { .. }) => {
                tracing::warn!("{e}; setting it aside");
                match persist::quarantine(&path) {
                    Ok(r) => recovered.push(r),
                    Err(e) => tracing::warn!("could not set aside {}: {e}", path.display()),
                }
            }
            Err(e) => tracing::warn!("skipping recipe {id}: {e}"),
        }
    }
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    (list, recovered)
}

/// Writes the instances whose `lastFiredAt` changed (the content flusher). Returns the count.
pub(crate) fn flush_dirty(rules: &crate::core::rules::Rules) -> u64 {
    let Some((dir, list)) = rules.take_dirty() else {
        return 0;
    };
    let mut n = 0;
    for inst in list {
        match write_instance(&dir, &inst) {
            Ok(()) => n += 1,
            Err(AppError::NewerVersion { .. }) => {}
            Err(e) => {
                tracing::warn!("saving recipe {}: {e}", inst.id);
                rules.mark_dirty(&inst.id);
            }
        }
    }
    n
}

// ---- Sample schedules ------------------------------------------------------------------------

fn sample_items(kind: &str, now: &localtime::Now) -> Vec<ScheduleItem> {
    let base = localtime::floor_minute(now.local());
    let at = |min: i64| localtime::format_local(base + time::Duration::minutes(min));
    let item = |title: &str, sub: &str, loc: &str, start: i64, len: i64| ScheduleItem {
        id: templates::random_id(),
        title: title.into(),
        subtitle: sub.into(),
        location: loc.into(),
        start: at(start),
        end: at(start + len),
        delay_min: 0,
        state: ItemState::Auto,
        started_at: None,
        ended_at: None,
    };
    match kind {
        "empty" => Vec::new(),
        "single" => vec![item("Doors open", "", "Main entrance", 30, 60)],
        _ => vec![
            item("Registration", "Badges and coffee", "Lobby", -75, 30),
            item(
                "Opening keynote",
                "Welcome to the event",
                "Main stage",
                -45,
                30,
            ),
            item("Panel discussion", "The year ahead", "Main stage", -15, 30),
            item("Workshop", "Hands-on session", "Room B", 15, 30),
            item("Lunch break", "", "Food court", 45, 30),
            item("Closing show", "Awards and thanks", "Main stage", 75, 30),
        ],
    }
}

// ---- Install / remove ----------------------------------------------------------------

/// What an install created so far (for the rollback on failure).
#[derive(Default)]
struct Created {
    /// Files written into `assets/` (by the name they got).
    assets: Vec<String>,
    source: Option<String>,
    templates: Vec<String>,
    screen: Option<String>,
    /// Flow files written (added to the store only once the install succeeded).
    flows: Vec<Flow>,
}

fn template_name(instance: &str, template: &str) -> String {
    let full = format!("{instance} · {template}");
    if full.chars().count() <= 100 {
        return full;
    }
    let keep = 100usize.saturating_sub(template.chars().count() + 3).max(1);
    let short: String = instance.chars().take(keep).collect();
    let s = format!("{short} · {template}");
    s.chars().take(100).collect()
}

/// A manifest layer's box and sizing on a `to` screen: its own `rect` (scaled from the manifest's
/// screen `from`) with its own `sizing` only (absent = reflow, as before sizing existed), or
/// without a `rect` placed from the preset's design box like a layer added in the editor.
fn layer_box(
    l: &ManifestLayer,
    design: Option<&TemplateDesign>,
    from: (u32, u32),
    to: (u32, u32),
) -> (Rect, Option<ContentSizing>) {
    match l.rect {
        Some(r) => (scale_rect(r, from, to), l.sizing),
        None => {
            let (r, s) = screens::place_layer(design, to.0, to.1);
            (r, l.sizing.or(s))
        }
    }
}

fn scale_rect(r: Rect, from: (u32, u32), to: (u32, u32)) -> Rect {
    if from == to || from.0 == 0 || from.1 == 0 {
        return r;
    }
    let sx = f64::from(to.0) / f64::from(from.0);
    let sy = f64::from(to.1) / f64::from(from.1);
    Rect {
        x: (f64::from(r.x) * sx).round() as i32,
        y: (f64::from(r.y) * sy).round() as i32,
        w: ((f64::from(r.w) * sx).round() as u32).max(1),
        h: ((f64::from(r.h) * sy).round() as u32).max(1),
    }
}

impl AppState {
    /// The project's recipe instances, in creation order.
    pub fn list_recipe_instances(&self) -> AppResult<Vec<RecipeInstance>> {
        self.project_dir()?;
        Ok(self.rules().instances())
    }

    /// Installs a recipe (see the module docs). Nothing is left behind when it fails. Touches
    /// the disk.
    pub fn install_recipe(&self, req: &InstallRecipe) -> AppResult<RecipeInstance> {
        let dir = self.project_dir()?;
        let m = load(&req.recipe_id)?;
        let name = req.name.trim().to_string();
        let n = name.chars().count();
        if n == 0 || n > 100 {
            return Err(AppError::Invalid(
                "The recipe's name must be 1 to 100 characters".into(),
            ));
        }
        // The target screen first, so nothing is created for a bad request.
        let target = match &req.screen {
            RecipeScreen::Existing { screen_id } => {
                if is_reserved_screen(screen_id) {
                    return Err(AppError::NotFound(format!("Screen not found: {screen_id}")));
                }
                let doc = self
                    .screens()
                    .doc(screen_id)
                    .ok_or_else(|| AppError::NotFound(format!("Screen not found: {screen_id}")))?;
                if doc.meta.layers.len() + m.layers.len() > crate::core::model::SCREEN_LAYERS_MAX {
                    return Err(AppError::Invalid(format!(
                        "The screen has no room for {} more layers",
                        m.layers.len()
                    )));
                }
                None
            }
            RecipeScreen::New {
                name,
                width,
                height,
            } => {
                screens::validate_name(name)?;
                screens::validate_size(*width, *height)?;
                Some(NewScreen {
                    name: name.clone(),
                    width: *width,
                    height: *height,
                    background: "transparent".into(),
                })
            }
        };
        if self.flows().len() + m.rules.len() > FLOWS_MAX {
            return Err(AppError::Invalid(format!(
                "A project has at most {FLOWS_MAX} flows"
            )));
        }
        let mut created = Created::default();
        let result = self.install_steps(&dir, &m, &name, req, target, &mut created);
        match result {
            Ok(inst) => {
                self.rules().put_instance(inst.clone());
                self.recipe_flows_installed(std::mem::take(&mut created.flows));
                self.replan();
                Ok(inst)
            }
            Err(e) => {
                self.rollback(&dir, &created);
                Err(e)
            }
        }
    }

    fn install_steps(
        &self,
        dir: &Path,
        m: &Manifest,
        name: &str,
        req: &InstallRecipe,
        target: Option<NewScreen>,
        created: &mut Created,
    ) -> AppResult<RecipeInstance> {
        // Bundled sample images into `assets/`, each under a free name (an existing file is never
        // replaced); `renamed` maps a bundled name to the one it got when that was taken.
        let mut renamed: Vec<(String, String)> = Vec::new();
        for (bundled, bytes) in bundled_assets(&m.id) {
            let got = assets::add_bytes(dir, &bundled, &bytes)?;
            created.assets.push(got.clone());
            if got != bundled {
                renamed.push((bundled, got));
            }
        }
        // Source.
        let source_id = match &m.source {
            None => None,
            Some(ManifestSource::Schedule {
                name: sname,
                sample,
                ripple,
                labels,
            }) => {
                let mut doc = self.create_schedule(&format!("{name} · {sname}"))?;
                created.source = Some(doc.id.clone());
                let now = self.sources().clock().now();
                doc.meta.kind = SourceKind::Schedule {
                    items: sample_items(sample.as_deref().unwrap_or("around-now"), &now),
                    ripple: *ripple,
                    next_override: None,
                    labels: labels.clone().unwrap_or_default(),
                };
                Some(self.save_source(doc)?.id)
            }
            Some(ManifestSource::Text {
                name: sname,
                columns,
                rows,
                auto,
            }) => {
                let mut doc = self.create_source(&format!("{name} · {sname}"))?;
                created.source = Some(doc.id.clone());
                let mut rows = rows.clone();
                if !renamed.is_empty() {
                    for v in rows.iter_mut().flat_map(|r| r.values_mut()) {
                        *v = assets::rename_refs(v, &renamed);
                    }
                }
                doc.meta.kind = SourceKind::Text {
                    columns: columns.clone(),
                    rows,
                    index: 0,
                    auto: *auto,
                };
                Some(self.save_source(doc)?.id)
            }
        };
        // Templates: one per distinct preset.
        let mut by_preset: HashMap<String, (String, Option<TemplateDesign>)> = HashMap::new();
        let mut layer_templates = Vec::with_capacity(m.layers.len());
        for l in &m.layers {
            let id = match by_preset.get(&l.preset) {
                Some(t) => t.clone(),
                None => {
                    let p = crate::core::presets::load(&l.preset)?;
                    let doc = self.templates().create(
                        dir,
                        &template_name(name, &p.info.name),
                        &TemplateSource::Preset {
                            preset_id: l.preset.clone(),
                        },
                    )?;
                    created.templates.push(doc.id.clone());
                    let t = (doc.id, p.meta.design);
                    by_preset.insert(l.preset.clone(), t.clone());
                    t
                }
            };
            layer_templates.push(id);
        }
        // Screen.
        let mut screen = match target {
            Some(new) => {
                let st = self.create_screen(&new)?;
                created.screen = Some(st.doc.id.clone());
                st.doc
            }
            None => {
                let RecipeScreen::Existing { screen_id } = &req.screen else {
                    return Err(AppError::Internal("screen target".into()));
                };
                self.screens()
                    .doc(screen_id)
                    .ok_or_else(|| AppError::NotFound(format!("Screen not found: {screen_id}")))?
            }
        };
        let mut taken: HashSet<String> = screen.meta.layers.iter().map(|l| l.id.clone()).collect();
        let layer_ids: Vec<String> = m
            .layers
            .iter()
            .map(|_| {
                let id = screens::new_layer_id(&taken);
                taken.insert(id.clone());
                id
            })
            .collect();
        let designs: Vec<Option<TemplateDesign>> = layer_templates.iter().map(|t| t.1).collect();
        let layer_templates: Vec<String> = layer_templates.into_iter().map(|t| t.0).collect();
        let ctx = Placeholders {
            source: source_id.clone(),
            screen: screen.id.clone(),
            layers: layer_ids.clone(),
            templates: layer_templates.clone(),
        };
        let from = (m.screen.width, m.screen.height);
        let to = (screen.meta.width, screen.meta.height);
        for (i, l) in m.layers.iter().enumerate() {
            let bindings = resolve_bindings(&l.bindings, &ctx).map_err(AppError::Invalid)?;
            let (rect, sizing) = layer_box(l, designs[i].as_ref(), from, to);
            screen.meta.layers.push(ScreenLayer {
                id: layer_ids[i].clone(),
                name: l.name.clone(),
                rect,
                kind: ScreenLayerKind::Template {
                    template_id: layer_templates[i].clone(),
                    fields: l.fields.clone(),
                    vars: l.vars.clone(),
                    bindings,
                    variant: None,
                    sizing,
                    opacity: None,
                },
                anchor: None,
            });
        }
        let screen_id = screen.id.clone();
        self.save_screen(screen)?;
        // Flows (from the manifest's rules) and the instance file.
        let rules = resolve_rules(&m.rules, &ctx).map_err(AppError::Invalid)?;
        let rules = validate_rules(rules).map_err(AppError::Invalid)?;
        let instance_id = new_instance_id(dir);
        let flows: Vec<Flow> = self
            .new_flow_ids(dir, rules.len())
            .into_iter()
            .zip(&rules)
            .map(|(id, r)| flows::rule_to_flow(r, id, Some(instance_id.clone())))
            .collect();
        self.write_recipe_flows(dir, &flows)?;
        let flow_ids = flows.iter().map(|f| f.id.clone()).collect();
        created.flows = flows;
        let mut template_ids: Vec<String> = Vec::new();
        for t in layer_templates {
            if !template_ids.contains(&t) {
                template_ids.push(t);
            }
        }
        let inst = RecipeInstance {
            id: instance_id,
            recipe_id: m.id.clone(),
            name: name.to_string(),
            panel: m.panel,
            source_id,
            screen_id,
            layer_ids,
            template_ids,
            rules: Vec::new(),
            flow_ids,
            created_at: fsx::now_rfc3339(),
        };
        write_instance(dir, &inst)?;
        Ok(inst)
    }

    /// Undoes a failed install (best effort).
    fn rollback(&self, dir: &Path, c: &Created) {
        let flow_ids: Vec<String> = c.flows.iter().map(|f| f.id.clone()).collect();
        self.recipe_flows_rollback(dir, &flow_ids);
        if let Some(s) = &c.screen {
            let _ = self.delete_screen(s);
        }
        for t in &c.templates {
            if self.templates().delete(dir, t).is_ok() {
                self.template_deleted(t);
            }
        }
        if let Some(s) = &c.source {
            let _ = self.delete_source(s);
        }
        let root = assets::assets_root(dir);
        for a in &c.assets {
            let _ = fsx::remove_file_own(&root.join(a));
        }
    }

    /// Removes an instance (see the module docs). `notFound` for an unknown id.
    pub fn remove_recipe(&self, id: &str, delete_created: bool) -> AppResult<()> {
        let dir = self.project_dir()?;
        let inst = self
            .rules()
            .instance(id)
            .ok_or_else(|| AppError::NotFound(format!("Recipe not found: {id}")))?;
        delete_instance(&dir, id)?;
        self.rules().remove_instance(id);
        self.recipe_flows_removed(&dir, &inst.flow_ids, delete_created);
        if delete_created {
            if let Some(mut doc) = self.screens().doc(&inst.screen_id) {
                let before = doc.meta.layers.len();
                doc.meta.layers.retain(|l| !inst.layer_ids.contains(&l.id));
                if doc.meta.layers.len() != before {
                    self.save_screen(doc)?;
                }
            }
            for t in &inst.template_ids {
                let used = self.screens().doc_ids().iter().any(|s| {
                    self.screens()
                        .doc(s)
                        .is_some_and(|d| screens::template_ids(&d).any(|x| x == t.as_str()))
                });
                if used {
                    continue;
                }
                match self.templates().delete(&dir, t) {
                    Ok(()) => self.template_deleted(t),
                    Err(templates::TemplateError::NotFound(_)) => {}
                    Err(e) => return Err(e.into()),
                }
            }
            if let Some(s) = &inst.source_id {
                match self.delete_source(s) {
                    Ok(()) | Err(AppError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
        }
        self.replan();
        Ok(())
    }
}

fn new_instance_id(project: &Path) -> String {
    let root = recipes_root(project);
    loop {
        let id = templates::random_id();
        if !root.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

#[cfg(test)]
mod tests;

/// The source kind a recipe drives: its manifest's, else by panel (slideshow: text;
/// schedule/countdown: schedule).
fn driven_kind(inst: &RecipeInstance) -> SourceKindName {
    match load(&inst.recipe_id).ok().and_then(|m| m.source) {
        Some(ManifestSource::Text { .. }) => SourceKindName::Text,
        Some(ManifestSource::Schedule { .. }) => SourceKindName::Schedule,
        None => match inst.panel {
            RecipePanel::Slideshow => SourceKindName::Text,
            RecipePanel::Schedule | RecipePanel::Countdown => SourceKindName::Schedule,
        },
    }
}

impl AppState {
    /// See `commands::recipes::relink_recipe_source`. Rewrites the old source in the recipe's
    /// flows (deployed and draft, redeployed, armed unchanged) and in the bindings of its own
    /// layers (saved through the screen path), then the instance file. Touches the disk.
    pub fn relink_recipe_source(&self, id: &str, source_id: &str) -> AppResult<RecipeInstance> {
        let dir = self.project_dir()?;
        let mut inst = self
            .rules()
            .instance(id)
            .ok_or_else(|| AppError::NotFound(format!("Recipe not found: {id}")))?;
        let doc = self
            .sources()
            .doc(source_id)
            .ok_or_else(|| AppError::NotFound(format!("Source not found: {source_id}")))?;
        let want = driven_kind(&inst);
        let have = match doc.meta.kind {
            // An API source is a table too.
            SourceKind::Text { .. } | SourceKind::Api { .. } => SourceKindName::Text,
            SourceKind::Schedule { .. } => SourceKindName::Schedule,
            SourceKind::Poll { .. } => SourceKindName::Poll,
        };
        if have != want {
            return Err(AppError::Invalid(match want {
                SourceKindName::Text | SourceKindName::Api | SourceKindName::Poll => {
                    "This recipe needs a text source".into()
                }
                SourceKindName::Schedule => "This recipe needs a schedule source".into(),
            }));
        }
        let old = inst.source_id.clone();
        if old.as_deref() == Some(source_id) {
            return Ok(inst);
        }
        if let Some(old) = &old {
            for f in &inst.flow_ids {
                match self.relink_flow_source(&dir, f, old, source_id) {
                    Ok(_) | Err(AppError::NotFound(_)) => {}
                    Err(e) => return Err(e),
                }
            }
            if let Some(mut screen) = self.screens().doc(&inst.screen_id) {
                let mut changed = false;
                for l in &mut screen.meta.layers {
                    if !inst.layer_ids.contains(&l.id) {
                        continue;
                    }
                    let ScreenLayerKind::Template { bindings, .. } = &mut l.kind else {
                        continue;
                    };
                    for b in bindings.values_mut() {
                        if &b.source_id == old {
                            b.source_id = source_id.to_string();
                            changed = true;
                        }
                    }
                }
                if changed {
                    self.save_screen(screen)?;
                }
            }
        }
        inst.source_id = Some(source_id.to_string());
        write_instance(&dir, &inst)?;
        self.rules().put_instance(inst.clone());
        self.replan();
        Ok(inst)
    }
}
