//! Built-in flow templates (`presets/flows/<id>.json`, embedded): premade graphs that point at
//! things the project already has (layers, sources, timers), chosen when adding one.
//!
//! # Manifest
//! `{ "schemaVersion": 1, id, name, description, category, slots: [FlowSlot], flows: [{ suffix?,
//! armed: "cue" | false, nodes, edges }] }` (1 to [`FLOWS_PER_TEMPLATE_MAX`] flows). Nodes and
//! edges are the flow file's (`core::model` "Flows"), with readable ids (`cue`, `take1`, `e1`)
//! that get fresh nanoids on add (edges remapped). `armed: "cue"` = armed when added (the flow
//! only runs on demand: a Cue, or Run by flow); `false` = added disarmed (automatic triggers:
//! nothing goes on air until the operator arms it).
//!
//! # Placeholders
//! In any string of a node config (object keys included, e.g. a `setFields` field key) and in
//! edge port names: `{{slot}}` is the slot's whole value; `{{slot.screenId}}`, `{{slot.layerId}}`
//! the parts of a layer / timer / field value; `{{slot.key}}` the key of a timer / field value;
//! `{{flow:<suffix>}}` the id of the template's flow with that suffix (cross-flow references,
//! e.g. a Cue flow that runs another flow of the same template). A `setFields` node fed by a
//! wire names both its `fields` key and the wire's `to.port` `{{slot.key}}`.
//!
//! # Slots
//! Values as [`FlowSlotKind`] documents: a screen / source / flow id, `screenId/layerId`, or
//! `screenId/layerId/key`. A required slot that is missing or empty, or a malformed value, is
//! `invalid` naming the slot's label. An optional slot left empty resolves to `""` everywhere it
//! is used, so optional slots are only for configs where an empty value is valid. Targets that
//! don't exist (a removed layer) are soft warnings of the created flows, like any flow.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::Value;

use crate::core::flows::{self, check_graph, validate_name};
use crate::core::fsx;
use crate::core::model::{
    AddFlowTemplate, Flow, FlowEdge, FlowInfo, FlowNode, FlowSlot, FlowSlotKind,
    FlowTemplateCategory, FlowTemplateInfo, Retrigger,
};
use crate::core::state::AppState;
use crate::core::templates::{is_id, is_key, random_id};
use crate::error::{AppError, AppResult};

#[derive(RustEmbed)]
#[folder = "../presets/flows/"]
#[allow_missing = true]
struct FlowFiles;

/// Gallery order; unknown templates follow, by id.
pub const ORDER: &[&str] = &[
    "show-open",
    "next-guest",
    "break-countdown",
    "stinger-once",
    "now-next-on-item",
    "five-minute-warning",
    "headliner-branch",
    "auto-delay",
    "doors-countdown",
    "main-sponsor-every-third",
    "sponsor-reel-between",
    "random-fact",
    "match-end-results",
    "new-leader-alert",
    "score-reaction",
    "clean-all-log",
    "reusable-lower-third",
    "manual-hold",
];

/// Flows one template may create.
pub const FLOWS_PER_TEMPLATE_MAX: usize = 3;
/// Slots of one template.
pub const SLOTS_MAX: usize = 8;

// ---- Manifests -------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub category: FlowTemplateCategory,
    #[serde(default)]
    pub slots: Vec<FlowSlot>,
    pub flows: Vec<ManifestFlow>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestFlow {
    #[serde(default)]
    pub suffix: Option<String>,
    /// `"cue"` (armed when added) or `false`.
    pub armed: Value,
    pub nodes: Vec<Value>,
    pub edges: Vec<Value>,
}

impl ManifestFlow {
    fn armed(&self) -> Result<bool, String> {
        match &self.armed {
            Value::String(s) if s == "cue" => Ok(true),
            Value::Bool(false) => Ok(false),
            v => Err(format!("\"armed\" is \"cue\" or false, not {v}")),
        }
    }
}

