//! `LiveHub`: the live state plus the broadcast channel renderers listen on. Shared by `AppState`
//! (Tauri commands) and the output server, which takes only an `Arc<LiveHub>` so it runs without
//! Tauri (see the `fixture_server` example).
//!
//! Every operation applies a pure `LiveState` op under the write lock, serializes each resulting
//! message ONCE and sends it on a `tokio::sync::broadcast` channel (capacity 256). All renderers
//! share the same `Arc<HubMsg>`. Messages are sent while the write lock is held, so a client that
//! subscribes and then reads a snapshot (read lock) never misses a change; at worst it sees a
//! change twice, which is harmless because `state`/`fields` carry absolute values.
//!
//! The lock is a std `RwLock`, never held across an await.
//!
//! # Auto-out
//! When a layer with `auto_out_ms` goes in (by any operation), the hub spawns ONE sleeping task on
//! a tokio runtime that hides the layer after that duration. Each layer's `out_gen` changes on
//! every on/off-air transition and template load; the task only hides the layer if the generation
//! it was scheduled for is still current, so a manual hide or a re-show cancels it. The superseded
//! task is also aborted right away, so nothing lingers. No polling.
//!
//! The runtime is injected with [`LiveHub::set_runtime`] (the app passes Tauri's runtime, the
//! fixture its own). Without one, the hub falls back to `Handle::try_current()` (the runtime of
//! the calling task, e.g. in `#[tokio::test]`); with neither, auto-out is skipped with a warning
//! and the layer simply stays on air. This keeps `core` free of Tauri types.
//!
//! A reorder or in-place update of an on-air layer (same `out_gen`) keeps its running countdown.
//!
//! # Countdown end
//! A `timer` field counting down with `end: hide` takes its layer off air at zero. The same
//! machinery: one sleeping task per on-air layer with such a running countdown (the earliest one
//! counts), keyed by the layer's `out_gen` and the zero time; any change of the timer state, an
//! off-air transition, a removal, PANIC or a reset cancels or reschedules it. No ticks.
//!
//! # Hook
//! [`LiveHub::set_hook`] installs a callback run after every operation that may change what is on
//! air (or how screens are bound: every `apply_screen`), with no hub lock held. `AppState` uses it
//! to arm and disarm source auto-advance.
//!
//! # Removing an on-air layer
//! [`LiveHub::apply_screen`] takes a removed on-air layer out of the live state at once and sends
//! `state out`, so renderers play its out-animation; `remove` follows after [`REMOVE_DELAY`] (the
//! renderer's 3 s out cap plus a margin) from a sleeping task on the same runtime, unless a layer
//! with that id is back by then. A renderer that reconnects meanwhile gets a snapshot without it.
//!
//! # `live-changed`
//! With a notifier set ([`LiveHub::set_notifier`]; `AppState` passes its event sink), every
//! operation on any path (commands, saves, watcher, auto-out timer, reset, panic) compares the
//! touched screen's [`LiveScreenView`] with the last one emitted for it and, only if it differs,
//! emits `live-changed` with the new view. So a rect drag or a no-op show emits nothing. `__*`
//! system screens are never emitted; removed screens are forgotten (not emitted). Views are
//! recorded in an outbox while the state lock is held (so their order is the order of the
//! changes) and emitted after it is released, in that order. No timers.
//!
//! # Studio mode
//! Every take, cue, T-bar move and (in Studio mode) every content edit of a screen goes through
//! ONE function, `LiveHub::studio_locked` (reached from `LiveHub::studio_op`, and from
//! `apply_screen_with` / `set_rect` while Studio mode is on). The Preview diff lives in the live
//! state (`ScreenPreview`); `preview-changed` (a `PreviewView`) is emitted like `live-changed`,
//! only when a screen's Preview view changed and only while Studio mode is on.
//!
//! **Preview bus.** Renderers on `/ws?screen=<id>&bus=preview` get Program + diff, with the same
//! message types as Program (see `core::protocol`). The hub keeps one derived screen per screen
//! that has preview subscribers ([`LiveHub::preview_attach`]) and, after every operation that
//! touches that screen, sends the difference to that bus only ([`HubMsg::preview`]). Screens
//! without preview subscribers cost nothing.
//!
//! **T-bar.** `tbar` messages are coalesced per screen: the first move goes out at once, later
//! ones at most every [`TBAR_INTERVAL`] from one sleeping task that sends the latest position
//! (none when the T-bar rests).
//!
//! # Media
//! Transport ([`LiveHub::media_control`]) acts on Program at once (Studio mode too). A media
//! layer playing to an end (`hold` or `clear`, out point known) has ONE sleeping task that
//! fires at the out point: `hold` pauses there (one `media` message), `clear` takes the layer
//! off air; then the media-ended hook runs ([`LiveHub::set_media_ended_hook`], flows' "Media
//! ended"). Pause, seek, stop, a new file or going off air reschedules or cancels it; nothing
//! is scheduled otherwise.
//!
//! **Readiness.** Each renderer connection registers with [`LiveHub::renderer_attach`] (Program
//! connections that are not app previews count) and reports `ready` per layer; the hub keeps
//! the reports by the layer's `templateRev` at the time (a new file needs new reports) and
//! writes each media layer's ready / total / first error into the live state after every
//! operation on its screen. A readiness-only change of a screen's view is announced at most
//! [`READINESS_NOTE_INTERVAL`] apart (one sleeping task while one is pending).
//!
//! Lock order: `state` before `timers` before `countdowns`, everywhere (including the timer
//! tasks); `state` before the T-bar clocks; the notifier's `last`/`outbox`/`bus`/preview maps are
//! taken briefly inside `state`; its `gate` is taken only with no other lock.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{
    Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak,
};
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::broadcast;
use tokio::task::AbortHandle;

use crate::core::events::{names, EventSink};
use crate::core::fieldvals;
use crate::core::fsx;
use crate::core::live::{
    bus_diff, is_reserved_screen, preview_of, LayerLive, LiveError, LiveLayerView, LiveScreenView,
    LiveState, PreviewView, ScreenLive, ScreenPlan, VariantSwitch,
};
use crate::core::model::{
    LiveEdit, MediaAction, MediaEndAction, MediaPlayback, MediaReadiness, PreviewAfterTake,
    StudioCues, StudioExit,
};
use crate::core::protocol::{
    FieldType, Fields, LayerState, Rect, ServerMsg, TakeTransition, WireErrorCode,
};

/// Broadcast capacity. A renderer that falls further behind gets a fresh snapshot instead.
pub const HUB_CAPACITY: usize = 256;

/// Delay between `state out` and `remove` for a layer removed while on air: the renderer's
/// out-animation cap (3 s, `OUT_CAP_MS` in renderer/src/layer.ts) plus a margin.
pub const REMOVE_DELAY: Duration = Duration::from_millis(3200);

/// Shortest time between two `tbar` messages of a screen (≈ 30 per second).
pub const TBAR_INTERVAL: Duration = Duration::from_millis(33);

/// Shortest time between two readiness-only `live-changed` of a screen (≤ 4 per second).
pub const READINESS_NOTE_INTERVAL: Duration = Duration::from_millis(250);

/// Runs (with no hub lock held) when a media layer reached its out point with `hold` or
/// `clear`: `(screen id, layer id)`.
pub type MediaEndedHook = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// A scheduled out point of a playing media layer.
struct MediaEnd {
    playback: MediaPlayback,
    abort: AbortHandle,
}

type MediaEnds = Mutex<HashMap<LayerKey, MediaEnd>>;

/// One renderer's media reports: layer id → (`LiveLayer::ready_key` reported for, ok, error).
struct RendererReports {
    screen: String,
    /// Counts toward `MediaReadiness.total` (Program, not an app preview).
    counted: bool,
    layers: HashMap<String, (u32, bool, Option<String>)>,
}

#[derive(Default)]
struct ReadyBook {
    next: u64,
    conns: HashMap<u64, RendererReports>,
    /// Readiness-only notes: last emitted per screen, and whether a delayed one is pending.
    notes: HashMap<String, (Instant, bool)>,
}

/// A renderer connection's registration for readiness (see [`LiveHub::renderer_attach`]);
/// dropping it unregisters the connection.
pub struct RendererReg {
    hub: Weak<HubShared>,
    id: u64,
}

/// What a [`RendererReg`] needs to unregister, and the exact-visibility book (`fades`).
struct HubShared {
    state: Weak<RwLock<LiveState>>,
    book: Arc<Mutex<ReadyBook>>,
    notifier: Weak<Notifier>,
    fades: Arc<Mutex<fades::FadeBook>>,
    visible_hook: Arc<HookSlot>,
}

impl Drop for RendererReg {
    fn drop(&mut self) {
        let Some(h) = self.hub.upgrade() else { return };
        let (Some(state), Some(notifier)) = (h.state.upgrade(), h.notifier.upgrade()) else {
            return;
        };
        let faded = {
            let mut s = write_lock(&state);
            let screen = lock(&h.book).conns.remove(&self.id).map(|c| c.screen);
            if let Some(screen) = screen {
                if refresh_readiness(&h.book, &mut s, &screen) {
                    notifier.note(&s, &screen);
                }
            }
            fades::forget_renderer(&h.fades, self.id)
        };
        notifier.flush();
        if faded {
            call_hook(&h.visible_hook);
        }
    }
}

/// Recomputes the readiness of every layer of a screen that has media to preload (media layers,
/// template layers with video / image fields) from the reports. Caller holds the state write
/// lock. `true` if any changed.
fn refresh_readiness(book: &Mutex<ReadyBook>, state: &mut LiveState, screen_id: &str) -> bool {
    let Some(screen) = state.screen(screen_id) else {
        return false;
    };
    let media: Vec<(String, u32)> = screen
        .layers
        .iter()
        .filter_map(|l| Some((l.id.clone(), l.ready_key()?)))
        .collect();
    if media.is_empty() {
        return false;
    }
    let readiness: Vec<(String, MediaReadiness)> = {
        let book = lock(book);
        let conns: Vec<&RendererReports> = book
            .conns
            .values()
            .filter(|c| c.counted && c.screen == screen_id)
            .collect();
        media
            .into_iter()
            .map(|(id, rev)| {
                let mut r = MediaReadiness {
                    ready: 0,
                    total: u32::try_from(conns.len()).unwrap_or(u32::MAX),
                    error: None,
                };
                for c in &conns {
                    match c.layers.get(&id) {
                        Some((v, true, _)) if *v == rev => r.ready += 1,
                        Some((v, false, e)) if *v == rev && r.error.is_none() => {
                            r.error = Some(e.clone().unwrap_or_else(|| "Can't load".into()));
                        }
                        _ => {}
                    }
                }
                (id, r)
            })
            .collect()
    };
    let mut changed = false;
    for (id, r) in readiness {
        changed |= state.set_readiness(screen_id, &id, r);
    }
    changed
}

/// One serialized protocol message for one screen, shared by every subscriber.
#[derive(Debug)]
pub struct HubMsg {
    pub screen_id: Arc<str>,
    /// The message as JSON text, serialized once.
    pub json: Arc<str>,
    /// For the screen's preview bus (`bus=preview`) instead of Program.
    pub preview: bool,
}

