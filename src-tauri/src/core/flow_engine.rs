//! The flow engine: compiled deployed graphs, the trigger index the event hooks look up,
//! instances, limits, Automation off. No polling; no task at rest. See docs/ARCHITECTURE.md "Flows".
//!
//! # Compiled graphs
//! Every deployed flow is compiled into a compact node table ([`Compiled`]): one 24-byte op per
//! node (strings interned per flow, rare large configs in a side table), event edges grouped by
//! source node in edge order, data edges grouped by target node. Deploying swaps the compiled
//! graph atomically. The **trigger index** ([`Index`], `HashMap<TriggerKey, Vec<TrigRef>>`) holds
//! the trigger nodes of every *armed* deployed flow; it is rebuilt on every deploy, arm, removal
//! and load, and the hooks in `core::rules` (schedules, timers, layers, slideshow wraps) and
//! `core::source_live` (source row changes) look it up instead of scanning flows.
//!
//! # Instances
//! A firing starts the flow's single instance (retrigger `restart` stops a running one first,
//! `ignore` drops the firing). An instance is ONE tokio task. It walks event wires depth-first
//! (an output with several wires runs them in edge order, each subtree to its end first) in
//! *segments* on the blocking pool (actions call the same synchronous core paths as Control).
//! Data inputs are pulled lazily when a node runs and memoised until the next action runs (an
//! action may change what a data node reads).
//!
//! A branch that reaches a Delay, Wait until, Debounce or a waiting Run flow **parks**: the rest
//! of the segment (sibling branches) carries on, and when nothing is left to run the task sleeps
//! (a sleeping tokio task, never a thread) until the earliest parked wake or a waited-for flow
//! finishing. The instance ends when nothing is left to run and nothing parked can wake.
//!
//! Exact logic semantics:
//! - **Sequence**: `s0`'s branch runs to its end, or until every path of it has parked, before
//!   `s1` starts; and so on.
//! - **Delay**: parks its branch for `ms` (or the `ms` data input, clamped to 24 h); `cancel`
//!   drops this instance's pending delays of that node.
//! - **Wait until**: parks until the next local `HH:MM` (server clock incl. the test clock), the
//!   `release` input (releases every wait of that node in this instance), or the timeout, which
//!   fires `timeout` instead of `out`. Without a time and a timeout it waits for `release` only;
//!   an instance with nothing else that could release it ends.
//! - **Debounce**: a new event at the node replaces its pending one in this instance; `out` fires
//!   `ms` after the last one. With retrigger `restart` (the default) every firing restarts the
//!   instance, which restarts the debounce window, so a burst of firings yields one `out`.
//! - **Throttle**: passes an event if the last pass of that node was ≥ `ms` ago (state kept across
//!   firings until the next deploy); others end there.
//! - **Random**: one of `r0..r{n-1}`, uniformly; with `noRepeat` never the last pick of that node.
//! - **Once / Counter / Gate / Random / Throttle state** lives per flow in memory and resets on
//!   deploy and project open.
//! - **Run flow**: fires the target's first Run by flow trigger with the `value` input; nesting
//!   ≤ [`FLOW_RUN_DEPTH_MAX`]; with `wait` the branch parks until the target instance ends
//!   (finished, failed or stopped). A chain started by the operator (a cue) runs while
//!   Automation is off.
//! - **Repeat**: `in` starts a loop (ignored while that node's loop runs in the instance). Each
//!   round fires `body` and waits until the body's whole subtree has finished: every event and
//!   parked branch descending from `body` carries the loop's round *tag* (nested loops carry
//!   their own, linked to it), and a round is complete when nothing carries its tag. Then, after
//!   `delayMs` (a parked branch of the instance task), the next round starts; after `count`
//!   rounds `done` fires. The pacing guard holds a round until [`FLOW_REPEAT_PACE_MS`] after
//!   the previous one of that node started, with one warning per run ("Looping faster than every
//!   100 ms — paced.": a live debug error on the node and a `running` flow-state message on it;
//!   every later `running` of that run carries the same message).
//!   `stop` drops what is left of the loop (pending events, parked delays/waits/Run flow waits,
//!   nested loops) and fires `done`. `round` = the current round (from 1); data memos are
//!   cleared at each round start. The step budget is per round of the outermost loop.
//! - **Stop flow**: stops the target's instance like the Stop button ("Stopped by <flow
//!   name>"), then `done` (also when it isn't running). Its own flow: this instance ends after
//!   the node, without `done`.
//! - **Actions**: a missing target is *skipped* (noted in the final message); any other error is
//!   noted and makes the final state `failed`. With the `failed` port wired, a failure follows it
//!   instead of `done` and is not noted.
//!
//! Limits: ≤ [`FLOW_STEPS_MAX`] event node runs per firing, or per round of an outermost Repeat
//! for what runs in its rounds ("Too many steps": the instance
//! stops, failed), ≤ [`FLOW_FIRINGS_PER_SEC`] firings per flow in a sliding second (extra
//! firings are dropped and reported once per window as `failed` on the trigger node),
//! ≤ [`FLOW_INSTANCES_MAX`] running instances.
//!
//! `flow-state`: `running` at the start (node = the trigger; `at` = the new `lastFiredAt`) and at
//! each action node, then exactly one of `finished`, `failed` or `stopped`. Only the instance's
//! current generation emits, under the engine lock, so a stop is never followed by a stale
//! report. `flow-log` (`{ id, node, message, at }`): a Log action's toast, ≤ 1 per 2 s per flow.
//!
//! # Live debug (`flow-debug`)
//! The node editor subscribes to a flow (`subscribe_flow_debug`, counted per flow; cleared by
//! `forget` on project close and reload, and when the flow is deleted). Only for subscribed flows
//! the engine records into a pending batch per flow, under the engine lock it already takes:
//! every node run (the trigger at the firing, every other node when the walk pops it; latest
//! `at` per node), the event edge that delivered it (deduped) and run-time errors (a failed or
//! skipped action, a limit). An unsubscribed flow costs one emptiness check of the subscription
//! map per node run, no allocation. A changed deploy drops the pending batch (old ids); the
//! subscription stays. One flusher task exists only while a batch is pending: it sleeps
//! [`DEBUG_GAP`], takes every pending batch under the lock, emits them outside it (no ordering
//! with `flow-state` is needed) and exits; the next recorded run starts a new one. So a flow gets
//! at most one batch per [`DEBUG_GAP`] and nothing runs at rest.
//!
//! Locking: the engine lock is a data lock, held briefly; never while calling the hub, the screen
//! store, the source store or the flow store. Events are emitted under it (ordering), except
//! `flow-debug`.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::core::events::names;
use crate::core::fsx;
use crate::core::live::is_reserved_screen;
use crate::core::localtime::Now;
use crate::core::model::{
    CompareOp, Constant, Flow, FlowDebug, FlowDebugNode, FlowIssue, FlowLog, FlowRun, FlowState,
    ItemPick, ItemStatus, MathOp, NodeKind, NowNext, PauseMode, Retrigger, RowPick, ScheduleAction,
    ScheduleOp, SourceKind, SourceStep, StudioOp, TakeTransition, TimerAction, VariantTransition,
    WireKind, FLOW_DELAY_MAX_MS, FLOW_FIRINGS_PER_SEC, FLOW_INSTANCES_MAX, FLOW_REPEAT_PACE_MS,
    FLOW_RUN_DEPTH_MAX, FLOW_STEPS_MAX,
};
use crate::core::protocol::Fields;
use crate::core::schedule;
use crate::core::sources;
use crate::core::state::{AppState, WeakState};
use crate::error::{AppError, AppResult};

static GEN: AtomicU64 = AtomicU64::new(1);

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Minimum time between two Log toasts of one flow.
const TOAST_GAP: Duration = Duration::from_secs(2);
/// Depth of nested data pulls (the graph is validated acyclic; this only guards the walk).
const PULL_DEPTH_MAX: u32 = 64;
/// Live debug: at most one `flow-debug` per flow per this long.
pub const DEBUG_GAP: Duration = Duration::from_millis(100);
/// Live debug: run-time errors kept per batch.
const DEBUG_ERRORS_MAX: usize = 32;

// ---- Values ------------------------------------------------------------------------------------

/// A value on a data wire.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Text(String),
    Num(f64),
    Bool(bool),
    /// Epoch ms in the schedule frame (the server clock, incl. the test clock).
    Time(i64),
    Row(Fields),
    Rows(Vec<Fields>),
}

