//! Flow files `flows/<id>.json`: the in-memory store, validation (with the `core::flow_nodes`
//! port table), migration of recipe rules and `macros.json`, coalesced writes of `armed` /
//! `lastFiredAt` / drafts, and watcher reloads. The engine (`core::flow_engine`) is told about
//! every deployed graph through the `engine_*` hooks.
//!
//! # File
//! `{ "schemaVersion": 1, "id", "name", "armed", "retrigger", "origin", "nodes", "edges",
//! "draft"?, "lastFiredAt", "updatedAt" }` (the id is also the file name, which wins). Written
//! atomically through `fsx` (so the watcher ignores Kinetra's own writes). A file with a newer
//! `schemaVersion` is loaded read-only: it is listed and runs, but is never written (every
//! mutation answers `invalid`). An unparseable file is set aside (`<file>.corrupt-<ts>`,
//! `store-recovered`). A file that parses but whose deployed graph fails validation is listed
//! with the error as a warning and never reaches the engine.
//!
//! # Validation
//! [`validate_graph`] (save, deploy, load, watcher): limits, node/edge ids, per-node config,
//! wires from an existing output to an existing input of the same kind with compatible data
//! types, one wire per data input, at most one Cue, no data cycles, event cycles only through a
//! Delay, Wait until, Debounce, Throttle or Repeat, no event path from a Repeat's `body` back
//! into its own `in`, and ([`validate_graph_of`], when the flow's id is known) no Run flow of the
//! flow itself. [`check_graph`] (the editor's `check_flow`) reports every
//! error with its node instead of the first. [`validate_draft`] (editor autosave) checks limits and
//! ids only, so a half-wired draft can be saved. Missing targets are soft: [`AppState::flow_warnings`].
//!
//! # Writes
//! Every write holds the store's write lock (a serialization lock, held across the file write)
//! and snapshots memory under the data lock, which is never held across I/O. Structural changes
//! (create, save, deploy, rename, duplicate, delete, discard) are written at once; `armed`,
//! `lastFiredAt` and drafts mark the flow dirty and the content flusher writes it (≤ 1 write
//! per ~0.8 s).
//!
//! # Migration (on project open, after a `pre-migrate` backup; idempotent)
//! Each rule of a `schemaVersion: 1` recipe instance becomes a flow ([`rule_to_flow`]) and the
//! instance is rewritten as `schemaVersion: 2` with `flowIds`; each macro of a `schemaVersion: 1`
//! `macros.json` becomes a Cue flow with the same id ([`macro_to_flow`]) and the file is
//! rewritten as `{ "schemaVersion": 2, "migratedTo": "flows" }`. Hotkeys `macro/<id>` become
//! `flow/<id>` on every open.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::Value;

use crate::core::events::names;
use crate::core::flow_nodes::{self, data_type_name, type_name};
use crate::core::fsx;
use crate::core::localtime;
use crate::core::macros;
use crate::core::model::{
    BackupReason, FileChange, FileKind, FilesChanged, Flow, FlowCheck, FlowDraft, FlowEdge,
    FlowInfo, FlowIssue, FlowNode, ItemPick, Macro, MacroStep, NewFlow, NodeKind, NodePos,
    PauseMode, PortRef, Retrigger, Rule, RuleAction, SaveFlow, ScheduleOp, TimerAction, Trigger,
    WireKind, FLOWS_MAX, FLOW_BRANCHES_MAX, FLOW_CASES_MAX, FLOW_DELAY_MAX_MS, FLOW_EDGES_MAX,
    FLOW_NAME_MAX, FLOW_NODES_MAX, FLOW_REPEAT_MAX,
};
use crate::core::persist::{self, Recovered};
use crate::core::recipes::{self, BEFORE_NEXT_MAX_MIN, TAKE_FOR_MAX_MS};
use crate::core::screens::VALUE_MAX;
use crate::core::sources;
use crate::core::state::AppState;
use crate::core::templates::{is_id, is_key, random_id};
use crate::error::{AppError, AppResult};

pub const FLOWS_DIR: &str = "flows";
/// `schemaVersion` written by this build.
pub const FLOW_VERSION: u32 = 1;
/// Longest node / edge id.
pub const FLOW_ITEM_ID_MAX: usize = 32;
/// Longest Cue label, in characters.
pub const CUE_LABEL_MAX: usize = 100;
/// Horizontal spacing of migrated / installed linear flows.
pub const LINEAR_STEP_X: f64 = 240.0;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(msg: impl Into<String>) -> AppError {
    AppError::Invalid(msg.into())
}

// ---- Store -----------------------------------------------------------------------------------

struct Entry {
    flow: Flow,
    /// From a newer app version: runs, never written.
    read_only: bool,
    /// Why the deployed graph is not valid (loaded from disk like that); never deployed.
    invalid: Option<String>,
    /// Changed in memory only (armed, lastFiredAt, draft); written by the content flusher.
    dirty: bool,
}

#[derive(Default)]
struct Inner {
    project: Option<PathBuf>,
    /// Creation order: load order (by name) then created this session.
    flows: Vec<Entry>,
}

impl Inner {
    fn find(&self, id: &str) -> Option<&Entry> {
        self.flows.iter().find(|e| e.flow.id == id)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.flows.iter_mut().find(|e| e.flow.id == id)
    }
}

/// The open project's flows (see the module docs). Owned by `AppState`.
#[derive(Default)]
pub struct FlowStore {
    inner: Mutex<Inner>,
    /// Serializes every flow file write (held across the write; never the data lock).
    write: Mutex<()>,
}

/// One flow as loaded from disk.
#[derive(Debug, Clone)]
pub struct LoadedFlow {
    pub flow: Flow,
    pub read_only: bool,
    pub invalid: Option<String>,
}

impl FlowStore {
    fn install(&self, project: &Path, list: Vec<LoadedFlow>) {
        *lock(&self.inner) = Inner {
            project: Some(project.to_path_buf()),
            flows: list
                .into_iter()
                .map(|l| Entry {
                    flow: l.flow,
                    read_only: l.read_only,
                    invalid: l.invalid,
                    dirty: false,
                })
                .collect(),
        };
    }

    /// Forgets the project (closed). Flush first.
    pub(crate) fn forget(&self) {
        *lock(&self.inner) = Inner::default();
    }

    /// The project folder moved.
    pub(crate) fn relocate(&self, dir: &Path) {
        let mut g = lock(&self.inner);
        if g.project.is_some() {
            g.project = Some(dir.to_path_buf());
        }
    }

    pub fn project(&self) -> Option<PathBuf> {
        lock(&self.inner).project.clone()
    }

    /// A flow as stored (deployed graph plus draft).
    pub fn flow(&self, id: &str) -> Option<Flow> {
        lock(&self.inner).find(id).map(|e| e.flow.clone())
    }

    /// Every flow, in creation order.
    pub fn all(&self) -> Vec<Flow> {
        lock(&self.inner)
            .flows
            .iter()
            .map(|e| e.flow.clone())
            .collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        lock(&self.inner).find(id).is_some()
    }

    pub fn len(&self) -> usize {
        lock(&self.inner).flows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the flow waits for the content flusher.
    pub fn is_dirty(&self, id: &str) -> bool {
        lock(&self.inner).find(id).is_some_and(|e| e.dirty)
    }

    fn snapshot(&self, id: &str) -> Option<LoadedFlow> {
        lock(&self.inner).find(id).map(|e| LoadedFlow {
            flow: e.flow.clone(),
            read_only: e.read_only,
            invalid: e.invalid.clone(),
        })
    }

    fn snapshots(&self) -> Vec<LoadedFlow> {
        lock(&self.inner)
            .flows
            .iter()
            .map(|e| LoadedFlow {
                flow: e.flow.clone(),
                read_only: e.read_only,
                invalid: e.invalid.clone(),
            })
            .collect()
    }

    /// Replaces (or appends) a flow just written. Keeps the in-memory `lastFiredAt` when it moved
    /// on meanwhile (the engine stamps it without the write lock); the flow stays dirty then.
    fn commit(&self, mut flow: Flow, read_only: bool, invalid: Option<String>) -> Flow {
        let mut g = lock(&self.inner);
        match g.find_mut(&flow.id) {
            Some(e) => {
                let dirty = e.flow.last_fired_at != flow.last_fired_at;
                if dirty {
                    flow.last_fired_at = e.flow.last_fired_at.clone();
                }
                e.flow = flow.clone();
                e.read_only = read_only;
                e.invalid = invalid;
                e.dirty = dirty && !read_only;
            }
            None => g.flows.push(Entry {
                flow: flow.clone(),
                read_only,
                invalid,
                dirty: false,
            }),
        }
        flow
    }

    fn remove(&self, id: &str) -> Option<Flow> {
        let mut g = lock(&self.inner);
        let pos = g.flows.iter().position(|e| e.flow.id == id)?;
        Some(g.flows.remove(pos).flow)
    }

    /// Takes the dirty flows to write, with the project folder.
    fn take_dirty(&self) -> Option<(PathBuf, Vec<Flow>)> {
        let mut g = lock(&self.inner);
        let project = g.project.clone()?;
        let mut list = Vec::new();
        for e in g.flows.iter_mut().filter(|e| e.dirty) {
            e.dirty = false;
            if !e.read_only {
                list.push(e.flow.clone());
            }
        }
        (!list.is_empty()).then_some((project, list))
    }

    fn mark_dirty(&self, id: &str) {
        if let Some(e) = lock(&self.inner).find_mut(id) {
            e.dirty = !e.read_only;
        }
    }
}

/// Writes the dirty flows (the content flusher). Returns the count.
pub(crate) fn flush_dirty(store: &FlowStore) -> u64 {
    let _w = lock(&store.write);
    let Some((dir, list)) = store.take_dirty() else {
        return 0;
    };
    let mut n = 0;
    for flow in list {
        match write_flow(&dir, &flow) {
            Ok(()) => n += 1,
            Err(AppError::NewerVersion { .. }) => {}
            Err(e) => {
                tracing::warn!("saving flow {}: {e}", flow.id);
                store.mark_dirty(&flow.id);
            }
        }
    }
    n
}

// ---- Files -----------------------------------------------------------------------------------

#[derive(Serialize)]
struct OnDisk<'a> {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(flatten)]
    flow: &'a Flow,
}

pub fn flows_root(project: &Path) -> PathBuf {
    project.join(FLOWS_DIR)
}

pub fn flow_path(project: &Path, id: &str) -> AppResult<PathBuf> {
    if !is_id(id) {
        return Err(invalid(format!("Invalid flow id: {id}")));
    }
    Ok(flows_root(project).join(format!("{id}.json")))
}