/// A studio operation on one screen: the operator's ops (`StudioOp`, with the project defaults
/// resolved) and the content edits that go to the Preview diff in Studio mode. All of them go
/// through `LiveHub::studio_locked`.
#[derive(Debug)]
pub enum StudioAction<'a> {
    Cue {
        layer_id: &'a str,
        visible: Option<bool>,
    },
    Take {
        transition: TakeTransition,
        after: PreviewAfterTake,
    },
    TbarBegin,
    TbarSet {
        p: f64,
    },
    TbarEnd {
        commit: bool,
        after: PreviewAfterTake,
    },
    Clear,
    /// A screen document applied by the live rules (save, Control edit, watcher, source).
    EditPlan {
        plan: ScreenPlan,
        switches: &'a [VariantSwitch],
    },
    /// A layer's box dragged.
    EditRect {
        layer_id: &'a str,
        rect: Rect,
    },
}

/// `set_studio` turning Studio mode off while a Preview has changes, without saying how.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("Preview has changes")]
pub struct PreviewHasChanges;

impl From<PreviewHasChanges> for crate::error::AppError {
    fn from(e: PreviewHasChanges) -> Self {
        Self::Invalid(e.to_string())
    }
}

/// The derived Preview of a screen with preview-bus subscribers.
struct BusScreen {
    subs: usize,
    /// What its renderers show (Program + diff), or `None` while the screen doesn't exist.
    last: Option<ScreenLive>,
}

/// T-bar message pacing of one screen.
struct TbarClock {
    last_sent: Option<Instant>,
    task: Option<AbortHandle>,
}

type TbarClocks = Mutex<HashMap<String, TbarClock>>;

/// `(screen id, layer id)`.
type LayerKey = (String, String);

/// A scheduled auto-out.
struct Timer {
    out_gen: u64,
    abort: AbortHandle,
}

type Timers = Mutex<HashMap<LayerKey, Timer>>;

/// A scheduled countdown end (`timer` field, `dir: down`, `end: hide`, running).
struct Countdown {
    out_gen: u64,
    /// Server epoch ms when it reaches zero.
    at: u64,
    abort: AbortHandle,
}

type Countdowns = Mutex<HashMap<LayerKey, Countdown>>;

/// Called (with no hub lock held) after an operation that may have changed what is on air or how
/// screens are built. `AppState` re-evaluates auto-advance arming with it.
pub type LiveHook = Arc<dyn Fn() + Send + Sync>;

type HookSlot = RwLock<Option<LiveHook>>;

/// One queued UI event.
enum Note {
    Live(LiveScreenView),
    Preview(PreviewView),
}

/// Emits `live-changed` / `preview-changed` for views that changed, feeds the preview buses and
/// tracks the persisted Studio cues (see the module docs).
struct Notifier {
    sink: RwLock<Option<Arc<dyn EventSink>>>,
    /// The last view emitted per screen.
    last: Mutex<HashMap<String, LiveScreenView>>,
    /// The last Preview view emitted per screen (Studio mode only).
    last_preview: Mutex<HashMap<String, PreviewView>>,
    /// Views to emit, in the order the changes happened.
    outbox: Mutex<Vec<Note>>,
    /// Held while draining the outbox and emitting, so views go out in order.
    gate: Mutex<()>,
    /// Preview buses with subscribers, by screen id.
    bus: Mutex<HashMap<String, BusScreen>>,
    tx: broadcast::Sender<Arc<HubMsg>>,
    /// The persisted cues last reported, per screen (see `LiveState::studio_cues`).
    cues: Mutex<HashMap<String, BTreeMap<String, bool>>>,
    /// The persisted cues changed: the cues hook runs on the next flush.
    cues_dirty: AtomicBool,
    cues_hook: RwLock<Option<LiveHook>>,
    /// Run after a flush that emitted a Preview view (remote control and the Control API show
    /// Preview; its changes don't go through the live hook, which follows Program).
    preview_hook: RwLock<Option<LiveHook>>,
}

impl Notifier {
    fn new(tx: broadcast::Sender<Arc<HubMsg>>) -> Self {
        Self {
            sink: RwLock::new(None),
            last: Mutex::new(HashMap::new()),
            last_preview: Mutex::new(HashMap::new()),
            outbox: Mutex::new(Vec::new()),
            gate: Mutex::new(()),
            bus: Mutex::new(HashMap::new()),
            tx,
            cues: Mutex::new(HashMap::new()),
            cues_dirty: AtomicBool::new(false),
            cues_hook: RwLock::new(None),
            preview_hook: RwLock::new(None),
        }
    }

    fn enabled(&self) -> bool {
        self.sink
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// After a change of the screen: feeds its preview bus, tracks its persisted cues and queues
    /// its views if they differ from the last ones emitted. Caller holds the state lock (so the
    /// queue order is the order of the changes).
    fn note(&self, state: &LiveState, screen_id: &str) {
        self.sync_bus(state, screen_id);
        if screen_id.starts_with("__") {
            return;
        }
        self.note_cues(state, screen_id);
        if !self.enabled() {
            return;
        }
        let Some(screen) = state.screen(screen_id) else {
            lock(&self.last).remove(screen_id);
            lock(&self.last_preview).remove(screen_id);
            return;
        };
        let view = screen.view_in(state.studio);
        let changed = {
            let mut last = lock(&self.last);
            let changed = last.get(screen_id) != Some(&view);
            if changed {
                last.insert(screen_id.to_string(), view.clone());
            }
            changed
        };
        if changed {
            lock(&self.outbox).push(Note::Live(view));
        }
        if !state.studio {
            return;
        }
        let pv = screen.preview_view();
        {
            let mut last = lock(&self.last_preview);
            if last.get(screen_id) == Some(&pv) {
                return;
            }
            last.insert(screen_id.to_string(), pv.clone());
        }
        lock(&self.outbox).push(Note::Preview(pv));
    }

    fn note_cues(&self, state: &LiveState, screen_id: &str) {
        let now = state
            .screen(screen_id)
            .map(ScreenLive::persisted_cues)
            .unwrap_or_default();
        let mut cues = lock(&self.cues);
        let before = cues.get(screen_id);
        if before.map_or(now.is_empty(), |b| *b == now) {
            return;
        }
        if now.is_empty() {
            cues.remove(screen_id);
        } else {
            cues.insert(screen_id.to_string(), now);
        }
        self.cues_dirty.store(true, AtomicOrdering::SeqCst);
    }

    /// Sends the preview bus of the screen what changed in its Preview, if it has subscribers.
    fn sync_bus(&self, state: &LiveState, screen_id: &str) {
        let mut bus = lock(&self.bus);
        let Some(b) = bus.get_mut(screen_id) else {
            return;
        };
        let msgs = match state.screen(screen_id) {
            None => {
                if b.last.take().is_none() {
                    return;
                }
                vec![ServerMsg::Error {
                    code: WireErrorCode::UnknownScreen,
                }]
            }
            Some(s) => {
                let mut new = preview_of(s);
                let msgs = match &b.last {
                    Some(old) => bus_diff(old, &mut new),
                    None => vec![new.snapshot()],
                };
                b.last = Some(new);
                msgs
            }
        };
        send_to(&self.tx, screen_id, &msgs, true);
    }

    /// Notes every screen of `state` and forgets screens that are gone. Caller holds the state lock.
    fn note_all(&self, state: &LiveState) {
        let gone: Vec<String> = lock(&self.bus)
            .keys()
            .filter(|id| state.screen(id).is_none())
            .cloned()
            .collect();
        for id in gone {
            self.sync_bus(state, &id);
        }
        let gone: Vec<String> = lock(&self.cues)
            .keys()
            .filter(|id| state.screen(id).is_none())
            .cloned()
            .collect();
        for id in gone {
            self.note_cues(state, &id);
        }
        lock(&self.last).retain(|id, _| state.screen(id).is_some());
        lock(&self.last_preview).retain(|id, _| state.studio && state.screen(id).is_some());
        for s in &state.screens {
            self.note(state, &s.info.id);
        }
    }

    fn forget(&self, state: &LiveState, screen_id: &str) {
        lock(&self.last).remove(screen_id);
        lock(&self.last_preview).remove(screen_id);
        self.sync_bus(state, screen_id);
        self.note_cues(state, screen_id);
    }

    /// Emits the queued views, then runs the cues hook if the persisted cues changed. Call with
    /// no state lock held.
    fn flush(&self) {
        let sink = self
            .sink
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut preview = false;
        if let Some(sink) = sink {
            let _gate = lock(&self.gate);
            loop {
                let batch = std::mem::take(&mut *lock(&self.outbox));
                if batch.is_empty() {
                    break;
                }
                for note in batch {
                    let (name, v) = match note {
                        Note::Live(v) => (names::LIVE_CHANGED, serde_json::to_value(&v)),
                        Note::Preview(v) => {
                            preview = true;
                            (names::PREVIEW_CHANGED, serde_json::to_value(&v))
                        }
                    };
                    match v {
                        Ok(v) => sink.emit(name, v),
                        Err(e) => tracing::warn!("{name} payload: {e}"),
                    }
                }
            }
        }
        if self.cues_dirty.swap(false, AtomicOrdering::SeqCst) {
            let hook = self
                .cues_hook
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(h) = hook {
                h();
            }
        }
        if preview {
            let hook = self
                .preview_hook
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(h) = hook {
                h();
            }
        }
    }
}

pub struct LiveHub {
    state: Arc<RwLock<LiveState>>,
    tx: broadcast::Sender<Arc<HubMsg>>,
    runtime: Mutex<Option<Handle>>,
    timers: Arc<Timers>,
    countdowns: Arc<Countdowns>,
    notifier: Arc<Notifier>,
    hook: Arc<HookSlot>,
    /// Automation off: no countdown end is scheduled (see [`LiveHub::set_hold_countdowns`]).
    hold_countdowns: AtomicBool,
    /// T-bar pacing per screen (see the module docs).
    tbar: Arc<TbarClocks>,
    /// Number of operations through `studio_locked` (tests: every path goes through it).
    studio_ops: AtomicU64,
    /// Out points of playing media layers (see the module docs).
    media_ends: Arc<MediaEnds>,
    media_ended: Arc<RwLock<Option<MediaEndedHook>>>,
    /// Renderer registrations and their media reports.
    shared: Arc<HubShared>,
}

fn call_hook(hook: &HookSlot) {
    let h = hook.read().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(h) = h {
        h();
    }
}

/// Whether messages may have changed what is on air.
fn on_air_changed(msgs: &[ServerMsg]) -> bool {
    msgs.iter().any(|m| {
        matches!(
            m,
            ServerMsg::State { .. }
                | ServerMsg::Cut {}
                | ServerMsg::Take { .. }
                | ServerMsg::Commit {}
        )
    })
}

/// `state in` stand-ins for the layers a take changed, for the timer bookkeeping (never sent).
fn touched_msgs(touched: &[String]) -> Vec<ServerMsg> {
    touched
        .iter()
        .map(|id| ServerMsg::State {
            layer_id: id.clone(),
            state: LayerState::In,
        })
        .collect()
}

/// Stops a screen's T-bar pacing (a pending send is cancelled).
fn stop_tbar_clock(clocks: &TbarClocks, screen_id: &str) {
    if let Some(c) = lock(clocks).remove(screen_id) {
        if let Some(t) = c.task {
            t.abort();
        }
    }
}

/// Unsubscribes from a screen's preview bus when dropped (see [`LiveHub::preview_attach`]).
pub struct PreviewSub {
    notifier: Weak<Notifier>,
    screen_id: String,
}

impl Drop for PreviewSub {
    fn drop(&mut self) {
        let Some(n) = self.notifier.upgrade() else {
            return;
        };
        let mut bus = lock(&n.bus);
        if let Some(b) = bus.get_mut(&self.screen_id) {
            b.subs = b.subs.saturating_sub(1);
            if b.subs == 0 {
                bus.remove(&self.screen_id);
            }
        }
    }
}

/// The earliest server time (epoch ms) at which a running countdown of the layer with
/// `end: hide` reaches zero.
fn countdown_at(layer: &LayerLive) -> Option<u64> {
    layer
        .template
        .fields
        .iter()
        .filter(|f| f.kind == FieldType::Timer)
        .filter_map(|f| layer.fields.get(&f.key))
        .filter_map(|v| fieldvals::parse_timer(v).ok())
        .filter_map(|s| fieldvals::countdown_hide_at(&s))
        .min()
}

impl Default for LiveHub {
    fn default() -> Self {
        Self::new(LiveState::default())
    }
}

fn read_lock(l: &RwLock<LiveState>) -> RwLockReadGuard<'_, LiveState> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

fn write_lock(l: &RwLock<LiveState>) -> RwLockWriteGuard<'_, LiveState> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveHub {
    pub fn new(state: LiveState) -> Self {
        let (tx, _) = broadcast::channel(HUB_CAPACITY);
        let state = Arc::new(RwLock::new(state));
        let notifier = Arc::new(Notifier::new(tx.clone()));
        let shared = Arc::new(HubShared {
            state: Arc::downgrade(&state),
            book: Arc::new(Mutex::new(ReadyBook::default())),
            notifier: Arc::downgrade(&notifier),
            fades: Arc::default(),
            visible_hook: Arc::new(RwLock::new(None)),
        });
        Self {
            state,
            notifier,
            shared,
            media_ends: Arc::new(Mutex::new(HashMap::new())),
            media_ended: Arc::new(RwLock::new(None)),
            tx,
            runtime: Mutex::new(None),
            timers: Arc::new(Mutex::new(HashMap::new())),
            countdowns: Arc::new(Mutex::new(HashMap::new())),
            hook: Arc::new(RwLock::new(None)),
            hold_countdowns: AtomicBool::new(false),
            tbar: Arc::new(Mutex::new(HashMap::new())),
            studio_ops: AtomicU64::new(0),
        }
    }

