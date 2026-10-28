//! The remote control state tree (`docs/CONTROL_PROTOCOL.md` §6) built from the views Control
//! already shows, its JSON Pointer diff (§7) and the per-path history the stale check reads
//! (§8.7).
//!
//! The tree is rebuilt whole from in-memory state (no disk reads: assets come from a cached
//! list) and diffed against the last one sent. Objects keyed by ID are diffed key by key; arrays
//! and leaves are replaced whole, so every change has a path.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::{json, Map, Value};

use super::proto::{
    Op, ASSETS_MAX, CUES_MAX, LAYERS_MAX, LIST_VALUE_MAX, NAME_MAX, ROWS_BYTES_MAX, ROWS_MAX,
    SCREENS_MAX, SOURCES_MAX, TEMPLATES_MAX, VALUE_MAX,
};
use crate::core::live::FieldDef;
use crate::core::model::{
    ColumnKind, MediaReadiness, NodeKind, ScreenLayerKind, SourceColumn, SourceDoc, VariableKind,
};
use crate::core::protocol::{FieldType, Fields};
use crate::core::sources;
use crate::core::state::AppState;

/// The one cue list (owner decision B): Control's cue flows in order.
pub const CUE_LIST_ID: &str = "cues";
pub const CUE_LIST_NAME: &str = "Cues";

/// An asset remote users may pick for an image / video field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAsset {
    /// `/assets/<name>`: the value the field gets.
    pub path: String,
    pub name: String,
    /// `image | video`.
    pub kind: &'static str,
}

/// Which client the tree is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Flavor {
    /// Cloud remote control (`docs/CONTROL_PROTOCOL.md` §6.1, exactly).
    #[default]
    Remote,
    /// The local Control API (`docs/CONTROL_API.md`): layer keys `<screen>.<layer>`, logical
    /// `on_air` (take → clear), no take lock or `remote_editable`, plus staged / readiness /
    /// variant / opacity / playback per layer, variants and range variables per template,
    /// running cues, sources and Automation off.
    Local,
}

/// What the tree needs besides the app's state.
#[derive(Debug, Clone, Default)]
pub struct BuildInput {
    pub take_locked: bool,
    /// The last cue run and its position in the order at that time (session state).
    pub last_cue: Option<(String, usize)>,
    pub assets: Vec<RemoteAsset>,
    pub flavor: Flavor,
    /// Protocol 2 state parts the server announced (remote flavor).
    pub parts: Parts,
}

/// Protocol 2 state parts (§4), each sent only when the server announced its feature
/// (`welcome.features`, §1): nothing of them reaches a protocol 1 server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Parts {
    /// `templates.*.variants`, `layers.*.variant` / `variant_next` (§4.1).
    pub variants: bool,
    /// `sources`, `source_order`, `layers.*.sources` (§4.2).
    pub rows: bool,
    /// `studio.take_scope` (§4.4).
    pub studio: bool,
    /// The `timer` command (no state of its own: §4.5).
    pub timers: bool,
    /// `panel` (§4.3) and `press_button`.
    pub panel: bool,
}

impl Parts {
    pub fn of(features: &[String]) -> Self {
        let has = |f: &str| features.iter().any(|x| x == f);
        Self {
            variants: has("variants"),
            rows: has("rows"),
            studio: has("studio"),
            timers: has("timers"),
            panel: has("panel"),
        }
    }
}

/// Local flavor: at most this many sources.
pub const LOCAL_SOURCES_MAX: usize = 256;

/// A cue: its id (the flow id) and the text on its button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueInfo {
    pub id: String,
    pub name: String,
}

/// The project's cue flows (a Cue trigger in the deployed graph), in Control's order (creation
/// order), at most [`CUES_MAX`].
pub fn cues(st: &AppState) -> Vec<CueInfo> {
    st.flows()
        .all()
        .into_iter()
        .filter_map(|f| {
            let label = f.nodes.iter().find_map(|n| match &n.kind {
                NodeKind::Cue { label } => Some(label.clone()),
                _ => None,
            })?;
            let label = label.unwrap_or_default();
            let name = if label.trim().is_empty() {
                f.name.clone()
            } else {
                label.trim().to_string()
            };
            Some(CueInfo {
                id: f.id,
                name: name_of(&name),
            })
        })
        .take(CUES_MAX)
        .collect()
}

/// `current` and `next` of the cue list (see the plan's answer B): `next` is the cue after the
/// last one run (the first before any ran; null after the last). A deleted current cue leaves
/// `current` null and `next` the cue now at its old position.
pub fn current_next(
    order: &[String],
    last: Option<&(String, usize)>,
) -> (Option<String>, Option<String>) {
    match last {
        None => (None, order.first().cloned()),
        Some((id, pos)) => match order.iter().position(|c| c == id) {
            Some(i) => (Some(id.clone()), order.get(i + 1).cloned()),
            None => (None, order.get(*pos).cloned()),
        },
    }
}