fn num_text(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// `HH`, `mm`, `ss` of a local time.
fn time_text(ms: i64, fmt: &str, now: &Now) -> String {
    let dt = now.zone.to_local(ms);
    fmt.replace("HH", &format!("{:02}", dt.hour()))
        .replace("mm", &format!("{:02}", dt.minute()))
        .replace("ss", &format!("{:02}", dt.second()))
}

impl Val {
    /// The implicit conversion to text (time as local `HH:MM`; rows as JSON).
    fn text(&self, now: &Now) -> String {
        match self {
            Val::Text(s) => s.clone(),
            Val::Num(n) => num_text(*n),
            Val::Bool(b) => b.to_string(),
            Val::Time(ms) => time_text(*ms, "HH:mm", now),
            Val::Row(r) => serde_json::to_string(r).unwrap_or_default(),
            Val::Rows(r) => serde_json::to_string(r).unwrap_or_default(),
        }
    }

    fn num(&self) -> Option<f64> {
        match self {
            Val::Text(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()),
            Val::Num(n) => Some(*n),
            Val::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Val::Time(ms) => Some(*ms as f64),
            Val::Row(_) | Val::Rows(_) => None,
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0,
            Val::Text(s) => matches!(s.trim(), "true" | "1" | "yes"),
            Val::Time(_) => true,
            Val::Row(r) => !r.is_empty(),
            Val::Rows(r) => !r.is_empty(),
        }
    }
}

// ---- Compiled graphs ---------------------------------------------------------------------------

/// Index into a flow's string table.
type S = u32;
const NONE: S = u32::MAX;

/// A node's runtime config (see the module docs). Strings are [`S`] indexes; large configs are
/// [`Extra`] indexes.
#[derive(Debug, Clone, Copy)]
enum Op {
    Cue,
    ItemStatus {
        source: S,
        status: ItemStatus,
        item: S,
    },
    BeforeNext {
        source: S,
        minutes: u32,
    },
    TimeOfDay {
        at: S,
        days: u8,
    },
    TimerZero {
        screen: S,
        layer: S,
        key: S,
    },
    LayerAir {
        screen: S,
        layer: S,
        on_air: bool,
    },
    SlideshowWrap {
        source: S,
    },
    SourceRowChanged {
        source: S,
    },
    ApiChanged {
        source: S,
        /// `NONE`: any row.
        column: S,
    },
    Osc {
        address: S,
    },
    Midi {
        /// `NONE`: any device.
        device: S,
        /// 1–16, 0: any channel.
        channel: u8,
        kind: crate::core::model::MidiKind,
        number: u8,
    },
    MediaEnded {
        screen: S,
        layer: S,
    },
    RunByFlow,
    Rest,
    SourceRow {
        source: S,
        row: RowPick,
    },
    ScheduleItem {
        source: S,
        which: NowNext,
    },
    FieldValue {
        screen: S,
        layer: S,
        key: S,
    },
    LayerState {
        screen: S,
        layer: S,
    },
    Constant {
        x: u32,
    },
    TimeNow,
    Format {
        pattern: S,
    },
    Math {
        op: MathOp,
        a: f64,
        b: f64,
    },
    Compare {
        op: CompareOp,
        a: S,
        b: S,
    },
    Branch,
    Switch {
        cases: u32,
    },
    Gate {
        open: bool,
    },
    Delay {
        ms: u64,
    },
    WaitUntil {
        at: S,
        timeout_ms: u64,
    },
    Debounce {
        ms: u64,
    },
    Throttle {
        ms: u64,
    },
    Sequence {
        count: u8,
    },
    Once,
    Counter {
        every: u32,
    },
    Random {
        count: u8,
        no_repeat: bool,
    },
    /// `count` 0 = forever.
    Repeat {
        count: u32,
        delay_ms: u64,
    },
    Take {
        screen: S,
        layer: S,
        for_ms: Option<u32>,
    },
    Clear {
        screen: S,
        layer: S,
    },
    ClearScreen {
        screen: S,
    },
    ClearAll,
    SetFields {
        screen: S,
        layer: S,
        fields: u32,
    },
    SetTimer {
        screen: S,
        layer: S,
        key: S,
        action: u32,
    },
    StepSource {
        source: S,
        step: SourceStep,
    },
    PauseSource {
        source: S,
        mode: PauseMode,
    },
    Schedule {
        source: S,
        x: u32,
    },
    RunFlow {
        flow: S,
        wait: bool,
    },
    StopFlow {
        flow: S,
    },
    SetVariant {
        screen: S,
        layer: S,
        variant: S,
        transition: VariantTransition,
    },
    TakePreview {
        screen: S,
        transition: Option<TakeTransition>,
    },
    Log {
        message: S,
        toast: bool,
    },
    Media {
        screen: S,
        layer: S,
        action: crate::core::model::MediaAction,
        /// `u32::MAX`: none.
        ms: u32,
    },
}

impl Op {
    fn is_action(&self) -> bool {
        matches!(
            self,
            Op::Take { .. }
                | Op::Clear { .. }
                | Op::ClearScreen { .. }
                | Op::ClearAll
                | Op::SetFields { .. }
                | Op::SetTimer { .. }
                | Op::StepSource { .. }
                | Op::PauseSource { .. }
                | Op::Schedule { .. }
                | Op::RunFlow { .. }
                | Op::StopFlow { .. }
                | Op::SetVariant { .. }
                | Op::TakePreview { .. }
                | Op::Log { .. }
                | Op::Media { .. }
        )
    }

    fn is_trigger(&self) -> bool {
        matches!(
            self,
            Op::Cue
                | Op::ItemStatus { .. }
                | Op::BeforeNext { .. }
                | Op::TimeOfDay { .. }
                | Op::TimerZero { .. }
                | Op::LayerAir { .. }
                | Op::SlideshowWrap { .. }
                | Op::SourceRowChanged { .. }
                | Op::ApiChanged { .. }
                | Op::Osc { .. }
                | Op::Midi { .. }
                | Op::MediaEnded { .. }
                | Op::RunByFlow
                | Op::Rest
        )
    }

    /// The source a node reads or acts on (API sources poll while a flow uses them).
    fn source(&self) -> Option<S> {
        match *self {
            Op::ItemStatus { source, .. }
            | Op::BeforeNext { source, .. }
            | Op::SlideshowWrap { source }
            | Op::SourceRowChanged { source }
            | Op::ApiChanged { source, .. }
            | Op::SourceRow { source, .. }
            | Op::ScheduleItem { source, .. }
            | Op::StepSource { source, .. }
            | Op::PauseSource { source, .. }
            | Op::Schedule { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Rare or large configs, out of line.
#[derive(Debug, Clone)]
enum Extra {
    Text(S),
    Num(f64),
    Bool(bool),
    Cases(Box<[S]>),
    Fields(Box<[(S, S)]>),
    Timer(TimerAction),
    Sched(Box<(ItemPick, ScheduleOp)>),
}

/// One node: its op and the ends of its id in [`Compiled::ids`] and of its edge ranges (each
/// starts where the previous node's ends). 32 bytes.
#[derive(Debug, Clone, Copy)]
struct CNode {
    op: Op,
    id_end: u16,
    ev_end: u16,
    data_end: u16,
}

/// Event edges (grouped by source node): `node`/`port` = the target, `other` = the source port,
/// `id` = the edge's index in [`Compiled::edge_id`] (live debug).
/// Data edges (grouped by target node): `node`/`port` = the source, `other` = the target port,
/// `id` = [`NO_EDGE`].
#[derive(Debug, Clone, Copy)]
struct Edge {
    node: u16,
    port: u16,
    other: u16,
    id: u16,
}

/// No edge (a data edge's id; the event that starts a walk at a trigger).
const NO_EDGE: u16 = u16::MAX;

/// A deployed flow, compiled (see the module docs).
#[derive(Debug)]
pub(crate) struct Compiled {
    name: Box<str>,
    retrigger: Retrigger,
    /// Hash of the deployed graph without positions: an identical deploy is a no-op.
    hash: u64,
    /// Node ids, concatenated.
    ids: String,
    /// Event edge ids, concatenated, and where each ends (for live debug).
    eids: String,
    eid_ends: Box<[u16]>,
    nodes: Box<[CNode]>,
    ev: Box<[Edge]>,
    data: Box<[Edge]>,
    strs: Box<[Box<str>]>,
    extra: Box<[Extra]>,
    cue: Option<u16>,
    run_by: Option<u16>,
    /// The `trigger.rest` node (the Control API's `run_flow`).
    rest: Option<u16>,
}

struct Interner {
    strs: Vec<Box<str>>,
    map: HashMap<String, S>,
}

impl Interner {
    fn s(&mut self, v: &str) -> S {
        if let Some(i) = self.map.get(v) {
            return *i;
        }
        let i = self.strs.len() as S;
        self.strs.push(v.into());
        self.map.insert(v.to_string(), i);
        i
    }

    fn opt(&mut self, v: Option<&str>) -> S {
        v.map_or(NONE, |v| self.s(v))
    }
}

fn days_mask(days: &[u8]) -> u8 {
    days.iter()
        .filter(|d| **d < 7)
        .fold(0u8, |m, d| m | (1 << d))
}

fn days_of(mask: u8) -> Vec<u8> {
    (0..7).filter(|d| mask & (1 << d) != 0).collect()
}

fn graph_hash(flow: &Flow) -> u64 {
    let nodes: Vec<(&str, &NodeKind)> = flow
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), &n.kind))
        .collect();
    let text = serde_json::to_string(&(nodes, &flow.edges)).unwrap_or_default();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Compiles a deployed graph. Defensive: edges with unknown ends are skipped (the store
/// validates; this never fails).
pub(crate) fn compile(flow: &Flow) -> Compiled {
    let mut i = Interner {
        strs: Vec::new(),
        map: HashMap::new(),
    };
    let mut extra: Vec<Extra> = Vec::new();
    let mut push = |x: Extra| {
        extra.push(x);
        (extra.len() - 1) as u32
    };
    let n = flow.nodes.len().min(u16::MAX as usize);
    let nodes_in = &flow.nodes[..n];
    let pos: HashMap<&str, u16> = nodes_in
        .iter()
        .enumerate()
        .map(|(k, x)| (x.id.as_str(), k as u16))
        .collect();
    let mut ops = Vec::with_capacity(n);
    let mut ids = String::new();
    let mut ends = Vec::with_capacity(n);
    let mut cue = None;
    let mut run_by = None;
    let mut rest = None;
    for (k, node) in nodes_in.iter().enumerate() {
        ids.push_str(&node.id);
        ends.push(ids.len().min(u16::MAX as usize) as u16);
        let op = match &node.kind {
            NodeKind::Cue { .. } => {
                cue.get_or_insert(k as u16);
                Op::Cue
            }
            NodeKind::ItemStatus {
                source_id,
                status,
                item_id,
            } => Op::ItemStatus {
                source: i.s(source_id),
                status: *status,
                item: i.opt(item_id.as_deref()),
            },
            NodeKind::BeforeNext { source_id, minutes } => Op::BeforeNext {
                source: i.s(source_id),
                minutes: *minutes,
            },
            NodeKind::TimeOfDay { at, days } => Op::TimeOfDay {
                at: i.s(at),
                days: days_mask(days),
            },
            NodeKind::TimerZero {
                screen_id,
                layer_id,
                key,
            } => Op::TimerZero {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                key: i.s(key),
            },
            NodeKind::LayerAir {
                screen_id,
                layer_id,
                on_air,
            } => Op::LayerAir {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                on_air: *on_air,
            },
            NodeKind::SlideshowWrap { source_id } => Op::SlideshowWrap {
                source: i.s(source_id),
            },
            NodeKind::SourceRowChanged { source_id } => Op::SourceRowChanged {
                source: i.s(source_id),
            },
            NodeKind::ApiChanged { source_id, column } => Op::ApiChanged {
                source: i.s(source_id),
                column: column
                    .as_deref()
                    .filter(|c| !c.is_empty())
                    .map_or(NONE, |c| i.s(c)),
            },
            NodeKind::Osc { address } => Op::Osc {
                address: i.s(address),
            },
            NodeKind::Midi {
                device,
                channel,
                kind,
                number,
            } => Op::Midi {
                device: device.as_deref().map_or(NONE, |d| i.s(d)),
                channel: channel.unwrap_or(0),
                kind: *kind,
                number: *number,
            },
            NodeKind::MediaEnded {
                screen_id,
                layer_id,
            } => Op::MediaEnded {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
            },
            NodeKind::Media {
                screen_id,
                layer_id,
                action,
                ms,
            } => Op::Media {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                action: *action,
                ms: ms.map_or(u32::MAX, |v| u32::try_from(v).unwrap_or(u32::MAX - 1)),
            },
            NodeKind::RunByFlow {} => {
                run_by.get_or_insert(k as u16);
                Op::RunByFlow
            }
            NodeKind::Rest {} => {
                rest.get_or_insert(k as u16);
                Op::Rest
            }
            NodeKind::SourceRow { source_id, row } => Op::SourceRow {
                source: i.s(source_id),
                row: *row,
            },
            NodeKind::ScheduleItem { source_id, which } => Op::ScheduleItem {
                source: i.s(source_id),
                which: *which,
            },
            NodeKind::FieldValue {
                screen_id,
                layer_id,
                key,
            } => Op::FieldValue {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                key: i.s(key),
            },
            NodeKind::LayerState {
                screen_id,
                layer_id,
            } => Op::LayerState {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
            },
            NodeKind::Constant { value } => Op::Constant {
                x: push(match value {
                    Constant::Text(t) => Extra::Text(i.s(t)),
                    Constant::Number(n) => Extra::Num(*n),
                    Constant::Bool(b) => Extra::Bool(*b),
                }),
            },
            NodeKind::TimeNow {} => Op::TimeNow,
            NodeKind::Format { pattern } => Op::Format {
                pattern: i.s(pattern),
            },
            NodeKind::Math { op, a, b } => Op::Math {
                op: *op,
                a: *a,
                b: *b,
            },
            NodeKind::Compare { op, a, b } => Op::Compare {
                op: *op,
                a: i.s(a),
                b: i.s(b),
            },
            NodeKind::Branch {} => Op::Branch,
            NodeKind::Switch { cases } => Op::Switch {
                cases: push(Extra::Cases(cases.iter().map(|c| i.s(c)).collect())),
            },
            NodeKind::Gate { open } => Op::Gate { open: *open },
            NodeKind::Delay { ms } => Op::Delay { ms: *ms },
            NodeKind::WaitUntil { at, timeout_ms } => Op::WaitUntil {
                at: i.opt(at.as_deref()),
                timeout_ms: timeout_ms.unwrap_or(u64::MAX),
            },
            NodeKind::Debounce { ms } => Op::Debounce { ms: *ms },
            NodeKind::Throttle { ms } => Op::Throttle { ms: *ms },
            NodeKind::Sequence { count } => Op::Sequence { count: *count },
            NodeKind::Once {} => Op::Once,
            NodeKind::Counter { every } => Op::Counter { every: *every },
            NodeKind::Random { count, no_repeat } => Op::Random {
                count: *count,
                no_repeat: *no_repeat,
            },
            NodeKind::Repeat { count, delay_ms } => Op::Repeat {
                count: count.unwrap_or(0),
                delay_ms: *delay_ms,
            },
            NodeKind::Take {
                screen_id,
                layer_id,
                for_ms,
            } => Op::Take {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                for_ms: *for_ms,
            },
            NodeKind::Clear {
                screen_id,
                layer_id,
            } => Op::Clear {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
            },
            NodeKind::ClearScreen { screen_id } => Op::ClearScreen {
                screen: i.s(screen_id),
            },
            NodeKind::ClearAll {} => Op::ClearAll,
            NodeKind::SetFields {
                screen_id,
                layer_id,
                fields,
            } => Op::SetFields {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                fields: push(Extra::Fields(
                    fields.iter().map(|(k, v)| (i.s(k), i.s(v))).collect(),
                )),
            },
            NodeKind::SetTimer {
                screen_id,
                layer_id,
                key,
                action,
            } => Op::SetTimer {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                key: i.s(key),
                action: push(Extra::Timer(*action)),
            },
            NodeKind::StepSource { source_id, step } => Op::StepSource {
                source: i.s(source_id),
                step: *step,
            },
            NodeKind::PauseSource { source_id, mode } => Op::PauseSource {
                source: i.s(source_id),
                mode: *mode,
            },
            NodeKind::Schedule {
                source_id,
                item,
                op,
            } => Op::Schedule {
                source: i.s(source_id),
                x: push(Extra::Sched(Box::new((item.clone(), op.clone())))),
            },
            NodeKind::RunFlow { flow_id, wait } => Op::RunFlow {
                flow: i.s(flow_id),
                wait: *wait,
            },
            NodeKind::StopFlow { flow_id } => Op::StopFlow { flow: i.s(flow_id) },
            NodeKind::SetVariant {
                screen_id,
                layer_id,
                variant,
                transition,
            } => Op::SetVariant {
                screen: i.s(screen_id),
                layer: i.s(layer_id),
                variant: i.s(variant),
                transition: *transition,
            },
            NodeKind::TakePreview {
                screen_id,
                transition,
            } => Op::TakePreview {
                screen: i.s(screen_id),
                transition: *transition,
            },
            NodeKind::Log { message, toast } => Op::Log {
                message: i.s(message),
                toast: *toast,
            },
        };
        ops.push(op);
    }
    // Edges: event edges grouped by source node, data edges by target node (stable: edge order).
    let mut ev: Vec<(u16, Edge)> = Vec::new();
    let mut data: Vec<(u16, Edge)> = Vec::new();
    let mut eids = String::new();
    let mut eid_ends: Vec<u16> = Vec::new();
    for e in &flow.edges {
        let (Some(&from), Some(&to)) = (pos.get(e.from.node.as_str()), pos.get(e.to.node.as_str()))
        else {
            continue;
        };
        let (fp, tp) = (i.s(&e.from.port), i.s(&e.to.port));
        if fp > u16::MAX as u32 || tp > u16::MAX as u32 {
            continue;
        }
        match e.kind {
            WireKind::Event => {
                let id = if eid_ends.len() < NO_EDGE as usize
                    && eids.len() + e.id.len() <= u16::MAX as usize
                {
                    eids.push_str(&e.id);
                    eid_ends.push(eids.len() as u16);
                    (eid_ends.len() - 1) as u16
                } else {
                    NO_EDGE
                };
                ev.push((
                    from,
                    Edge {
                        node: to,
                        port: tp as u16,
                        other: fp as u16,
                        id,
                    },
                ))
            }
            WireKind::Data => data.push((
                to,
                Edge {
                    node: from,
                    port: fp as u16,
                    other: tp as u16,
                    id: NO_EDGE,
                },
            )),
        }
    }
    ev.sort_by_key(|(n, _)| *n);
    data.sort_by_key(|(n, _)| *n);
    // u16 offsets: the store caps a flow far below (200 nodes, 400 edges, ids ≤ 32 bytes).
    ev.truncate(u16::MAX as usize);
    data.truncate(u16::MAX as usize);
    let end = |list: &[(u16, Edge)], k: u16| list.partition_point(|(n, _)| *n <= k) as u16;
    let nodes: Vec<CNode> = ops
        .into_iter()
        .enumerate()
        .map(|(k, op)| CNode {
            op,
            id_end: ends[k],
            ev_end: end(&ev, k as u16),
            data_end: end(&data, k as u16),
        })
        .collect();
    Compiled {
        name: flow.name.as_str().into(),
        retrigger: flow.retrigger,
        hash: graph_hash(flow),
        ids,
        eids,
        eid_ends: eid_ends.into(),
        nodes: nodes.into(),
        ev: ev.into_iter().map(|(_, e)| e).collect(),
        data: data.into_iter().map(|(_, e)| e).collect(),
        strs: i.strs.into(),
        extra: extra.into(),
        cue,
        run_by,
        rest,
    }
}

impl Compiled {
    fn s(&self, i: S) -> &str {
        self.strs.get(i as usize).map_or("", |s| s)
    }

    fn opt(&self, i: S) -> Option<&str> {
        (i != NONE).then(|| self.s(i))
    }

    fn node_id(&self, k: u16) -> &str {
        let k = k as usize;
        let Some(n) = self.nodes.get(k) else {
            return "";
        };
        let start = if k == 0 {
            0
        } else {
            self.nodes[k - 1].id_end as usize
        };
        self.ids.get(start..n.id_end as usize).unwrap_or("")
    }

    /// An event edge's id (empty for [`NO_EDGE`]).
    fn edge_id(&self, k: u16) -> &str {
        let k = k as usize;
        let Some(end) = self.eid_ends.get(k) else {
            return "";
        };
        let start = if k == 0 {
            0
        } else {
            self.eid_ends[k - 1] as usize
        };
        self.eids.get(start..*end as usize).unwrap_or("")
    }

    fn op(&self, k: u16) -> Op {
        self.nodes[k as usize].op
    }

    /// The previous node (where node `k`'s ranges start).
    fn prev(&self, k: u16) -> Option<&CNode> {
        (k as usize).checked_sub(1).and_then(|p| self.nodes.get(p))
    }

    fn ev_out(&self, k: u16) -> &[Edge] {
        let start = self.prev(k).map_or(0, |p| p.ev_end as usize);
        let end = self.nodes[k as usize].ev_end as usize;
        self.ev.get(start..end).unwrap_or(&[])
    }

    fn data_in(&self, k: u16) -> &[Edge] {
        let start = self.prev(k).map_or(0, |p| p.data_end as usize);
        let end = self.nodes[k as usize].data_end as usize;
        self.data.get(start..end).unwrap_or(&[])
    }

    fn has_out(&self, k: u16, port: &str) -> bool {
        self.ev_out(k).iter().any(|e| self.s(e.other as S) == port)
    }

    fn extra(&self, x: u32) -> Option<&Extra> {
        self.extra.get(x as usize)
    }

    /// Bytes held by this compiled graph (struct sizes plus heap), for the memory budget.
    pub(crate) fn approx_bytes(&self) -> usize {
        use std::mem::size_of;
        let extra: usize = self
            .extra
            .iter()
            .map(|x| {
                size_of::<Extra>()
                    + match x {
                        Extra::Cases(c) => c.len() * size_of::<S>(),
                        Extra::Fields(f) => f.len() * size_of::<(S, S)>(),
                        Extra::Sched(_) => size_of::<(ItemPick, ScheduleOp)>(),
                        _ => 0,
                    }
            })
            .sum();
        size_of::<Compiled>()
            + self.name.len()
            + self.ids.len()
            + self.eids.len()
            + self.eid_ends.len() * size_of::<u16>()
            + self.nodes.len() * size_of::<CNode>()
            + (self.ev.len() + self.data.len()) * size_of::<Edge>()
            + self
                .strs
                .iter()
                .map(|s| size_of::<Box<str>>() + s.len())
                .sum::<usize>()
            + extra
    }
}

// ---- Trigger index -----------------------------------------------------------------------------

/// What a trigger node is keyed by in the [`Index`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TriggerKey {
    Cue(String),
    ItemStatus(String),
    BeforeNext(String),
    TimeOfDay,
    TimerZero(String, String, String),
    Layer(String, String),
    SlideshowWrap(String),
    SourceRow(String),
    ApiChanged(String),
    /// `trigger.osc` with an exact address (patterns with `*` are in `Index::osc_wild`).
    Osc(String),
    /// `trigger.midi` by kind and number (device and channel are checked when it fires).
    Midi(crate::core::model::MidiKind, u8),
    /// `trigger.mediaEnded` by screen and layer.
    MediaEnded(String, String),
    RunByFlow(String),
    Rest(String),
}

/// A trigger node of an armed deployed flow.
#[derive(Debug, Clone)]
pub(crate) struct TrigRef {
    flow: Arc<Compiled>,
    id: Arc<str>,
    node: u16,
}

/// A trigger's config as the detection in `core::rules` reads it.
pub(crate) enum Trig<'a> {
    ItemStatus {
        status: ItemStatus,
        item: Option<&'a str>,
    },
    BeforeNext {
        source: &'a str,
        minutes: u32,
    },
    TimeOfDay {
        at: &'a str,
        days: Vec<u8>,
    },
    TimerZero {
        screen: &'a str,
        layer: &'a str,
        key: &'a str,
    },
    Layer {
        screen: &'a str,
        layer: &'a str,
        on_air: bool,
    },
    Other,
}

impl TrigRef {
    /// `flowId/nodeId`: what the detection's once-only bookkeeping is keyed by.
    pub(crate) fn key(&self) -> String {
        format!("{}/{}", self.id, self.flow.node_id(self.node))
    }

    pub(crate) fn trig(&self) -> Trig<'_> {
        let c = &self.flow;
        match c.op(self.node) {
            Op::ItemStatus { status, item, .. } => Trig::ItemStatus {
                status,
                item: c.opt(item),
            },
            Op::BeforeNext { source, minutes } => Trig::BeforeNext {
                source: c.s(source),
                minutes,
            },
            Op::TimeOfDay { at, days } => Trig::TimeOfDay {
                at: c.s(at),
                days: days_of(days),
            },
            Op::TimerZero { screen, layer, key } => Trig::TimerZero {
                screen: c.s(screen),
                layer: c.s(layer),
                key: c.s(key),
            },
            Op::LayerAir {
                screen,
                layer,
                on_air,
            } => Trig::Layer {
                screen: c.s(screen),
                layer: c.s(layer),
                on_air,
            },
            _ => Trig::Other,
        }
    }
}

/// The trigger index (see the module docs).
#[derive(Debug, Default)]
pub(crate) struct Index {
    by_key: HashMap<TriggerKey, Vec<TrigRef>>,
    /// Every source a node of an armed flow reads or acts on (API sources poll while used).
    sources: std::collections::HashSet<String>,
    timed: Vec<TrigRef>,
    layers: Vec<TrigRef>,
    timer_zero: bool,
    /// `trigger.osc` patterns with a `*` segment (matched one by one).
    osc_wild: Vec<TrigRef>,
    /// Any `trigger.osc` (exact or pattern).
    osc: bool,
    /// Any `trigger.midi` (MIDI inputs open while one is armed).
    midi: bool,
}

impl Index {
    fn build(flows: &HashMap<Arc<str>, Entry>) -> Self {
        let mut ix = Index::default();
        // Deterministic order (by flow id) so fan-in of several flows is stable.
        let mut list: Vec<(&Arc<str>, &Entry)> = flows.iter().filter(|(_, e)| e.armed).collect();
        list.sort_by(|a, b| a.0.cmp(b.0));
        for (id, entry) in list {
            let c = &entry.c;
            for k in 0..c.nodes.len() as u16 {
                let op = c.op(k);
                if let Some(src) = op.source() {
                    ix.sources.insert(c.s(src).to_string());
                }
                if !op.is_trigger() {
                    continue;
                }
                let t = TrigRef {
                    flow: c.clone(),
                    id: id.clone(),
                    node: k,
                };
                let key = match op {
                    Op::Cue => TriggerKey::Cue(id.to_string()),
                    Op::ItemStatus { source, .. } => TriggerKey::ItemStatus(c.s(source).into()),
                    Op::BeforeNext { source, .. } => {
                        ix.timed.push(t.clone());
                        TriggerKey::BeforeNext(c.s(source).into())
                    }
                    Op::TimeOfDay { .. } => {
                        ix.timed.push(t.clone());
                        TriggerKey::TimeOfDay
                    }
                    Op::TimerZero { screen, layer, key } => {
                        ix.timed.push(t.clone());
                        ix.timer_zero = true;
                        TriggerKey::TimerZero(
                            c.s(screen).into(),
                            c.s(layer).into(),
                            c.s(key).into(),
                        )
                    }
                    Op::LayerAir { screen, layer, .. } => {
                        ix.layers.push(t.clone());
                        TriggerKey::Layer(c.s(screen).into(), c.s(layer).into())
                    }
                    Op::SlideshowWrap { source } => TriggerKey::SlideshowWrap(c.s(source).into()),
                    Op::SourceRowChanged { source } => TriggerKey::SourceRow(c.s(source).into()),
                    Op::ApiChanged { source, .. } => TriggerKey::ApiChanged(c.s(source).into()),
                    Op::Osc { address } => {
                        ix.osc = true;
                        let address = c.s(address);
                        if crate::core::osc::has_wildcard(address) {
                            ix.osc_wild.push(t);
                            continue;
                        }
                        TriggerKey::Osc(address.into())
                    }
                    Op::Midi { kind, number, .. } => {
                        ix.midi = true;
                        TriggerKey::Midi(kind, number)
                    }
                    Op::RunByFlow => TriggerKey::RunByFlow(id.to_string()),
                    Op::Rest => TriggerKey::Rest(id.to_string()),
                    Op::MediaEnded { screen, layer } => {
                        TriggerKey::MediaEnded(c.s(screen).into(), c.s(layer).into())
                    }
                    _ => continue,
                };
                ix.by_key.entry(key).or_default().push(t);
            }
        }
        ix
    }

    pub(crate) fn get(&self, key: &TriggerKey) -> &[TrigRef] {
        self.by_key.get(key).map_or(&[], |v| v)
    }

    /// Whether a node of an armed flow reads or acts on the source.
    pub(crate) fn uses_source(&self, source_id: &str) -> bool {
        self.sources.contains(source_id)
    }

    /// `beforeNext`, `timeOfDay` and `timerZero` triggers (the planner's).
    pub(crate) fn timed(&self) -> &[TrigRef] {
        &self.timed
    }

    /// `layer` triggers.
    pub(crate) fn layers(&self) -> &[TrigRef] {
        &self.layers
    }

    pub(crate) fn has_timer_zero(&self) -> bool {
        self.timer_zero
    }

    /// Whether an armed deployed flow has a `trigger.midi` (MIDI is in use).
    pub(crate) fn uses_midi(&self) -> bool {
        self.midi
    }

    /// Schedules named by `itemStatus` / `beforeNext` triggers (watched by the planner).
    pub(crate) fn schedule_sources(&self) -> impl Iterator<Item = &str> {
        self.by_key.keys().filter_map(|k| match k {
            TriggerKey::ItemStatus(s) | TriggerKey::BeforeNext(s) => Some(s.as_str()),
            _ => None,
        })
    }
}

// ---- Engine state ------------------------------------------------------------------------------

struct Entry {
    c: Arc<Compiled>,
    armed: bool,
}

/// A running instance.
struct Inst {
    gen: u64,
    abort: Option<AbortHandle>,
    /// Run-flow callers waiting for this instance to end: (their channel, their token).
    waiters: Vec<(mpsc::UnboundedSender<u64>, u64)>,
}

impl Inst {
    fn end(self) {
        if let Some(a) = self.abort {
            a.abort();
        }
        for (tx, token) in self.waiters {
            let _ = tx.send(token);
        }
    }
}

#[derive(Default)]
struct Rate {
    times: VecDeque<Instant>,
    reported: Option<Instant>,
}

/// Logic state of one node (Once, Counter, Gate, Random, Throttle).
#[derive(Debug, Default, Clone, Copy)]
struct NodeState {
    open: Option<bool>,
    count: u32,
    done: bool,
    last: Option<u8>,
    passed: Option<Instant>,
}

#[derive(Default)]
struct Eng {
    flows: HashMap<Arc<str>, Entry>,
    index: Arc<Index>,
    off: bool,
    insts: HashMap<Arc<str>, Inst>,
    logic: HashMap<Arc<str>, HashMap<u16, NodeState>>,
    rate: HashMap<Arc<str>, Rate>,
    toast: HashMap<Arc<str>, Instant>,
    rng: u64,
    /// Live debug: flows an editor subscribed to (only those; empty = nothing is recorded).
    debug: HashMap<Arc<str>, DebugSub>,
    /// The generation of the live debug flusher task (0 = none).
    flusher: u64,
}

/// Live debug of one subscribed flow.
#[derive(Default)]
struct DebugSub {
    subs: u32,
    /// What ran since the last `flow-debug` (`None` = nothing).
    pending: Option<Batch>,
}

/// A pending `flow-debug` batch of one flow.
#[derive(Default)]
struct Batch {
    /// Latest run per node, in first-run order.
    nodes: Vec<FlowDebugNode>,
    /// Event edges traversed, deduped.
    edges: Vec<String>,
    errors: Vec<FlowIssue>,
}

impl Batch {
    fn node(&mut self, id: &str, at: String) {
        match self.nodes.iter_mut().find(|n| n.node == id) {
            Some(n) => n.at = at,
            None => self.nodes.push(FlowDebugNode {
                node: id.to_string(),
                at,
            }),
        }
    }

    fn edge(&mut self, id: &str) {
        if !id.is_empty() && !self.edges.iter().any(|e| e == id) {
            self.edges.push(id.to_string());
        }
    }

    fn error(&mut self, node: Option<&str>, message: String) {
        if self.errors.len() < DEBUG_ERRORS_MAX {
            self.errors.push(FlowIssue {
                node: node.map(str::to_string),
                message,
            });
        }
    }
}

impl Eng {
    /// The pending debug batch of a flow an editor subscribed to; `None` (one emptiness check,
    /// no allocation) when nobody subscribed to it.
    fn debug_batch(&mut self, id: &str) -> Option<&mut Batch> {
        if self.debug.is_empty() {
            return None;
        }
        let d = self.debug.get_mut(id)?;
        Some(d.pending.get_or_insert_with(Batch::default))
    }

    /// Whether a batch waits for a flusher that doesn't exist yet.
    fn debug_needs_flusher(&self) -> bool {
        self.flusher == 0 && self.debug.values().any(|d| d.pending.is_some())
    }

    /// Starts the flusher if a batch waits for one (`handle` fetched before the lock).
    fn debug_spawn(&mut self, st: &AppState, handle: &tokio::runtime::Handle) {
        if self.debug_needs_flusher() {
            let gen = next_gen();
            self.flusher = gen;
            handle.spawn(debug_flusher(st.downgrade(), gen));
        }
    }

    fn reindex(&mut self) {
        self.index = Arc::new(Index::build(&self.flows));
    }

    fn random(&mut self) -> u64 {
        if self.rng == 0 {
            self.rng = (fsx::now_epoch_ms() ^ 0x9E37_79B9_7F4A_7C15) | 1;
        }
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }
}

/// The engine's state. Owned by `AppState`.
#[derive(Default)]
pub struct FlowEngine {
    inner: Mutex<Eng>,
    /// Live debug flushers that took their batches and are still emitting them (outside the
    /// engine lock). Counted by [`FlowEngine::task_count`], so "no task" means every batch has
    /// been emitted.
    emitting: AtomicUsize,
    /// Tests: the Repeat pacing interval in ms (0 = [`FLOW_REPEAT_PACE_MS`]).
    #[cfg(test)]
    pace_ms: AtomicU64,
    /// Tests: the walk's state size at the end of every segment (the memory test).
    #[cfg(test)]
    walk_bytes: Mutex<Vec<usize>>,
}

fn emit_state(
    st: &AppState,
    id: &str,
    state: FlowRun,
    node: Option<&str>,
    message: Option<String>,
    at: String,
) {
    st.notify_change(crate::core::state::StateChange::FlowRun {
        id,
        running: state == FlowRun::Running,
    });
    let payload = FlowState {
        id: id.to_string(),
        state,
        node: node.map(str::to_string),
        message,
        at,
    };
    match serde_json::to_value(&payload) {
        Ok(v) => st.events().emit(names::FLOW_STATE, v),
        Err(e) => tracing::warn!("flow-state payload: {e}"),
    }
}

impl FlowEngine {
    /// The Repeat pacing interval ([`FLOW_REPEAT_PACE_MS`]; tests may shorten it).
    fn pace(&self) -> Duration {
        #[cfg(test)]
        {
            let ms = self.pace_ms.load(Ordering::Relaxed);
            if ms != 0 {
                // Never 0: a round always waits a little (no busy loop).
                return Duration::from_millis(ms.max(1));
            }
        }
        Duration::from_millis(FLOW_REPEAT_PACE_MS)
    }

    /// Engine tasks alive: running instances (each one task; its Delay / Wait / Debounce waits
    /// park inside it) and the live debug flusher while a batch is pending. The planner is
    /// counted by `Rules::task_count`.
    pub fn task_count(&self) -> usize {
        let e = lock(&self.inner);
        e.insts.len() + usize::from(e.flusher != 0) + self.emitting.load(Ordering::SeqCst)
    }

    /// Live debug subscriptions of a flow (0 when none).
    pub fn debug_subscribers(&self, id: &str) -> u32 {
        lock(&self.inner).debug.get(id).map_or(0, |d| d.subs)
    }

    /// Whether the instance `w` is still the current one; if so and an editor subscribed to its
    /// flow, records the node run (and the event edge that got there). Returns (current, a
    /// flusher must be started).
    fn step(&self, w: &Walk, node: u16, edge: u16) -> (bool, bool) {
        let mut e = lock(&self.inner);
        if !e.insts.get(&w.id).is_some_and(|i| i.gen == w.gen) {
            return (false, false);
        }
        let idle = e.flusher == 0;
        let Some(b) = e.debug_batch(&w.id) else {
            return (true, false);
        };
        b.node(w.c.node_id(node), fsx::now_rfc3339());
        if edge != NO_EDGE {
            b.edge(w.c.edge_id(edge));
        }
        (true, idle)
    }

    /// Records a run-time error of a subscribed flow. Returns whether a flusher must be started.
    fn debug_error(&self, id: &str, node: Option<&str>, message: impl FnOnce() -> String) -> bool {
        let mut e = lock(&self.inner);
        let idle = e.flusher == 0;
        match e.debug_batch(id) {
            Some(b) => {
                b.error(node, message());
                idle
            }
            None => false,
        }
    }

    /// Whether an instance of the flow is running.
    pub fn is_running(&self, id: &str) -> bool {
        lock(&self.inner).insts.contains_key(id)
    }

    /// Whether Automation is off.
    pub fn automation_off(&self) -> bool {
        lock(&self.inner).off
    }

    /// Whether the flow is deployed and armed (MIDI cue LEDs).
    pub(crate) fn is_armed(&self, id: &str) -> bool {
        lock(&self.inner).flows.get(id).is_some_and(|e| e.armed)
    }

    /// The trigger index of the armed deployed flows.
    pub(crate) fn index(&self) -> Arc<Index> {
        lock(&self.inner).index.clone()
    }

    /// Whether any armed flow listens to the source's row changes (cheap; for the hooks).
    pub(crate) fn watches_source_row(&self, source_id: &str) -> bool {
        !lock(&self.inner)
            .index
            .get(&TriggerKey::SourceRow(source_id.to_string()))
            .is_empty()
    }

    /// Bytes held by the compiled graphs (for the budget).
    pub fn compiled_bytes(&self) -> usize {
        lock(&self.inner)
            .flows
            .values()
            .map(|e| e.c.approx_bytes())
            .sum()
    }

    /// Stops every instance (with `flow-state: stopped` and `message` when `Some`); forgets
    /// nothing else.
    fn stop_all(&self, st: &AppState, message: Option<&str>) {
        let mut e = lock(&self.inner);
        let insts: Vec<(Arc<str>, Inst)> = e.insts.drain().collect();
        for (id, inst) in insts {
            inst.end();
            if let Some(m) = message {
                emit_state(
                    st,
                    &id,
                    FlowRun::Stopped,
                    None,
                    Some(m.to_string()),
                    fsx::now_rfc3339(),
                );
            }
        }
    }

    /// Forgets everything (project closed): no instance, no flow, no state.
    pub(crate) fn forget(&self) {
        let mut e = lock(&self.inner);
        for (_, inst) in e.insts.drain() {
            inst.end();
        }
        *e = Eng::default();
    }
}

// ---- Firing ------------------------------------------------------------------------------------

/// Why the Control API couldn't run a flow ([`AppState::run_flow_rest`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestRunRefusal {
    /// No such flow, or it has no API call trigger.
    NotFound,
    Disarmed,
    /// Over the per-flow firing limit.
    TooOften,
    /// No runtime (the app is starting or closing).
    Unavailable,
}