    /// Installs the callback run (with no hub lock held) after a Studio Preview view changed (cues
    /// or staged content) without going through the live hook.
    pub fn set_preview_hook(&self, hook: LiveHook) {
        *self
            .notifier
            .preview_hook
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// Installs the callback run (with no hub lock held) when the persisted Studio cues changed
    /// (`AppState` saves them in `ui-state.json`, coalesced).
    pub fn set_cues_hook(&self, hook: LiveHook) {
        *self
            .notifier
            .cues_hook
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// The Studio visibility cues to persist.
    pub fn studio_cues(&self) -> StudioCues {
        self.read().studio_cues()
    }

    /// Whether Studio mode is on.
    pub fn is_studio(&self) -> bool {
        self.read().studio
    }

    /// Operations that went through the studio entry point so far (tests and diagnostics).
    pub fn studio_op_count(&self) -> u64 {
        self.studio_ops.load(AtomicOrdering::Relaxed)
    }

    /// A screen's Preview view.
    pub fn preview_view(&self, screen_id: &str) -> Result<PreviewView, LiveError> {
        self.read().preview_view(screen_id)
    }

    /// Whether a layer has content staged (for the next take, or in the Preview diff).
    pub fn has_staged(&self, screen_id: &str, layer_id: &str) -> bool {
        self.read().has_staged(screen_id, layer_id)
    }

    /// Subscribes a preview-bus renderer of `screen_id`: the hub derives that screen's Preview
    /// from now on (until the returned guard and every other one is dropped). Returns the guard
    /// and the bus's current `snapshot` (`None` while the screen doesn't exist: its renderer is
    /// told `unknownScreen`). Subscribe to [`LiveHub::subscribe`] BEFORE calling this.
    pub fn preview_attach(&self, screen_id: &str) -> (PreviewSub, Option<String>) {
        let state = self.read();
        let mut bus = lock(&self.notifier.bus);
        let b = bus.entry(screen_id.to_string()).or_insert(BusScreen {
            subs: 0,
            last: None,
        });
        b.subs += 1;
        if b.last.is_none() {
            b.last = state.screen(screen_id).map(preview_of);
        }
        let snapshot = b.last.as_ref().and_then(|s| serialize(&s.snapshot()));
        let sub = PreviewSub {
            notifier: Arc::downgrade(&self.notifier),
            screen_id: screen_id.to_string(),
        };
        (sub, snapshot)
    }

    /// The preview bus's current `snapshot` (a lagging preview renderer resyncs from it).
    pub fn preview_snapshot_json(&self, screen_id: &str) -> Option<String> {
        let _state = self.read();
        let bus = lock(&self.notifier.bus);
        let last = bus.get(screen_id)?.last.as_ref()?;
        serialize(&last.snapshot())
    }

    /// Number of screens whose Preview the hub derives for preview-bus subscribers.
    pub fn preview_buses(&self) -> usize {
        lock(&self.notifier.bus).len()
    }

    /// Studio op on one screen (the operator's take, cue, T-bar, clear): see
    /// `studio_locked`. Returns the screen's Preview view afterwards.
    pub fn studio_op(
        &self,
        screen_id: &str,
        action: StudioAction<'_>,
    ) -> Result<PreviewView, LiveError> {
        let (view, changed) = {
            let mut state = self.write();
            let changed = self.studio_locked(&mut state, screen_id, action)?;
            (state.preview_view(screen_id), changed)
        };
        self.finish(changed);
        view
    }

    /// THE studio entry point: every take, cue, T-bar step and Preview clear, and in Studio mode
    /// every content edit (saves, Control field / variant edits, drags, watcher reloads, source
    /// steps by the operator), on any path. Layer locks will be enforced here. Caller holds the
    /// state write lock; sends, timer bookkeeping and notes are done here. Returns whether what
    /// is on air may have changed.
    ///
    /// A change of the diff while a T-bar is in progress ends it first (`abort`), so what the
    /// renderers prepared always matches the diff a commit applies.
    fn studio_locked(
        &self,
        state: &mut LiveState,
        screen_id: &str,
        action: StudioAction<'_>,
    ) -> Result<bool, LiveError> {
        self.studio_ops.fetch_add(1, AtomicOrdering::Relaxed);
        let rev = state.preview_rev(screen_id);
        let mut msgs = Vec::new();
        let mut touched = Vec::new();
        let mut removed_on_air = Vec::new();
        let mut abort_on_change = true;
        match action {
            StudioAction::Cue { layer_id, visible } => {
                state.cue(screen_id, layer_id, visible)?;
            }
            StudioAction::Clear => {
                state.clear_preview(screen_id)?;
            }
            StudioAction::Take { transition, after } => {
                msgs.extend(state.tbar_abort(screen_id));
                stop_tbar_clock(&self.tbar, screen_id);
                let out = state.take_preview(screen_id, transition, after)?;
                msgs.extend(out.msgs);
                touched = out.touched;
                abort_on_change = false;
            }
            StudioAction::TbarBegin => {
                msgs.extend(state.tbar_begin(screen_id)?);
                abort_on_change = false;
            }
            StudioAction::TbarSet { p } => {
                msgs.extend(state.tbar_set(screen_id, p)?);
                abort_on_change = false;
            }
            StudioAction::TbarEnd { commit, after } => {
                stop_tbar_clock(&self.tbar, screen_id);
                let out = state.tbar_end(screen_id, commit, after)?;
                msgs.extend(out.msgs);
                touched = out.touched;
                abort_on_change = false;
            }
            StudioAction::EditPlan { plan, switches } => {
                let out = state.apply_screen_with(plan, LiveEdit::OnTake, switches);
                msgs.extend(out.msgs);
                removed_on_air = out.removed_on_air;
            }
            StudioAction::EditRect { layer_id, rect } => {
                msgs.extend(state.set_rect(screen_id, layer_id, rect, LiveEdit::OnTake)?);
            }
        }
        let diff_changed = state.preview_rev(screen_id) != rev;
        if abort_on_change && diff_changed && state.tbar_active(screen_id) {
            msgs.extend(state.tbar_abort(screen_id));
            stop_tbar_clock(&self.tbar, screen_id);
        }
        self.send(screen_id, &msgs);
        let mut book = msgs.clone();
        book.extend(touched_msgs(&touched));
        self.after_op(state, screen_id, &book);
        for layer_id in removed_on_air {
            self.schedule_remove(screen_id.to_string(), layer_id);
        }
        if state.tbar_active(screen_id) {
            self.tbar_tick(state, screen_id);
        }
        self.notifier.note(state, screen_id);
        Ok(on_air_changed(&msgs))
    }

    /// Sends the T-bar position if it changed and the screen's last `tbar` is at least
    /// [`TBAR_INTERVAL`] old; otherwise makes sure ONE task sends the latest position when the
    /// interval is up. Without a runtime it sends at once. Caller holds the state write lock.
    fn tbar_tick(&self, state: &mut LiveState, screen_id: &str) {
        let now = Instant::now();
        let wait = {
            let mut clocks = lock(&self.tbar);
            let c = clocks.entry(screen_id.to_string()).or_insert(TbarClock {
                last_sent: None,
                task: None,
            });
            if c.task.is_some() {
                return;
            }
            let due = c
                .last_sent
                .map_or(Duration::ZERO, |t| TBAR_INTERVAL.saturating_sub(now - t));
            if due.is_zero() || self.runtime_handle().is_none() {
                if let Some(p) = state.tbar_unsent(screen_id) {
                    c.last_sent = Some(now);
                    self.send(screen_id, &[ServerMsg::Tbar { p }]);
                }
                return;
            }
            due
        };
        let Some(handle) = self.runtime_handle() else {
            return;
        };
        let ctx = self.task_ctx();
        let id = screen_id.to_string();
        let task = handle.spawn(async move {
            tokio::time::sleep(wait).await;
            ctx.send_tbar(&id);
        });
        if let Some(c) = lock(&self.tbar).get_mut(screen_id) {
            c.task = Some(task.abort_handle());
        }
    }

    /// After the project's ui-state was reloaded from disk: Studio mode as `on` says, with its
    /// visibility cues replaced by `cues`. Staged content stays (as On-take staging when Studio
    /// mode goes off). Every T-bar ends.
    pub fn restore_studio(&self, on: bool, cues: &StudioCues) {
        {
            let mut state = self.write();
            for id in state.tbar_screens() {
                let msgs = state.tbar_abort(&id);
                stop_tbar_clock(&self.tbar, &id);
                self.send(&id, &msgs);
            }
            if state.studio && !on {
                for (id, msgs) in state.studio_off_keep_staged() {
                    self.send(&id, &msgs);
                    self.after_op(&mut state, &id, &msgs);
                }
            }
            state.studio = on;
            if on {
                state.drop_visibility_cues();
                state.restore_cues(cues);
            }
            self.notifier.note_all(&state);
        }
        self.finish(false);
    }

    /// Turns Studio mode on or off. Off with a non-empty Preview needs `exit` (`take` with
    /// `transition`, `discard`, or `keepStaged`); without it nothing changes and
    /// [`PreviewHasChanges`] is returned. Every T-bar ends.
    pub fn set_studio(
        &self,
        on: bool,
        exit: Option<StudioExit>,
        transition: TakeTransition,
    ) -> Result<(), PreviewHasChanges> {
        let changed = {
            let mut state = self.write();
            if on == state.studio {
                return Ok(());
            }
            let mut changed = false;
            if on {
                state.studio = true;
            } else {
                let pending = state.screens_with_preview();
                let exit = match (pending.is_empty(), exit) {
                    (true, _) => StudioExit::Discard,
                    (false, Some(e)) => e,
                    (false, None) => return Err(PreviewHasChanges),
                };
                for id in state.tbar_screens() {
                    let msgs = state.tbar_abort(&id);
                    stop_tbar_clock(&self.tbar, &id);
                    self.send(&id, &msgs);
                }
                match exit {
                    StudioExit::Take => {
                        for id in &pending {
                            let after = PreviewAfterTake::Keep;
                            let action = StudioAction::Take { transition, after };
                            if let Ok(c) = self.studio_locked(&mut state, id, action) {
                                changed |= c;
                            }
                        }
                        state.studio = false;
                    }
                    StudioExit::Discard => {
                        for id in &pending {
                            let _ = state.clear_preview(id);
                        }
                        state.studio = false;
                    }
                    StudioExit::KeepStaged => {
                        for (id, msgs) in state.studio_off_keep_staged() {
                            self.send(&id, &msgs);
                            self.after_op(&mut state, &id, &msgs);
                        }
                    }
                }
            }
            self.notifier.note_all(&state);
            changed
        };
        self.finish(changed);
        Ok(())
    }

    /// Installs the hook called after operations that may change what is on air (see
    /// [`LiveHook`]). It must not call back into an operation of this hub that changes state.
    pub fn set_hook(&self, hook: LiveHook) {
        *self.hook.write().unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// The runtime timers and automation run on: the injected one, else the caller's.
    pub fn runtime_handle(&self) -> Option<Handle> {
        lock(&self.runtime)
            .clone()
            .or_else(|| Handle::try_current().ok())
    }

    /// Whether a layer is on air (false when it doesn't exist).
    pub fn is_on_air(&self, screen_id: &str, layer_id: &str) -> bool {
        self.with_layer(screen_id, layer_id, |l| l.on_air)
            .unwrap_or(false)
    }

    /// Sets field values on air at once, bypassing `liveEdit = onTake` (automation: timers,
    /// auto-advance), and patches the same keys into staged content so a later take keeps them.
    /// Other staged changes stay staged (the staged content is dropped only when it now equals
    /// what is on air). Keys the live template doesn't have are ignored. One `fields` message
    /// with the changed keys, or nothing.
    pub fn force_fields(
        &self,
        screen_id: &str,
        layer_id: &str,
        fields: &Fields,
    ) -> Result<LiveLayerView, LiveError> {
        self.apply_layer(screen_id, layer_id, |s| {
            let l = s.layer_mut(screen_id, layer_id)?;
            let mut changed = Fields::new();
            for (k, v) in fields {
                if l.template.has_field(k) && l.fields.get(k) != Some(v) {
                    l.fields.insert(k.clone(), v.clone());
                    changed.insert(k.clone(), v.clone());
                }
            }
            let id = l.id.clone();
            // In Studio mode `apply_layer` rebases the Preview diff instead (its values win).
            if !s.studio {
                s.patch_staged_fields(screen_id, layer_id, fields);
            }
            if changed.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![ServerMsg::Fields {
                layer_id: id,
                fields: changed,
            }])
        })
    }

    /// Number of pending countdown ends (for tests and diagnostics).
    pub fn pending_countdowns(&self) -> usize {
        lock(&self.countdowns).len()
    }

    /// Number of pending media out points (for tests and diagnostics).
    pub fn pending_media_ends(&self) -> usize {
        lock(&self.media_ends).len()
    }

    /// Installs the hook run when a media layer reaches its out point (`hold` / `clear`).
    pub fn set_media_ended_hook(&self, hook: MediaEndedHook) {
        *self
            .media_ended
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(hook);
    }

    /// Transport on a media layer's Program playback: at once, in Studio mode too (never
    /// staged). `seek` takes `ms`. Errors for an unknown layer or one that isn't media.
    pub fn media_control(
        &self,
        screen_id: &str,
        layer_id: &str,
        action: MediaAction,
        ms: Option<u64>,
    ) -> Result<LiveLayerView, LiveError> {
        let now = fsx::now_epoch_ms();
        self.apply_layer(screen_id, layer_id, |s| {
            s.media_control(screen_id, layer_id, action, ms, now)
        })
    }

    /// Registers a renderer connection of `screen_id` for readiness. `counted`: a Program
    /// connection that isn't an app preview (it counts toward `total`). Drop the result when
    /// the connection ends.
    pub fn renderer_attach(&self, screen_id: &str, counted: bool) -> RendererReg {
        let id = {
            let mut state = self.write();
            let id = {
                let mut book = lock(&self.shared.book);
                book.next += 1;
                let id = book.next;
                book.conns.insert(
                    id,
                    RendererReports {
                        screen: screen_id.to_string(),
                        counted,
                        layers: HashMap::new(),
                    },
                );
                id
            };
            if counted && refresh_readiness(&self.shared.book, &mut state, screen_id) {
                self.note_readiness(&state, screen_id);
            }
            id
        };
        self.notifier.flush();
        RendererReg {
            hub: Arc::downgrade(&self.shared),
            id,
        }
    }

    /// A renderer's `ready` report for one of its screen's layers (kept for the layer's current
    /// `ready_key`). Unknown layers and layers without media are ignored.
    pub fn renderer_ready(
        &self,
        reg: &RendererReg,
        layer_id: &str,
        ok: bool,
        error: Option<String>,
    ) {
        {
            let mut state = self.write();
            let screen = {
                let mut book = lock(&self.shared.book);
                let Some(c) = book.conns.get_mut(&reg.id) else {
                    return;
                };
                let rev = state
                    .screen(&c.screen)
                    .and_then(|s| s.layers.iter().find(|l| l.id == layer_id))
                    .and_then(|l| l.ready_key());
                let Some(rev) = rev else { return };
                let error = (!ok).then(|| {
                    let mut e = error.unwrap_or_else(|| "Can't load".into());
                    e.truncate(200);
                    e
                });
                c.layers.insert(layer_id.to_string(), (rev, ok, error));
                c.screen.clone()
            };
            if refresh_readiness(&self.shared.book, &mut state, &screen) {
                self.note_readiness(&state, &screen);
            }
        }
        self.notifier.flush();
    }

    /// Notes a readiness change of a screen at most every [`READINESS_NOTE_INTERVAL`]: at once
    /// when the last one is older, else ONE task notes it when the interval is up. Caller holds
    /// the state lock.
    fn note_readiness(&self, state: &LiveState, screen_id: &str) {
        let now = Instant::now();
        let wait = {
            let mut book = lock(&self.shared.book);
            let e = book
                .notes
                .entry(screen_id.to_string())
                .or_insert((now - READINESS_NOTE_INTERVAL, false));
            if e.1 {
                return;
            }
            let due = READINESS_NOTE_INTERVAL.saturating_sub(now - e.0);
            if due.is_zero() || self.runtime_handle().is_none() {
                e.0 = now;
                None
            } else {
                e.1 = true;
                Some(due)
            }
        };
        let Some(wait) = wait else {
            self.notifier.note(state, screen_id);
            return;
        };
        let Some(handle) = self.runtime_handle() else {
            return;
        };
        let st = Arc::downgrade(&self.state);
        let notifier = Arc::downgrade(&self.notifier);
        let book = Arc::downgrade(&self.shared.book);
        let id = screen_id.to_string();
        handle.spawn(async move {
            tokio::time::sleep(wait).await;
            let (Some(st), Some(notifier), Some(book)) =
                (st.upgrade(), notifier.upgrade(), book.upgrade())
            else {
                return;
            };
            {
                let s = read_lock(&st);
                if let Some(e) = lock(&book).notes.get_mut(&id) {
                    *e = (Instant::now(), false);
                }
                notifier.note(&s, &id);
            }
            notifier.flush();
        });
    }

    /// Schedules (or cancels) the out-point task of a media layer: on air, playing to an end
    /// (hold / clear) with a known out point. Caller holds the state write lock.
    fn reschedule_media_end(&self, key: &LayerKey, layer: Option<&LayerLive>) {
        let want = layer.filter(|l| l.on_air).and_then(|l| {
            let m = l.media.as_ref()?;
            m.end_at(&l.playback)
                .map(|at| (at, l.playback, m.config.end_action, l.out_gen))
        });
        {
            let mut ends = lock(&self.media_ends);
            if let (Some(e), Some((_, pb, _, _))) = (ends.get(key), want) {
                if e.playback == pb {
                    return;
                }
            }
            if let Some(old) = ends.remove(key) {
                old.abort.abort();
            }
        }
        let Some((at, playback, action, out_gen)) = want else {
            return;
        };
        let Some(handle) = self.runtime_handle() else {
            tracing::warn!(
                "no async runtime: media end of {}/{} not scheduled",
                key.0,
                key.1
            );
            return;
        };
        let now = fsx::now_epoch_ms();
        let ctx = self.task_ctx();
        let ends = Arc::downgrade(&self.media_ends);
        let hook = Arc::downgrade(&self.media_ended);
        let task_key = key.clone();
        let task = handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(at.saturating_sub(now))).await;
            ctx.media_end_due(&task_key, &ends, &hook, playback, action, out_gen, at);
        });
        lock(&self.media_ends).insert(
            key.clone(),
            MediaEnd {
                playback,
                abort: task.abort_handle(),
            },
        );
    }

    /// Automation off: countdowns with `end: hide` stay on air at zero (`true` cancels every
    /// pending countdown end and schedules none); `false` schedules the ones still ahead again
    /// (one already past zero holds, nothing catches up).
    pub fn set_hold_countdowns(&self, hold: bool) {
        if self.hold_countdowns.swap(hold, AtomicOrdering::SeqCst) == hold {
            return;
        }
        let state = self.write();
        if hold {
            lock(&self.countdowns).retain(|_, c| {
                c.abort.abort();
                false
            });
            return;
        }
        for screen in &state.screens {
            for layer in &screen.layers {
                let key = (screen.info.id.clone(), layer.id.clone());
                self.reschedule_countdown(&key, Some(layer));
            }
        }
    }

    /// A hub seeded with the test screen.
    pub fn with_test_screen() -> Self {
        Self::new(LiveState::with_test_screen())
    }

    /// The runtime auto-out timers run on. See the module docs.
    pub fn set_runtime(&self, handle: Handle) {
        *lock(&self.runtime) = Some(handle);
    }

    /// Where `live-changed` goes (see the module docs). The current views become the baseline:
    /// nothing is emitted for them.
    pub fn set_notifier(&self, sink: Arc<dyn EventSink>) {
        let state = self.read();
        *self
            .notifier
            .sink
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(sink);
        let mut last = lock(&self.notifier.last);
        let mut last_preview = lock(&self.notifier.last_preview);
        last.clear();
        last_preview.clear();
        for s in state
            .screens
            .iter()
            .filter(|s| !s.info.id.starts_with("__"))
        {
            last.insert(s.info.id.clone(), s.view_in(state.studio));
            if state.studio {
                last_preview.insert(s.info.id.clone(), s.preview_view());
            }
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, LiveState> {
        read_lock(&self.state)
    }

    fn write(&self) -> RwLockWriteGuard<'_, LiveState> {
        write_lock(&self.state)
    }

    /// Subscribes to every message for every screen. Subscribe BEFORE reading a snapshot so no
    /// change can fall between the two.
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<HubMsg>> {
        self.tx.subscribe()
    }

    /// Serialized `snapshot` message for a screen, or `None` if it doesn't exist.
    pub fn snapshot_json(&self, screen_id: &str) -> Option<String> {
        let msg = self.read().snapshot(screen_id)?;
        serialize(&msg)
    }

    pub fn has_screen(&self, screen_id: &str) -> bool {
        self.read().screen(screen_id).is_some()
    }

    pub fn screen_view(&self, screen_id: &str) -> Result<LiveScreenView, LiveError> {
        self.read().screen_view(screen_id)
    }

    pub fn layer_view(&self, screen_id: &str, layer_id: &str) -> Result<LiveLayerView, LiveError> {
        self.read().layer_view(screen_id, layer_id)
    }

    /// Reads a layer's full live state (CSS vars, rev, ...) under the read lock.
    pub fn with_layer<R>(
        &self,
        screen_id: &str,
        layer_id: &str,
        f: impl FnOnce(&LayerLive) -> R,
    ) -> Result<R, LiveError> {
        let state = self.read();
        let layer = state
            .screen(screen_id)
            .ok_or_else(|| LiveError::UnknownScreen(screen_id.to_string()))?
            .layers
            .iter()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))?;
        Ok(f(layer))
    }

    /// A view of every screen, in order.
    pub fn screen_views(&self) -> Vec<LiveScreenView> {
        self.read().screen_views()
    }

    /// Views of the user (non-reserved) screens, in order: what `list_screens` returns and what
    /// `live-changed` carries.
    pub fn user_screen_views(&self) -> Vec<LiveScreenView> {
        let mut views = self.screen_views();
        views.retain(|v| !is_reserved_screen(&v.screen.id));
        views
    }

    pub fn show(&self, screen_id: &str, layer_id: &str) -> Result<LiveLayerView, LiveError> {
        self.apply_layer(screen_id, layer_id, |s| s.show(screen_id, layer_id))
    }

    pub fn hide(&self, screen_id: &str, layer_id: &str) -> Result<LiveLayerView, LiveError> {
        self.apply_layer(screen_id, layer_id, |s| s.hide(screen_id, layer_id))
    }

    pub fn set_fields(
        &self,
        screen_id: &str,
        layer_id: &str,
        fields: Fields,
    ) -> Result<LiveLayerView, LiveError> {
        self.apply_layer(screen_id, layer_id, |s| {
            s.set_fields(screen_id, layer_id, fields)
        })
    }

    /// Runs `op` on one layer under the write lock and broadcasts the messages it returns (the op
    /// must leave the layer unchanged when it fails). Used for template loads and drafts
    /// (`core::preview`).
    pub fn update_layer<E: From<LiveError>>(
        &self,
        screen_id: &str,
        layer_id: &str,
        op: impl FnOnce(&mut LayerLive) -> Result<Vec<ServerMsg>, E>,
    ) -> Result<LiveLayerView, E> {
        self.apply_layer(screen_id, layer_id, |s| {
            let layer = s.layer_mut(screen_id, layer_id)?;
            op(layer)
        })
    }

    /// Every on-air layer of the screen plays out (animated). A T-bar in progress ends (`abort`);
    /// the Preview diff stays.
    pub fn clear(&self, screen_id: &str) -> Result<LiveScreenView, LiveError> {
        let (view, changed) = {
            let mut state = self.write();
            let msgs = self.clear_locked(&mut state, screen_id)?;
            (state.screen_view(screen_id), on_air_changed(&msgs))
        };
        self.finish(changed);
        view
    }

    fn clear_locked(
        &self,
        state: &mut LiveState,
        screen_id: &str,
    ) -> Result<Vec<ServerMsg>, LiveError> {
        let mut msgs = state.tbar_abort(screen_id);
        stop_tbar_clock(&self.tbar, screen_id);
        msgs.extend(state.clear(screen_id)?);
        state.program_changed(screen_id, &msgs, None);
        self.send(screen_id, &msgs);
        self.after_op(state, screen_id, &msgs);
        self.notifier.note(state, screen_id);
        Ok(msgs)
    }

    /// After an operation, with no lock held: emits `live-changed` and, if `hook`, calls the hook.
    fn finish(&self, hook: bool) {
        self.notifier.flush();
        self.refresh_fades();
        if hook {
            call_hook(&self.hook);
        }
    }

    /// [`LiveHub::clear`] on every user screen (animated). Returns their views.
    pub fn clear_all(&self) -> Vec<LiveScreenView> {
        let (views, changed) = {
            let mut state = self.write();
            let ids: Vec<String> = state
                .screens
                .iter()
                .filter(|s| !is_reserved_screen(&s.info.id))
                .map(|s| s.info.id.clone())
                .collect();
            let mut views = Vec::with_capacity(ids.len());
            let mut changed = false;
            for id in ids {
                let Ok(msgs) = self.clear_locked(&mut state, &id) else {
                    continue;
                };
                changed |= on_air_changed(&msgs);
                if let Ok(v) = state.screen_view(&id) {
                    views.push(v);
                }
            }
            (views, changed)
        };
        self.finish(changed);
        views
    }

    /// Cancels pending auto-outs, countdown ends and media out points of the layers `which`
    /// selects. Caller holds the state write lock.
    fn cancel_timers(&self, which: impl Fn(&LayerKey) -> bool) {
        lock(&self.media_ends).retain(|k, m| {
            let hit = which(k);
            if hit {
                m.abort.abort();
            }
            !hit
        });
        lock(&self.timers).retain(|k, t| {
            let hit = which(k);
            if hit {
                t.abort.abort();
            }
            !hit
        });
        lock(&self.countdowns).retain(|k, c| {
            let hit = which(k);
            if hit {
                c.abort.abort();
            }
            !hit
        });
    }

    /// PANIC: every layer of every screen (user screens and `test`; not the `__*` system screens)
    /// off air at once. Each of those screens gets one `cut` (no out-animation), every pending
    /// auto-out and countdown end is cancelled, staged changes are kept outside Studio mode. In
    /// Studio mode every T-bar ends first (`abort` before the `cut`) and every Preview diff is
    /// cleared. Returns the user screens' views.
    pub fn panic(&self) -> Vec<LiveScreenView> {
        let views = {
            let mut state = self.write();
            for id in state.tbar_screens() {
                let msgs = state.tbar_abort(&id);
                stop_tbar_clock(&self.tbar, &id);
                self.send(&id, &msgs);
            }
            let ids = state.cut();
            // Cut, not animated: nothing stays visible.
            self.scan_fades(&state, true);
            self.cancel_timers(|(screen, _)| ids.contains(screen));
            for id in &ids {
                self.send(id, &[ServerMsg::Cut {}]);
                self.notifier.note(&state, id);
            }
            ids.iter()
                .filter(|id| !is_reserved_screen(id))
                .filter_map(|id| state.screen_view(id).ok())
                .collect()
        };
        self.finish(true);
        views
    }

    /// Replaces the whole live state (project opened or closed). Renderers of every screen in the
    /// new state get a fresh snapshot; renderers of screens that no longer exist get `clear`.
    /// Pending auto-outs and countdown ends are cancelled.
    pub fn reset(&self, next: LiveState) {
        {
            let mut state = self.write();
            self.cancel_timers(|_| true);
            for (_, c) in lock(&self.tbar).drain() {
                if let Some(t) = c.task {
                    t.abort();
                }
            }
            let old = std::mem::replace(&mut *state, next);
            for screen in &old.screens {
                if state.screen(&screen.info.id).is_none() {
                    self.send(&screen.info.id, &[ServerMsg::Clear {}]);
                }
            }
            let ids: Vec<String> = state.screens.iter().map(|s| s.info.id.clone()).collect();
            for id in &ids {
                refresh_readiness(&self.shared.book, &mut state, id);
            }
            for screen in &state.screens {
                self.send(&screen.info.id, &[screen.snapshot()]);
            }
            // Preview buses start over like Program: a snapshot, or `clear` when the screen is
            // gone.
            for (id, b) in lock(&self.notifier.bus).iter_mut() {
                if b.last.take().is_some() && state.screen(id).is_none() {
                    send_to(&self.tx, id, &[ServerMsg::Clear {}], true);
                }
            }
            // Another project: nothing of the old one counts as visible.
            self.scan_fades(&state, true);
            self.notifier.note_all(&state);
        }
        self.finish(true);
    }

    /// Creates or updates a project screen by the live-edit rules (see
    /// [`LiveState::apply_screen`]) and broadcasts the result. Layers removed while on air get
    /// `state out` now and `remove` after [`REMOVE_DELAY`], so their out-animation plays and the
    /// renderer keeps nothing behind. Always calls the hook (bindings may have changed).
    pub fn apply_screen(
        &self,
        plan: ScreenPlan,
        mode: LiveEdit,
    ) -> Result<LiveScreenView, LiveError> {
        self.apply_screen_with(plan, mode, &[])
    }

    /// [`LiveHub::apply_screen`] with variant switches for some layers (see
    /// [`LiveState::apply_screen_with`]).
    pub fn apply_screen_with(
        &self,
        plan: ScreenPlan,
        mode: LiveEdit,
        switches: &[VariantSwitch],
    ) -> Result<LiveScreenView, LiveError> {
        // Without a guard nothing is refused.
        self.apply_screen_guarded(plan, mode, switches, None)?
            .ok_or_else(|| LiveError::UnknownScreen(String::new()))
    }

    /// [`LiveHub::apply_screen_with`], refused (`Ok(None)`, nothing changed, no hook) when
    /// `hidden_layer` can be seen on the output or is cued in on Preview — checked under the same
    /// write lock as the change, so a take in between can never be half-overtaken (remote data
    /// editors only edit layers that aren't on air).
    pub fn apply_screen_unless_shown(
        &self,
        plan: ScreenPlan,
        mode: LiveEdit,
        hidden_layer: &str,
    ) -> Result<Option<LiveScreenView>, LiveError> {
        self.apply_screen_guarded(plan, mode, &[], Some(hidden_layer))
    }

    fn apply_screen_guarded(
        &self,
        plan: ScreenPlan,
        mode: LiveEdit,
        switches: &[VariantSwitch],
        hidden_layer: Option<&str>,
    ) -> Result<Option<LiveScreenView>, LiveError> {
        let screen_id = plan.info.id.clone();
        let view = {
            let mut state = self.write();
            if let Some(layer) = hidden_layer {
                if self.visible_locked(&state, &screen_id, layer)
                    || Self::cued_locked(&state, &screen_id, layer)
                {
                    return Ok(None);
                }
            }
            if state.studio {
                let action = StudioAction::EditPlan { plan, switches };
                self.studio_locked(&mut state, &screen_id, action)?;
                let view = state.screen_view(&screen_id);
                drop(state);
                self.finish(true);
                return view.map(Some);
            }
            let out = state.apply_screen_with(plan, mode, switches);
            self.send(&screen_id, &out.msgs);
            self.after_op(&mut state, &screen_id, &out.msgs);
            for layer_id in out.removed_on_air {
                self.schedule_remove(screen_id.clone(), layer_id);
            }
            self.notifier.note(&state, &screen_id);
            state.screen_view(&screen_id)
        };
        self.finish(true);
        view.map(Some)
    }

    /// Drops a screen (deleted, or its file removed). Its renderers get `error unknownScreen`;
    /// its auto-outs and countdown ends are cancelled. `false` if it did not exist.
    pub fn remove_screen(&self, screen_id: &str) -> bool {
        {
            let mut state = self.write();
            let Some((_, msgs)) = state.remove_screen(screen_id) else {
                return false;
            };
            self.cancel_timers(|(screen, _)| screen == screen_id);
            stop_tbar_clock(&self.tbar, screen_id);
            self.send(screen_id, &msgs);
            self.notifier.forget(&state, screen_id);
        }
        self.notifier.flush();
        call_hook(&self.hook);
        true
    }

    /// Moves or resizes a layer's box (in memory only), following the live-edit rules.
    pub fn set_rect(
        &self,
        screen_id: &str,
        layer_id: &str,
        rect: Rect,
        mode: LiveEdit,
    ) -> Result<LiveLayerView, LiveError> {
        {
            let mut state = self.write();
            if state.studio {
                let action = StudioAction::EditRect { layer_id, rect };
                self.studio_locked(&mut state, screen_id, action)?;
                let view = state.layer_view(screen_id, layer_id);
                drop(state);
                self.finish(false);
                return view;
            }
        }
        self.apply_layer(screen_id, layer_id, |s| {
            s.set_rect(screen_id, layer_id, rect, mode)
        })
    }

    /// Every layer of the user (non-reserved) screens with its on-air state, in order (cheap: no
    /// views built; for OSC / MIDI feedback).
    pub fn user_layers_on_air(&self) -> Vec<(String, String, bool)> {
        let state = self.read();
        let mut out = Vec::new();
        for s in state
            .screens
            .iter()
            .filter(|s| !is_reserved_screen(&s.info.id))
        {
            for l in &s.layers {
                out.push((s.info.id.clone(), l.id.clone(), l.on_air));
            }
        }
        out
    }

    /// Whether any layer of a user (non-reserved) screen is on air in Program (an update must
    /// not install then). Preview cues and the template preview screen don't count.
    pub fn any_on_air(&self) -> bool {
        self.read()
            .screens
            .iter()
            .filter(|s| !is_reserved_screen(&s.info.id))
            .any(|s| s.layers.iter().any(|l| l.on_air))
    }

    /// Ids of the user (non-reserved) screens in the live state.
    pub fn user_screen_ids(&self) -> Vec<String> {
        self.read()
            .screens
            .iter()
            .filter(|s| !is_reserved_screen(&s.info.id))
            .map(|s| s.info.id.clone())
            .collect()
    }

    /// Size of a screen, if it exists.
    pub fn screen_size(&self, screen_id: &str) -> Option<(u32, u32)> {
        self.read()
            .screen(screen_id)
            .map(|s| (s.info.width, s.info.height))
    }

    /// Sends `remove` for a layer that was taken out on removal, once its out-animation has had
    /// time; skipped if a layer with that id is back on the screen by then. Without a runtime
    /// nothing is sent: the renderer still detaches the layer when its out-animation ends, and the
    /// next snapshot drops it.
    fn schedule_remove(&self, screen_id: String, layer_id: String) {
        let handle = lock(&self.runtime)
            .clone()
            .or_else(|| Handle::try_current().ok());
        let Some(handle) = handle else {
            tracing::debug!("no async runtime: remove of {screen_id}/{layer_id} not scheduled");
            return;
        };
        let state = Arc::downgrade(&self.state);
        let tx = self.tx.clone();
        handle.spawn(async move {
            tokio::time::sleep(REMOVE_DELAY).await;
            let Some(state) = state.upgrade() else { return };
            let s = write_lock(&state);
            let gone = s
                .screen(&screen_id)
                .is_some_and(|sc| sc.layers.iter().all(|l| l.id != layer_id));
            if gone {
                send_to(&tx, &screen_id, &[ServerMsg::Remove { layer_id }], false);
            }
        });
    }

    /// Stages content for a layer (as an `onTake` edit of an on-air layer would), for tests.
    #[cfg(test)]
    pub(crate) fn stage_for_test(
        &self,
        screen_id: &str,
        layer_id: &str,
        f: impl FnOnce(&mut crate::core::live::LayerContent),
    ) {
        let mut state = self.write();
        let s = state.screens.iter_mut().find(|s| s.info.id == screen_id);
        let s = s.expect("screen");
        let l = s.layers.iter().find(|l| l.id == layer_id).expect("layer");
        let mut c = l.content();
        f(&mut c);
        s.preview.layers.insert(
            layer_id.to_string(),
            crate::core::live::Cue {
                content: Some(Box::new(c)),
                ..Default::default()
            },
        );
    }

    /// Number of pending auto-out timers (for tests and diagnostics).
    pub fn pending_auto_outs(&self) -> usize {
        lock(&self.timers).len()
    }

    fn apply_layer<E: From<LiveError>>(
        &self,
        screen_id: &str,
        layer_id: &str,
        op: impl FnOnce(&mut LiveState) -> Result<Vec<ServerMsg>, E>,
    ) -> Result<LiveLayerView, E> {
        let (view, changed) = {
            let mut state = self.write();
            // Studio mode: this goes straight to Program (automation); the Preview diff of the
            // layer is rebased on top (see `LiveState::program_changed`).
            let old = if state.studio {
                state.content_if_staged(screen_id, layer_id)
            } else {
                None
            };
            let msgs = op(&mut state)?;
            state.program_changed(screen_id, &msgs, old.as_ref().map(|c| (layer_id, c)));
            self.send(screen_id, &msgs);
            self.after_op(&mut state, screen_id, &msgs);
            self.notifier.note(&state, screen_id);
            (state.layer_view(screen_id, layer_id), on_air_changed(&msgs))
        };
        self.finish(changed);
        Ok(view?)
    }

    fn send(&self, screen_id: &str, msgs: &[ServerMsg]) {
        send_to(&self.tx, screen_id, msgs, false);
    }

    /// Timer bookkeeping after an op, with the write lock held, for every layer a message touched:
    /// - auto-out: a layer whose on-air state or template changed loses its pending timer, and
    ///   gets a new one if it is now on air with an auto-out;
    /// - countdown end: an on-air layer whose running `end: hide` countdown (from its `timer`
    ///   fields) differs from the scheduled one is rescheduled; off air, or none, cancels it. A
    ///   countdown already at zero when it is shown is not scheduled (it holds at zero).
    fn after_op(&self, state: &mut LiveState, screen_id: &str, msgs: &[ServerMsg]) {
        refresh_readiness(&self.shared.book, state, screen_id);
        let state = &*state;
        for msg in msgs {
            let (layer_id, structural) = match msg {
                ServerMsg::State { layer_id, .. } => (layer_id, true),
                ServerMsg::Layer { layer } => (&layer.id, true),
                ServerMsg::Fields { layer_id, .. } => (layer_id, false),
                ServerMsg::Media { layer_id, .. } => {
                    let key = (screen_id.to_string(), layer_id.clone());
                    let layer = state
                        .screen(screen_id)
                        .and_then(|s| s.layers.iter().find(|l| l.id == *layer_id));
                    self.reschedule_media_end(&key, layer);
                    continue;
                }
                _ => continue,
            };
            let key = (screen_id.to_string(), layer_id.clone());
            let layer = state
                .screen(screen_id)
                .and_then(|s| s.layers.iter().find(|l| l.id == *layer_id));
            self.reschedule_countdown(&key, layer);
            if structural {
                self.reschedule_media_end(&key, layer);
            }
            if !structural {
                continue;
            }
            {
                let mut timers = lock(&self.timers);
                // Same on-air period (a reorder or an in-place update): the countdown continues.
                let keep = layer.is_some_and(|l| {
                    l.on_air
                        && l.auto_out_ms.is_some()
                        && timers.get(&key).is_some_and(|t| t.out_gen == l.out_gen)
                });
                if keep {
                    continue;
                }
                if let Some(old) = timers.remove(&key) {
                    old.abort.abort();
                }
            }
            let Some(layer) = layer else {
                continue;
            };
            if let (true, Some(ms)) = (layer.on_air, layer.auto_out_ms) {
                self.schedule(key, layer.out_gen, ms);
            }
        }
    }

    /// See [`LiveHub::after_op`]. Caller holds the state write lock.
    fn reschedule_countdown(&self, key: &LayerKey, layer: Option<&LayerLive>) {
        let hold = self.hold_countdowns.load(AtomicOrdering::SeqCst);
        let want = layer
            .filter(|l| l.on_air && !hold)
            .and_then(|l| countdown_at(l).map(|at| (l.out_gen, at)));
        {
            let mut cds = lock(&self.countdowns);
            let same = matches!(
                (cds.get(key), want),
                (Some(c), Some((g, at))) if c.out_gen == g && c.at == at
            );
            if same {
                return;
            }
            if let Some(old) = cds.remove(key) {
                old.abort.abort();
            }
        }
        let Some((out_gen, at)) = want else { return };
        let now = fsx::now_epoch_ms();
        if at <= now {
            return;
        }
        let Some(handle) = self.runtime_handle() else {
            tracing::warn!(
                "no async runtime: countdown end of {}/{} not scheduled",
                key.0,
                key.1
            );
            return;
        };
        let ctx = self.task_ctx();
        let task_key = key.clone();
        let task = handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(at - now)).await;
            ctx.hide_due(&task_key, out_gen, |timers, cds| {
                let mine = cds
                    .get(&task_key)
                    .is_some_and(|c| c.out_gen == out_gen && c.at == at);
                if mine {
                    cds.remove(&task_key);
                    if let Some(t) = timers.remove(&task_key) {
                        t.abort.abort();
                    }
                }
                mine
            });
        });
        lock(&self.countdowns).insert(
            key.clone(),
            Countdown {
                out_gen,
                at,
                abort: task.abort_handle(),
            },
        );
    }

    fn task_ctx(&self) -> TaskCtx {
        TaskCtx {
            state: Arc::downgrade(&self.state),
            timers: Arc::downgrade(&self.timers),
            countdowns: Arc::downgrade(&self.countdowns),
            notifier: Arc::downgrade(&self.notifier),
            hook: Arc::downgrade(&self.hook),
            tbar: Arc::downgrade(&self.tbar),
            tx: self.tx.clone(),
        }
    }

    /// Spawns the auto-out task. Caller holds the state write lock, so the task (which takes the
    /// state lock first) cannot run its body before the timer is registered.
    fn schedule(&self, key: LayerKey, out_gen: u64, ms: u32) {
        let Some(handle) = self.runtime_handle() else {
            tracing::warn!(
                "no async runtime: auto-out of {}/{} not scheduled",
                key.0,
                key.1
            );
            return;
        };
        let ctx = self.task_ctx();
        let task_key = key.clone();
        let task = handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(u64::from(ms))).await;
            ctx.hide_due(&task_key, out_gen, |timers, cds| {
                if timers.get(&task_key).is_some_and(|t| t.out_gen == out_gen) {
                    timers.remove(&task_key);
                }
                // The layer goes off air: its countdown end (if any) is moot.
                if let Some(c) = cds.remove(&task_key) {
                    c.abort.abort();
                }
                true
            });
        });
        lock(&self.timers).insert(
            key,
            Timer {
                out_gen,
                abort: task.abort_handle(),
            },
        );
    }
}