/// A media layer's file as the protocol shows it: `image` | `video` and the file's display
/// name (never a path); `None` for audio (or an unknown file).
pub(crate) fn media_of(asset: &str) -> Option<(&'static str, String)> {
    let name = asset.rsplit('/').next().unwrap_or(asset);
    let kind = if crate::core::assets::is_image_name(name) {
        "image"
    } else if crate::core::assets::asset_ext(name)
        .is_some_and(|e| crate::core::model::MEDIA_VIDEO_EXTENSIONS.contains(&e))
    {
        "video"
    } else {
        return None;
    };
    Some((kind, name_of(name)))
}

/// A layer's key in the tree: its own id, unless another screen's layer already has that id
/// (layer ids are unique per screen only): then `<screen>_<layer>`. Commands map a key back
/// with [`local_layer_id`].
fn layer_key(screen: &str, layer: &str, taken: &Map<String, Value>) -> String {
    if taken.contains_key(layer) {
        format!("{screen}_{layer}")
    } else {
        layer.to_string()
    }
}

/// A layer's key in the local tree: `<screen>.<layer>` (ids never contain `.`).
pub fn local_key(screen: &str, layer: &str) -> String {
    format!("{screen}.{layer}")
}

/// The screen and layer ids of a local tree key; `None` when it isn't one.
pub fn split_local_key(key: &str) -> Option<(&str, &str)> {
    let (s, l) = key.split_once('.')?;
    (crate::core::templates::is_id(s) && crate::core::templates::is_id(l)).then_some((s, l))
}

/// The screen's own layer id for a tree key (see [`layer_key`]).
pub fn local_layer_id<'a>(key: &'a str, screen: &str) -> &'a str {
    key.strip_prefix(screen)
        .and_then(|r| r.strip_prefix('_'))
        .filter(|r| !r.is_empty())
        .unwrap_or(key)
}

/// A name as the protocol takes it: trimmed, 1–100 characters ("Untitled" when empty).
pub fn name_of(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        "Untitled".into()
    } else {
        t.chars().take(NAME_MAX).collect()
    }
}

fn field_json(d: &FieldDef) -> Value {
    let mut m = Map::new();
    m.insert("key".into(), json!(d.key));
    m.insert("label".into(), json!(name_of(&d.label)));
    m.insert(
        "type".into(),
        serde_json::to_value(d.kind).unwrap_or(json!("text")),
    );
    if d.kind == FieldType::List {
        let cols: Vec<Value> = d
            .columns
            .iter()
            .map(|c| json!({"key": c.key, "label": name_of(&c.label)}))
            .collect();
        m.insert("columns".into(), Value::Array(cols));
    }
    if d.kind == FieldType::Clock {
        m.insert(
            "format".into(),
            json!(d.format.clone().unwrap_or_else(|| "HH:mm:ss".into())),
        );
    }
    Value::Object(m)
}