/// Why a firing did not start an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fired {
    Started,
    Ignored,
    Dropped,
    Blocked,
    Disarmed,
    Missing,
    NoRuntime,
}

struct Firing {
    id: Arc<str>,
    /// The compiled graph the trigger belongs to (a stale index entry is dropped).
    flow: Option<Arc<Compiled>>,
    node: u16,
    data: Vec<(&'static str, Val)>,
    operator: bool,
    depth: u32,
    waiter: Option<(mpsc::UnboundedSender<u64>, u64)>,
}

/// A parked branch. `tag`: the Repeat round it belongs to (see [`Round`]).
struct Parked {
    node: u16,
    kind: Park,
    wake: Option<tokio::time::Instant>,
    tag: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Park {
    Delay,
    /// Wait until: `timeout` = the wake is the timeout (fires `timeout`), else `out`.
    Wait {
        timeout: bool,
    },
    Debounce,
    /// A Run flow waiting for its target: the token the target's end sends back.
    Flow {
        token: u64,
    },
    /// A Repeat waiting to start its next round (the delay between rounds, the pacing guard).
    /// Tagged with the Repeat's own round, so the loop counts as busy meanwhile.
    Round,
}

/// An event to deliver: (node, input port, edge index or [`NO_EDGE`]) and the Repeat round it
/// belongs to (0 = none).
#[derive(Debug, Clone, Copy)]
struct Ev {
    node: u16,
    port: u16,
    edge: u16,
    tag: u32,
}

/// The input port of a synthetic event that starts a Repeat's next round (after its wait).
const NEXT_ROUND: u16 = u16::MAX;

/// An active Repeat loop of an instance. Every event and parked branch descending from its
/// `body` carries its `tag` (a nested Repeat's events carry the nested loop's tag, whose
/// `parent` is this one), so a round is complete exactly when no event, parked branch or nested
/// loop carries its tag any more ([`Walk::busy`]).
#[derive(Debug, Clone, Copy)]
struct Round {
    tag: u32,
    node: u16,
    /// The round tag of the event that started the loop (0 = none).
    parent: u32,
    /// The current round, from 1 (0 before the first one starts).
    round: u32,
    /// Node runs in the current round (outermost loops only: nested ones count in theirs).
    steps: u32,
}

/// Per Repeat node of an instance: when its last round started (the pacing guard) and its last
/// round number (the `round` output).
#[derive(Debug, Clone, Copy)]
struct RepNode {
    node: u16,
    last_start: Option<tokio::time::Instant>,
    round: u32,
}

/// The pacing warning: "Looping faster than every 100 ms — paced.".
fn paced_message() -> String {
    format!("Looping faster than every {FLOW_REPEAT_PACE_MS} ms — paced.")
}

/// Notes kept for the final message (distinct ones; a long loop adds no more).
const NOTES_MAX: usize = 16;

/// One instance's walk (see the module docs).
struct Walk {
    c: Arc<Compiled>,
    id: Arc<str>,
    gen: u64,
    depth: u32,
    operator: bool,
    /// RFC 3339 of the firing (the flow's new `lastFiredAt`).
    at: String,
    trigger: u16,
    tdata: Vec<(&'static str, Val)>,
    /// Events to deliver.
    stack: Vec<Ev>,
    parked: Vec<Parked>,
    /// Node runs outside any Repeat round.
    steps: u32,
    notes: Vec<String>,
    /// The last action that failed (not a missing target).
    failed: Option<u16>,
    memo: HashMap<(u16, u16), Val>,
    tx: mpsc::UnboundedSender<u64>,
    next_token: u64,
    /// The round tag of the event being run (0 = outside any Repeat round).
    tag: u32,
    next_tag: u32,
    /// Active Repeat loops (at most one per Repeat node).
    rounds: Vec<Round>,
    reps: Vec<RepNode>,
    /// The pacing warning was given (once per run).
    paced: bool,
    /// A Stop flow of this flow ended the instance: the walk stops after that node.
    halted: bool,
}

impl Walk {
    /// Delivers `port` of `node` to its wires in the current round: pushed in reverse so the
    /// first wire runs first.
    fn out(&mut self, node: u16, port: &str) {
        self.out_tagged(node, port, self.tag);
    }