/// What a timer task needs from the hub, held weakly so a dropped hub ends it.
struct TaskCtx {
    state: Weak<RwLock<LiveState>>,
    timers: Weak<Timers>,
    countdowns: Weak<Countdowns>,
    notifier: Weak<Notifier>,
    hook: Weak<HookSlot>,
    tbar: Weak<TbarClocks>,
    tx: broadcast::Sender<Arc<HubMsg>>,
}

impl TaskCtx {
    /// A media layer's out point: `hold` pauses it there, `clear` takes it off air; then the
    /// media-ended hook runs. Only if this task is still the scheduled one and the layer is in
    /// the same on-air period with the same playback.
    #[allow(clippy::too_many_arguments)]
    fn media_end_due(
        &self,
        key: &LayerKey,
        ends: &Weak<MediaEnds>,
        hook: &Weak<RwLock<Option<MediaEndedHook>>>,
        playback: MediaPlayback,
        action: MediaEndAction,
        out_gen: u64,
        at: u64,
    ) {
        let (Some(state), Some(ends), Some(notifier)) = (
            self.state.upgrade(),
            ends.upgrade(),
            self.notifier.upgrade(),
        ) else {
            return;
        };
        let (screen_id, layer_id) = key;
        let mut fired = false;
        let mut hidden = false;
        {
            let mut s = write_lock(&state);
            let mine = {
                let mut e = lock(&ends);
                let mine = e.get(key).is_some_and(|m| m.playback == playback);
                if mine {
                    e.remove(key);
                }
                mine
            };
            let current = mine
                && s.layer_mut(screen_id, layer_id)
                    .is_ok_and(|l| l.on_air && l.out_gen == out_gen && l.playback == playback);
            if current {
                fired = true;
                let msgs = match action {
                    MediaEndAction::Clear => {
                        let m = s.hide(screen_id, layer_id).unwrap_or_default();
                        s.program_changed(screen_id, &m, None);
                        hidden = !m.is_empty();
                        m
                    }
                    _ => s.media_hold(screen_id, layer_id, &playback, at),
                };
                send_to(&self.tx, screen_id, &msgs, false);
                notifier.note(&s, screen_id);
            }
        }
        notifier.flush();
        if hidden {
            if let Some(h) = self.hook.upgrade() {
                call_hook(&h);
            }
        }
        if fired {
            let h = hook
                .upgrade()
                .and_then(|h| h.read().unwrap_or_else(PoisonError::into_inner).clone());
            if let Some(h) = h {
                h(screen_id, layer_id);
            }
        }
    }