/// Builds the whole tree (§6.1). Reads memory only; call from a thread that may block briefly
/// (the screen store's lock, a template not loaded yet).
pub fn build(st: &AppState, input: &BuildInput) -> Value {
    let local = input.flavor == Flavor::Local;
    let hub = st.live();
    // Exact visibility (fades) is remote control's; the local tree uses the logical state.
    let visible = if local {
        Default::default()
    } else {
        hub.refresh_fades();
        hub.visible_layers()
    };
    let studio = hub.is_studio();
    let dir = st.project_dir().ok();
    let screens_max = if local { usize::MAX } else { SCREENS_MAX };

    let mut screens = Map::new();
    let mut screen_order = Vec::new();
    let mut templates = Map::new();
    let mut layers = Map::new();
    let mut preview = Map::new();
    // Protocol 2 rows (§4.2): the table sources (text and API; schedules and polls have no row
    // to pick) and those whose current row feeds a listed layer's field.
    let tables: Option<HashMap<String, SourceDoc>> = (!local && input.parts.rows).then(|| {
        st.sources()
            .docs()
            .into_iter()
            .filter(|d| d.meta.kind.table().is_some())
            .map(|d| (d.id.clone(), d))
            .collect()
    });
    let mut row_sources: BTreeSet<String> = BTreeSet::new();

    for view in hub.user_screen_views().into_iter().take(screens_max) {
        let sid = view.screen.id.clone();
        let doc = st.screens().doc(&sid);
        let loaded = match (&dir, &doc) {
            (Some(d), Some(doc)) => st.screens().templates_for(d, doc),
            _ => Default::default(),
        };
        let pv = if studio {
            hub.preview_view(&sid).ok()
        } else {
            None
        };
        let mut ids = Vec::new();
        // Local layer id → its key in the tree (see `layer_key`), for Preview below.
        let mut keys: Vec<(String, String)> = Vec::new();
        // Views are bottom first; the protocol lists layers top first.
        for lv in view.layers.iter().rev() {
            if layers.len() >= LAYERS_MAX {
                break;
            }
            let key = if local {
                local_key(&sid, &lv.id)
            } else {
                layer_key(&sid, &lv.id, &layers)
            };
            let Some(dl) = doc
                .as_ref()
                .and_then(|d| d.meta.layers.iter().find(|l| l.id == lv.id))
            else {
                continue;
            };
            let on_air = visible
                .get(&(sid.clone(), lv.id.clone()))
                .copied()
                .unwrap_or(lv.on_air);
            let (template_id, bindings) = match &dl.kind {
                ScreenLayerKind::Template {
                    template_id,
                    bindings,
                    ..
                } => (template_id, bindings),
                ScreenLayerKind::Media { media, opacity } if local => {
                    let (kind, name) = media_of(&media.asset).unwrap_or_else(|| {
                        let file = media.asset.rsplit('/').next().unwrap_or(&media.asset);
                        ("audio", name_of(file))
                    });
                    let mut m = json!({"type": kind, "name": name});
                    if let Some(mv) = &lv.media {
                        m["playback"] = json!({
                            "state": mv.playback.state,
                            "pos_ms": mv.playback.pos_ms,
                            "at": mv.playback.at,
                            "in_ms": media.in_ms,
                            "out_ms": media.out_ms,
                            "duration_ms": mv.duration_ms,
                        });
                    }
                    let mut obj = json!({
                        "kind": "media",
                        "screen": sid,
                        "name": name_of(&dl.name),
                        "on_air": on_air,
                        "media": m,
                        "opacity": opacity.unwrap_or(100),
                    });
                    obj["ready"] = ready_json(lv.media.as_ref().map(|m| &m.readiness));
                    layers.insert(key.clone(), obj);
                    ids.push(key.clone());
                    keys.push((lv.id.clone(), key));
                    continue;
                }
                // Read-only, with its on-air state (§6.1 `kind: "media"`). Audio layers have
                // nothing to show and no protocol type: left out.
                ScreenLayerKind::Media { media, .. } => {
                    if let Some((kind, name)) = media_of(&media.asset) {
                        layers.insert(
                            key.clone(),
                            json!({
                                "kind": "media",
                                "screen": sid,
                                "name": name_of(&dl.name),
                                "on_air": on_air,
                                "media": {"type": kind, "name": name},
                            }),
                        );
                        ids.push(key.clone());
                        keys.push((lv.id.clone(), key));
                    }
                    continue;
                }
            };
            // In Studio mode the values are Preview's: what the next take shows, as Control.
            let shown = pv
                .as_ref()
                .and_then(|p| p.layers.iter().find(|l| l.id == lv.id))
                .unwrap_or(lv);
            let defs: &[FieldDef] = match loaded.get(template_id.as_str()) {
                Some(t) => &t.meta.fields,
                None => &shown.field_defs,
            };
            if !templates.contains_key(template_id) {
                if templates.len() >= TEMPLATES_MAX {
                    continue;
                }
                let lt = loaded.get(template_id.as_str());
                let name = lt
                    .map(|t| t.meta.name.clone())
                    .unwrap_or_else(|| template_id.clone());
                let mut t = json!({
                    "name": name_of(&name),
                    "fields": defs.iter().map(field_json).collect::<Vec<_>>(),
                });
                let variants: Vec<Value> = lt
                    .map(|t| t.variants.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|v| json!({"id": v.id, "name": name_of(&v.name)}))
                    .collect();
                if !local && input.parts.variants {
                    t["variants"] = Value::Array(variants.clone());
                }
                if local {
                    let ranges: Vec<Value> = lt
                        .map(|t| t.meta.variables.as_slice())
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|v| match &v.kind {
                            VariableKind::Range { min, max, step, .. } => Some(json!({
                                "key": v.key, "label": name_of(&v.label),
                                "min": min, "max": max, "step": step,
                            })),
                            _ => None,
                        })
                        .collect();
                    t["variants"] = Value::Array(variants);
                    t["ranges"] = Value::Array(ranges);
                }
                templates.insert(template_id.clone(), t);
            }
            let mut editable = true;
            let mut values = Map::new();
            for d in defs.iter().filter(|d| d.kind != FieldType::Clock) {
                let v = shown
                    .fields
                    .get(&d.key)
                    .cloned()
                    .unwrap_or_else(|| d.default.clone());
                let max = if d.kind == FieldType::List {
                    LIST_VALUE_MAX
                } else {
                    VALUE_MAX
                };
                if v.len() > max {
                    // Too big to send: shown empty and kept local.
                    editable = false;
                    values.insert(d.key.clone(), json!(""));
                } else {
                    values.insert(d.key.clone(), Value::String(v));
                }
            }
            let bound: Vec<&String> = bindings
                .keys()
                .filter(|k| defs.iter().any(|d| &d.key == *k))
                .collect();
            let mut obj = json!({
                "kind": "template",
                "screen": sid,
                "name": name_of(&dl.name),
                "template": template_id,
                "on_air": on_air,
                "values": values,
                "bound": bound,
            });
            if local {
                let opacity = match &dl.kind {
                    ScreenLayerKind::Template { opacity, .. } => opacity.unwrap_or(100),
                    ScreenLayerKind::Media { .. } => 100,
                };
                obj["pending"] = json!(shown.pending);
                obj["variant"] = json!(shown.variant);
                obj["opacity"] = json!(opacity);
                obj["ready"] = ready_json(lv.readiness.as_ref());
            } else {
                obj["remote_editable"] = json!(editable);
                if input.parts.variants {
                    // In Studio mode Preview's variant, like the values; otherwise what the
                    // output shows, with a change staged for the next take as `variant_next`.
                    if studio {
                        obj["variant"] = json!(shown.variant);
                    } else {
                        obj["variant"] = json!(lv.on_air_variant);
                        if lv.on_air && lv.pending && lv.variant != lv.on_air_variant {
                            obj["variant_next"] = json!(lv.variant);
                        }
                    }
                }
                if let Some(tables) = &tables {
                    let mut used: Vec<&String> = bindings
                        .iter()
                        .filter(|(k, b)| {
                            !b.column.is_empty()
                                && tables.contains_key(&b.source_id)
                                && defs
                                    .iter()
                                    .any(|d| &d.key == *k && d.kind != FieldType::List)
                        })
                        .map(|(_, b)| &b.source_id)
                        .collect();
                    used.sort();
                    used.dedup();
                    row_sources.extend(used.iter().map(|s| (*s).clone()));
                    obj["sources"] = json!(used);
                }
            }
            layers.insert(key.clone(), obj);
            ids.push(key.clone());
            keys.push((lv.id.clone(), key));
        }
        if let Some(p) = &pv {
            let cued: Vec<&String> = p
                .layers
                .iter()
                .rev()
                .filter(|l| l.on_air)
                .filter_map(|l| keys.iter().find(|(id, _)| *id == l.id).map(|(_, k)| k))
                .collect();
            preview.insert(sid.clone(), json!(cued));
        }
        screens.insert(
            sid.clone(),
            json!({"name": name_of(&view.screen.name), "layers": ids}),
        );
        screen_order.push(sid);
    }

    let cue_list = cues(st);
    let order: Vec<String> = cue_list.iter().map(|c| c.id.clone()).collect();
    let (current, next) = current_next(&order, input.last_cue.as_ref());
    let cue_map: Map<String, Value> = cue_list
        .iter()
        .map(|c| {
            let mut cue = json!({"name": c.name, "note": ""});
            if local {
                cue["running"] = json!(st.flow_engine().is_running(&c.id));
            }
            (c.id.clone(), cue)
        })
        .collect();

    let assets: Vec<Value> = input
        .assets
        .iter()
        .take(ASSETS_MAX)
        .map(|a| json!({"path": a.path, "name": a.name, "kind": a.kind}))
        .collect();

    let mut tree = json!({
        "screens": screens,
        "screen_order": screen_order,
        "templates": templates,
        "layers": layers,
        "cue_lists": {
            CUE_LIST_ID: {
                "name": CUE_LIST_NAME,
                "cues": cue_map,
                "order": order,
                "current": current,
                "next": next,
            }
        },
        "cue_list_order": [CUE_LIST_ID],
        "studio_mode": studio,
        "preview": preview,
        "assets": assets,
    });
    if local {
        let (sources, order) = local_sources(st);
        tree["sources"] = Value::Object(sources);
        tree["source_order"] = json!(order);
        tree["automation_off"] = json!(st.flow_engine().automation_off());
        tree["api_flows"] = Value::Object(local_api_flows(st));
    } else {
        tree["remote"] = json!({"take_locked": input.take_locked});
        if let Some(tables) = &tables {
            let (sources, order) = remote_sources(tables, &row_sources);
            // A layer only names sources the tree lists (more than SOURCES_MAX are left out).
            if row_sources.len() > sources.len() {
                for l in tree["layers"].as_object_mut().into_iter().flatten() {
                    if let Some(a) = l.1.get_mut("sources").and_then(Value::as_array_mut) {
                        a.retain(|s| sources.contains_key(s.as_str().unwrap_or("")));
                    }
                }
            }
            tree["sources"] = Value::Object(sources);
            tree["source_order"] = json!(order);
        }
        if input.parts.studio {
            tree["studio"] = json!({"take_scope": "screen"});
        }
        if input.parts.panel {
            tree["panel"] = remote_panel(st);
        }
    }
    tree
}