/// Why a flow file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Missing,
    /// Not JSON, or not the shape of a flow.
    Corrupt(String),
    /// From a newer app version and not readable as a flow.
    Newer(String),
    Io(String),
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Missing => f.write_str("the file does not exist"),
            ReadError::Corrupt(m) | ReadError::Newer(m) | ReadError::Io(m) => f.write_str(m),
        }
    }
}

fn schema_version(obj: &serde_json::Map<String, Value>) -> Result<u32, String> {
    match obj.get("schemaVersion") {
        None => Ok(1),
        Some(n) => n
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n >= 1)
            .ok_or_else(|| format!("bad schemaVersion: {n}")),
    }
}

/// Decodes a flow file; the id is the file's. Returns the flow and whether it is read-only (a
/// newer schema).
pub fn decode_flow(id: &str, bytes: &[u8]) -> Result<(Flow, bool), ReadError> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| ReadError::Corrupt(e.to_string()))?;
    let Value::Object(mut obj) = v else {
        return Err(ReadError::Corrupt("expected a JSON object".into()));
    };
    let version = schema_version(&obj).map_err(ReadError::Corrupt)?;
    obj.remove("schemaVersion");
    obj.insert("id".into(), Value::String(id.to_string()));
    let read_only = version > FLOW_VERSION;
    match serde_json::from_value::<Flow>(Value::Object(obj)) {
        Ok(flow) => Ok((flow, read_only)),
        Err(e) if read_only => Err(ReadError::Newer(format!(
            "schema {version} from a newer Kinetra: {e}"
        ))),
        Err(e) => Err(ReadError::Corrupt(e.to_string())),
    }
}

/// Reads `flows/<id>.json`.
pub fn read_flow(project: &Path, id: &str) -> Result<(Flow, bool), ReadError> {
    let path = flow_path(project, id).map_err(|e| ReadError::Io(e.to_string()))?;
    match fs::read(&path) {
        Ok(bytes) => decode_flow(id, &bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(ReadError::Missing),
        Err(e) => Err(ReadError::Io(e.to_string())),
    }
}

/// Writes a flow atomically. A file from a newer Kinetra is never overwritten.
pub fn write_flow(project: &Path, flow: &Flow) -> AppResult<()> {
    let path = flow_path(project, &flow.id)?;
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(&bytes) {
            if let Ok(found) = schema_version(&obj) {
                if found > FLOW_VERSION {
                    return Err(AppError::NewerVersion {
                        found,
                        supported: FLOW_VERSION,
                    });
                }
            }
        }
    }
    fs::create_dir_all(flows_root(project))?;
    fsx::write_json_atomic(
        &path,
        &OnDisk {
            schema_version: FLOW_VERSION,
            flow,
        },
    )?;
    Ok(())
}