struct Template {
    manifest: Manifest,
    info: FlowTemplateInfo,
}

fn bad(id: &str, msg: impl std::fmt::Display) -> String {
    format!("Flow template {id} is not valid: {msg}")
}

/// Parses and checks a manifest's shape (slots, flow count, suffixes, armed). The graphs are
/// checked when resolved ([`build`]); the tests resolve every template.
pub(crate) fn parse(bytes: &[u8]) -> Result<Manifest, String> {
    let m: Manifest = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if m.schema_version != 1 {
        return Err(bad(&m.id, "unknown schemaVersion"));
    }
    if !is_id(&m.id) || m.name.trim().is_empty() {
        return Err(bad(&m.id, "id or name"));
    }
    if m.flows.is_empty() || m.flows.len() > FLOWS_PER_TEMPLATE_MAX {
        return Err(bad(&m.id, format!("1 to {FLOWS_PER_TEMPLATE_MAX} flows")));
    }
    if m.slots.len() > SLOTS_MAX {
        return Err(bad(&m.id, format!("at most {SLOTS_MAX} slots")));
    }
    let mut keys = HashSet::new();
    for s in &m.slots {
        let shaped = s.key.len() <= 32
            && s.key.starts_with(|c: char| c.is_ascii_lowercase())
            && s.key.chars().all(|c| c.is_ascii_alphanumeric());
        if !shaped || !keys.insert(s.key.as_str()) || s.label.trim().is_empty() {
            return Err(bad(&m.id, format!("slot \"{}\"", s.key)));
        }
    }
    let mut suffixes = HashSet::new();
    for f in &m.flows {
        f.armed().map_err(|e| bad(&m.id, e))?;
        match &f.suffix {
            Some(s) if s.trim().is_empty() || !suffixes.insert(s.as_str()) => {
                return Err(bad(&m.id, format!("suffix \"{s}\"")));
            }
            None if m.flows.len() > 1 => {
                return Err(bad(
                    &m.id,
                    "every flow of a multi-flow template needs a suffix",
                ));
            }
            _ => {}
        }
    }
    Ok(m)
}

/// The embedded manifests, parsed once, in gallery order. A manifest that doesn't parse is
/// left out (logged); the tests make sure none is.
fn all() -> &'static [Template] {
    static ALL: OnceLock<Vec<Template>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut list: Vec<Template> = FlowFiles::iter()
            .filter(|p| p.ends_with(".json"))
            .filter_map(|p| {
                let file = FlowFiles::get(&p)?;
                match parse(&file.data) {
                    Ok(manifest) => {
                        let info = info(&manifest);
                        Some(Template { manifest, info })
                    }
                    Err(e) => {
                        tracing::warn!("flow template {p}: {e}");
                        None
                    }
                }
            })
            .collect();
        list.sort_by_key(|t| {
            let id = &t.manifest.id;
            (
                ORDER.iter().position(|o| o == id).unwrap_or(ORDER.len()),
                id.clone(),
            )
        });
        list
    })
}

/// The gallery, in order.
pub fn list() -> Vec<FlowTemplateInfo> {
    all().iter().map(|t| t.info.clone()).collect()
}

pub(crate) fn manifest(id: &str) -> Option<&'static Manifest> {
    all()
        .iter()
        .find(|t| t.manifest.id == id)
        .map(|t| &t.manifest)
}