/// Protocol 2 button panel (§4.3): what the control site shows; never the actions.
fn remote_panel(st: &AppState) -> Value {
    let doc = st.panel_doc().unwrap_or_default();
    let buttons: Map<String, Value> = doc
        .buttons
        .iter()
        .map(|b| {
            (
                b.id.clone(),
                json!({
                    "label": b.label,
                    "color": b.color,
                    "confirm": b.confirm,
                    "on_air": st.panel_button_on_air(b),
                }),
            )
        })
        .collect();
    let order: Vec<&String> = doc.buttons.iter().map(|b| &b.id).collect();
    json!({"buttons": buttons, "order": order})
}

/// Protocol 2 rows (§4.2): the sources in `used` in name order (≤ [`SOURCES_MAX`]), each with
/// its rows (≤ [`ROWS_MAX`], and together ≤ [`ROWS_BYTES_MAX`]; more → `truncated`) and the
/// id of its current row.
fn remote_sources(
    tables: &HashMap<String, SourceDoc>,
    used: &BTreeSet<String>,
) -> (Map<String, Value>, Vec<String>) {
    let mut docs: Vec<&SourceDoc> = used.iter().filter_map(|id| tables.get(id)).collect();
    docs.sort_by(|a, b| {
        a.meta
            .name
            .to_lowercase()
            .cmp(&b.meta.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut budget = ROWS_BYTES_MAX;
    let mut map = Map::new();
    let mut order = Vec::new();
    for doc in docs.into_iter().take(SOURCES_MAX) {
        let Some(t) = doc.meta.kind.table() else {
            continue;
        };
        let ids = row_ids(t.columns, t.rows);
        let mut rows = Vec::new();
        for (i, (id, row)) in ids.iter().zip(t.rows).enumerate().take(ROWS_MAX) {
            let label = row_label(t.columns, row, i);
            // `{"id":"…","label":"…"},` around them: 21 bytes; a label may be escaped.
            let cost = id.len() + label.len() + 24;
            if cost > budget {
                break;
            }
            budget -= cost;
            rows.push(json!({"id": id, "label": label}));
        }
        let current = ids.get(t.index as usize).cloned();
        map.insert(
            doc.id.clone(),
            json!({
                "name": name_of(&doc.meta.name),
                "truncated": rows.len() < t.rows.len(),
                "rows": rows,
                "current": current,
            }),
        );
        order.push(doc.id.clone());
    }
    (map, order)
}

/// Row ids (protocol 2 §1): a short hash of the row's cells in column order, `-<n>` for the
/// n-th identical row. The same across runs and reorders while the row is unchanged.
pub fn row_ids(columns: &[SourceColumn], rows: &[Fields]) -> Vec<String> {
    let mut seen: HashMap<u64, u32> = HashMap::new();
    rows.iter()
        .map(|row| {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            let mut mix = |bytes: &[u8]| {
                for b in bytes.iter().chain([&0xffu8]) {
                    h ^= u64::from(*b);
                    h = h.wrapping_mul(0x0100_0000_01b3);
                }
            };
            for c in columns {
                mix(c.key.as_bytes());
                mix(row.get(&c.key).map_or("", String::as_str).as_bytes());
            }
            let n = seen.entry(h).or_insert(0);
            *n += 1;
            let id = format!("{:012x}", h & 0xffff_ffff_ffff);
            if *n == 1 {
                id
            } else {
                format!("{id}-{n}")
            }
        })
        .collect()
}

/// A row's label: its first two text columns' values joined, as Control's sources strip shows
/// the current row; "Row N" when both are empty.
pub(crate) fn row_label(columns: &[SourceColumn], row: &Fields, index: usize) -> String {
    let parts: Vec<&str> = columns
        .iter()
        .filter(|c| c.kind != ColumnKind::Image)
        .take(2)
        .filter_map(|c| row.get(&c.key).map(|v| v.trim()))
        .filter(|v| !v.is_empty())
        .collect();
    if parts.is_empty() {
        format!("Row {}", index + 1)
    } else {
        name_of(&parts.join(" · "))
    }
}

/// Local flavor: flows with an API call trigger (`trigger.rest`), which `run_flow` may start.
fn local_api_flows(st: &AppState) -> Map<String, Value> {
    st.flows()
        .all()
        .into_iter()
        .filter(|f| f.nodes.iter().any(|n| matches!(n.kind, NodeKind::Rest {})))
        .map(|f| (f.id, json!({"name": name_of(&f.name), "armed": f.armed})))
        .collect()
}

/// Local flavor: a layer's preload status, `{loaded, total}` (null when it has none).
fn ready_json(r: Option<&MediaReadiness>) -> Value {
    match r {
        Some(r) => json!({"loaded": r.ready, "total": r.total}),
        None => Value::Null,
    }
}

/// Local flavor: the project's sources (name order) with their position and the first
/// column of the current row as a label.
fn local_sources(st: &AppState) -> (Map<String, Value>, Vec<String>) {
    let now = st.sources().clock().now();
    let mut docs = st.sources().docs();
    docs.sort_by(|a, b| {
        a.meta
            .name
            .to_lowercase()
            .cmp(&b.meta.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut map = Map::new();
    let mut order = Vec::new();
    for doc in docs.iter().take(LOCAL_SOURCES_MAX) {
        let (count, index) = sources::position(doc);
        let row = sources::current_row(doc, &now);
        let label: String = sources::columns_of(doc)
            .first()
            .and_then(|c| row.get(&c.key))
            .map(|v| v.trim().chars().take(NAME_MAX).collect())
            .unwrap_or_default();
        map.insert(
            doc.id.clone(),
            json!({
                "name": name_of(&doc.meta.name),
                "kind": sources::kind_name(doc),
                "index": index,
                "count": count,
                "label": label,
            }),
        );
        order.push(doc.id.clone());
    }
    (map, order)
}

// ---- Diff ----------------------------------------------------------------------------------

/// One JSON Pointer token (RFC 6901).
pub fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// The ops turning `old` into `new`: objects key by key (deletes first, then puts in key
/// order), anything else replaced whole.
pub fn diff(old: &Value, new: &Value) -> Vec<Op> {
    let mut ops = Vec::new();
    diff_at(old, new, &mut String::new(), &mut ops);
    ops
}

fn diff_at(old: &Value, new: &Value, path: &mut String, ops: &mut Vec<Op>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            let len = path.len();
            for k in a.keys().filter(|k| !b.contains_key(*k)) {
                path.push('/');
                path.push_str(&escape(k));
                ops.push(Op::Del { path: path.clone() });
                path.truncate(len);
            }
            for (k, vb) in b {
                path.push('/');
                path.push_str(&escape(k));
                match a.get(k) {
                    Some(va) => diff_at(va, vb, path, ops),
                    None => ops.push(Op::Put {
                        path: path.clone(),
                        value: vb.clone(),
                    }),
                }
                path.truncate(len);
            }
        }
        _ if old == new => {}
        _ => ops.push(Op::Put {
            path: path.clone(),
            value: new.clone(),
        }),
    }
}

/// The tree as the server last got it, its `seq`, and when each path last changed.
#[derive(Debug, Default)]
pub struct Model {
    pub tree: Value,
    /// The `seq` of the last snapshot or diff sent.
    pub seq: u64,
    /// The `seq` of the last snapshot: the history starts there.
    pub snapshot_seq: u64,
    sent_any: bool,
    /// Path → the `seq` of the last diff that changed it (since the last snapshot).
    history: HashMap<String, u64>,
}

impl Model {
    /// A snapshot of `tree` goes out: returns its `seq` (0 for the first of a connection, then
    /// the next number) and resets the history.
    pub fn snapshot(&mut self, tree: Value) -> u64 {
        let seq = if self.sent_any { self.seq + 1 } else { 0 };
        self.sent_any = true;
        self.seq = seq;
        self.snapshot_seq = seq;
        self.history.clear();
        self.tree = tree;
        seq
    }

    /// Whether a snapshot was sent on this connection (diffs need one first).
    pub fn has_snapshot(&self) -> bool {
        self.sent_any
    }

    /// Records a diff of `ops` turning the tree into `tree`; returns its `seq`.
    pub fn commit_diff(&mut self, tree: Value, ops: &[Op]) -> u64 {
        self.seq += 1;
        for op in ops {
            self.history.insert(op.path().to_string(), self.seq);
        }
        self.tree = tree;
        self.seq
    }

    /// Whether the value at `path` changed after `seen_seq` (§8.7): a diff touched the path or
    /// one of its ancestors after it, or `seen_seq` is older than the history (before the last
    /// snapshot).
    pub fn changed_since(&self, path: &str, seen_seq: u64) -> bool {
        if seen_seq < self.snapshot_seq {
            return true;
        }
        let mut end = path.len();
        loop {
            if self
                .history
                .get(&path[..end])
                .is_some_and(|s| *s > seen_seq)
            {
                return true;
            }
            match path[..end].rfind('/') {
                Some(i) if i > 0 => end = i,
                _ => return false,
            }
        }
    }

    /// The value at a pointer path of the last tree sent.
    pub fn at(&self, path: &str) -> Option<&Value> {
        self.tree.pointer(path)
    }
}

/// Every `[A-Za-z0-9_-]{1,64}` id the tree refers to exists (§6.1); `Err` names the first
/// broken reference. Tests and a debug check before sending.
pub fn check_refs(tree: &Value) -> Result<(), String> {
    let obj = |k: &str| tree.get(k).and_then(Value::as_object);
    let screens = obj("screens").ok_or("screens")?;
    let layers = obj("layers").ok_or("layers")?;
    let templates = obj("templates").ok_or("templates")?;
    let id_ok = |s: &str| {
        (1..=64).contains(&s.len())
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    for (sid, s) in screens {
        if !id_ok(sid) {
            return Err(format!("screen id {sid}"));
        }
        for l in s["layers"].as_array().into_iter().flatten() {
            if !layers.contains_key(l.as_str().unwrap_or("")) {
                return Err(format!("screen {sid} layer {l}"));
            }
        }
    }
    for (lid, l) in layers {
        let screen = l["screen"].as_str().unwrap_or("");
        let key_ok = id_ok(lid) || split_local_key(lid).is_some_and(|(s, _)| s == screen);
        if !key_ok || !screens.contains_key(screen) {
            return Err(format!("layer {lid} screen"));
        }
        if l["kind"] == "media" {
            // No template, values, bound or remote_editable on a media layer.
            for k in ["template", "values", "bound", "remote_editable"] {
                if l.get(k).is_some() {
                    return Err(format!("media layer {lid} has {k}"));
                }
            }
            continue;
        }
        let Some(t) = templates.get(l["template"].as_str().unwrap_or("")) else {
            return Err(format!("layer {lid} template"));
        };
        // Protocol 2: the variant shown and the staged one are the template's.
        if let Some(vs) = t["variants"].as_array().filter(|v| !v.is_empty()) {
            for k in ["variant", "variant_next"] {
                if let Some(v) = l[k].as_str().filter(|v| !v.is_empty()) {
                    if !vs.iter().any(|x| x["id"] == v) {
                        return Err(format!("layer {lid} {k} {v}"));
                    }
                }
            }
        }
        for s in l["sources"].as_array().into_iter().flatten() {
            if obj("sources").is_none_or(|m| !m.contains_key(s.as_str().unwrap_or(""))) {
                return Err(format!("layer {lid} source {s}"));
            }
        }
    }
    for (id, s) in obj("sources").into_iter().flatten() {
        let Some(rows) = s["rows"].as_array() else {
            continue;
        };
        if let Some(c) = s["current"].as_str() {
            if s["truncated"] != true && !rows.iter().any(|r| r["id"] == c) {
                return Err(format!("source {id} current"));
            }
        }
    }
    for s in tree["source_order"].as_array().into_iter().flatten() {
        if obj("sources").is_none_or(|m| !m.contains_key(s.as_str().unwrap_or(""))) {
            return Err(format!("source_order {s}"));
        }
    }
    for s in tree["screen_order"].as_array().into_iter().flatten() {
        if !screens.contains_key(s.as_str().unwrap_or("")) {
            return Err(format!("screen_order {s}"));
        }
    }
    let lists = obj("cue_lists").ok_or("cue_lists")?;
    for (id, list) in lists {
        let cues = list["cues"].as_object().ok_or("cues")?;
        for c in list["order"].as_array().into_iter().flatten() {
            if !cues.contains_key(c.as_str().unwrap_or("")) {
                return Err(format!("cue list {id} order {c}"));
            }
        }
        for k in ["current", "next"] {
            if let Some(c) = list[k].as_str() {
                if !cues.contains_key(c) {
                    return Err(format!("cue list {id} {k}"));
                }
            }
        }
    }
    for s in tree["cue_list_order"].as_array().into_iter().flatten() {
        if !lists.contains_key(s.as_str().unwrap_or("")) {
            return Err(format!("cue_list_order {s}"));
        }
    }
    let previews: BTreeMap<String, Value> = tree["preview"]
        .as_object()
        .map(|m| m.clone().into_iter().collect())
        .unwrap_or_default();
    for (sid, ids) in previews {
        if !screens.contains_key(&sid) {
            return Err(format!("preview {sid}"));
        }
        for l in ids.as_array().into_iter().flatten() {
            if !layers.contains_key(l.as_str().unwrap_or("")) {
                return Err(format!("preview {sid} layer {l}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_puts_and_deletes_by_path() {
        let a = json!({"layers":{"l1":{"on_air":false,"values":{"name":"A","x/y":"1"}},"l9":{}},
            "screen_order":["s1"]});
        let b = json!({"layers":{"l1":{"on_air":true,"values":{"name":"B","x/y":"1"}},"l2":{"n":1}},
            "screen_order":["s1","s2"]});
        let ops = diff(&a, &b);
        assert_eq!(
            ops,
            vec![
                Op::Del {
                    path: "/layers/l9".into()
                },
                Op::Put {
                    path: "/layers/l1/on_air".into(),
                    value: json!(true)
                },
                Op::Put {
                    path: "/layers/l1/values/name".into(),
                    value: json!("B")
                },
                Op::Put {
                    path: "/layers/l2".into(),
                    value: json!({"n":1})
                },
                Op::Put {
                    path: "/screen_order".into(),
                    value: json!(["s1", "s2"])
                },
            ]
        );
        assert!(diff(&b, &b).is_empty());
        assert_eq!(escape("a/b~c"), "a~1b~0c");
    }

    #[test]
    fn history_answers_the_stale_check() {
        let mut m = Model::default();
        assert_eq!(m.snapshot(json!({})), 0);
        let s1 = m.commit_diff(
            json!({}),
            &[Op::Put {
                path: "/layers/l1/values/name".into(),
                value: json!("x"),
            }],
        );
        assert_eq!(s1, 1);
        let s2 = m.commit_diff(
            json!({}),
            &[Op::Put {
                path: "/layers/l2".into(),
                value: json!({}),
            }],
        );
        assert!(m.changed_since("/layers/l1/values/name", 0));
        assert!(!m.changed_since("/layers/l1/values/name", 1));
        assert!(!m.changed_since("/layers/l1/values/title", 0));
        // A whole layer put changes every path under it.
        assert!(m.changed_since("/layers/l2/values/name", 1));
        assert!(!m.changed_since("/layers/l2/values/name", s2));
        // A snapshot resets the history: anything seen before it is stale.
        let s3 = m.snapshot(json!({}));
        assert_eq!(s3, 3);
        assert!(m.changed_since("/layers/l9/on_air", 2));
        assert!(!m.changed_since("/layers/l9/on_air", 3));
    }

    #[test]
    fn current_and_next_follow_the_last_cue_run() {
        let order: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(current_next(&order, None), (None, Some("a".into())));
        let at = |id: &str, pos| Some((id.to_string(), pos));
        assert_eq!(
            current_next(&order, at("a", 0).as_ref()),
            (Some("a".into()), Some("b".into()))
        );
        assert_eq!(
            current_next(&order, at("c", 2).as_ref()),
            (Some("c".into()), None)
        );
        // `b` deleted: the cue now at its place is next.
        let without_b: Vec<String> = vec!["a".into(), "c".into()];
        assert_eq!(
            current_next(&without_b, at("b", 1).as_ref()),
            (None, Some("c".into()))
        );
        assert_eq!(current_next(&[], None), (None, None));
    }

    fn text_source(id: &str, rows: Vec<Fields>) -> SourceDoc {
        let mut doc: SourceDoc = serde_json::from_value(json!({
            "id": id, "schemaVersion": 1, "name": id, "kind": "text",
            "columns": [{"key": "a", "label": "A"}, {"key": "b", "label": "B"}],
            "rows": [], "index": 0, "updatedAt": "",
        }))
        .unwrap();
        if let Some(t) = doc.meta.kind.table_mut() {
            *t.rows = rows;
        }
        doc
    }

    fn cells(a: &str, b: &str) -> Fields {
        Fields::from([("a".into(), a.into()), ("b".into(), b.into())])
    }

    #[test]
    fn row_ids_follow_the_content_not_the_position() {
        let doc = text_source("s", vec![cells("x", "1"), cells("y", "2"), cells("x", "1")]);
        let t = doc.meta.kind.table().unwrap();
        let ids = row_ids(t.columns, t.rows);
        assert_eq!(
            ids[2],
            format!("{}-2", ids[0]),
            "identical rows get an occurrence"
        );
        assert_ne!(ids[0], ids[1]);
        assert!(ids.iter().all(|i| i.len() >= 12 && i.is_ascii()));
        // Reordered: each row keeps its id; edited: a new one.
        let moved = text_source("s", vec![cells("y", "2"), cells("x", "1")]);
        let t = moved.meta.kind.table().unwrap();
        assert_eq!(row_ids(t.columns, t.rows), [ids[1].clone(), ids[0].clone()]);
        let edited = text_source("s", vec![cells("x", "1!")]);
        let t = edited.meta.kind.table().unwrap();
        assert_ne!(row_ids(t.columns, t.rows)[0], ids[0]);
    }

    #[test]
    fn rows_are_capped_per_source_and_in_bytes() {
        let long = "w".repeat(90);
        let big = |id: &str| {
            text_source(
                id,
                (0..1500)
                    .map(|i| cells(&format!("{long}{i}"), ""))
                    .collect(),
            )
        };
        let tables: HashMap<String, SourceDoc> = (0..6)
            .map(|i| format!("s{i}"))
            .map(|id| (id.clone(), big(&id)))
            .collect();
        let used: BTreeSet<String> = tables.keys().cloned().collect();
        let (map, order) = remote_sources(&tables, &used);
        assert_eq!(order.len(), 6);
        let mut total = 0;
        for s in map.values() {
            let rows = s["rows"].as_array().unwrap();
            assert!(rows.len() <= ROWS_MAX);
            assert_eq!(s["truncated"], true);
            total += serde_json::to_string(rows).unwrap().len();
        }
        assert!(total <= ROWS_BYTES_MAX, "{total}");
        assert_eq!(map["s0"]["rows"].as_array().unwrap().len(), ROWS_MAX);
    }
}