    /// Takes the layer off air if `claim` (run with the timer maps locked, state lock held) says
    /// this task is still the current one, and the layer is still in the on-air period `out_gen`.
    fn hide_due(
        &self,
        key: &LayerKey,
        out_gen: u64,
        claim: impl FnOnce(&mut HashMap<LayerKey, Timer>, &mut HashMap<LayerKey, Countdown>) -> bool,
    ) {
        let (Some(state), Some(timers), Some(cds), Some(notifier)) = (
            self.state.upgrade(),
            self.timers.upgrade(),
            self.countdowns.upgrade(),
            self.notifier.upgrade(),
        ) else {
            return;
        };
        let mut hidden = false;
        {
            let mut s = write_lock(&state);
            let mine = {
                let mut t = lock(&timers);
                let mut c = lock(&cds);
                claim(&mut t, &mut c)
            };
            let (screen_id, layer_id) = key;
            let due = mine
                && s.layer_mut(screen_id, layer_id)
                    .is_ok_and(|l| l.on_air && l.out_gen == out_gen);
            if due {
                if let Ok(msgs) = s.hide(screen_id, layer_id) {
                    s.program_changed(screen_id, &msgs, None);
                    send_to(&self.tx, screen_id, &msgs, false);
                    notifier.note(&s, screen_id);
                    hidden = !msgs.is_empty();
                }
            }
        }
        notifier.flush();
        if hidden {
            if let Some(hook) = self.hook.upgrade() {
                call_hook(&hook);
            }
        }
    }