/// A node type's name as the UI shows it (`src/lib/components/flows/graph/labels.ts`).
pub fn node_label(ty: &str) -> &str {
    match ty {
        "trigger.cue" => "Cue",
        "trigger.itemStatus" => "Schedule item status",
        "trigger.beforeNext" => "Before next item",
        "trigger.timeOfDay" => "Time of day",
        "trigger.timerZero" => "Timer reaches zero",
        "trigger.layer" => "Layer on/off air",
        "trigger.slideshowWrap" => "Slideshow wrap",
        "trigger.sourceRow" => "Source row changed",
        "trigger.runByFlow" => "Run by flow",
        "data.sourceRow" => "Source row",
        "data.scheduleItem" => "Schedule now/next",
        "data.fieldValue" => "Field value",
        "data.layerState" => "Layer state",
        "data.constant" => "Constant",
        "data.timeNow" => "Time now",
        "data.format" => "Format text",
        "data.math" => "Math",
        "data.compare" => "Compare",
        "logic.branch" => "Branch",
        "logic.switch" => "Switch",
        "logic.gate" => "Gate",
        "logic.delay" => "Delay",
        "logic.waitUntil" => "Wait until",
        "logic.debounce" => "Debounce",
        "logic.throttle" => "Throttle",
        "logic.sequence" => "Sequence",
        "logic.once" => "Once",
        "logic.counter" => "Counter",
        "logic.random" => "Random pick",
        "logic.repeat" => "Repeat",
        "action.take" => "Take",
        "action.clear" => "Clear layer",
        "action.clearScreen" => "Clear screen",
        "action.clearAll" => "Clear all screens",
        "action.setFields" => "Set fields",
        "action.setTimer" => "Set timer",
        "action.stepSource" => "Step source",
        "action.pauseSource" => "Pause/Resume source",
        "action.schedule" => "Schedule action",
        "action.runFlow" => "Run flow",
        "action.stopFlow" => "Stop flow",
        "action.log" => "Log",
        "trigger.mediaEnded" => "Media ends",
        "action.media" => "Media",
        other => other,
    }
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_str()
}

/// The first flow's main path: from its first trigger, each step along the first event wire
/// leaving the node (in edge order).
fn chain(f: &ManifestFlow) -> String {
    let ty = |n: &Value| str_at(n, &["type"]).unwrap_or_default().to_string();
    let Some(start) = f
        .nodes
        .iter()
        .find(|n| ty(n).starts_with("trigger."))
        .or_else(|| f.nodes.first())
    else {
        return String::new();
    };
    let by_id: HashMap<&str, &Value> = f
        .nodes
        .iter()
        .filter_map(|n| Some((str_at(n, &["id"])?, n)))
        .collect();
    let mut labels = vec![node_label(&ty(start)).to_string()];
    let mut seen = HashSet::new();
    let mut cur = str_at(start, &["id"]).unwrap_or_default();
    seen.insert(cur);
    while let Some(next) = f.edges.iter().find_map(|e| {
        (str_at(e, &["kind"]) == Some("event") && str_at(e, &["from", "node"]) == Some(cur))
            .then(|| str_at(e, &["to", "node"]))
            .flatten()
    }) {
        let Some(node) = by_id.get(next) else { break };
        if !seen.insert(next) {
            break;
        }
        labels.push(node_label(&ty(node)).to_string());
        cur = next;
    }
    labels.join(" → ")
}

fn info(m: &Manifest) -> FlowTemplateInfo {
    FlowTemplateInfo {
        id: m.id.clone(),
        name: m.name.clone(),
        description: m.description.clone(),
        category: m.category,
        chain: m.flows.first().map(chain).unwrap_or_default(),
        slots: m.slots.clone(),
        flows: m.flows.len() as u8,
    }
}

// ---- Resolution ------------------------------------------------------------------------------

fn kind_hint(kind: FlowSlotKind) -> &'static str {
    match kind {
        FlowSlotKind::Screen => "a screen",
        FlowSlotKind::Layer => "a layer",
        FlowSlotKind::TextSource => "a text source",
        FlowSlotKind::ScheduleSource => "a schedule",
        FlowSlotKind::Timer => "a timer field",
        FlowSlotKind::Field => "a field",
        FlowSlotKind::Flow => "a flow",
    }
}