fn delete_file(project: &Path, id: &str) -> AppResult<()> {
    let path = flow_path(project, id)?;
    match fsx::remove_file_own(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Every flow file of the project, ordered by name (then id). Corrupt files are set aside
/// (reported); unreadable newer ones are logged and skipped; a graph that fails validation is
/// kept with `invalid` set.
pub fn load_all(project: &Path) -> (Vec<LoadedFlow>, Vec<Recovered>) {
    let mut list = Vec::new();
    let mut recovered = Vec::new();
    let Ok(entries) = fs::read_dir(flows_root(project)) else {
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
        match decode_flow(id, &bytes) {
            Ok((flow, read_only)) => {
                let invalid = validate_flow(&flow).err().map(|e| e.to_string());
                if let Some(e) = &invalid {
                    tracing::warn!("flow {id} is not valid, it will not run: {e}");
                }
                list.push(LoadedFlow {
                    flow,
                    read_only,
                    invalid,
                });
            }
            Err(ReadError::Corrupt(e)) => {
                tracing::warn!("{}: {e}; setting it aside", path.display());
                match persist::quarantine(&path) {
                    Ok(r) => recovered.push(r),
                    Err(e) => tracing::warn!("could not set aside {}: {e}", path.display()),
                }
            }
            Err(e) => tracing::warn!("skipping flow {id}: {e}"),
        }
        if list.len() >= FLOWS_MAX {
            tracing::warn!("more than {FLOWS_MAX} flows; the rest are not loaded");
            break;
        }
    }
    list.sort_by(|a, b| {
        a.flow
            .name
            .to_lowercase()
            .cmp(&b.flow.name.to_lowercase())
            .then_with(|| a.flow.id.cmp(&b.flow.id))
    });
    (list, recovered)
}

fn new_flow_id(project: &Path, store: &FlowStore) -> String {
    let root = flows_root(project);
    loop {
        let id = random_id();
        if !store.contains(&id) && !root.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

// ---- Validation ------------------------------------------------------------------------------

/// A node or edge id: `[A-Za-z0-9_-]{1,32}`.
pub fn is_item_id(s: &str) -> bool {
    (1..=FLOW_ITEM_ID_MAX).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A flow name, trimmed, 1..=[`FLOW_NAME_MAX`] characters.
pub fn validate_name(name: &str) -> AppResult<String> {
    let n = name.trim();
    let len = n.chars().count();
    if len == 0 || len > FLOW_NAME_MAX {
        return Err(invalid(format!(
            "Flow names must be 1 to {FLOW_NAME_MAX} characters"
        )));
    }
    Ok(n.to_string())
}

/// One validation problem: the node it belongs to (if any), the message the editor shows on that
/// node, and the prefix deploy puts in front of it ("Node x (type)", "Wire w").
#[derive(Debug, Clone)]
struct Problem {
    node: Option<String>,
    prefix: Option<String>,
    message: String,
}

impl Problem {
    fn plain(message: String) -> Self {
        Self {
            node: None,
            prefix: None,
            message,
        }
    }

    fn on(node: &str, message: String) -> Self {
        Self {
            node: Some(node.to_string()),
            prefix: None,
            message,
        }
    }

    /// The full text deploy and save report.
    fn text(&self) -> String {
        match &self.prefix {
            Some(p) => format!("{p}: {}", self.message),
            None => self.message.clone(),
        }
    }

    fn issue(self) -> FlowIssue {
        FlowIssue {
            node: self.node,
            message: self.message,
        }
    }
}

/// Pushes a problem; in first-error mode, returns at once.
macro_rules! report {
    ($out:ident, $all:expr, $p:expr) => {{
        $out.push($p);
        if !$all {
            return $out;
        }
    }};
}

fn first_error(list: Vec<Problem>) -> AppResult<()> {
    match list.into_iter().next() {
        Some(p) => Err(invalid(p.text())),
        None => Ok(()),
    }
}

/// Limits and ids (names and limits carry no node; a bad position does).
fn draft_problems(nodes: &[FlowNode], edges: &[FlowEdge], all: bool) -> Vec<Problem> {
    let mut out = Vec::new();
    if nodes.len() > FLOW_NODES_MAX {
        report!(
            out,
            all,
            Problem::plain(format!(
                "Too many nodes ({}, at most {FLOW_NODES_MAX})",
                nodes.len()
            ))
        );
    }
    if edges.len() > FLOW_EDGES_MAX {
        report!(
            out,
            all,
            Problem::plain(format!(
                "Too many wires ({}, at most {FLOW_EDGES_MAX})",
                edges.len()
            ))
        );
    }
    if !out.is_empty() {
        // Too large to look at further.
        return out;
    }
    let mut ids = HashSet::new();
    for n in nodes {
        if !is_item_id(&n.id) {
            report!(
                out,
                all,
                Problem::plain(format!("Invalid node id: {}", n.id))
            );
        } else if !ids.insert(n.id.as_str()) {
            report!(
                out,
                all,
                Problem::plain(format!("Duplicate node id: {}", n.id))
            );
        }
        if !n.position.x.is_finite() || !n.position.y.is_finite() {
            report!(
                out,
                all,
                Problem {
                    node: Some(n.id.clone()),
                    prefix: Some(format!("Node {}", n.id)),
                    message: "invalid position".into(),
                }
            );
        }
    }
    let mut ids = HashSet::new();
    for e in edges {
        if !is_item_id(&e.id) {
            report!(
                out,
                all,
                Problem::plain(format!("Invalid wire id: {}", e.id))
            );
        } else if !ids.insert(e.id.as_str()) {
            report!(
                out,
                all,
                Problem::plain(format!("Duplicate wire id: {}", e.id))
            );
        }
    }
    out
}

/// Limits and ids only (a draft may be wired half-way).
pub fn validate_draft(nodes: &[FlowNode], edges: &[FlowEdge]) -> AppResult<()> {
    first_error(draft_problems(nodes, edges, false))
}

fn need_id(what: &str, id: &str) -> Result<(), String> {
    if is_id(id) {
        Ok(())
    } else if id.is_empty() {
        Err(format!("pick a {what}"))
    } else {
        Err(format!("invalid {what} id: {id}"))
    }
}

fn need_key(key: &str) -> Result<(), String> {
    if is_key(key) {
        Ok(())
    } else {
        Err(format!("invalid field key: {key}"))
    }
}

fn need_text(what: &str, s: &str) -> Result<(), String> {
    if s.len() > VALUE_MAX {
        Err(format!(
            "the {what} is too long (at most {} KB)",
            VALUE_MAX / 1024
        ))
    } else {
        Ok(())
    }
}

fn need_ms(what: &str, ms: u64, min: u64) -> Result<(), String> {
    if ms < min || ms > FLOW_DELAY_MAX_MS {
        Err(format!(
            "the {what} must be {min} ms to {} hours",
            FLOW_DELAY_MAX_MS / 3_600_000
        ))
    } else {
        Ok(())
    }
}

fn need_hhmm(t: &str) -> Result<(), String> {
    localtime::parse_hhmm(t)
        .map(drop)
        .ok_or_else(|| format!("\"{t}\" is not a time like 09:30"))
}

fn need_local(t: &str) -> Result<(), String> {
    if t.len() == 16 && localtime::parse_local(t).is_some() {
        Ok(())
    } else {
        Err(format!(
            "\"{t}\" is not a date and time like 2026-10-04T09:30"
        ))
    }
}

fn need_count(what: &str, n: u8) -> Result<(), String> {
    if (2..=FLOW_BRANCHES_MAX).contains(&n) {
        Ok(())
    } else {
        Err(format!("{what} needs 2 to {FLOW_BRANCHES_MAX} outputs"))
    }
}

/// Checks one node's config (shapes and ranges; targets may be missing).
pub fn check_config(kind: &NodeKind) -> Result<(), String> {
    use NodeKind as K;
    match kind {
        K::Cue { label } => {
            if label
                .as_ref()
                .is_some_and(|l| l.chars().count() > CUE_LABEL_MAX)
            {
                return Err(format!("the label is at most {CUE_LABEL_MAX} characters"));
            }
        }
        K::ItemStatus {
            source_id, item_id, ..
        } => {
            need_id("source", source_id)?;
            if let Some(i) = item_id {
                need_id("item", i)?;
            }
        }
        K::BeforeNext { source_id, minutes } => {
            need_id("source", source_id)?;
            if *minutes > BEFORE_NEXT_MAX_MIN {
                return Err(format!("at most {BEFORE_NEXT_MAX_MIN} minutes before"));
            }
        }
        K::TimeOfDay { at, days } => {
            need_hhmm(at)?;
            let mut seen = HashSet::new();
            if days.len() > 7 || days.iter().any(|d| *d > 6 || !seen.insert(*d)) {
                return Err("days are 0 (Monday) to 6 (Sunday), each once".into());
            }
        }
        K::TimerZero {
            screen_id,
            layer_id,
            key,
        }
        | K::FieldValue {
            screen_id,
            layer_id,
            key,
        } => {
            need_id("screen", screen_id)?;
            need_id("layer", layer_id)?;
            need_key(key)?;
        }
        K::LayerAir {
            screen_id,
            layer_id,
            ..
        }
        | K::LayerState {
            screen_id,
            layer_id,
        }
        | K::Take {
            screen_id,
            layer_id,
            for_ms: None,
        }
        | K::Clear {
            screen_id,
            layer_id,
        }
        | K::MediaEnded {
            screen_id,
            layer_id,
        }
        | K::Media {
            screen_id,
            layer_id,
            ..
        } => {
            need_id("screen", screen_id)?;
            need_id("layer", layer_id)?;
        }
        K::Take {
            screen_id,
            layer_id,
            for_ms: Some(ms),
        } => {
            need_id("screen", screen_id)?;
            need_id("layer", layer_id)?;
            if *ms == 0 || *ms > TAKE_FOR_MAX_MS {
                return Err(format!(
                    "\"take for\" must be 1 ms to {} minutes",
                    TAKE_FOR_MAX_MS / 60_000
                ));
            }
        }
        K::ApiChanged { source_id, column } => {
            need_id("source", source_id)?;
            if let Some(c) = column.as_deref().filter(|c| !c.is_empty()) {
                if !crate::core::templates::is_key(c) {
                    return Err(format!("\"{c}\" is not a column"));
                }
            }
        }
        K::Osc { address } => {
            if address.is_empty() {
                return Err("enter an OSC address".into());
            }
            if !crate::core::osc::is_osc_pattern(address) {
                return Err(format!(
                    "an OSC address starts with / and has at most {} characters, like /stage/go",
                    crate::core::osc::OSC_ADDRESS_MAX
                ));
            }
        }
        K::Midi {
            device,
            channel,
            number,
            ..
        } => {
            if let Some(d) = device {
                if !crate::core::state::is_midi_name(d) {
                    return Err("pick a MIDI device".into());
                }
            }
            if channel.is_some_and(|c| !(1..=16).contains(&c)) {
                return Err("the MIDI channel must be 1 to 16".into());
            }
            if *number > 127 {
                return Err("the note or CC number must be 0 to 127".into());
            }
        }
        K::SlideshowWrap { source_id }
        | K::SourceRowChanged { source_id }
        | K::SourceRow { source_id, .. }
        | K::ScheduleItem { source_id, .. }
        | K::StepSource { source_id, .. }
        | K::PauseSource { source_id, .. } => need_id("source", source_id)?,
        K::RunByFlow {} | K::TimeNow {} | K::Branch {} | K::Once {} | K::ClearAll {} => {}
        K::Constant { value } => match value {
            crate::core::model::Constant::Text(t) => need_text("value", t)?,
            crate::core::model::Constant::Number(n) if !n.is_finite() => {
                return Err("enter a valid number".into())
            }
            _ => {}
        },
        K::Format { pattern } => need_text("pattern", pattern)?,
        K::Math { a, b, .. } => {
            if !a.is_finite() || !b.is_finite() {
                return Err("enter valid numbers".into());
            }
        }
        K::Compare { a, b, .. } => {
            need_text("value a", a)?;
            need_text("value b", b)?;
        }
        K::Switch { cases } => {
            if cases.is_empty() || cases.len() > FLOW_CASES_MAX {
                return Err(format!("a switch has 1 to {FLOW_CASES_MAX} cases"));
            }
            for c in cases {
                need_text("case", c)?;
            }
        }
        K::Gate { .. } => {}
        K::Delay { ms } => need_ms("delay", *ms, 0)?,
        K::WaitUntil { at, timeout_ms } => {
            if let Some(t) = at {
                need_hhmm(t)?;
            }
            if let Some(ms) = timeout_ms {
                need_ms("timeout", *ms, 1)?;
            }
        }
        K::Debounce { ms } => need_ms("debounce time", *ms, 1)?,
        K::Throttle { ms } => need_ms("throttle time", *ms, 1)?,
        K::Sequence { count } => need_count("a sequence", *count)?,
        K::Random { count, .. } => need_count("a random pick", *count)?,
        K::Counter { every } => {
            if *every == 0 {
                return Err("count every 1 or more events".into());
            }
        }
        K::Repeat { count, delay_ms } => {
            if count.is_some_and(|n| n == 0 || n > FLOW_REPEAT_MAX) {
                return Err(format!("repeat 1 to {FLOW_REPEAT_MAX} times, or forever"));
            }
            need_ms("delay between rounds", *delay_ms, 0)?;
        }
        K::ClearScreen { screen_id } | K::TakePreview { screen_id, .. } => {
            need_id("screen", screen_id)?
        }
        K::SetFields {
            screen_id,
            layer_id,
            fields,
        } => {
            need_id("screen", screen_id)?;
            need_id("layer", layer_id)?;
            if fields.is_empty() {
                return Err("no field values to set".into());
            }
            for (k, v) in fields {
                need_key(k)?;
                need_text(&format!("value of \"{k}\""), v)?;
            }
        }
        K::SetTimer {
            screen_id,
            layer_id,
            key,
            action,
        } => {
            need_id("screen", screen_id)?;
            need_id("layer", layer_id)?;
            need_key(key)?;
            if let TimerAction::Set { ms } = action {
                if *ms > FLOW_DELAY_MAX_MS * 7 {
                    return Err("the timer value is too large".into());
                }
            }
        }
        K::Schedule {
            source_id,
            item,
            op,
        } => {
            need_id("source", source_id)?;
            if let ItemPick::Item { item_id } = item {
                need_id("item", item_id)?;
            }
            match op {
                ScheduleOp::SetNext { start: Some(s) } | ScheduleOp::SetStart { start: s } => {
                    need_local(s)?
                }
                ScheduleOp::Delay { minutes }
                    if *minutes == 0 || minutes.unsigned_abs() > 24 * 60 =>
                {
                    return Err("a delay is 1 to 1440 minutes (or minus that)".into());
                }
                _ => {}
            }
        }
        K::RunFlow { flow_id, .. } | K::StopFlow { flow_id } => need_id("flow", flow_id)?,
        K::SetVariant {
            screen_id,
            layer_id,
            variant,
            ..
        } => {
            need_id("screen", screen_id)?;
            need_id("layer", layer_id)?;
            if variant.is_empty() {
                return Err("no variant to show".into());
            }
            if !crate::core::templates::is_variant_id(variant) {
                return Err(format!("\"{variant}\" is not a valid variant id"));
            }
        }
        K::Log { message, .. } => need_text("message", message)?,
    }
    Ok(())
}

/// Finds a cycle among `nodes` along `adj`, as a path that ends where it starts.
fn find_cycle<'a>(nodes: &[&'a str], adj: &HashMap<&'a str, Vec<&'a str>>) -> Option<Vec<&'a str>> {
    #[derive(Clone, Copy, PartialEq)]
    enum C {
        White,
        Gray,
        Black,
    }
    fn visit<'a>(
        n: &'a str,
        adj: &HashMap<&'a str, Vec<&'a str>>,
        color: &mut HashMap<&'a str, C>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<&'a str>> {
        color.insert(n, C::Gray);
        stack.push(n);
        for &m in adj.get(n).map(Vec::as_slice).unwrap_or(&[]) {
            match color.get(m).copied().unwrap_or(C::White) {
                C::Gray => {
                    let start = stack.iter().position(|x| *x == m).unwrap_or(0);
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(m);
                    return Some(cycle);
                }
                C::White => {
                    if let Some(c) = visit(m, adj, color, stack) {
                        return Some(c);
                    }
                }
                C::Black => {}
            }
        }
        stack.pop();
        color.insert(n, C::Black);
        None
    }
    let mut color: HashMap<&str, C> = HashMap::new();
    for &n in nodes {
        if color.get(n).copied().unwrap_or(C::White) == C::White {
            let mut stack = Vec::new();
            if let Some(c) = visit(n, adj, &mut color, &mut stack) {
                return Some(c);
            }
        }
    }
    None
}

/// The strongly connected components of `adj` that hold a cycle (more than one node, or a
/// self-loop), each in the order of `nodes`, ordered by their first node. Tarjan; the graph is
/// small (≤ [`FLOW_NODES_MAX`] nodes), so the recursion is shallow.
fn cyclic_components<'a>(
    nodes: &[&'a str],
    adj: &HashMap<&'a str, Vec<&'a str>>,
) -> Vec<Vec<&'a str>> {
    struct Tarjan<'a, 'b> {
        adj: &'b HashMap<&'a str, Vec<&'a str>>,
        index: HashMap<&'a str, usize>,
        low: HashMap<&'a str, usize>,
        on_stack: HashSet<&'a str>,
        stack: Vec<&'a str>,
        next: usize,
        out: Vec<Vec<&'a str>>,
    }
    impl<'a> Tarjan<'a, '_> {
        fn visit(&mut self, v: &'a str) {
            self.index.insert(v, self.next);
            self.low.insert(v, self.next);
            self.next += 1;
            self.stack.push(v);
            self.on_stack.insert(v);
            let succ: Vec<&'a str> = self.adj.get(v).cloned().unwrap_or_default();
            for w in succ {
                if !self.index.contains_key(w) {
                    self.visit(w);
                    let lw = self.low.get(w).copied().unwrap_or(usize::MAX);
                    if let Some(lv) = self.low.get_mut(v) {
                        *lv = (*lv).min(lw);
                    }
                } else if self.on_stack.contains(w) {
                    let iw = self.index.get(w).copied().unwrap_or(usize::MAX);
                    if let Some(lv) = self.low.get_mut(v) {
                        *lv = (*lv).min(iw);
                    }
                }
            }
            if self.low.get(v) == self.index.get(v) {
                let mut comp = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on_stack.remove(w);
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                let self_loop = self.adj.get(v).is_some_and(|s| s.contains(&v));
                if comp.len() > 1 || self_loop {
                    self.out.push(comp);
                }
            }
        }
    }
    let mut t = Tarjan {
        adj,
        index: HashMap::new(),
        low: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for &n in nodes {
        if !t.index.contains_key(n) {
            t.visit(n);
        }
    }
    let order: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    let rank = |n: &str| order.get(n).copied().unwrap_or(usize::MAX);
    let mut comps = t.out;
    for c in &mut comps {
        c.sort_by_key(|n| rank(n));
    }
    comps.sort_by_key(|c| c.first().map_or(usize::MAX, |n| rank(n)));
    comps
}

/// One problem per node of every cycle along `adj` (`what`: the message before the path).
fn cycle_problems(
    nodes: &[&str],
    adj: &HashMap<&str, Vec<&str>>,
    what: &str,
    all: bool,
) -> Vec<Problem> {
    let mut out = Vec::new();
    for comp in cyclic_components(nodes, adj) {
        let inside: HashSet<&str> = comp.iter().copied().collect();
        let sub: HashMap<&str, Vec<&str>> = comp
            .iter()
            .map(|n| {
                let next = adj
                    .get(n)
                    .map(|v| v.iter().copied().filter(|m| inside.contains(m)).collect())
                    .unwrap_or_default();
                (*n, next)
            })
            .collect();
        let path = find_cycle(&comp, &sub).unwrap_or_else(|| comp.clone());
        let message = format!("{what}: {}", path.join(" → "));
        for n in &comp {
            report!(out, all, Problem::on(n, message.clone()));
        }
    }
    out
}

/// Every problem of a deployed graph, in check order (limits and ids, node configs, a Run flow
/// of the flow itself when `self_id` is known, Cues, wires in edge order, data cycles, event
/// cycles, a Repeat body wired back to its own `in`); `all = false` stops at the first.
fn graph_problems(
    nodes: &[FlowNode],
    edges: &[FlowEdge],
    self_id: Option<&str>,
    all: bool,
) -> Vec<Problem> {
    let mut out = draft_problems(nodes, edges, all);
    if !out.is_empty() && !all {
        return out;
    }
    if nodes.len() > FLOW_NODES_MAX || edges.len() > FLOW_EDGES_MAX {
        return out;
    }
    let mut cue_seen = false;
    for n in nodes {
        if let Err(e) = check_config(&n.kind) {
            report!(
                out,
                all,
                Problem {
                    node: Some(n.id.clone()),
                    prefix: Some(format!("Node {} ({})", n.id, type_name(&n.kind))),
                    message: e,
                }
            );
        }
    }
    if let Some(me) = self_id {
        for n in nodes {
            if matches!(&n.kind, NodeKind::RunFlow { flow_id, .. } if flow_id == me) {
                report!(
                    out,
                    all,
                    Problem::on(&n.id, "A flow can't run itself; use Repeat".into())
                );
            }
        }
    }
    for n in nodes {
        if matches!(n.kind, NodeKind::Cue { .. }) {
            if cue_seen {
                report!(
                    out,
                    all,
                    Problem::on(&n.id, "A flow has at most one Cue trigger".into())
                );
            }
            cue_seen = true;
        }
    }
    let by_id: HashMap<&str, &FlowNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut data_inputs: HashSet<(&str, &str)> = HashSet::new();
    let mut pairs: HashSet<(&str, &str, &str, &str)> = HashSet::new();
    let mut data_adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut event_adj: HashMap<&str, Vec<&str>> = HashMap::new();
    // Every valid event wire, by source node: (port, target node, target port).
    let mut event_out: HashMap<&str, Vec<(&str, &str, &str)>> = HashMap::new();
    for e in edges {
        let wire = |node: Option<&str>, m: String| Problem {
            node: node.map(str::to_string),
            prefix: Some(format!("Wire {}", e.id)),
            message: m,
        };
        let from = by_id.get(e.from.node.as_str());
        let to = by_id.get(e.to.node.as_str());
        let (from, to) = match (from, to) {
            (Some(f), Some(t)) => (*f, *t),
            (None, other) => {
                let node = other.map(|_| e.to.node.as_str());
                report!(
                    out,
                    all,
                    wire(node, format!("node {} does not exist", e.from.node))
                );
                continue;
            }
            (Some(_), None) => {
                report!(
                    out,
                    all,
                    wire(
                        Some(&e.from.node),
                        format!("node {} does not exist", e.to.node)
                    )
                );
                continue;
            }
        };
        let Some(outp) = flow_nodes::output(&from.kind, &e.from.port) else {
            report!(
                out,
                all,
                wire(
                    Some(&e.from.node),
                    format!(
                        "{} has no output \"{}\"",
                        type_name(&from.kind),
                        e.from.port
                    )
                )
            );
            continue;
        };
        let Some(inp) = flow_nodes::input(&to.kind, &e.to.port) else {
            report!(
                out,
                all,
                wire(
                    Some(&e.to.node),
                    format!("{} has no input \"{}\"", type_name(&to.kind), e.to.port)
                )
            );
            continue;
        };
        if outp.wire != e.kind || inp.wire != e.kind {
            report!(
                out,
                all,
                wire(
                    Some(&e.to.node),
                    "event and data ports only connect to their own kind".into()
                )
            );
            continue;
        }
        if !pairs.insert((&e.from.node, &e.from.port, &e.to.node, &e.to.port)) {
            report!(
                out,
                all,
                wire(Some(&e.to.node), "these ports are already connected".into())
            );
            continue;
        }
        match e.kind {
            WireKind::Data => {
                let from_ty = outp.ty.unwrap_or(crate::core::model::DataType::Text);
                if !flow_nodes::accepts(&to.kind, &inp, from_ty) {
                    let to_ty = inp.ty.map_or("event", data_type_name);
                    report!(
                        out,
                        all,
                        wire(
                            Some(&e.to.node),
                            format!("{} can't go into a {to_ty} input", data_type_name(from_ty))
                        )
                    );
                    continue;
                }
                if !data_inputs.insert((&e.to.node, &e.to.port)) {
                    report!(
                        out,
                        all,
                        wire(
                            Some(&e.to.node),
                            format!(
                                "input \"{}\" of node {} already has a wire",
                                e.to.port, e.to.node
                            )
                        )
                    );
                    continue;
                }
                data_adj
                    .entry(e.from.node.as_str())
                    .or_default()
                    .push(e.to.node.as_str());
            }
            WireKind::Event => {
                event_out.entry(e.from.node.as_str()).or_default().push((
                    e.from.port.as_str(),
                    e.to.node.as_str(),
                    e.to.port.as_str(),
                ));
                // A waiting node breaks the cycle: leave it out.
                if !flow_nodes::is_wait(&from.kind) && !flow_nodes::is_wait(&to.kind) {
                    event_adj
                        .entry(e.from.node.as_str())
                        .or_default()
                        .push(e.to.node.as_str());
                }
            }
        }
    }
    let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    for p in cycle_problems(&ids, &data_adj, "Data loop", all) {
        report!(out, all, p);
    }
    for p in cycle_problems(
        &ids,
        &event_adj,
        "Event loop without a Delay, Wait until, Debounce, Throttle or Repeat",
        all,
    ) {
        report!(out, all, p);
    }
    for n in nodes {
        if matches!(n.kind, NodeKind::Repeat { .. }) && body_loops_back(&n.id, &event_out) {
            report!(
                out,
                all,
                Problem::on(&n.id, "Repeat already loops; wire to stop instead".into())
            );
        }
    }
    out
}

/// Whether an event path leads from Repeat `rep`'s `body` back into its own `in` (through any
/// nodes, waiting ones included; the walk stops at the Repeat itself).
fn body_loops_back(rep: &str, event_out: &HashMap<&str, Vec<(&str, &str, &str)>>) -> bool {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut todo: Vec<&str> = Vec::new();
    let starts = event_out.get(rep).map(Vec::as_slice).unwrap_or(&[]);
    for &(port, to, to_port) in starts {
        if port != "body" {
            continue;
        }
        if to == rep {
            if to_port == "in" {
                return true;
            }
        } else if seen.insert(to) {
            todo.push(to);
        }
    }
    while let Some(n) = todo.pop() {
        for &(_, to, to_port) in event_out.get(n).map(Vec::as_slice).unwrap_or(&[]) {
            if to == rep {
                if to_port == "in" {
                    return true;
                }
            } else if seen.insert(to) {
                todo.push(to);
            }
        }
    }
    false
}

/// The full check of a deployed graph (see the module docs): the first error.
pub fn validate_graph(nodes: &[FlowNode], edges: &[FlowEdge]) -> AppResult<()> {
    first_error(graph_problems(nodes, edges, None, false))
}

/// [`validate_graph`] of flow `id`'s graph: also refuses a Run flow of the flow itself.
pub fn validate_graph_of(id: &str, nodes: &[FlowNode], edges: &[FlowEdge]) -> AppResult<()> {
    first_error(graph_problems(nodes, edges, Some(id), false))
}

/// Every error of a graph (what [`validate_graph`] checks), with the node each belongs to:
/// a config error, a wire's problem (on the wire's target node; on its source when the target
/// does not exist), every Cue after the first, every node of a data cycle or of an event cycle
/// without a waiting node. Limits, invalid and duplicate ids carry no node.
pub fn check_graph(nodes: &[FlowNode], edges: &[FlowEdge]) -> Vec<FlowIssue> {
    check_graph_of(None, nodes, edges)
}

/// [`check_graph`] of flow `id`'s graph (when known): also a Run flow of the flow itself.
pub fn check_graph_of(id: Option<&str>, nodes: &[FlowNode], edges: &[FlowEdge]) -> Vec<FlowIssue> {
    graph_problems(nodes, edges, id, true)
        .into_iter()
        .map(Problem::issue)
        .collect()
}

/// Everything about a flow file: id, name, deployed graph (full) and draft (loose).
/// Sets a node's config `sourceId` to `new` when it is `old` (any node kind: triggers, data
/// nodes, actions). Returns whether it changed.
pub fn relink_node_source(kind: &mut NodeKind, old: &str, new: &str) -> bool {
    let Ok(mut v) = serde_json::to_value(&*kind) else {
        return false;
    };
    let Some(src) = v.get_mut("config").and_then(|c| c.get_mut("sourceId")) else {
        return false;
    };
    if src.as_str() != Some(old) || old == new {
        return false;
    }
    *src = Value::String(new.to_string());
    match serde_json::from_value(v) {
        Ok(k) => {
            *kind = k;
            true
        }
        Err(_) => false,
    }
}

pub fn validate_flow(flow: &Flow) -> AppResult<()> {
    if !is_id(&flow.id) {
        return Err(invalid(format!("Invalid flow id: {}", flow.id)));
    }
    validate_name(&flow.name)?;
    if flow.origin.as_ref().is_some_and(|o| !is_id(o)) {
        return Err(invalid("Invalid origin"));
    }
    validate_graph_of(&flow.id, &flow.nodes, &flow.edges)?;
    if let Some(d) = &flow.draft {
        validate_draft(&d.nodes, &d.edges).map_err(|e| invalid(format!("Draft: {e}")))?;
    }
    Ok(())
}

// ---- Migration and conversion ----------------------------------------------------------------

fn node(id: String, x: f64, kind: NodeKind) -> FlowNode {
    FlowNode {
        id,
        position: NodePos { x, y: 0.0 },
        kind,
    }
}

fn event_edge(from: &str, from_port: &str, to: &str) -> FlowEdge {
    FlowEdge {
        id: random_id(),
        from: PortRef {
            node: from.to_string(),
            port: from_port.to_string(),
        },
        to: PortRef {
            node: to.to_string(),
            port: "in".into(),
        },
        kind: WireKind::Event,
    }
}

/// The event output a linear chain continues from.
fn next_port(kind: &NodeKind) -> &'static str {
    if flow_nodes::is_trigger(kind) {
        "fired"
    } else if flow_nodes::is_action(kind) {
        "done"
    } else {
        "out"
    }
}