    /// The paced T-bar send (see `LiveHub::tbar_tick`): the latest position, if the T-bar is
    /// still in progress and it changed since the last send.
    fn send_tbar(&self, screen_id: &str) {
        let (Some(state), Some(clocks), Some(notifier)) = (
            self.state.upgrade(),
            self.tbar.upgrade(),
            self.notifier.upgrade(),
        ) else {
            return;
        };
        {
            let mut s = write_lock(&state);
            let mut clocks = lock(&clocks);
            let Some(c) = clocks.get_mut(screen_id) else {
                return;
            };
            c.task = None;
            if let Some(p) = s.tbar_unsent(screen_id) {
                c.last_sent = Some(Instant::now());
                send_to(&self.tx, screen_id, &[ServerMsg::Tbar { p }], false);
                drop(clocks);
                notifier.note(&s, screen_id);
            }
        }
        notifier.flush();
    }
}

/// Serializes each message once and broadcasts it to the screen's Program renderers, or with
/// `preview` to its preview bus. Caller holds the write lock.
fn send_to(
    tx: &broadcast::Sender<Arc<HubMsg>>,
    screen_id: &str,
    msgs: &[ServerMsg],
    preview: bool,
) {
    if msgs.is_empty() || tx.receiver_count() == 0 {
        return;
    }
    let screen: Arc<str> = Arc::from(screen_id);
    for msg in msgs {
        let Some(json) = serialize(msg) else { continue };
        // An error only means nobody is subscribed right now.
        let _ = tx.send(Arc::new(HubMsg {
            screen_id: screen.clone(),
            json: Arc::from(json),
            preview,
        }));
    }
}