/// The placeholder values of every slot (see the module docs).
fn slot_values(
    m: &Manifest,
    given: &BTreeMap<String, String>,
) -> AppResult<HashMap<String, String>> {
    if let Some(k) = given.keys().find(|k| !m.slots.iter().any(|s| &s.key == *k)) {
        return Err(AppError::Invalid(format!("Unknown slot: {k}")));
    }
    let mut out = HashMap::new();
    for s in &m.slots {
        let v = given.get(&s.key).map(|v| v.trim()).unwrap_or_default();
        let parts_n = match s.kind {
            FlowSlotKind::Layer => 2,
            FlowSlotKind::Timer | FlowSlotKind::Field => 3,
            _ => 1,
        };
        let names: &[&str] = match parts_n {
            2 => &["screenId", "layerId"],
            3 => &["screenId", "layerId", "key"],
            _ => &[],
        };
        if v.is_empty() {
            if !s.optional {
                return Err(AppError::Invalid(format!(
                    "{}: pick {}",
                    s.label,
                    kind_hint(s.kind)
                )));
            }
            out.insert(s.key.clone(), String::new());
            for n in names {
                out.insert(format!("{}.{n}", s.key), String::new());
            }
            continue;
        }
        let parts: Vec<&str> = v.split('/').collect();
        let ok = parts.len() == parts_n
            && parts
                .iter()
                .enumerate()
                .all(|(i, p)| if i == 2 { is_key(p) } else { is_id(p) });
        if !ok {
            let shape = match parts_n {
                2 => " (screenId/layerId)",
                3 => " (screenId/layerId/key)",
                _ => "",
            };
            return Err(AppError::Invalid(format!(
                "{}: not {}{shape}: {v}",
                s.label,
                kind_hint(s.kind)
            )));
        }
        out.insert(s.key.clone(), v.to_string());
        for (n, p) in names.iter().zip(&parts) {
            out.insert(format!("{}.{n}", s.key), p.to_string());
        }
    }
    Ok(out)
}

/// Replaces every `{{name}}` in `s`.
fn resolve_str(s: &str, ctx: &HashMap<String, String>) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let j = after
            .find("}}")
            .ok_or_else(|| format!("unterminated placeholder in \"{s}\""))?;
        let name = &after[..j];
        let v = ctx
            .get(name)
            .ok_or_else(|| format!("unknown placeholder {{{{{name}}}}}"))?;
        out.push_str(v);
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn resolve_value(v: &mut Value, ctx: &HashMap<String, String>) -> Result<(), String> {
    match v {
        Value::String(s) => {
            if s.contains("{{") {
                *s = resolve_str(s, ctx)?;
            }
        }
        Value::Array(a) => {
            for x in a {
                resolve_value(x, ctx)?;
            }
        }
        Value::Object(o) => {
            let old = std::mem::take(o);
            for (k, mut x) in old {
                resolve_value(&mut x, ctx)?;
                let k = if k.contains("{{") {
                    resolve_str(&k, ctx)?
                } else {
                    k
                };
                o.insert(k, x);
            }
        }
        _ => {}
    }
    Ok(())
}

fn remap(edge: &mut Value, end: &str, ids: &HashMap<String, String>) -> Result<(), String> {
    let node = edge
        .get_mut(end)
        .and_then(|e| e.get_mut("node"))
        .ok_or_else(|| format!("an edge without \"{end}.node\""))?;
    let old = node.as_str().unwrap_or_default().to_string();
    let new = ids
        .get(&old)
        .ok_or_else(|| format!("an edge names node {old}, which is not in the flow"))?;
    *node = Value::String(new.clone());
    Ok(())
}