    fn out_tagged(&mut self, node: u16, port: &str, tag: u32) {
        let start = self.stack.len();
        for e in self.c.ev_out(node) {
            if self.c.s(e.other as S) == port {
                self.stack.push(Ev {
                    node: e.node,
                    port: e.port,
                    edge: e.id,
                    tag,
                });
            }
        }
        self.stack[start..].reverse();
    }

    fn park(&mut self, node: u16, kind: Park, after: Option<Duration>) {
        self.parked.push(Parked {
            node,
            kind,
            wake: after.map(|d| tokio::time::Instant::now() + d),
            tag: self.tag,
        });
    }

    /// Whether something parked can still wake (a time or a waited-for flow).
    fn can_wake(&self) -> bool {
        self.parked
            .iter()
            .any(|p| p.wake.is_some() || matches!(p.kind, Park::Flow { .. }))
    }

    fn earliest(&self) -> Option<tokio::time::Instant> {
        self.parked.iter().filter_map(|p| p.wake).min()
    }

    /// Resumes the branches parked until `now` or earlier, earliest first.
    fn wake_due(&mut self, now: tokio::time::Instant) {
        let mut due: Vec<Parked> = Vec::new();
        let mut keep = Vec::with_capacity(self.parked.len());
        for p in self.parked.drain(..) {
            if p.wake.is_some_and(|w| w <= now) {
                due.push(p);
            } else {
                keep.push(p);
            }
        }
        self.parked = keep;
        due.sort_by_key(|p| p.wake);
        for p in due.into_iter().rev() {
            let port = match p.kind {
                Park::Round => {
                    self.stack.push(Ev {
                        node: p.node,
                        port: NEXT_ROUND,
                        edge: NO_EDGE,
                        tag: p.tag,
                    });
                    continue;
                }
                Park::Wait { timeout: true } => "timeout",
                Park::Flow { .. } => "done",
                _ => "out",
            };
            self.out_tagged(p.node, port, p.tag);
        }
    }

    fn flow_done(&mut self, token: u64) {
        if let Some(pos) = self
            .parked
            .iter()
            .position(|p| p.kind == Park::Flow { token })
        {
            let p = self.parked.remove(pos);
            self.out_tagged(p.node, "done", p.tag);
        }
    }

    fn note(&mut self, text: String) {
        if self.notes.len() < NOTES_MAX && !self.notes.contains(&text) {
            self.notes.push(text);
        }
    }

    fn round_of(&self, tag: u32) -> Option<&Round> {
        if tag == 0 {
            return None;
        }
        self.rounds.iter().find(|r| r.tag == tag)
    }

    /// The outermost active loop that round `tag` belongs to (whose step budget it uses).
    fn root_of(&self, tag: u32) -> Option<usize> {
        let mut cur = self.round_of(tag)?;
        // Parents are older loops of this instance, so the chain ends; bounded anyway.
        for _ in 0..=self.rounds.len() {
            match self.round_of(cur.parent) {
                Some(p) => cur = p,
                None => break,
            }
        }
        let tag = cur.tag;
        self.rounds.iter().position(|r| r.tag == tag)
    }

    /// Whether round tag `tag` is `root` or nested in it.
    fn within(&self, mut tag: u32, root: u32) -> bool {
        for _ in 0..=self.rounds.len() {
            if tag == root {
                return true;
            }
            match self.round_of(tag) {
                Some(r) => tag = r.parent,
                None => return false,
            }
        }
        false
    }

    /// Whether something of round `tag` is still to run or waiting (an event, a parked branch,
    /// a nested loop).
    fn busy(&self, tag: u32) -> bool {
        self.stack.iter().any(|e| e.tag == tag)
            || self.parked.iter().any(|p| p.tag == tag)
            || self.rounds.iter().any(|r| r.parent == tag)
    }

    /// Drops loop `k` and everything of its rounds (events, parked branches, nested loops).
    fn cancel_loop(&mut self, k: usize) -> Round {
        let root = self.rounds[k];
        let tags: Vec<u32> = self
            .rounds
            .iter()
            .map(|r| r.tag)
            .filter(|t| self.within(*t, root.tag))
            .collect();
        self.stack.retain(|e| !tags.contains(&e.tag));
        self.parked.retain(|p| !tags.contains(&p.tag));
        self.rounds.retain(|r| !tags.contains(&r.tag));
        root
    }

    fn rep_node(&mut self, node: u16) -> &mut RepNode {
        let k = match self.reps.iter().position(|r| r.node == node) {
            Some(k) => k,
            None => {
                self.reps.push(RepNode {
                    node,
                    last_start: None,
                    round: 0,
                });
                self.reps.len() - 1
            }
        };
        &mut self.reps[k]
    }

    /// Bytes held by the walk's state (struct sizes plus heap capacity), for the memory test.
    #[cfg(test)]
    fn approx_bytes(&self) -> usize {
        use std::mem::size_of;
        size_of::<Walk>()
            + self.stack.capacity() * size_of::<Ev>()
            + self.parked.capacity() * size_of::<Parked>()
            + self.memo.capacity() * size_of::<((u16, u16), Val)>()
            + self.rounds.capacity() * size_of::<Round>()
            + self.reps.capacity() * size_of::<RepNode>()
            + self.notes.capacity() * size_of::<String>()
            + self.notes.iter().map(String::capacity).sum::<usize>()
            + self.tdata.capacity() * size_of::<(&str, Val)>()
    }
}

enum Seg {
    Parked,
    Over,
}

async fn instance_task(weak: WeakState, mut w: Walk, mut rx: mpsc::UnboundedReceiver<u64>) {
    loop {
        let Some(st) = weak.upgrade() else { return };
        let res = tokio::task::spawn_blocking(move || {
            let out = st.walk_segment(&mut w);
            (w, out)
        })
        .await;
        let Ok((back, out)) = res else { return };
        w = back;
        if matches!(out, Seg::Over) {
            return;
        }
        match w.earliest() {
            Some(at) => {
                tokio::select! {
                    _ = tokio::time::sleep_until(at) => {}
                    tok = rx.recv() => {
                        if let Some(t) = tok {
                            w.flow_done(t);
                        }
                    }
                }
            }
            None => {
                if let Some(t) = rx.recv().await {
                    w.flow_done(t);
                }
            }
        }
        w.wake_due(tokio::time::Instant::now());
    }
}

impl AppState {
    // ---- Store → engine hooks --------------------------------------------------------------