/// A trigger and steps wired in a line, laid out left to right.
pub fn linear(trigger: NodeKind, steps: Vec<NodeKind>) -> (Vec<FlowNode>, Vec<FlowEdge>) {
    let mut nodes: Vec<FlowNode> = Vec::with_capacity(steps.len() + 1);
    let mut edges = Vec::with_capacity(steps.len());
    nodes.push(node(random_id(), 0.0, trigger));
    for (i, kind) in steps.into_iter().enumerate() {
        let n = node(random_id(), LINEAR_STEP_X * (i + 1) as f64, kind);
        if let Some(prev) = nodes.last() {
            edges.push(event_edge(&prev.id, next_port(&prev.kind), &n.id));
        }
        nodes.push(n);
    }
    (nodes, edges)
}

pub fn trigger_node(t: &Trigger) -> NodeKind {
    match t.clone() {
        Trigger::ItemStatus {
            source_id,
            status,
            item_id,
        } => NodeKind::ItemStatus {
            source_id,
            status,
            item_id,
        },
        Trigger::BeforeNext { source_id, minutes } => NodeKind::BeforeNext { source_id, minutes },
        Trigger::TimeOfDay { at, days } => NodeKind::TimeOfDay { at, days },
        Trigger::TimerZero {
            screen_id,
            layer_id,
            key,
        } => NodeKind::TimerZero {
            screen_id,
            layer_id,
            key,
        },
        Trigger::Layer {
            screen_id,
            layer_id,
            on_air,
        } => NodeKind::LayerAir {
            screen_id,
            layer_id,
            on_air,
        },
        Trigger::SlideshowWrap { source_id } => NodeKind::SlideshowWrap { source_id },
    }
}