fn serialize(msg: &ServerMsg) -> Option<String> {
    match serde_json::to_string(msg) {
        Ok(s) => Some(s),
        Err(e) => {
            tracing::error!("serializing a live message failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod studio_tests;

mod fades;
pub use fades::{VisibleMap, FADE_CAP};

#[cfg(test)]
mod media_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::live::{PREVIEW_LAYER_ID, PREVIEW_SCREEN_ID, TEST_LAYER_ID, TEST_SCREEN_ID};
    use serde_json::Value;
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};

    fn json(m: &HubMsg) -> Value {
        serde_json::from_str(&m.json).unwrap()
    }

    #[test]
    fn ops_broadcast_one_shared_payload() {
        let hub = LiveHub::with_test_screen();
        let mut a = hub.subscribe();
        let mut b = hub.subscribe();
        let view = hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert!(view.on_air);
        let ma = a.try_recv().unwrap();
        let mb = b.try_recv().unwrap();
        assert!(Arc::ptr_eq(&ma, &mb), "one message object for all clients");
        assert!(Arc::ptr_eq(&ma.json, &mb.json), "serialized once");
        assert_eq!(&*ma.screen_id, TEST_SCREEN_ID);
        assert_eq!(
            &*ma.json,
            r#"{"t":"state","layerId":"lower-third","state":"in"}"#
        );
        // Idempotent show sends nothing.
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert!(matches!(a.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn any_on_air_counts_user_screens_in_program_only() {
        let mut live = LiveState::with_test_screen();
        let mut main = live.screens[0].clone();
        main.info.id = "main".into();
        live.screens.push(main);
        let layer = live.screens[1].layers[0].id.clone();
        let hub = LiveHub::new(live);
        assert!(!hub.any_on_air());
        // The reserved test screen doesn't count.
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert!(!hub.any_on_air());
        hub.show("main", &layer).unwrap();
        assert!(hub.any_on_air());
        hub.panic();
        assert!(!hub.any_on_air());
    }

    #[test]
    fn set_fields_and_clear_broadcast_diffs() {
        let hub = LiveHub::with_test_screen();
        let mut rx = hub.subscribe();
        let f: Fields = [("title".to_string(), "Engineer".to_string())].into();
        let view = hub.set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, f).unwrap();
        assert_eq!(view.fields["title"], "Engineer");
        let m = json(&rx.try_recv().unwrap());
        assert_eq!(m["t"], "fields");
        assert_eq!(m["fields"], serde_json::json!({"title": "Engineer"}));

        let bad: Fields = [("nope".to_string(), "x".to_string())].into();
        assert_eq!(
            hub.set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, bad),
            Err(LiveError::UnknownField("nope".into()))
        );
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));

        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        rx.try_recv().unwrap();
        let screen = hub.clear(TEST_SCREEN_ID).unwrap();
        assert!(!screen.layers[0].on_air);
        assert_eq!(json(&rx.try_recv().unwrap())["state"], "out");
    }

    #[test]
    fn reset_sends_snapshots_and_clears() {
        let hub = LiveHub::with_test_screen();
        let mut rx = hub.subscribe();
        hub.reset(LiveState::default());
        let m = rx.try_recv().unwrap();
        assert_eq!(&*m.screen_id, TEST_SCREEN_ID);
        assert_eq!(&*m.json, r#"{"t":"clear"}"#);
        assert!(!hub.has_screen(TEST_SCREEN_ID));
        assert!(hub.snapshot_json(TEST_SCREEN_ID).is_none());
        assert!(matches!(
            hub.show(TEST_SCREEN_ID, TEST_LAYER_ID),
            Err(LiveError::UnknownScreen(_))
        ));

        hub.reset(LiveState::with_test_screen());
        let m = rx.try_recv().unwrap();
        assert_eq!(json(&m)["t"], "snapshot");
        assert_eq!(&*m.json, hub.snapshot_json(TEST_SCREEN_ID).unwrap());
    }

    fn with_auto_out(ms: u32) -> LiveHub {
        let hub = LiveHub::with_test_screen();
        hub.update_layer(TEST_SCREEN_ID, TEST_LAYER_ID, |l| {
            l.auto_out_ms = Some(ms);
            Ok::<_, LiveError>(Vec::new())
        })
        .unwrap();
        hub
    }

    fn on_air(hub: &LiveHub) -> bool {
        hub.layer_view(TEST_SCREEN_ID, TEST_LAYER_ID)
            .unwrap()
            .on_air
    }

    async fn sleep_ms(ms: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }

    /// Yields until the auto-out took the test layer off air (generous deadline: a loaded
    /// machine may run the timer late, never early).
    async fn wait_off_air(hub: &LiveHub) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while on_air(hub) {
            assert!(std::time::Instant::now() < deadline, "auto-out never fired");
            sleep_ms(5).await;
        }
    }

    #[tokio::test]
    async fn auto_out_hides_after_the_duration() {
        // Long against a stall of this thread, so "not before" can't see the timer fire.
        let hub = with_auto_out(400);
        let mut rx = hub.subscribe();
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(json(&rx.try_recv().unwrap())["state"], "in");
        assert_eq!(hub.pending_auto_outs(), 1);
        sleep_ms(10).await;
        assert!(on_air(&hub), "not before the duration");
        wait_off_air(&hub).await;
        let m = json(&rx.try_recv().unwrap());
        assert_eq!(m["t"], "state");
        assert_eq!(m["state"], "out");
        assert_eq!(hub.pending_auto_outs(), 0, "timer forgets itself");
        // Without auto-out nothing is scheduled.
        let plain = LiveHub::with_test_screen();
        plain.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(plain.pending_auto_outs(), 0);
    }

    #[tokio::test]
    async fn auto_out_is_cancelled_by_hide() {
        let hub = with_auto_out(50);
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        hub.hide(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(hub.pending_auto_outs(), 0);
        let mut rx = hub.subscribe();
        sleep_ms(120).await;
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(!on_air(&hub));
    }

    #[tokio::test]
    async fn auto_out_restarts_on_reshow() {
        // Long timings: the check at 1.2 s (after the first timer's 1 s) still comes well before
        // the second timer's 1.6 s on a loaded machine.
        let hub = with_auto_out(1000);
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        sleep_ms(600).await;
        // Out and in again: the first timer must not cut the second appearance short.
        hub.hide(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(hub.pending_auto_outs(), 1);
        sleep_ms(600).await;
        assert!(on_air(&hub), "old timer cancelled");
        // The new timer fires.
        wait_off_air(&hub).await;
    }

    #[tokio::test]
    async fn auto_out_cancelled_by_clear_and_reset() {
        let hub = with_auto_out(40);
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        hub.clear(TEST_SCREEN_ID).unwrap();
        assert_eq!(hub.pending_auto_outs(), 0);
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        hub.reset(LiveState::with_test_screen());
        assert_eq!(hub.pending_auto_outs(), 0);
        // The fresh state's layer has no auto-out; showing it stays on air.
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        sleep_ms(90).await;
        assert!(on_air(&hub));
    }

    #[test]
    fn auto_out_uses_the_injected_runtime() {
        // Called from a plain thread (like a sync Tauri command): no current runtime.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_time()
            .build()
            .unwrap();
        let hub = with_auto_out(30);
        hub.set_runtime(rt.handle().clone());
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while on_air(&hub) {
            assert!(std::time::Instant::now() < deadline, "auto-out never fired");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // No runtime at all: auto-out is skipped, the layer stays on air.
        let lonely = with_auto_out(30);
        lonely.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(lonely.pending_auto_outs(), 0);
    }

    #[test]
    fn update_layer_broadcasts_and_leaves_state_on_error() {
        let hub = LiveHub::with_test_screen();
        let mut rx = hub.subscribe();
        let err = hub
            .update_layer(TEST_SCREEN_ID, TEST_LAYER_ID, |_| {
                Err::<Vec<ServerMsg>, _>(LiveError::UnknownField("x".into()))
            })
            .unwrap_err();
        assert_eq!(err, LiveError::UnknownField("x".into()));
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        assert!(matches!(
            hub.update_layer(TEST_SCREEN_ID, "nope", |_| Ok::<_, LiveError>(Vec::new())),
            Err(LiveError::UnknownLayer(_))
        ));
        let view = hub
            .update_layer(TEST_SCREEN_ID, TEST_LAYER_ID, |l| {
                l.vars.insert("accent".into(), "#fff".into());
                Ok::<_, LiveError>(vec![ServerMsg::Vars {
                    layer_id: l.id.clone(),
                    vars: l.vars.clone(),
                }])
            })
            .unwrap();
        assert_eq!(view.id, TEST_LAYER_ID);
        let m = json(&rx.try_recv().unwrap());
        assert_eq!(
            m,
            serde_json::json!({"t": "vars", "layerId": "lower-third", "vars": {"accent": "#fff"}})
        );
        assert_eq!(
            hub.with_layer(TEST_SCREEN_ID, TEST_LAYER_ID, |l| l.vars.clone())
                .unwrap()["accent"],
            "#fff"
        );
    }

    #[test]
    fn lagged_subscriber_resyncs_from_snapshot() {
        let hub = LiveHub::with_test_screen();
        let mut rx = hub.subscribe();
        for i in 0..(HUB_CAPACITY + 10) {
            let f: Fields = [("name".to_string(), format!("n{i}"))].into();
            hub.set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, f).unwrap();
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let lag = rt.block_on(rx.recv());
        assert!(matches!(lag, Err(RecvError::Lagged(10))));
        // What a server connection does on lag: resubscribe at the tail, then a fresh snapshot.
        let mut rx = rx.resubscribe();
        let snap: Value =
            serde_json::from_str(&hub.snapshot_json(TEST_SCREEN_ID).unwrap()).unwrap();
        assert_eq!(
            snap["layers"][0]["fields"]["name"],
            format!("n{}", HUB_CAPACITY + 9)
        );
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    // ---- live-changed --------------------------------------------------------------------

    #[derive(Default)]
    struct Views(Mutex<Vec<(String, Value)>>);

    impl EventSink for Views {
        fn emit(&self, event: &str, payload: Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    impl Views {
        fn take(&self) -> Vec<Value> {
            let all = std::mem::take(&mut *self.0.lock().unwrap());
            assert!(all.iter().all(|(n, _)| n == names::LIVE_CHANGED));
            all.into_iter().map(|(_, v)| v).collect()
        }
    }

    fn notified(hub: &LiveHub) -> Arc<Views> {
        let views = Arc::new(Views::default());
        hub.set_notifier(views.clone());
        views
    }

    #[test]
    fn live_changed_only_for_real_view_changes() {
        let hub = LiveHub::new(LiveState::for_project());
        let views = notified(&hub);
        assert!(views.take().is_empty(), "the current state is the baseline");

        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let got = views.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["screen"]["id"], TEST_SCREEN_ID);
        assert_eq!(got[0]["layers"][0]["onAir"], true);
        // Same shape as list_screens.
        assert_eq!(
            got[0],
            serde_json::to_value(hub.screen_view(TEST_SCREEN_ID).unwrap()).unwrap()
        );

        // No-ops emit nothing.
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let same: Fields = [("name".to_string(), String::new())].into();
        hub.set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, same).unwrap();
        let _ = hub.hide(TEST_SCREEN_ID, "nope");
        // A rect drag is not in the view.
        for x in 0..5 {
            let rect = Rect {
                x,
                y: 0,
                w: 100,
                h: 100,
            };
            hub.set_rect(TEST_SCREEN_ID, TEST_LAYER_ID, rect, LiveEdit::Immediate)
                .unwrap();
        }
        assert!(views.take().is_empty());

        let f: Fields = [("name".to_string(), "Ada".to_string())].into();
        hub.set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, f).unwrap();
        let got = views.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["layers"][0]["fields"]["name"], "Ada");

        hub.clear(TEST_SCREEN_ID).unwrap();
        assert_eq!(views.take().len(), 1);

        // The system preview screen is never emitted.
        hub.update_layer(PREVIEW_SCREEN_ID, PREVIEW_LAYER_ID, |l| {
            l.on_air = true;
            Ok::<_, LiveError>(vec![l.state_msg()])
        })
        .unwrap();
        assert!(views.take().is_empty());

        // Reset: changed screens emit; removed ones are forgotten.
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        views.take();
        hub.reset(LiveState::default());
        assert!(views.take().is_empty(), "removed screens are not emitted");
        hub.reset(LiveState::for_project());
        let got = views.take();
        assert_eq!(got.len(), 1, "the test screen is back (preview never)");
        assert_eq!(got[0]["layers"][0]["onAir"], false);
    }

    #[tokio::test]
    async fn auto_out_emits_live_changed() {
        let hub = with_auto_out(30);
        let views = notified(&hub);
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(views.take().len(), 1);
        wait_off_air(&hub).await;
        let got = views.take();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["layers"][0]["onAir"], false);
    }

    #[tokio::test]
    async fn panic_cuts_everything_and_cancels_auto_outs() {
        let mut live = LiveState::for_project();
        let mut user = live.screens[0].clone();
        user.info.id = "main".into();
        live.screens.push(user);
        let hub = LiveHub::new(live);
        hub.update_layer(TEST_SCREEN_ID, TEST_LAYER_ID, |l| {
            l.auto_out_ms = Some(40);
            Ok::<_, LiveError>(Vec::new())
        })
        .unwrap();
        let views = notified(&hub);
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        hub.show("main", TEST_LAYER_ID).unwrap();
        // Staged content on the on-air user layer survives the cut.
        hub.stage_for_test("main", TEST_LAYER_ID, |c| {
            c.fields.insert("name".into(), "Next".into());
        });
        assert_eq!(hub.pending_auto_outs(), 1);
        views.take();
        let mut rx = hub.subscribe();

        let out = hub.panic();
        assert_eq!(out.len(), 1, "user screens only");
        assert_eq!(out[0].screen.id, "main");
        assert!(!out[0].layers[0].on_air);
        assert!(out[0].layers[0].pending, "staged changes kept");
        assert_eq!(hub.pending_auto_outs(), 0);
        assert!(!on_air(&hub));
        let mut msgs = Vec::new();
        while let Ok(m) = rx.try_recv() {
            msgs.push((m.screen_id.to_string(), m.json.to_string()));
        }
        assert_eq!(
            msgs,
            [
                (TEST_SCREEN_ID.to_string(), r#"{"t":"cut"}"#.to_string()),
                ("main".to_string(), r#"{"t":"cut"}"#.to_string()),
            ],
            "one cut per screen, no state messages, nothing for __preview"
        );
        let ids: Vec<String> = views
            .take()
            .iter()
            .map(|v| v["screen"]["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, [TEST_SCREEN_ID, "main"]);

        // The cancelled auto-out never fires; the next take applies the staged change.
        sleep_ms(100).await;
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        let v = hub.show("main", TEST_LAYER_ID).unwrap();
        assert!(v.on_air && !v.pending);
        assert_eq!(v.fields["name"], "Next");
        // Panic with nothing on air still cuts, and emits nothing new for unchanged views.
        hub.hide("main", TEST_LAYER_ID).unwrap();
        views.take();
        hub.panic();
        assert!(views.take().is_empty());
    }

    #[test]
    fn clear_all_takes_every_user_screen_out() {
        let mut live = LiveState::for_project();
        for id in ["a", "b"] {
            let mut user = live.screens[0].clone();
            user.info.id = id.into();
            live.screens.push(user);
        }
        let hub = LiveHub::new(live);
        hub.show("a", TEST_LAYER_ID).unwrap();
        hub.show("b", TEST_LAYER_ID).unwrap();
        hub.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let mut rx = hub.subscribe();
        let views = hub.clear_all();
        assert_eq!(views.len(), 2);
        assert!(views.iter().all(|v| !v.layers[0].on_air));
        assert!(on_air(&hub), "the test screen is not a user screen");
        let mut n = 0;
        while let Ok(m) = rx.try_recv() {
            assert_eq!(json(&m)["state"], "out", "animated");
            n += 1;
        }
        assert_eq!(n, 2);
    }
}