    /// Store → engine: every flow of the project just opened (after migration), and the
    /// persisted Automation off flag. Replaces whatever was loaded.
    pub(crate) fn engine_load(&self, flows: Vec<Flow>, automation_off: bool) {
        let compiled: Vec<(Arc<str>, Entry)> = flows
            .iter()
            .map(|f| {
                (
                    Arc::from(f.id.as_str()),
                    Entry {
                        c: Arc::new(compile(f)),
                        armed: f.armed,
                    },
                )
            })
            .collect();
        {
            let eng = self.flow_engine();
            eng.stop_all(self, Some("Stopped: flows reloaded"));
            let mut e = lock(&eng.inner);
            e.flows = compiled.into_iter().collect();
            e.logic.clear();
            e.rate.clear();
            e.toast.clear();
            e.off = automation_off;
            e.reindex();
        }
        self.live().set_hold_countdowns(automation_off);
        self.rearm();
        self.rules_baseline();
        self.replan();
        self.api_reconcile();
        self.midi_reconcile();
    }

    /// Store → engine: a flow was created or its deployed graph changed (save, deploy, watcher).
    /// Compiles it and swaps it in atomically; a running instance is stopped ("Stopped by
    /// deploy"), logic state resets, pending timed clears stay. An identical graph (positions
    /// aside) only updates the name, retrigger and armed flag.
    pub(crate) fn engine_deploy(&self, flow: &Flow) {
        let c = compile(flow);
        let id: Arc<str> = Arc::from(flow.id.as_str());
        {
            let eng = self.flow_engine();
            let mut e = lock(&eng.inner);
            let same = e
                .flows
                .get(&id)
                .filter(|x| x.c.hash == c.hash)
                .map(|x| x.armed);
            match same {
                Some(armed) => {
                    let arm_changed = armed != flow.armed;
                    // Same graph: keep instances and logic state, take the new name/retrigger.
                    let entry = Entry {
                        c: Arc::new(c),
                        armed: flow.armed,
                    };
                    e.flows.insert(id.clone(), entry);
                    if arm_changed && !flow.armed {
                        if let Some(inst) = e.insts.remove(&id) {
                            inst.end();
                            emit_state(
                                self,
                                &id,
                                FlowRun::Stopped,
                                None,
                                Some("Stopped: disarmed".into()),
                                fsx::now_rfc3339(),
                            );
                        }
                    }
                    e.reindex();
                    if !arm_changed {
                        return;
                    }
                }
                None => {
                    if let Some(inst) = e.insts.remove(&id) {
                        inst.end();
                        emit_state(
                            self,
                            &id,
                            FlowRun::Stopped,
                            None,
                            Some("Stopped by deploy".into()),
                            fsx::now_rfc3339(),
                        );
                    }
                    e.logic.remove(&id);
                    if let Some(d) = e.debug.get_mut(&id) {
                        // Node and edge ids of the old graph; the subscription stays.
                        d.pending = None;
                    }
                    e.flows.insert(
                        id.clone(),
                        Entry {
                            c: Arc::new(c),
                            armed: flow.armed,
                        },
                    );
                    e.reindex();
                }
            }
        }
        // Nothing that is already due fires (a new or re-armed trigger doesn't catch up).
        self.rules_baseline();
        self.replan();
        self.api_reconcile();
        self.midi_reconcile();
    }

    /// Store → engine: the flow was armed or disarmed (disarming stops its instance). Arming
    /// doesn't catch up on what passed while it was disarmed.
    pub(crate) fn engine_set_armed(&self, id: &str, armed: bool) {
        {
            let eng = self.flow_engine();
            let mut e = lock(&eng.inner);
            let Some(entry) = e.flows.get_mut(id) else {
                return;
            };
            if entry.armed == armed {
                return;
            }
            entry.armed = armed;
            if !armed {
                if let Some(inst) = e.insts.remove(id) {
                    inst.end();
                    emit_state(
                        self,
                        id,
                        FlowRun::Stopped,
                        None,
                        Some("Stopped: disarmed".into()),
                        fsx::now_rfc3339(),
                    );
                }
            }
            e.reindex();
        }
        self.rules_baseline();
        self.replan();
        self.api_reconcile();
        self.midi_reconcile();
    }

    /// Store → engine: the flow was deleted (stops its instance).
    pub(crate) fn engine_remove(&self, id: &str) {
        {
            let eng = self.flow_engine();
            let mut e = lock(&eng.inner);
            if let Some(inst) = e.insts.remove(id) {
                inst.end();
                emit_state(
                    self,
                    id,
                    FlowRun::Stopped,
                    None,
                    Some("Stopped: flow deleted".into()),
                    fsx::now_rfc3339(),
                );
            }
            e.flows.remove(id);
            e.logic.remove(id);
            e.rate.remove(id);
            e.toast.remove(id);
            e.debug.remove(id);
            e.reindex();
        }
        self.replan();
        self.api_reconcile();
        self.midi_reconcile();
    }

    /// `UiState.control.automationOff` changed (from `save_ui_state`). Off: automatic triggers
    /// don't fire, running instances stop, pending timed clears are cancelled, auto-advance holds
    /// and a countdown with `end: hide` stays on air at zero. On again: nothing catches up.
    pub(crate) fn engine_set_automation_off(&self, off: bool) {
        {
            let mut e = lock(&self.flow_engine().inner);
            if e.off == off {
                return;
            }
            e.off = off;
        }
        if off {
            self.flow_engine()
                .stop_all(self, Some("Stopped: automation off"));
            self.rules_cancel_clears();
            self.live().set_hold_countdowns(true);
            self.rearm();
            // Barrier: a rotation tick checks it is armed and steps under the screen write lock,
            // so one that passed its check before the disarm above finishes now; none steps
            // after this returns.
            drop(self.screens().write_lock());
        } else {
            self.live().set_hold_countdowns(false);
            self.rules_baseline();
            self.rearm();
            self.replan();
        }
    }

    /// PANIC: every instance stops ("PANIC").
    pub(crate) fn engine_panic(&self) {
        self.flow_engine().stop_all(self, Some("PANIC"));
    }

    /// Runs a flow from its Cue trigger (a cue button, a hotkey, the Control API's `run_cue`).
    /// Works while Automation is off; `invalid` when disarmed or without a Cue trigger.
    pub fn run_flow(&self, id: &str, value: Option<String>) -> AppResult<()> {
        self.project_dir()?;
        let (key, flow, node) = {
            let e = lock(&self.flow_engine().inner);
            let (key, entry) = e
                .flows
                .get_key_value(id)
                .ok_or_else(|| AppError::NotFound(format!("Flow not found: {id}")))?;
            if !entry.armed {
                return Err(AppError::Invalid(format!(
                    "Flow \"{}\" is disarmed",
                    entry.c.name
                )));
            }
            let node = entry.c.cue.ok_or_else(|| {
                AppError::Invalid(format!("Flow \"{}\" has no Cue trigger", entry.c.name))
            })?;
            (key.clone(), entry.c.clone(), node)
        };
        let data = value
            .map(|v| vec![("value", Val::Text(v))])
            .unwrap_or_default();
        match self.flow_fire(Firing {
            id: key,
            flow: Some(flow),
            node,
            data,
            operator: true,
            depth: 0,
            waiter: None,
        }) {
            Fired::NoRuntime => Err(AppError::Internal(
                "Flows can't run right now. Restart Kinetra.".into(),
            )),
            _ => Ok(()),
        }
    }

    /// Runs a flow from its API call trigger (`trigger.rest`, the Control API's `run_flow`): an
    /// outside operator action, so it runs while Automation is off, like a cue. `value` is the
    /// trigger's `value` output (empty when absent).
    pub fn run_flow_rest(&self, id: &str, value: Option<String>) -> Result<(), RestRunRefusal> {
        self.project_dir().map_err(|_| RestRunRefusal::NotFound)?;
        let (key, flow, node) = {
            let e = lock(&self.flow_engine().inner);
            let (key, entry) = e.flows.get_key_value(id).ok_or(RestRunRefusal::NotFound)?;
            let node = entry.c.rest.ok_or(RestRunRefusal::NotFound)?;
            if !entry.armed {
                return Err(RestRunRefusal::Disarmed);
            }
            (key.clone(), entry.c.clone(), node)
        };
        match self.flow_fire(Firing {
            id: key,
            flow: Some(flow),
            node,
            data: vec![("value", Val::Text(value.unwrap_or_default()))],
            operator: true,
            depth: 0,
            waiter: None,
        }) {
            Fired::Dropped => Err(RestRunRefusal::TooOften),
            Fired::Disarmed => Err(RestRunRefusal::Disarmed),
            Fired::Missing => Err(RestRunRefusal::NotFound),
            Fired::NoRuntime => Err(RestRunRefusal::Unavailable),
            _ => Ok(()),
        }
    }

    /// Stops a running instance (`flow-state: stopped`); no-op if it isn't running.
    pub fn stop_flow(&self, id: &str) -> AppResult<()> {
        self.project_dir()?;
        self.stop_instance(id, "Stopped");
        Ok(())
    }

    /// Stops the flow's running instance, if any (`flow-state: stopped` with `message`).
    fn stop_instance(&self, id: &str, message: &str) {
        let mut e = lock(&self.flow_engine().inner);
        if let Some(inst) = e.insts.remove(id) {
            inst.end();
            emit_state(
                self,
                id,
                FlowRun::Stopped,
                None,
                Some(message.to_string()),
                fsx::now_rfc3339(),
            );
        }
    }

    // ---- Firing ----------------------------------------------------------------------------