pub fn rule_action_node(a: &RuleAction) -> NodeKind {
    match a.clone() {
        RuleAction::Take {
            screen_id,
            layer_id,
            for_ms,
        } => NodeKind::Take {
            screen_id,
            layer_id,
            for_ms,
        },
        RuleAction::Clear {
            screen_id,
            layer_id,
        } => NodeKind::Clear {
            screen_id,
            layer_id,
        },
        RuleAction::RunMacro { macro_id } => NodeKind::RunFlow {
            flow_id: macro_id,
            wait: false,
        },
        RuleAction::StepSource { source_id, step } => NodeKind::StepSource { source_id, step },
        RuleAction::PauseSource { source_id, paused } => NodeKind::PauseSource {
            source_id,
            mode: if paused {
                PauseMode::Pause
            } else {
                PauseMode::Resume
            },
        },
        RuleAction::SetFields {
            screen_id,
            layer_id,
            fields,
        } => NodeKind::SetFields {
            screen_id,
            layer_id,
            fields,
        },
    }
}

pub fn macro_step_node(s: &MacroStep) -> NodeKind {
    match s.clone() {
        MacroStep::Take {
            screen_id,
            layer_id,
        } => NodeKind::Take {
            screen_id,
            layer_id,
            for_ms: None,
        },
        MacroStep::Clear {
            screen_id,
            layer_id,
        } => NodeKind::Clear {
            screen_id,
            layer_id,
        },
        MacroStep::ClearScreen { screen_id } => NodeKind::ClearScreen { screen_id },
        MacroStep::StepSource { source_id, step } => NodeKind::StepSource { source_id, step },
        MacroStep::SetFields {
            screen_id,
            layer_id,
            fields,
        } => NodeKind::SetFields {
            screen_id,
            layer_id,
            fields,
        },
        MacroStep::Wait { ms } => NodeKind::Delay { ms: u64::from(ms) },
    }
}

/// A rule as a flow: its trigger wired through its actions in order. `armed` = `enabled`,
/// `lastFiredAt` kept.
pub fn rule_to_flow(rule: &Rule, id: String, origin: Option<String>) -> Flow {
    let (nodes, edges) = linear(
        trigger_node(&rule.when),
        rule.then.iter().map(rule_action_node).collect(),
    );
    Flow {
        id,
        name: rule.name.trim().to_string(),
        armed: rule.enabled,
        retrigger: Retrigger::Restart,
        origin,
        nodes,
        edges,
        draft: None,
        last_fired_at: rule.last_fired_at.clone(),
        updated_at: fsx::now_rfc3339(),
    }
}

/// A macro as a Cue flow with the same id (armed, no origin, restart on retrigger).
pub fn macro_to_flow(m: &Macro) -> Flow {
    let (nodes, edges) = linear(
        NodeKind::Cue { label: None },
        m.steps.iter().map(macro_step_node).collect(),
    );
    Flow {
        id: m.id.clone(),
        name: m.name.trim().to_string(),
        armed: true,
        retrigger: Retrigger::Restart,
        origin: None,
        nodes,
        edges,
        draft: None,
        last_fired_at: None,
        updated_at: fsx::now_rfc3339(),
    }
}

/// What the migration found to do.
struct Plan {
    macros: Option<Vec<Macro>>,
    /// The instances of `schemaVersion: 1` recipe files (id from the file name).
    recipes: Vec<crate::core::model::RecipeInstance>,
}

fn migration_plan(dir: &Path) -> Plan {
    let macros = match macros::read_legacy(dir) {
        macros::Legacy::Macros(list) => Some(list),
        _ => None,
    };
    let mut list = Vec::new();
    if let Ok(entries) = fs::read_dir(recipes::recipes_root(dir)) {
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Some(id) = name.strip_suffix(".json").filter(|id| is_id(id)) else {
                continue;
            };
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if schema_version(&obj) != Ok(1) {
                continue;
            }
            if let Ok(mut inst) =
                persist::decode::<crate::core::model::RecipeInstance>(&entry.path(), &bytes)
            {
                inst.id = id.to_string();
                list.push(inst);
            }
        }
    }
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    Plan {
        macros,
        recipes: list,
    }
}

/// Every flow file on disk by id (for idempotent re-runs after an interrupted migration).
fn existing_flows(dir: &Path) -> Vec<Flow> {
    load_all(dir).0.into_iter().map(|l| l.flow).collect()
}

impl AppState {
    // ---- Project lifecycle -------------------------------------------------------------------

    /// Takes the `pre-migrate` backup (once per project open: `backed_up` says whether it was
    /// taken already, and is set when this one succeeds). On failure `what` (e.g. "Rules and
    /// macros were not converted to flows") is reported and `false` returned: don't migrate.
    pub(crate) fn pre_migrate_backup(&self, dir: &Path, backed_up: &mut bool, what: &str) -> bool {
        if *backed_up {
            return true;
        }
        match crate::core::backup::snapshot(dir, BackupReason::PreMigrate) {
            Ok(info) => {
                match serde_json::to_value(&info) {
                    Ok(v) => self.events().emit(names::BACKUP_CREATED, v),
                    Err(e) => tracing::warn!("backup-created payload: {e}"),
                }
                *backed_up = true;
                true
            }
            Err(e) => {
                tracing::error!("pre-migrate backup failed, not migrated: {e}");
                self.events().emit(
                    names::BACKGROUND_ERROR,
                    serde_json::json!({
                        "message": format!("{what}: the backup failed ({e})")
                    }),
                );
                false
            }
        }
    }

    /// Migrates rules and macros to flows (see the module docs) and renames `macro/<id>`
    /// hotkeys. Caller holds `io` (the `pre-migrate` backup must not race a store write);
    /// `backed_up`: see [`AppState::pre_migrate_backup`].
    pub(crate) fn flows_migrate_locked(&self, dir: &Path, backed_up: &mut bool) {
        self.migrate_hotkeys();
        let plan = migration_plan(dir);
        if plan.macros.is_none() && plan.recipes.is_empty() {
            return;
        }
        if !self.pre_migrate_backup(
            dir,
            backed_up,
            "Rules and macros were not converted to flows",
        ) {
            return;
        }
        if let Err(e) = migrate(dir, plan) {
            tracing::error!("migrating to flows failed: {e}");
            self.events().emit(
                names::BACKGROUND_ERROR,
                serde_json::json!({
                    "message": format!("Converting rules and macros to flows failed: {e}")
                }),
            );
        }
    }

    /// Hotkeys `macro/<id>` → `flow/<id>` (other keys kept).
    fn migrate_hotkeys(&self) {
        let Ok(mut ui) = self.ui_state() else { return };
        let old: Vec<String> = ui
            .control
            .hotkeys
            .keys()
            .filter(|k| k.starts_with("macro/"))
            .cloned()
            .collect();
        if old.is_empty() {
            return;
        }
        for k in old {
            if let Some(combo) = ui.control.hotkeys.remove(&k) {
                let id = &k["macro/".len()..];
                ui.control
                    .hotkeys
                    .entry(format!("flow/{id}"))
                    .or_insert(combo);
            }
        }
        if let Err(e) = self.set_ui_state(ui) {
            tracing::warn!("migrating macro hotkeys: {e}");
        }
    }