/// The flows a template creates with these slot values and flow ids (one per manifest flow),
/// each resolved, given fresh node/edge ids and validated (`check_graph`). Nothing is written.
pub(crate) fn build(
    m: &Manifest,
    req: &AddFlowTemplate,
    flow_ids: &[String],
) -> AppResult<Vec<Flow>> {
    if flow_ids.len() != m.flows.len() {
        return Err(AppError::Internal(
            "flow template: wrong number of ids".into(),
        ));
    }
    let mut ctx = slot_values(m, &req.slots)?;
    for (f, id) in m.flows.iter().zip(flow_ids) {
        if let Some(s) = &f.suffix {
            ctx.insert(format!("flow:{s}"), id.clone());
        }
    }
    let base = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(&m.name)
        .to_string();
    let manifest_err = |e: String| AppError::Internal(bad(&m.id, e));
    let now = fsx::now_rfc3339();
    let mut out = Vec::with_capacity(m.flows.len());
    for (f, id) in m.flows.iter().zip(flow_ids) {
        let name = match &f.suffix {
            Some(s) if m.flows.len() > 1 => format!("{base} · {s}"),
            _ => base.clone(),
        };
        let name = validate_name(&name)?;
        let mut ids: HashMap<String, String> = HashMap::new();
        let mut nodes = f.nodes.clone();
        for n in &mut nodes {
            let old = str_at(n, &["id"])
                .ok_or_else(|| manifest_err("a node without an id".into()))?
                .to_string();
            let fresh = loop {
                let r = random_id();
                if !ids.values().any(|v| *v == r) {
                    break r;
                }
            };
            if ids.insert(old.clone(), fresh.clone()).is_some() {
                return Err(manifest_err(format!("node id {old} twice")));
            }
            n["id"] = Value::String(fresh);
            resolve_value(n, &ctx).map_err(manifest_err)?;
        }
        let mut edges = f.edges.clone();
        for e in &mut edges {
            e["id"] = Value::String(random_id());
            remap(e, "from", &ids).map_err(manifest_err)?;
            remap(e, "to", &ids).map_err(manifest_err)?;
            resolve_value(e, &ctx).map_err(manifest_err)?;
        }
        let leftover = |v: &[Value]| v.iter().any(|x| x.to_string().contains("{{"));
        if leftover(&nodes) || leftover(&edges) {
            return Err(manifest_err("a placeholder is left".into()));
        }
        let nodes: Vec<FlowNode> = serde_json::from_value(Value::Array(nodes))
            .map_err(|e| manifest_err(format!("nodes: {e}")))?;
        let edges: Vec<FlowEdge> = serde_json::from_value(Value::Array(edges))
            .map_err(|e| manifest_err(format!("edges: {e}")))?;
        if let Some(p) = check_graph(&nodes, &edges).into_iter().next() {
            return Err(AppError::Invalid(format!("{name}: {}", p.message)));
        }
        out.push(Flow {
            id: id.clone(),
            name,
            armed: f.armed().map_err(manifest_err)?,
            retrigger: Retrigger::default(),
            origin: None,
            nodes,
            edges,
            draft: None,
            last_fired_at: None,
            updated_at: now.clone(),
        });
    }
    for f in &out {
        flows::validate_flow(f)?;
    }
    Ok(out)
}

impl AppState {
    /// Resolves the template's slots, validates every flow (`check_graph`), creates them (cue-only
    /// flows armed, automatic ones disarmed) and returns them in order. All or nothing.
    pub fn add_flow_template(&self, req: AddFlowTemplate) -> AppResult<Vec<FlowInfo>> {
        let m = manifest(&req.template_id).ok_or_else(|| {
            AppError::NotFound(format!("Flow template not found: {}", req.template_id))
        })?;
        self.add_flow_template_from(m, &req)
    }

    pub(crate) fn add_flow_template_from(
        &self,
        m: &Manifest,
        req: &AddFlowTemplate,
    ) -> AppResult<Vec<FlowInfo>> {
        let dir = self.store_dir()?;
        let ids = self.new_flow_ids(&dir, m.flows.len());
        let created = build(m, req, &ids)?;
        // Same path as a recipe install: every file written or none, then deployed.
        self.write_recipe_flows(&dir, &created)?;
        self.recipe_flows_installed(created);
        ids.iter().map(|id| self.info(id)).collect()
    }
}

#[cfg(test)]
mod tests;