    /// An automatic trigger (from the detection in `core::rules` and the hooks) fired.
    pub(crate) fn flow_trigger(&self, t: &TrigRef, data: Vec<(&'static str, Val)>) {
        self.flow_fire(Firing {
            id: t.id.clone(),
            flow: Some(t.flow.clone()),
            node: t.node,
            data,
            operator: false,
            depth: 0,
            waiter: None,
        });
    }

    /// The current row of a text source changed by any path: `trigger.sourceRow`.
    pub(crate) fn flow_source_row_changed(&self, source_id: &str) {
        let index = self.flow_engine().index();
        let list = index.get(&TriggerKey::SourceRow(source_id.to_string()));
        if list.is_empty() {
            return;
        }
        let Some(doc) = self.sources().doc(source_id) else {
            return;
        };
        let now = self.sources().clock().now();
        let row = sources::current_row(&doc, &now);
        let (_, i) = sources::position(&doc);
        for t in list {
            self.flow_trigger(
                t,
                vec![
                    ("row", Val::Row(row.clone())),
                    ("index", Val::Num(f64::from(i))),
                ],
            );
        }
    }

    /// New rows of an API source arrived (`trigger.apiChanged`). `before` / `after`: the current
    /// row before and after. With a column, a trigger fires when that cell of the current row
    /// changed (● value: the new text); without, on any change of the rows (● row: the current
    /// row). Both carry `value` and `row`.
    pub(crate) fn flow_api_changed(&self, source_id: &str, before: &Fields, after: &Fields) {
        let index = self.flow_engine().index();
        let list = index.get(&TriggerKey::ApiChanged(source_id.to_string()));
        for t in list {
            let column = match t.flow.op(t.node) {
                Op::ApiChanged { column, .. } => t.flow.opt(column),
                _ => continue,
            };
            let value = match column {
                Some(col) => {
                    let now = after.get(col).cloned().unwrap_or_default();
                    if before.get(col).cloned().unwrap_or_default() == now {
                        continue;
                    }
                    now
                }
                None => String::new(),
            };
            self.flow_trigger(
                t,
                vec![
                    ("value", Val::Text(value)),
                    ("row", Val::Row(after.clone())),
                ],
            );
        }
    }

    /// A media layer reached its out point (hold / clear; the hub's media-ended hook): fires
    /// its `trigger.mediaEnded` triggers.
    pub(crate) fn flow_media_ended(&self, screen: &str, layer: &str) {
        let index = self.flow_engine().index();
        let key = TriggerKey::MediaEnded(screen.to_string(), layer.to_string());
        for t in index.get(&key) {
            self.flow_trigger(t, Vec::new());
        }
    }

    /// An OSC message arrived (`trigger.osc`): fires the triggers whose address is `address`
    /// exactly, then those whose pattern matches it (`*` = one segment). `value` (● value: the
    /// first argument as text) is computed only when something matches. Nothing to do without
    /// an OSC trigger.
    pub(crate) fn flow_osc(&self, address: &str, value: impl FnOnce() -> String) {
        let index = self.flow_engine().index();
        if !index.osc {
            return;
        }
        let mut hits: Vec<&TrigRef> = index
            .get(&TriggerKey::Osc(address.to_string()))
            .iter()
            .collect();
        hits.extend(index.osc_wild.iter().filter(|t| match t.flow.op(t.node) {
            Op::Osc { address: p } => crate::core::osc::address_matches(t.flow.s(p), address),
            _ => false,
        }));
        if hits.is_empty() {
            return;
        }
        let value = value();
        for t in hits {
            self.flow_trigger(t, vec![("value", Val::Text(value.clone()))]);
        }
    }

    /// A MIDI note-on (velocity > 0) or CC change arrived (`trigger.midi`): fires the triggers of
    /// that kind and number whose device and channel match (● value: the velocity or CC value).
    /// Nothing to do without a MIDI trigger.
    pub(crate) fn flow_midi(
        &self,
        device: &str,
        channel: u8,
        kind: crate::core::model::MidiKind,
        number: u8,
        value: u8,
    ) {
        let index = self.flow_engine().index();
        if !index.midi {
            return;
        }
        for t in index.get(&TriggerKey::Midi(kind, number)) {
            let hit = match t.flow.op(t.node) {
                Op::Midi {
                    device: d,
                    channel: c,
                    ..
                } => {
                    (c == 0 || c == channel)
                        && t.flow.opt(d).is_none_or(|d| self.midi_device_is(d, device))
                }
                _ => false,
            };
            if hit {
                self.flow_trigger(t, vec![("value", Val::Num(f64::from(value)))]);
            }
        }
    }

    /// Starts an instance for a firing (see the module docs). Never runs a node inline.
    fn flow_fire(&self, f: Firing) -> Fired {
        let Some(handle) = self.live().runtime_handle() else {
            tracing::warn!("no async runtime: flow {} not run", f.id);
            return Fired::NoRuntime;
        };
        let at = fsx::now_rfc3339();
        {
            let eng = self.flow_engine();
            let mut e = lock(&eng.inner);
            let Some(entry) = e.flows.get(&f.id) else {
                return Fired::Missing;
            };
            if f.flow.as_ref().is_some_and(|c| !Arc::ptr_eq(c, &entry.c)) {
                // A stale index entry: the flow was redeployed meanwhile.
                return Fired::Missing;
            }
            if !entry.armed {
                return Fired::Disarmed;
            }
            if !f.operator && e.off {
                return Fired::Blocked;
            }
            let c = entry.c.clone();
            if (f.node as usize) >= c.nodes.len() {
                return Fired::Missing;
            }
            let trigger_id = c.node_id(f.node).to_string();
            // Sliding one-second window.
            let now = Instant::now();
            let r = e.rate.entry(f.id.clone()).or_default();
            while r
                .times
                .front()
                .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1))
            {
                r.times.pop_front();
            }
            if r.times.len() >= FLOW_FIRINGS_PER_SEC as usize {
                let report = r
                    .reported
                    .is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1));
                if report {
                    r.reported = Some(now);
                    let m = format!(
                        "Fired more than {FLOW_FIRINGS_PER_SEC} times in a second: extra firings dropped"
                    );
                    if let Some(b) = e.debug_batch(&f.id) {
                        b.error(Some(&trigger_id), m.clone());
                        e.debug_spawn(self, &handle);
                    }
                    emit_state(
                        self,
                        &f.id,
                        FlowRun::Failed,
                        Some(&trigger_id),
                        Some(m),
                        at.clone(),
                    );
                }
                return Fired::Dropped;
            }
            r.times.push_back(now);
            if e.insts.contains_key(&f.id) {
                match c.retrigger {
                    Retrigger::Ignore => return Fired::Ignored,
                    Retrigger::Restart => {
                        if let Some(old) = e.insts.remove(&f.id) {
                            old.end();
                            emit_state(
                                self,
                                &f.id,
                                FlowRun::Stopped,
                                None,
                                Some("Restarted".into()),
                                at.clone(),
                            );
                        }
                    }
                }
            }
            if e.insts.len() >= FLOW_INSTANCES_MAX {
                let m = format!("Too many flows running (at most {FLOW_INSTANCES_MAX})");
                if let Some(b) = e.debug_batch(&f.id) {
                    b.error(Some(&trigger_id), m.clone());
                    e.debug_spawn(self, &handle);
                }
                emit_state(
                    self,
                    &f.id,
                    FlowRun::Failed,
                    Some(&trigger_id),
                    Some(m),
                    at.clone(),
                );
                return Fired::Dropped;
            }
            let gen = next_gen();
            let (tx, rx) = mpsc::unbounded_channel();
            let mut w = Walk {
                c: c.clone(),
                id: f.id.clone(),
                gen,
                depth: f.depth,
                operator: f.operator,
                at: at.clone(),
                trigger: f.node,
                tdata: f.data,
                stack: Vec::new(),
                parked: Vec::new(),
                steps: 0,
                notes: Vec::new(),
                failed: None,
                memo: HashMap::new(),
                tx,
                next_token: 1,
                tag: 0,
                next_tag: 1,
                rounds: Vec::new(),
                reps: Vec::new(),
                paced: false,
                halted: false,
            };
            w.out(f.node, "fired");
            if let Some(b) = e.debug_batch(&f.id) {
                b.node(&trigger_id, at.clone());
                e.debug_spawn(self, &handle);
            }
            emit_state(
                self,
                &f.id,
                FlowRun::Running,
                Some(&trigger_id),
                None,
                at.clone(),
            );
            let task = handle.spawn(instance_task(self.downgrade(), w, rx));
            e.insts.insert(
                f.id.clone(),
                Inst {
                    gen,
                    abort: Some(task.abort_handle()),
                    waiters: f.waiter.into_iter().collect(),
                },
            );
        }
        self.flow_fired(&f.id, &at);
        Fired::Started
    }

    /// Ends instance `w` (if it is still the current one): `finished`, or `failed` with the
    /// last failing node; `fail` overrides (a limit).
    fn instance_end(&self, w: &Walk, fail: Option<(u16, String)>) {
        if let Some((n, m)) = &fail {
            if self
                .flow_engine()
                .debug_error(&w.id, Some(w.c.node_id(*n)), || m.clone())
            {
                self.debug_kick();
            }
        }
        let notes = (!w.notes.is_empty()).then(|| w.notes.join("; "));
        let (state, node, message) = match (fail, w.failed) {
            (Some((n, m)), _) => (FlowRun::Failed, Some(n), Some(m)),
            (None, Some(n)) => (FlowRun::Failed, Some(n), notes),
            (None, None) => (FlowRun::Finished, None, notes),
        };
        let mut e = lock(&self.flow_engine().inner);
        if !e.insts.get(&w.id).is_some_and(|i| i.gen == w.gen) {
            return;
        }
        if let Some(mut inst) = e.insts.remove(&w.id) {
            // Ending from inside the task: don't abort ourselves.
            inst.abort = None;
            inst.end();
        }
        emit_state(
            self,
            &w.id,
            state,
            node.map(|n| w.c.node_id(n)),
            message,
            fsx::now_rfc3339(),
        );
    }

    /// Runs the instance until everything left is parked (or it ends). On the blocking pool.
    fn walk_segment(&self, w: &mut Walk) -> Seg {
        while let Some(ev) = w.stack.pop() {
            let node = ev.node;
            w.tag = ev.tag;
            let (current, kick) = self.flow_engine().step(w, node, ev.edge);
            if kick {
                self.debug_kick();
            }
            if !current || self.project_dir().is_err() {
                return Seg::Over;
            }
            if ev.port == NEXT_ROUND {
                // A Repeat's wait between rounds is over (not a node run of its own).
                if let Some(k) = w.rounds.iter().position(|r| r.tag == ev.tag) {
                    self.repeat_start(w, k);
                }
            } else {
                // The budget: per round of the outermost loop the event is in, else per firing.
                let (steps, what) = match w.root_of(ev.tag) {
                    Some(k) => {
                        w.rounds[k].steps += 1;
                        (w.rounds[k].steps, "in one Repeat round")
                    }
                    None => {
                        w.steps += 1;
                        (w.steps, "in one firing")
                    }
                };
                if steps > FLOW_STEPS_MAX {
                    self.instance_end(
                        w,
                        Some((
                            node,
                            format!("Too many steps (more than {FLOW_STEPS_MAX} node runs {what})"),
                        )),
                    );
                    return Seg::Over;
                }
                let port = w.c.s(ev.port as S).to_string();
                self.run_node(w, node, &port);
            }
            if w.halted {
                return Seg::Over;
            }
            if !w.rounds.is_empty() {
                self.repeat_settle(w);
            }
        }
        #[cfg(test)]
        lock(&self.flow_engine().walk_bytes).push(w.approx_bytes());
        if w.can_wake() {
            return Seg::Parked;
        }
        self.instance_end(w, None);
        Seg::Over
    }

    // ---- Repeat ----------------------------------------------------------------------------

    /// `in` of a Repeat: starts its loop (the first round at once, unless the pacing guard holds
    /// it). Ignored while this node's loop is already running in the instance.
    fn repeat_in(&self, w: &mut Walk, node: u16) {
        if w.rounds.iter().any(|r| r.node == node) {
            return;
        }
        let tag = w.next_tag;
        w.next_tag = w.next_tag.wrapping_add(1).max(1);
        w.rounds.push(Round {
            tag,
            node,
            parent: w.tag,
            round: 0,
            steps: 0,
        });
        let k = w.rounds.len() - 1;
        self.repeat_next(w, k, 0);
    }

    /// `stop` of a Repeat: drops what is left of its rounds (pending events, parked delays and
    /// waits, nested loops) and fires `done`. Nothing when its loop isn't running.
    fn repeat_stop(&self, w: &mut Walk, node: u16) {
        if let Some(k) = w.rounds.iter().position(|r| r.node == node) {
            let r = w.cancel_loop(k);
            w.out_tagged(node, "done", r.parent);
        }
    }

    /// Starts loop `k`'s next round after `delay_ms`, held by the pacing guard: a round never
    /// starts sooner than [`FLOW_REPEAT_PACE_MS`] after the previous one of that node started.
    fn repeat_next(&self, w: &mut Walk, k: usize, delay_ms: u64) {
        let r = w.rounds[k];
        let now = tokio::time::Instant::now();
        let wanted = now + Duration::from_millis(delay_ms.min(FLOW_DELAY_MAX_MS));
        let pace = self.flow_engine().pace();
        let paced = w.rep_node(r.node).last_start.map(|t| t + pace);
        let held = paced.is_some_and(|p| p > wanted);
        let at = paced.filter(|_| held).unwrap_or(wanted);
        if at <= now {
            self.repeat_start(w, k);
            return;
        }
        w.parked.push(Parked {
            node: r.node,
            kind: Park::Round,
            wake: Some(at),
            tag: r.tag,
        });
        if held && !w.paced {
            w.paced = true;
            self.repeat_paced(w, r.node);
        }
    }

    /// The pacing warning (once per run): a run-time error on the Repeat in live debug, and the
    /// text as the message of a `running` flow-state on that node.
    fn repeat_paced(&self, w: &Walk, node: u16) {
        let id = w.c.node_id(node);
        let message = paced_message();
        if self
            .flow_engine()
            .debug_error(&w.id, Some(id), || message.clone())
        {
            self.debug_kick();
        }
        let e = lock(&self.flow_engine().inner);
        if e.insts.get(&w.id).is_some_and(|i| i.gen == w.gen) {
            emit_state(
                self,
                &w.id,
                FlowRun::Running,
                Some(id),
                Some(message),
                w.at.clone(),
            );
        }
    }

    /// Starts loop `k`'s next round now: `body` fires in the loop's round (a fresh step budget
    /// for an outermost loop; data read again).
    fn repeat_start(&self, w: &mut Walk, k: usize) {
        let now = tokio::time::Instant::now();
        let r = &mut w.rounds[k];
        r.round = r.round.saturating_add(1);
        if w.round_of(w.rounds[k].parent).is_none() {
            w.rounds[k].steps = 0;
        }
        let r = w.rounds[k];
        let rep = w.rep_node(r.node);
        rep.last_start = Some(now);
        rep.round = r.round;
        w.memo.clear();
        w.out_tagged(r.node, "body", r.tag);
    }

    /// Ends every round whose body has finished: the loop's next round (after its delay), or
    /// `done` after the last one.
    fn repeat_settle(&self, w: &mut Walk) {
        // Each pass ends a loop or makes it busy again (a pushed body or a parked wait).
        for _ in 0..=w.rounds.len() * 2 {
            let Some(k) = w.rounds.iter().position(|r| !w.busy(r.tag)) else {
                return;
            };
            let r = w.rounds[k];
            let Op::Repeat { count, delay_ms } = w.c.op(r.node) else {
                w.rounds.remove(k);
                continue;
            };
            if count != 0 && r.round >= count {
                w.rounds.remove(k);
                w.out_tagged(r.node, "done", r.parent);
            } else {
                self.repeat_next(w, k, delay_ms);
            }
        }
    }

    fn with_node_state<R>(
        &self,
        w: &Walk,
        node: u16,
        f: impl FnOnce(&mut NodeState, &mut u64) -> R,
    ) -> R {
        let mut guard = lock(&self.flow_engine().inner);
        let e = &mut *guard;
        if e.rng == 0 {
            e.random();
        }
        let s = e
            .logic
            .entry(w.id.clone())
            .or_default()
            .entry(node)
            .or_default();
        f(s, &mut e.rng)
    }