    /// Loads the open project's flows (after migration) and hands them to the engine with the
    /// persisted Automation off flag. Touches the disk.
    pub(crate) fn flows_project_opened(&self) {
        let Ok(dir) = self.project_dir() else { return };
        let (list, recovered) = load_all(&dir);
        for r in recovered {
            self.events().emit(names::STORE_RECOVERED, r.payload());
        }
        let deployable: Vec<Flow> = list
            .iter()
            .filter(|l| l.invalid.is_none())
            .map(|l| l.flow.clone())
            .collect();
        self.flows().install(&dir, list);
        let off = self
            .ui_state()
            .map(|u| u.control.automation_off)
            .unwrap_or(false);
        self.engine_load(deployable, off);
    }

    // ---- Queries -----------------------------------------------------------------------------

    pub(crate) fn store_dir(&self) -> AppResult<PathBuf> {
        self.project_dir()?;
        self.flows().project().ok_or(AppError::NoProject)
    }

    fn info_of(&self, l: LoadedFlow) -> FlowInfo {
        let mut warnings = Vec::new();
        if let Some(e) = &l.invalid {
            warnings.push(FlowIssue {
                node: None,
                message: format!("Not valid, so it doesn't run: {e}"),
            });
        }
        warnings.extend(self.flow_warnings(&l.flow));
        let running = self.flow_engine().is_running(&l.flow.id);
        FlowInfo {
            flow: l.flow,
            warnings,
            running,
            read_only: l.read_only,
        }
    }

    pub(crate) fn info(&self, id: &str) -> AppResult<FlowInfo> {
        self.store_dir()?;
        let l = self
            .flows()
            .snapshot(id)
            .ok_or_else(|| AppError::NotFound(format!("Flow not found: {id}")))?;
        Ok(self.info_of(l))
    }

    /// Missing targets (screen, layer, source, flow, field, source column) of the deployed
    /// graph, and a flow without a trigger. Never errors.
    pub fn flow_warnings(&self, flow: &Flow) -> Vec<FlowIssue> {
        self.graph_warnings(&flow.nodes, &flow.edges)
    }

    /// [`Self::flow_warnings`] of any graph (e.g. an editor draft). The messages are shown to
    /// operators verbatim: short, sentence case, things named by their names, never by id.
    pub fn graph_warnings(&self, nodes: &[FlowNode], edges: &[FlowEdge]) -> Vec<FlowIssue> {
        let mut out = Vec::new();
        let screens = self.screens();
        let templates = screens.templates();
        let warn = |out: &mut Vec<FlowIssue>, n: &FlowNode, m: String| {
            out.push(FlowIssue {
                node: Some(n.id.clone()),
                message: m,
            })
        };
        // The layer's name and its template's field keys (`None` when the template is unknown),
        // or the warning when the screen or layer is gone. `verb`: what the node does with the
        // layer ("takes", "clears", …).
        let layer_fields = |screen_id: &str,
                            layer_id: &str,
                            verb: &str|
         -> Result<(String, Option<Vec<String>>), String> {
            let doc = screens
                .doc(screen_id)
                .ok_or_else(|| "Screen no longer exists".to_string())?;
            let layer = doc
                .meta
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .ok_or_else(|| {
                    format!(
                        "The layer it {verb} was deleted (screen \u{201c}{}\u{201d})",
                        doc.meta.name
                    )
                })?;
            let Some(template_id) = layer.kind.template_id() else {
                // A media layer: no fields.
                return Ok((layer.name.clone(), Some(Vec::new())));
            };
            Ok((
                layer.name.clone(),
                templates
                    .get(template_id)
                    .map(|t| t.meta.fields.iter().map(|f| f.key.clone()).collect()),
            ))
        };
        // The layer's loaded template (`None` when it is unknown), or the warning when the screen
        // or layer is gone.
        let layer_template = |screen_id: &str,
                              layer_id: &str,
                              verb: &str|
         -> Result<
            Option<std::sync::Arc<crate::core::screens::LoadedTemplate>>,
            String,
        > {
            layer_fields(screen_id, layer_id, verb)?;
            let doc = screens
                .doc(screen_id)
                .ok_or_else(|| "Screen no longer exists".to_string())?;
            Ok(doc
                .meta
                .layers
                .iter()
                .find(|l| l.id == layer_id)
                .and_then(|l| templates.get(l.kind.template_id()?).cloned()))
        };
        // The source's name and column keys, or the warning when it is gone.
        let source_columns = |id: &str| -> Result<(String, Vec<String>), String> {
            self.sources()
                .doc(id)
                .map(|d| {
                    let cols = sources::columns_of(&d).into_iter().map(|c| c.key).collect();
                    (d.meta.name, cols)
                })
                .ok_or_else(|| "Source no longer exists".to_string())
        };
        for n in nodes {
            use NodeKind as K;
            // `what`: "Field" or "Timer field".
            let field = |out: &mut Vec<FlowIssue>,
                         s: &str,
                         l: &str,
                         verb: &str,
                         what: &str,
                         keys: &[&str]| {
                match layer_fields(s, l, verb) {
                    Err(m) => warn(out, n, m),
                    Ok((layer, Some(have))) => {
                        for k in keys {
                            if !have.iter().any(|h| h == k) {
                                warn(
                                    out,
                                    n,
                                    format!(
                                        "{what} \u{201c}{k}\u{201d} isn't on the layer \u{201c}{layer}\u{201d}"
                                    ),
                                );
                            }
                        }
                    }
                    Ok((_, None)) => {}
                }
            };
            match &n.kind {
                K::TimerZero {
                    screen_id,
                    layer_id,
                    key,
                } => field(
                    &mut out,
                    screen_id,
                    layer_id,
                    "watches",
                    "Timer field",
                    &[key.as_str()],
                ),
                K::SetTimer {
                    screen_id,
                    layer_id,
                    key,
                    ..
                } => field(
                    &mut out,
                    screen_id,
                    layer_id,
                    "controls",
                    "Timer field",
                    &[key.as_str()],
                ),
                K::FieldValue {
                    screen_id,
                    layer_id,
                    key,
                } => field(
                    &mut out,
                    screen_id,
                    layer_id,
                    "reads",
                    "Field",
                    &[key.as_str()],
                ),
                K::SetFields {
                    screen_id,
                    layer_id,
                    fields,
                } => {
                    let keys: Vec<&str> = fields.keys().map(String::as_str).collect();
                    field(&mut out, screen_id, layer_id, "fills", "Field", &keys);
                }
                K::LayerAir {
                    screen_id,
                    layer_id,
                    ..
                } => field(&mut out, screen_id, layer_id, "watches", "Field", &[]),
                K::LayerState {
                    screen_id,
                    layer_id,
                } => field(&mut out, screen_id, layer_id, "reads", "Field", &[]),
                K::Take {
                    screen_id,
                    layer_id,
                    ..
                } => field(&mut out, screen_id, layer_id, "takes", "Field", &[]),
                K::Clear {
                    screen_id,
                    layer_id,
                } => field(&mut out, screen_id, layer_id, "clears", "Field", &[]),
                K::MediaEnded {
                    screen_id,
                    layer_id,
                } => field(&mut out, screen_id, layer_id, "watches", "Field", &[]),
                K::Media {
                    screen_id,
                    layer_id,
                    ..
                } => field(&mut out, screen_id, layer_id, "plays", "Field", &[]),
                K::SetVariant {
                    screen_id,
                    layer_id,
                    variant,
                    ..
                } => match layer_template(screen_id, layer_id, "switches") {
                    Err(m) => warn(&mut out, n, m),
                    Ok(Some(t)) if !t.has_variant(variant) => warn(
                        &mut out,
                        n,
                        format!(
                            "Variant \u{201c}{variant}\u{201d} isn't in the template \u{201c}{}\u{201d}",
                            t.meta.name
                        ),
                    ),
                    Ok(_) => {}
                },
                K::ClearScreen { screen_id } | K::TakePreview { screen_id, .. } => {
                    if screens.doc(screen_id).is_none() {
                        warn(&mut out, n, "Screen no longer exists".into());
                    }
                }
                K::ItemStatus { source_id, .. }
                | K::BeforeNext { source_id, .. }
                | K::SlideshowWrap { source_id }
                | K::SourceRowChanged { source_id }
                | K::ScheduleItem { source_id, .. }
                | K::StepSource { source_id, .. }
                | K::PauseSource { source_id, .. }
                | K::Schedule { source_id, .. } => {
                    if let Err(m) = source_columns(source_id) {
                        warn(&mut out, n, m);
                    }
                }
                K::SourceRow { source_id, .. } => match source_columns(source_id) {
                    Err(m) => warn(&mut out, n, m),
                    Ok((name, cols)) => {
                        for e in edges.iter().filter(|e| e.from.node == n.id) {
                            let p = e.from.port.as_str();
                            if !flow_nodes::SOURCE_ROW_OUTPUTS.contains(&p)
                                && !cols.iter().any(|c| c == p)
                            {
                                warn(
                                    &mut out,
                                    n,
                                    format!(
                                        "Column \u{201c}{p}\u{201d} isn't in the source \u{201c}{name}\u{201d}"
                                    ),
                                );
                            }
                        }
                    }
                },
                K::ApiChanged { source_id, column } => match self.sources().doc(source_id) {
                    None => warn(&mut out, n, "Source no longer exists".into()),
                    Some(d) if !sources::is_api(&d) => warn(
                        &mut out,
                        n,
                        format!("“{}” is not an API source", d.meta.name),
                    ),
                    Some(d) => {
                        if let Some(c) = column.as_deref().filter(|c| !c.is_empty()) {
                            if !sources::columns_of(&d).iter().any(|x| x.key == c) {
                                warn(
                                    &mut out,
                                    n,
                                    format!(
                                        "Column “{c}” isn't in the source “{}”",
                                        d.meta.name
                                    ),
                                );
                            }
                        }
                    }
                },
                K::RunFlow { flow_id, .. } if !self.flows().contains(flow_id) => {
                    warn(&mut out, n, "The flow it runs no longer exists".into());
                }
                K::StopFlow { flow_id } if !self.flows().contains(flow_id) => {
                    warn(&mut out, n, "The flow it stops no longer exists".into());
                }
                _ => {}
            }
        }
        if !nodes.iter().any(|n| flow_nodes::is_trigger(&n.kind)) {
            out.push(FlowIssue {
                node: None,
                message: "No trigger: this flow never runs".into(),
            });
        }
        out
    }

    /// Every flow, in creation order.
    pub fn list_flows(&self) -> AppResult<Vec<FlowInfo>> {
        self.store_dir()?;
        Ok(self
            .flows()
            .snapshots()
            .into_iter()
            .map(|l| self.info_of(l))
            .collect())
    }

    pub fn get_flow(&self, id: &str) -> AppResult<FlowInfo> {
        self.info(id)
    }

    /// Validation without saving (the node editor, while editing a draft): every error that
    /// would block deploy, with the node it belongs to ([`check_graph`]), and the soft warnings
    /// (missing targets, no trigger). The structural check needs no project ([`check_graph`]);
    /// this answers `noProject` only when none is open (the warnings read its screens, sources
    /// and flows).
    pub fn check_flow(&self, nodes: Vec<FlowNode>, edges: Vec<FlowEdge>) -> AppResult<FlowCheck> {
        self.check_flow_of(None, nodes, edges)
    }

    /// [`Self::check_flow`] of the flow `id` (when given): also reports a Run flow of the flow
    /// itself ("A flow can't run itself; use Repeat").
    pub fn check_flow_of(
        &self,
        id: Option<&str>,
        nodes: Vec<FlowNode>,
        edges: Vec<FlowEdge>,
    ) -> AppResult<FlowCheck> {
        self.store_dir()?;
        let errors = check_graph_of(id, &nodes, &edges);
        let too_big = nodes.len() > FLOW_NODES_MAX || edges.len() > FLOW_EDGES_MAX;
        let warnings = if too_big {
            Vec::new()
        } else {
            // A node whose config is invalid (e.g. no screen picked yet) shows that error, not
            // also "Screen no longer exists".
            let bad: HashSet<&str> = nodes
                .iter()
                .filter(|n| check_config(&n.kind).is_err())
                .map(|n| n.id.as_str())
                .collect();
            let mut list = self.graph_warnings(&nodes, &edges);
            list.retain(|w| w.node.as_deref().is_none_or(|n| !bad.contains(n)));
            list
        };
        Ok(FlowCheck { errors, warnings })
    }

    // ---- Mutations ---------------------------------------------------------------------------

    /// The flow to change (not read-only), under the write lock.
    fn editable(&self, id: &str) -> AppResult<LoadedFlow> {
        let l = self
            .flows()
            .snapshot(id)
            .ok_or_else(|| AppError::NotFound(format!("Flow not found: {id}")))?;
        if l.read_only {
            return Err(invalid(
                "This flow is from a newer version of Kinetra and can't be changed",
            ));
        }
        Ok(l)
    }

    /// Validates, writes and deploys a new flow.
    pub fn create_flow(&self, req: NewFlow) -> AppResult<FlowInfo> {
        let dir = self.store_dir()?;
        let name = validate_name(&req.name)?;
        validate_graph(&req.nodes, &req.edges)?;
        if let Some(origin) = &req.origin {
            if self.rules().instance(origin).is_none() {
                return Err(invalid(format!("No installed recipe {origin}")));
            }
        }
        let flow = {
            let _w = lock(&self.flows().write);
            if self.flows().len() >= FLOWS_MAX {
                return Err(invalid(format!("A project has at most {FLOWS_MAX} flows")));
            }
            let flow = Flow {
                id: new_flow_id(&dir, self.flows()),
                name,
                armed: req.armed,
                retrigger: req.retrigger,
                origin: req.origin,
                nodes: req.nodes,
                edges: req.edges,
                draft: None,
                last_fired_at: None,
                updated_at: fsx::now_rfc3339(),
            };
            write_flow(&dir, &flow)?;
            self.flows().commit(flow, false, None)
        };
        // A recipe's own rule: listed with the recipe (its `flowIds`). The flow already carries
        // the origin, so a failed write only leaves `flowIds` behind.
        if let Some(mut inst) = flow.origin.as_ref().and_then(|o| self.rules().instance(o)) {
            if !inst.flow_ids.contains(&flow.id) {
                inst.flow_ids.push(flow.id.clone());
                match recipes::write_instance(&dir, &inst) {
                    Ok(()) => self.rules().put_instance(inst),
                    Err(e) => tracing::warn!("adding flow {} to recipe {}: {e}", flow.id, inst.id),
                }
            }
        }
        self.engine_deploy(&flow);
        self.info(&flow.id)
    }

    /// Replaces the deployed graph (validated, deployed at once; drops the draft).
    pub fn save_flow(&self, req: SaveFlow) -> AppResult<FlowInfo> {
        let dir = self.store_dir()?;
        let name = validate_name(&req.name)?;
        validate_graph_of(&req.id, &req.nodes, &req.edges)?;
        let flow = {
            let _w = lock(&self.flows().write);
            let mut flow = self.editable(&req.id)?.flow;
            flow.name = name;
            flow.retrigger = req.retrigger;
            flow.nodes = req.nodes;
            flow.edges = req.edges;
            flow.draft = None;
            flow.updated_at = fsx::now_rfc3339();
            write_flow(&dir, &flow)?;
            self.flows().commit(flow, false, None)
        };
        self.engine_deploy(&flow);
        self.info(&flow.id)
    }

    /// Stores undeployed editor changes (coalesced write); the engine is not told. A draft equal
    /// to the deployed graph is dropped.
    pub fn save_flow_draft(
        &self,
        id: &str,
        nodes: Vec<FlowNode>,
        edges: Vec<FlowEdge>,
    ) -> AppResult<FlowInfo> {
        self.store_dir()?;
        validate_draft(&nodes, &edges)?;
        {
            let _w = lock(&self.flows().write);
            self.editable(id)?;
            let mut g = lock(&self.flows().inner);
            let e = g
                .find_mut(id)
                .ok_or_else(|| AppError::NotFound(format!("Flow not found: {id}")))?;
            e.flow.draft = if e.flow.nodes == nodes && e.flow.edges == edges {
                None
            } else {
                Some(FlowDraft {
                    nodes,
                    edges,
                    updated_at: fsx::now_rfc3339(),
                })
            };
            e.dirty = true;
        }
        self.schedule_content_flush();
        self.info(id)
    }

    /// Validates the draft, moves it into the deployed graph and deploys it. Without a draft,
    /// nothing changes.
    pub fn deploy_flow(&self, id: &str) -> AppResult<FlowInfo> {
        let dir = self.store_dir()?;
        let flow = {
            let _w = lock(&self.flows().write);
            let mut flow = self.editable(id)?.flow;
            let Some(draft) = flow.draft.take() else {
                drop(_w);
                return self.info(id);
            };
            validate_graph_of(id, &draft.nodes, &draft.edges)?;
            flow.nodes = draft.nodes;
            flow.edges = draft.edges;
            flow.updated_at = fsx::now_rfc3339();
            write_flow(&dir, &flow)?;
            self.flows().commit(flow, false, None)
        };
        self.engine_deploy(&flow);
        self.info(id)
    }

    pub fn discard_flow_draft(&self, id: &str) -> AppResult<FlowInfo> {
        let dir = self.store_dir()?;
        {
            let _w = lock(&self.flows().write);
            let l = self.editable(id)?;
            if l.flow.draft.is_some() {
                let mut flow = l.flow;
                flow.draft = None;
                write_flow(&dir, &flow)?;
                self.flows().commit(flow, false, l.invalid);
            }
        }
        self.info(id)
    }

    /// Renames a flow (no redeploy: the graph is unchanged).
    pub fn rename_flow(&self, id: &str, name: &str) -> AppResult<FlowInfo> {
        let dir = self.store_dir()?;
        let name = validate_name(name)?;
        {
            let _w = lock(&self.flows().write);
            let l = self.editable(id)?;
            let mut flow = l.flow;
            if flow.name != name {
                flow.name = name;
                flow.updated_at = fsx::now_rfc3339();
                write_flow(&dir, &flow)?;
                self.flows().commit(flow, false, l.invalid);
            }
        }
        self.info(id)
    }

    /// A disarmed copy named "<name> copy" (no origin, no hotkey, never fired).
    pub fn duplicate_flow(&self, id: &str) -> AppResult<FlowInfo> {
        let dir = self.store_dir()?;
        let (flow, deployable) = {
            let _w = lock(&self.flows().write);
            let src = self
                .flows()
                .snapshot(id)
                .ok_or_else(|| AppError::NotFound(format!("Flow not found: {id}")))?;
            if self.flows().len() >= FLOWS_MAX {
                return Err(invalid(format!("A project has at most {FLOWS_MAX} flows")));
            }
            let mut flow = src.flow;
            let mut name = format!("{} copy", flow.name);
            if name.chars().count() > FLOW_NAME_MAX {
                let keep = FLOW_NAME_MAX - " copy".len();
                name = format!(
                    "{} copy",
                    flow.name.chars().take(keep).collect::<String>().trim_end()
                );
            }
            flow.id = new_flow_id(&dir, self.flows());
            flow.name = name;
            flow.armed = false;
            flow.origin = None;
            flow.last_fired_at = None;
            flow.updated_at = fsx::now_rfc3339();
            write_flow(&dir, &flow)?;
            let deployable = src.invalid.is_none();
            (self.flows().commit(flow, false, src.invalid), deployable)
        };
        if deployable {
            self.engine_deploy(&flow);
        }
        self.info(&flow.id)
    }

    /// Deletes the file, stops its instance, drops its hotkey and its id from its recipe.
    pub fn delete_flow(&self, id: &str) -> AppResult<()> {
        let dir = self.store_dir()?;
        self.delete_flow_in(&dir, id, true)
    }