    fn run_node(&self, w: &mut Walk, node: u16, port: &str) {
        let op = w.c.op(node);
        if op.is_action() {
            self.run_action_node(w, node, op);
            return;
        }
        match op {
            Op::Branch => {
                let c = self
                    .pull(w, node, "condition", 0)
                    .is_some_and(|v| v.truthy());
                w.out(node, if c { "then" } else { "else" });
            }
            Op::Switch { cases } => {
                let now = self.sources().clock().now();
                let v = self
                    .pull(w, node, "value", 0)
                    .map(|v| v.text(&now))
                    .unwrap_or_default();
                let hit = match w.c.extra(cases) {
                    Some(Extra::Cases(list)) => list.iter().position(|c| w.c.s(*c) == v),
                    _ => None,
                };
                match hit {
                    Some(i) => w.out(node, &format!("case{i}")),
                    None => w.out(node, "default"),
                }
            }
            Op::Gate { open } => {
                let pass = self.with_node_state(w, node, |s, _| {
                    let cur = s.open.get_or_insert(open);
                    match port {
                        "open" => {
                            *cur = true;
                            false
                        }
                        "close" => {
                            *cur = false;
                            false
                        }
                        _ => *cur,
                    }
                });
                if pass {
                    w.out(node, "out");
                }
            }
            Op::Delay { ms } => match port {
                "cancel" => w
                    .parked
                    .retain(|p| !(p.node == node && p.kind == Park::Delay)),
                _ => {
                    let ms = self
                        .pull(w, node, "ms", 0)
                        .and_then(|v| v.num())
                        .map_or(ms, |n| n.max(0.0) as u64)
                        .min(FLOW_DELAY_MAX_MS);
                    w.park(node, Park::Delay, Some(Duration::from_millis(ms)));
                }
            },
            Op::WaitUntil { at, timeout_ms } => match port {
                "release" => {
                    let released: Vec<usize> = w
                        .parked
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.node == node && matches!(p.kind, Park::Wait { .. }))
                        .map(|(i, _)| i)
                        .collect();
                    for i in released.into_iter().rev() {
                        // Each released branch goes on in its own round.
                        let p = w.parked.remove(i);
                        w.out_tagged(node, "out", p.tag);
                    }
                }
                _ => {
                    let now = self.sources().clock().now();
                    let until =
                        w.c.opt(at)
                            .and_then(|t| crate::core::rules::next_time_of_day(t, &[], &now))
                            .map(|t| Duration::from_millis(u64::try_from(t - now.ms).unwrap_or(0)));
                    let timeout = (timeout_ms != u64::MAX)
                        .then(|| Duration::from_millis(timeout_ms.min(FLOW_DELAY_MAX_MS)));
                    match (until, timeout) {
                        (Some(u), Some(t)) if t < u => {
                            w.park(node, Park::Wait { timeout: true }, Some(t))
                        }
                        (Some(u), _) => w.park(node, Park::Wait { timeout: false }, Some(u)),
                        (None, Some(t)) => w.park(node, Park::Wait { timeout: true }, Some(t)),
                        (None, None) => w.park(node, Park::Wait { timeout: false }, None),
                    }
                }
            },
            Op::Debounce { ms } => {
                w.parked
                    .retain(|p| !(p.node == node && p.kind == Park::Debounce));
                w.park(
                    node,
                    Park::Debounce,
                    Some(Duration::from_millis(ms.min(FLOW_DELAY_MAX_MS))),
                );
            }
            Op::Throttle { ms } => {
                let now = Instant::now();
                let pass = self.with_node_state(w, node, |s, _| {
                    let ok = s
                        .passed
                        .is_none_or(|t| now.duration_since(t) >= Duration::from_millis(ms));
                    if ok {
                        s.passed = Some(now);
                    }
                    ok
                });
                if pass {
                    w.out(node, "out");
                }
            }
            Op::Sequence { count } => {
                for i in (0..count).rev() {
                    w.out(node, &format!("s{i}"));
                }
            }
            Op::Once => {
                let pass = self.with_node_state(w, node, |s, _| {
                    if port == "reset" {
                        s.done = false;
                        false
                    } else if s.done {
                        false
                    } else {
                        s.done = true;
                        true
                    }
                });
                if pass {
                    w.out(node, "out");
                }
            }
            Op::Counter { every } => {
                let pass = self.with_node_state(w, node, |s, _| {
                    if port == "reset" {
                        s.count = 0;
                        false
                    } else {
                        s.count = s.count.saturating_add(1);
                        every > 0 && s.count % every == 0
                    }
                });
                w.memo.clear();
                if pass {
                    w.out(node, "out");
                }
            }
            Op::Random { count, no_repeat } => {
                if count == 0 {
                    return;
                }
                let pick = self.with_node_state(w, node, |s, rng| {
                    let mut next = || {
                        let mut x = *rng;
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        *rng = x;
                        x
                    };
                    let mut r = (next() % u64::from(count)) as u8;
                    if no_repeat && count > 1 && s.last == Some(r) {
                        r = ((u64::from(r) + 1 + next() % u64::from(count - 1)) % u64::from(count))
                            as u8;
                    }
                    s.last = Some(r);
                    r
                });
                w.out(node, &format!("r{pick}"));
            }
            Op::Repeat { .. } => match port {
                "stop" => self.repeat_stop(w, node),
                _ => self.repeat_in(w, node),
            },
            // Triggers and data nodes take no events.
            _ => {}
        }
    }

    fn run_action_node(&self, w: &mut Walk, node: u16, op: Op) {
        {
            let e = lock(&self.flow_engine().inner);
            if !e.insts.get(&w.id).is_some_and(|i| i.gen == w.gen) {
                return;
            }
            // Once a loop of this run was paced, every `running` carries the warning (the UI
            // keeps it until a `running` without a message, i.e. a new run).
            emit_state(
                self,
                &w.id,
                FlowRun::Running,
                Some(w.c.node_id(node)),
                w.paced.then(paced_message),
                w.at.clone(),
            );
        }
        let result = match op {
            Op::RunFlow { flow, wait } => match self.run_flow_action(w, node, flow, wait) {
                Ok(true) => {
                    w.memo.clear();
                    return; // parked until the target ends
                }
                Ok(false) => Ok(()),
                Err(e) => Err(e),
            },
            Op::StopFlow { flow } => {
                let target = w.c.s(flow).to_string();
                let message = format!("Stopped by {}", w.c.name);
                if target == *w.id {
                    // This instance ends after the node (no `done`).
                    let mut e = lock(&self.flow_engine().inner);
                    if e.insts.get(&w.id).is_some_and(|i| i.gen == w.gen) {
                        if let Some(mut inst) = e.insts.remove(&w.id) {
                            // Ending from inside the task: don't abort ourselves.
                            inst.abort = None;
                            inst.end();
                        }
                        emit_state(
                            self,
                            &w.id,
                            FlowRun::Stopped,
                            None,
                            Some(message),
                            fsx::now_rfc3339(),
                        );
                    }
                    w.halted = true;
                    return;
                }
                self.stop_instance(&target, &message);
                Ok(())
            }
            _ => self.run_action(w, node, op),
        };
        w.memo.clear();
        match result {
            Ok(()) => w.out(node, "done"),
            Err(e) => {
                let kick = self
                    .flow_engine()
                    .debug_error(&w.id, Some(w.c.node_id(node)), || match &e {
                        AppError::NotFound(m) => format!("Skipped: {m}"),
                        e => e.to_string(),
                    });
                if kick {
                    self.debug_kick();
                }
                if w.c.has_out(node, "failed") {
                    w.out(node, "failed");
                    return;
                }
                match e {
                    AppError::NotFound(m) => {
                        w.note(format!("{}: skipped ({m})", w.c.node_id(node)))
                    }
                    e => {
                        w.note(format!("{}: {e}", w.c.node_id(node)));
                        w.failed = Some(node);
                    }
                }
                w.out(node, "done");
            }
        }
    }

    /// `Ok(true)`: parked waiting for the target.
    fn run_flow_action(&self, w: &mut Walk, node: u16, flow: S, wait: bool) -> AppResult<bool> {
        let target = w.c.s(flow).to_string();
        if w.depth + 1 > FLOW_RUN_DEPTH_MAX {
            return Err(AppError::Invalid(format!(
                "Run flow nested too deep (at most {FLOW_RUN_DEPTH_MAX})"
            )));
        }
        let now = self.sources().clock().now();
        let value = self.pull(w, node, "value", 0).map(|v| v.text(&now));
        let (key, c, trig) = {
            let e = lock(&self.flow_engine().inner);
            let (key, entry) = e
                .flows
                .get_key_value(target.as_str())
                .ok_or_else(|| AppError::NotFound(format!("Flow not found: {target}")))?;
            if !entry.armed {
                return Err(AppError::Invalid(format!(
                    "Flow \"{}\" is disarmed",
                    entry.c.name
                )));
            }
            let trig = entry.c.run_by.ok_or_else(|| {
                AppError::Invalid(format!(
                    "Flow \"{}\" has no Run by flow trigger",
                    entry.c.name
                ))
            })?;
            (key.clone(), entry.c.clone(), trig)
        };
        let token = w.next_token;
        w.next_token += 1;
        let name = c.name.clone();
        let fired = self.flow_fire(Firing {
            id: key,
            flow: Some(c),
            node: trig,
            data: value
                .map(|v| vec![("value", Val::Text(v))])
                .unwrap_or_else(|| vec![("value", Val::Text(String::new()))]),
            operator: w.operator,
            depth: w.depth + 1,
            waiter: wait.then(|| (w.tx.clone(), token)),
        });
        match fired {
            Fired::Started if wait => {
                w.park(node, Park::Flow { token }, None);
                Ok(true)
            }
            Fired::Started | Fired::Ignored => Ok(false),
            Fired::Blocked => Err(AppError::Invalid("Automation is off".into())),
            Fired::Dropped => Err(AppError::Invalid(format!(
                "Flow \"{name}\" didn't start (a limit was hit)"
            ))),
            Fired::Disarmed => Err(AppError::Invalid(format!("Flow \"{name}\" is disarmed"))),
            Fired::Missing => Err(AppError::NotFound(format!("Flow not found: {target}"))),
            Fired::NoRuntime => Err(AppError::Internal(
                "Flows can't run right now. Restart Kinetra.".into(),
            )),
        }
    }

    fn run_action(&self, w: &mut Walk, node: u16, op: Op) -> AppResult<()> {
        let user_screen = |s: &str| {
            if is_reserved_screen(s) {
                Err(AppError::NotFound(format!("Screen not found: {s}")))
            } else {
                Ok(())
            }
        };
        let c = w.c.clone();
        match op {
            Op::Take {
                screen,
                layer,
                for_ms,
            } => {
                let (s, l) = (c.s(screen), c.s(layer));
                user_screen(s)?;
                self.live().show(s, l)?;
                if let Some(ms) = for_ms {
                    self.clear_later(s, l, ms);
                }
                Ok(())
            }
            Op::Clear { screen, layer } => {
                let (s, l) = (c.s(screen), c.s(layer));
                user_screen(s)?;
                self.live().hide(s, l)?;
                Ok(())
            }
            Op::ClearScreen { screen } => {
                let s = c.s(screen);
                user_screen(s)?;
                self.live_clear(s).map(drop)
            }
            Op::ClearAll => self.live_clear_all().map(drop),
            Op::SetFields {
                screen,
                layer,
                fields,
            } => {
                let mut values = match c.extra(fields) {
                    Some(Extra::Fields(f)) => f
                        .iter()
                        .map(|(k, v)| (c.s(*k).to_string(), c.s(*v).to_string()))
                        .collect(),
                    _ => Fields::new(),
                };
                let now = self.sources().clock().now();
                let keys: Vec<String> = c
                    .data_in(node)
                    .iter()
                    .map(|e| c.s(e.other as S).to_string())
                    .collect();
                let (s, l) = (c.s(screen), c.s(layer));
                for k in keys {
                    let text = match self.pull(w, node, &k, 0) {
                        // Rows into a list field: only the list's columns (a source usually has
                        // more).
                        Some(Val::Rows(rows)) => {
                            let cols: Vec<String> =
                                self.live()
                                    .with_layer(s, l, |x| {
                                        x.template.fields.iter().find(|f| f.key == k).map(|f| {
                                            f.columns.iter().map(|c| c.key.clone()).collect()
                                        })
                                    })
                                    .ok()
                                    .flatten()
                                    .unwrap_or_default();
                            let rows: Vec<Fields> = rows
                                .into_iter()
                                .map(|mut r| {
                                    r.retain(|key, _| cols.contains(key));
                                    r
                                })
                                .collect();
                            Val::Rows(rows).text(&now)
                        }
                        Some(v) => v.text(&now),
                        None => continue,
                    };
                    values.insert(k, text);
                }
                // In Studio mode automation goes to Program (outside it, the live rules).
                self.set_layer_fields_auto(s, l, values).map(drop)
            }
            Op::SetTimer {
                screen,
                layer,
                key,
                action,
            } => {
                let Some(Extra::Timer(mut a)) = c.extra(action).cloned() else {
                    return Err(AppError::Invalid("Unknown timer action".into()));
                };
                if let TimerAction::Set { ms } = &mut a {
                    if let Some(n) = self.pull(w, node, "ms", 0).and_then(|v| v.num()) {
                        *ms = n.max(0.0) as u64;
                    }
                }
                self.set_timer(c.s(screen), c.s(layer), c.s(key), a)
                    .map(drop)
            }
            Op::Media {
                screen,
                layer,
                action,
                ms,
            } => {
                let ms = self
                    .pull(w, node, "ms", 0)
                    .and_then(|v| v.num())
                    .map(|n| n.max(0.0) as u64)
                    .or((ms != u32::MAX).then_some(u64::from(ms)));
                self.live()
                    .media_control(c.s(screen), c.s(layer), action, ms)
                    .map(drop)
                    .map_err(Into::into)
            }
            Op::StepSource { source, step } => {
                let step = match self.pull(w, node, "index", 0).and_then(|v| v.num()) {
                    Some(n) => SourceStep::Index {
                        index: n.max(0.0) as u32,
                    },
                    None => step,
                };
                self.step_source(c.s(source), step).map(drop)
            }
            Op::PauseSource { source, mode } => {
                let id = c.s(source);
                let paused = match mode {
                    PauseMode::Pause => true,
                    PauseMode::Resume => false,
                    PauseMode::Toggle => {
                        let doc = self.loaded_source(id)?;
                        !sources::auto_of(&doc).is_some_and(|a| a.paused)
                    }
                };
                self.set_auto_paused(id, paused)
            }
            Op::Schedule { source, x } => {
                let Some(Extra::Sched(b)) = c.extra(x) else {
                    return Err(AppError::Invalid("Unknown schedule action".into()));
                };
                let (pick, sop) = (**b).clone();
                let id = c.s(source);
                let item_id = self.resolve_item(w, node, id, &pick)?;
                let action = match sop {
                    ScheduleOp::StartNow => ScheduleAction::StartNow { item_id },
                    ScheduleOp::End => ScheduleAction::End { item_id },
                    ScheduleOp::SetNext { start } => ScheduleAction::SetNext { item_id, start },
                    ScheduleOp::Delay { minutes } => ScheduleAction::Delay { item_id, minutes },
                    ScheduleOp::SetStart { start } => ScheduleAction::SetStart { item_id, start },
                    ScheduleOp::Cancel => ScheduleAction::Cancel { item_id },
                    ScheduleOp::Restore => ScheduleAction::Restore { item_id },
                    ScheduleOp::Auto => ScheduleAction::Auto { item_id },
                };
                self.schedule_action(id, action).map(drop)
            }
            Op::SetVariant {
                screen,
                layer,
                variant,
                transition,
            } => {
                let s = c.s(screen);
                user_screen(s)?;
                // Automation goes on air directly (bypasses liveEdit = onTake).
                self.set_layer_variant_direct(s, c.s(layer), c.s(variant), transition)
                    .map(drop)
            }
            Op::TakePreview { screen, transition } => {
                let s = c.s(screen);
                user_screen(s)?;
                // The studio entry point, like Control's TAKE (fails when Studio mode is off).
                self.apply_studio_op(s, StudioOp::Take { transition })
                    .map(drop)
            }
            Op::Log { message, toast } => {
                let now = self.sources().clock().now();
                let msg = self
                    .pull(w, node, "text", 0)
                    .map(|v| v.text(&now))
                    .unwrap_or_else(|| c.s(message).to_string());
                tracing::info!("flow \"{}\" ({}): {msg}", c.name, w.id);
                if toast {
                    let mut e = lock(&self.flow_engine().inner);
                    let ok = e.toast.get(&w.id).is_none_or(|t| t.elapsed() >= TOAST_GAP);
                    if ok {
                        e.toast.insert(w.id.clone(), Instant::now());
                        let log = FlowLog {
                            id: w.id.to_string(),
                            node: c.node_id(node).to_string(),
                            message: msg,
                            at: fsx::now_rfc3339(),
                        };
                        match serde_json::to_value(&log) {
                            Ok(v) => self.events().emit(names::FLOW_LOG, v),
                            Err(e) => tracing::warn!("flow-log payload: {e}"),
                        }
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The schedule item an `action.schedule` acts on.
    fn resolve_item(
        &self,
        w: &mut Walk,
        node: u16,
        source: &str,
        pick: &ItemPick,
    ) -> AppResult<String> {
        match pick {
            ItemPick::Item { item_id } => Ok(item_id.clone()),
            ItemPick::Input => match self.pull(w, node, "item", 0) {
                Some(Val::Row(r)) => r
                    .get("_id")
                    .cloned()
                    .ok_or_else(|| AppError::Invalid("The item input has no schedule item".into())),
                Some(v) => {
                    let now = self.sources().clock().now();
                    Ok(v.text(&now))
                }
                None => Err(AppError::Invalid("The item input is not wired".into())),
            },
            ItemPick::Now | ItemPick::Next => {
                let doc = self.loaded_source(source)?;
                let SourceKind::Schedule {
                    items,
                    ripple,
                    next_override,
                    ..
                } = &doc.meta.kind
                else {
                    return Err(AppError::Invalid(format!(
                        "\u{201c}{}\u{201d} is not a schedule",
                        doc.meta.name
                    )));
                };
                let now = self.sources().clock().now();
                let c = schedule::compute(items, *ripple, next_override.as_deref(), &now);
                let (id, what) = if *pick == ItemPick::Now {
                    (c.now_id, "NOW")
                } else {
                    (c.next_id, "UP NEXT")
                };
                id.ok_or_else(|| AppError::NotFound(format!("No {what} item")))
            }
        }
    }

    // ---- Data ------------------------------------------------------------------------------

    /// The value wired into `port` of `node`, if any.
    fn pull(&self, w: &mut Walk, node: u16, port: &str, depth: u32) -> Option<Val> {
        let edge =
            w.c.data_in(node)
                .iter()
                .find(|e| w.c.s(e.other as S) == port)
                .copied()?;
        self.eval(w, edge.node, edge.port, depth + 1)
    }

    /// Output `port` of data (or trigger) node `node`, memoised until the next action.
    fn eval(&self, w: &mut Walk, node: u16, port: u16, depth: u32) -> Option<Val> {
        if depth > PULL_DEPTH_MAX {
            return None;
        }
        if let Some(v) = w.memo.get(&(node, port)) {
            return Some(v.clone());
        }
        let c = w.c.clone();
        let name = c.s(port as S);
        let now = self.sources().clock().now();
        let v = match c.op(node) {
            op if op.is_trigger() => {
                if node != w.trigger {
                    return None;
                }
                w.tdata
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.clone())
            }
            Op::SourceRow { source, row } => {
                self.eval_source_row(w, node, c.s(source), row, name, depth)
            }
            Op::ScheduleItem { source, which } => {
                self.eval_schedule_item(c.s(source), which, name, &now)
            }
            Op::FieldValue { screen, layer, key } => self
                .live()
                .with_layer(c.s(screen), c.s(layer), |l| l.fields.get(c.s(key)).cloned())
                .ok()
                .flatten()
                .map(Val::Text),
            Op::LayerState { screen, layer } => match name {
                "pending" => Some(Val::Bool(self.live().has_staged(c.s(screen), c.s(layer)))),
                _ => self
                    .live()
                    .with_layer(c.s(screen), c.s(layer), |l| Val::Bool(l.on_air))
                    .ok()
                    .or(Some(Val::Bool(false))),
            },
            Op::Constant { x } => match c.extra(x) {
                Some(Extra::Text(t)) => Some(Val::Text(c.s(*t).to_string())),
                Some(Extra::Num(n)) => Some(Val::Num(*n)),
                Some(Extra::Bool(b)) => Some(Val::Bool(*b)),
                _ => None,
            },
            Op::TimeNow => Some(Val::Time(now.ms)),
            Op::Format { pattern } => {
                Some(Val::Text(self.format(w, node, c.s(pattern), &now, depth)))
            }
            Op::Math { op, a, b } => {
                let a = self
                    .pull(w, node, "a", depth)
                    .and_then(|v| v.num())
                    .unwrap_or(a);
                let b = self
                    .pull(w, node, "b", depth)
                    .and_then(|v| v.num())
                    .unwrap_or(b);
                let r = match op {
                    MathOp::Add => a + b,
                    MathOp::Sub => a - b,
                    MathOp::Mul => a * b,
                    MathOp::Div => {
                        if b == 0.0 {
                            0.0
                        } else {
                            a / b
                        }
                    }
                    MathOp::Min => a.min(b),
                    MathOp::Max => a.max(b),
                    MathOp::Round => a.round(),
                };
                Some(Val::Num(if r.is_finite() { r } else { 0.0 }))
            }
            Op::Compare { op, a, b } => {
                let a = self
                    .pull(w, node, "a", depth)
                    .unwrap_or_else(|| Val::Text(c.s(a).to_string()));
                let b = self
                    .pull(w, node, "b", depth)
                    .unwrap_or_else(|| Val::Text(c.s(b).to_string()));
                Some(Val::Bool(compare(op, &a, &b, &now)))
            }
            Op::Counter { .. } if name == "count" => {
                let n = self.with_node_state(w, node, |s, _| s.count);
                Some(Val::Num(f64::from(n)))
            }
            // The current round (the last one after the loop ended; 0 before it ran).
            Op::Repeat { .. } if name == "round" => Some(Val::Num(f64::from(
                w.reps
                    .iter()
                    .find(|r| r.node == node)
                    .map_or(0, |r| r.round),
            ))),
            _ => None,
        };
        if let Some(v) = &v {
            w.memo.insert((node, port), v.clone());
        }
        v
    }

    fn eval_source_row(
        &self,
        w: &mut Walk,
        node: u16,
        source: &str,
        pick: RowPick,
        port: &str,
        depth: u32,
    ) -> Option<Val> {
        let doc = self.sources().doc(source)?;
        let now = self.sources().clock().now();
        let cur = sources::Current::of(&doc, &now);
        let rows: Vec<Fields> = cur
            .rows
            .iter()
            .map(|r| {
                let mut r = r.clone();
                r.retain(|k, _| !k.starts_with('_'));
                r
            })
            .collect();
        if port == "rows" {
            return Some(Val::Rows(rows));
        }
        let n = rows.len();
        let current = match &doc.meta.kind {
            SourceKind::Text { index, .. }
            | SourceKind::Api { index, .. }
            | SourceKind::Socket { index, .. } => *index as usize,
            SourceKind::Schedule {
                items,
                ripple,
                next_override,
                ..
            } => {
                let c = schedule::compute(items, *ripple, next_override.as_deref(), &now);
                c.now_id
                    .and_then(|id| c.items.iter().position(|e| items[e.index].id == id))
                    .unwrap_or(0)
            }
            // A poll is read from its first answer (the poll values are on every row).
            SourceKind::Poll { .. } => 0,
        };
        let idx = match self.pull(w, node, "index", depth).and_then(|v| v.num()) {
            Some(i) => i.max(0.0) as usize,
            None => match pick {
                RowPick::Current => current,
                RowPick::First => 0,
                RowPick::Last => n.saturating_sub(1),
                RowPick::Index { index } => index as usize,
            },
        };
        let mut row: Fields = cur
            .columns
            .iter()
            .map(|k| (k.clone(), String::new()))
            .collect();
        if let Some(r) = rows.get(idx) {
            row.extend(r.clone());
        }
        match port {
            "row" => Some(Val::Row(row)),
            "index" => Some(Val::Num(idx.min(n.saturating_sub(1)) as f64)),
            col => Some(Val::Text(row.get(col).cloned().unwrap_or_default())),
        }
    }

    fn eval_schedule_item(
        &self,
        source: &str,
        which: NowNext,
        port: &str,
        now: &Now,
    ) -> Option<Val> {
        let row = self.schedule_item_row(source, which, now);
        match port {
            "exists" => Some(Val::Bool(row.is_some())),
            "item" => Some(Val::Row(row.unwrap_or_default())),
            col => Some(Val::Text(
                row.and_then(|r| r.get(col).cloned()).unwrap_or_default(),
            )),
        }
    }

    /// The NOW / UP NEXT item of a schedule as a row (schedule columns plus `_id`).
    pub(crate) fn schedule_item_row(
        &self,
        source: &str,
        which: NowNext,
        now: &Now,
    ) -> Option<Fields> {
        let doc = self.sources().doc(source)?;
        let SourceKind::Schedule {
            items,
            ripple,
            next_override,
            labels,
        } = &doc.meta.kind
        else {
            return None;
        };
        let c = schedule::compute(items, *ripple, next_override.as_deref(), now);
        let id = match which {
            NowNext::Now => c.now_id.clone(),
            NowNext::Next => c.next_id.clone(),
        }?;
        schedule_row_of(items, labels, &c, now, &id)
    }

    fn format(&self, w: &mut Walk, node: u16, pattern: &str, now: &Now, depth: u32) -> String {
        let mut out = String::with_capacity(pattern.len());
        let mut rest = pattern;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else {
                out.push_str(&rest[open..]);
                return out;
            };
            let inner = &after[..close];
            let (key, fmt) = match inner.split_once(':') {
                Some((k, f)) => (k, Some(f)),
                None => (inner, None),
            };
            if key.len() == 1 && ("a"..="h").contains(&key) {
                let v = self.pull(w, node, key, depth);
                let text = match (v, fmt) {
                    (Some(Val::Time(ms)), Some(f)) => time_text(ms, f, now),
                    (Some(v), _) => v.text(now),
                    (None, _) => String::new(),
                };
                out.push_str(&text);
            } else {
                out.push('{');
                out.push_str(inner);
                out.push('}');
            }
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        out
    }
}

/// A schedule item's binding row (schedule columns) plus `_id`.
pub(crate) fn schedule_row_of(
    items: &[crate::core::model::ScheduleItem],
    labels: &crate::core::model::ScheduleLabels,
    c: &schedule::Computed,
    now: &Now,
    item_id: &str,
) -> Option<Fields> {
    let pos = c.items.iter().position(|e| items[e.index].id == item_id)?;
    let rows = schedule::rows(items, labels, c, now);
    let mut r = rows.into_iter().nth(pos)?;
    r.remove("_status");
    r.insert("_id".into(), item_id.to_string());
    Some(r)
}

fn compare(op: CompareOp, a: &Val, b: &Val, now: &Now) -> bool {
    let (ta, tb) = (a.text(now), b.text(now));
    let nums = match (a, b) {
        (Val::Num(x), Val::Num(y)) => Some((*x, *y)),
        _ => a.num().zip(b.num()).filter(|_| {
            !matches!(a, Val::Row(_) | Val::Rows(_)) && !matches!(b, Val::Row(_) | Val::Rows(_))
        }),
    };
    let ord = match nums {
        Some((x, y)) => x.partial_cmp(&y),
        None => Some(ta.cmp(&tb)),
    };
    use std::cmp::Ordering as O;
    match op {
        CompareOp::Eq => ord == Some(O::Equal),
        CompareOp::Ne => ord != Some(O::Equal),
        CompareOp::Lt => ord == Some(O::Less),
        CompareOp::Le => matches!(ord, Some(O::Less | O::Equal)),
        CompareOp::Gt => ord == Some(O::Greater),
        CompareOp::Ge => matches!(ord, Some(O::Greater | O::Equal)),
        CompareOp::Contains => ta.contains(tb.as_str()),
        CompareOp::Empty => ta.trim().is_empty(),
    }
}

#[cfg(test)]
mod tests;

// ---- Live debug ------------------------------------------------------------------------------

/// The live debug flusher: sleeps [`DEBUG_GAP`], then emits every pending batch (outside the
/// engine lock) and exits. A new one starts with the next recorded batch, so batches of a flow
/// are at least [`DEBUG_GAP`] apart and no task exists while nothing is pending.
async fn debug_flusher(weak: WeakState, gen: u64) {
    tokio::time::sleep(DEBUG_GAP).await;
    let Some(st) = weak.upgrade() else { return };
    let engine = st.flow_engine();
    let mut batches: Vec<FlowDebug> = {
        let mut e = lock(&engine.inner);
        if e.flusher != gen {
            // Forgotten (project closed or reloaded) meanwhile.
            return;
        }
        e.flusher = 0;
        // Still alive until the batches are out (incremented under the lock, so `task_count`
        // never sees a gap between the two).
        engine.emitting.fetch_add(1, Ordering::SeqCst);
        e.debug
            .iter_mut()
            .filter_map(|(id, d)| {
                d.pending.take().map(|b| FlowDebug {
                    id: id.to_string(),
                    nodes: b.nodes,
                    edges: b.edges,
                    errors: b.errors,
                })
            })
            .collect()
    };
    batches.sort_by(|a, b| a.id.cmp(&b.id));
    for b in batches {
        match serde_json::to_value(&b) {
            Ok(v) => st.events().emit(names::FLOW_DEBUG, v),
            Err(e) => tracing::warn!("flow-debug payload: {e}"),
        }
    }
    engine.emitting.fetch_sub(1, Ordering::SeqCst);
}

impl AppState {
    /// Starts the live debug flusher if a batch waits for one. Never called under the engine
    /// lock (it fetches the runtime handle first).
    fn debug_kick(&self) {
        let Some(handle) = self.live().runtime_handle() else {
            return;
        };
        let mut e = lock(&self.flow_engine().inner);
        e.debug_spawn(self, &handle);
    }

    /// An editor shows this flow: record what runs and emit `flow-debug` batches (≤ 10/s).
    /// Counted: every subscribe needs an unsubscribe. Cleared on project close and reload.
    /// `notFound` for a flow that doesn't exist.
    pub fn subscribe_flow_debug(&self, id: &str) -> AppResult<()> {
        self.project_dir()?;
        if !self.flows().contains(id) {
            return Err(AppError::NotFound(format!("Flow not found: {id}")));
        }
        let mut e = lock(&self.flow_engine().inner);
        let d = e.debug.entry(Arc::from(id)).or_default();
        d.subs = d.subs.saturating_add(1);
        Ok(())
    }

    /// Drops one subscription of the flow; recording stops at zero (a pending batch is
    /// dropped). Never fails: unsubscribing an unknown flow, below zero or with no project
    /// is a no-op.
    pub fn unsubscribe_flow_debug(&self, id: &str) -> AppResult<()> {
        let mut e = lock(&self.flow_engine().inner);
        if let Some(d) = e.debug.get_mut(id) {
            d.subs = d.subs.saturating_sub(1);
            if d.subs == 0 {
                e.debug.remove(id);
            }
        }
        Ok(())
    }
}