    fn delete_flow_in(&self, dir: &Path, id: &str, update_recipe: bool) -> AppResult<()> {
        let flow = {
            let _w = lock(&self.flows().write);
            let flow = self.editable(id)?.flow;
            delete_file(dir, id)?;
            self.flows().remove(id);
            flow
        };
        self.engine_remove(id);
        if let Ok(mut ui) = self.ui_state() {
            if ui.control.hotkeys.remove(&format!("flow/{id}")).is_some() {
                if let Err(e) = self.set_ui_state(ui) {
                    tracing::warn!("dropping the hotkey of flow {id}: {e}");
                }
            }
        }
        if update_recipe {
            if let Some(origin) = &flow.origin {
                if let Some(mut inst) = self.rules().instance(origin) {
                    let before = inst.flow_ids.len();
                    inst.flow_ids.retain(|f| f != id);
                    if inst.flow_ids.len() != before {
                        recipes::write_instance(dir, &inst)?;
                        self.rules().put_instance(inst);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn set_flow_armed(&self, id: &str, armed: bool) -> AppResult<FlowInfo> {
        self.store_dir()?;
        let changed = {
            let _w = lock(&self.flows().write);
            let l = self.editable(id)?;
            let changed = l.flow.armed != armed;
            if changed {
                let mut g = lock(&self.flows().inner);
                if let Some(e) = g.find_mut(id) {
                    e.flow.armed = armed;
                    e.dirty = true;
                }
            }
            changed
        };
        if changed {
            self.schedule_content_flush();
            self.engine_set_armed(id, armed);
        }
        self.info(id)
    }

    /// Engine → store: the flow fired at `at` (RFC 3339); `lastFiredAt` is written coalesced.
    pub(crate) fn flow_fired(&self, id: &str, at: &str) {
        let marked = {
            let mut g = lock(&self.flows().inner);
            match g.find_mut(id) {
                Some(e) => {
                    e.flow.last_fired_at = Some(at.to_string());
                    e.dirty = !e.read_only;
                    e.dirty
                }
                None => false,
            }
        };
        if marked {
            self.schedule_content_flush();
        }
    }

    // ---- Recipes -----------------------------------------------------------------------------

    /// Writes flows created by a recipe install (not yet in the store, all or none);
    /// on failure the ones written are removed again.
    pub(crate) fn write_recipe_flows(&self, dir: &Path, flows: &[Flow]) -> AppResult<()> {
        let _w = lock(&self.flows().write);
        if self.flows().len() + flows.len() > FLOWS_MAX {
            return Err(invalid(format!("A project has at most {FLOWS_MAX} flows")));
        }
        for (i, f) in flows.iter().enumerate() {
            if let Err(e) = write_flow(dir, f) {
                for done in &flows[..i] {
                    let _ = delete_file(dir, &done.id);
                }
                return Err(e);
            }
        }
        Ok(())
    }

    /// Fresh ids for flows a recipe is about to create.
    pub(crate) fn new_flow_ids(&self, dir: &Path, n: usize) -> Vec<String> {
        let mut ids: Vec<String> = Vec::with_capacity(n);
        while ids.len() < n {
            let id = new_flow_id(dir, self.flows());
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// Adds flows a recipe install wrote, and deploys them.
    pub(crate) fn recipe_flows_installed(&self, flows: Vec<Flow>) {
        for f in flows {
            let flow = self.flows().commit(f, false, None);
            self.engine_deploy(&flow);
        }
    }

    /// Removes the files of flows whose recipe install failed (not in the store yet).
    pub(crate) fn recipe_flows_rollback(&self, dir: &Path, ids: &[String]) {
        let _w = lock(&self.flows().write);
        for id in ids {
            if !self.flows().contains(id) {
                let _ = delete_file(dir, id);
            }
        }
    }

    /// A removed recipe's flows: deleted with it, or kept as own flows (origin cleared).
    pub(crate) fn recipe_flows_removed(&self, dir: &Path, ids: &[String], delete: bool) {
        for id in ids {
            let result = if delete {
                self.delete_flow_in(dir, id, false)
            } else {
                self.clear_origin(dir, id)
            };
            match result {
                Ok(()) | Err(AppError::NotFound(_)) => {}
                Err(e) => tracing::warn!("flow {id} of the removed recipe: {e}"),
            }
        }
    }

    /// Points every node whose config `sourceId` is `old` at `new`, in the deployed graph and in
    /// the draft (a recipe relinked to another source). The deployed graph is validated, written
    /// and deployed like `save_flow` (armed unchanged; a running instance stops). Returns whether
    /// the flow changed.
    pub(crate) fn relink_flow_source(
        &self,
        dir: &Path,
        id: &str,
        old: &str,
        new: &str,
    ) -> AppResult<bool> {
        let (flow, deploy) = {
            let _w = lock(&self.flows().write);
            let l = self.editable(id)?;
            let mut flow = l.flow;
            let mut deploy = false;
            for n in &mut flow.nodes {
                deploy |= relink_node_source(&mut n.kind, old, new);
            }
            let mut draft = false;
            if let Some(d) = &mut flow.draft {
                for n in &mut d.nodes {
                    draft |= relink_node_source(&mut n.kind, old, new);
                }
            }
            if !deploy && !draft {
                return Ok(false);
            }
            let invalid = if deploy {
                validate_graph_of(&flow.id, &flow.nodes, &flow.edges)?;
                flow.updated_at = fsx::now_rfc3339();
                None
            } else {
                l.invalid
            };
            write_flow(dir, &flow)?;
            (self.flows().commit(flow, false, invalid), deploy)
        };
        if deploy {
            self.engine_deploy(&flow);
        }
        Ok(true)
    }

    fn clear_origin(&self, dir: &Path, id: &str) -> AppResult<()> {
        let _w = lock(&self.flows().write);
        let l = self.editable(id)?;
        if l.flow.origin.is_some() {
            let mut flow = l.flow;
            flow.origin = None;
            write_flow(dir, &flow)?;
            self.flows().commit(flow, false, l.invalid);
        }
        Ok(())
    }

    // ---- Watcher -----------------------------------------------------------------------------

    /// `flows/<id>.json` changed outside Kinetra: a valid file replaces the flow (deployed when
    /// the graph changed, re-armed when only `armed` did); a removed one is removed; an invalid
    /// one keeps the last good flow. A reload equal to memory announces nothing. The editor's
    /// draft is kept (and written back) unless the file carries a draft of its own or the new
    /// deployed graph equals it: an edit of the deployed graph never drops undeployed work.
    pub(crate) fn reload_flow(&self, dir: &Path, id: &str) -> Option<FilesChanged> {
        enum Then {
            Deploy(Box<Flow>),
            Arm(bool),
            Remove,
            Nothing,
        }
        let changed = |change, message| FilesChanged {
            kind: FileKind::Flow,
            id: id.to_string(),
            change,
            message,
        };
        let mut keep_draft = false;
        let (result, then) = {
            let _w = lock(&self.flows().write);
            if self.flows().project().as_deref() != Some(dir) {
                return None;
            }
            let prev = self.flows().snapshot(id);
            match read_flow(dir, id) {
                Err(ReadError::Missing) => match prev {
                    Some(_) => {
                        self.flows().remove(id);
                        (Some(changed(FileChange::Removed, None)), Then::Remove)
                    }
                    None => (None, Then::Nothing),
                },
                Err(e) => {
                    tracing::warn!("flow {id} changed on disk but is not valid: {e}");
                    (
                        Some(changed(FileChange::Invalid, Some(e.to_string()))),
                        Then::Nothing,
                    )
                }
                Ok((mut flow, read_only)) => {
                    if let Some(d) = prev.as_ref().and_then(|p| p.flow.draft.as_ref()) {
                        if flow.draft.is_none()
                            && !read_only
                            && (d.nodes != flow.nodes || d.edges != flow.edges)
                        {
                            flow.draft = Some(d.clone());
                            keep_draft = true;
                        }
                    }
                    let check = validate_flow(&flow).and_then(|()| {
                        if prev.is_none() && self.flows().len() >= FLOWS_MAX {
                            Err(invalid(format!("A project has at most {FLOWS_MAX} flows")))
                        } else {
                            Ok(())
                        }
                    });
                    match (check, prev) {
                        (Err(e), _) => {
                            tracing::warn!("flow {id} changed on disk but is not valid: {e}");
                            (
                                Some(changed(FileChange::Invalid, Some(e.to_string()))),
                                Then::Nothing,
                            )
                        }
                        (Ok(()), Some(p))
                            if p.flow == flow
                                && p.read_only == read_only
                                && p.invalid.is_none() =>
                        {
                            // Only the draft was missing from the file: write it back.
                            if keep_draft {
                                self.flows().mark_dirty(id);
                            }
                            (None, Then::Nothing)
                        }
                        (Ok(()), prev) => {
                            let then = match &prev {
                                Some(p)
                                    if p.invalid.is_none()
                                        && p.flow.nodes == flow.nodes
                                        && p.flow.edges == flow.edges
                                        && p.flow.retrigger == flow.retrigger =>
                                {
                                    if p.flow.armed != flow.armed {
                                        Then::Arm(flow.armed)
                                    } else {
                                        Then::Nothing
                                    }
                                }
                                _ => Then::Deploy(Box::new(flow.clone())),
                            };
                            // The file edited outside wins over unsaved in-memory changes.
                            {
                                let mut g = lock(&self.flows().inner);
                                if let Some(e) = g.find_mut(id) {
                                    e.dirty = false;
                                }
                            }
                            self.flows().commit(flow, read_only, None);
                            if keep_draft {
                                self.flows().mark_dirty(id);
                            }
                            (Some(changed(FileChange::Changed, None)), then)
                        }
                    }
                }
            }
        };
        if keep_draft {
            self.schedule_content_flush();
        }
        match then {
            Then::Deploy(flow) => self.engine_deploy(&flow),
            Then::Arm(armed) => self.engine_set_armed(id, armed),
            Then::Remove => self.engine_remove(id),
            Then::Nothing => {}
        }
        result
    }
}

/// Writes the migrated flows, rewrites the recipe instances (v2) and marks `macros.json`.
/// Flow files come first, so an interrupted run is re-done on the next open; a rule whose flow
/// already exists (same origin and name) is not created twice, a macro's flow is rewritten.
fn migrate(dir: &Path, plan: Plan) -> AppResult<()> {
    let existing = existing_flows(dir);
    if let Some(list) = &plan.macros {
        for m in list {
            if !is_id(&m.id) {
                continue;
            }
            match write_flow(dir, &macro_to_flow(m)) {
                Ok(()) | Err(AppError::NewerVersion { .. }) => {}
                Err(e) => return Err(e),
            }
        }
    }
    let mut taken: HashSet<String> = existing.iter().map(|f| f.id.clone()).collect();
    if let Some(list) = &plan.macros {
        taken.extend(list.iter().map(|m| m.id.clone()));
    }
    for mut inst in plan.recipes {
        let mut ids = inst.flow_ids.clone();
        for rule in std::mem::take(&mut inst.rules) {
            let name = rule.name.trim().to_string();
            if let Some(f) = existing
                .iter()
                .find(|f| f.origin.as_deref() == Some(inst.id.as_str()) && f.name == name)
            {
                if !ids.contains(&f.id) {
                    ids.push(f.id.clone());
                }
                continue;
            }
            let id = loop {
                let id = random_id();
                if taken.insert(id.clone()) {
                    break id;
                }
            };
            write_flow(dir, &rule_to_flow(&rule, id.clone(), Some(inst.id.clone())))?;
            ids.push(id);
        }
        inst.flow_ids = ids;
        recipes::write_instance(dir, &inst)?;
    }
    if plan.macros.is_some() {
        macros::write_migrated_marker(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
