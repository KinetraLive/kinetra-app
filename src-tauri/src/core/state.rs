//! `AppState`: the single in-memory source of truth for app-level stores (prefs, recents) and the
//! open project (`project.json` + `ui-state.json`).
//!
//! # Reads
//! Every getter is served from memory; nothing here reads the disk except `load`, opening /
//! reloading a project, and the flusher.
//!
//! # Writes
//! A setter updates memory, marks the store dirty and pings ONE long-lived flusher thread over an
//! `mpsc` channel. The flusher blocks on `recv`, then waits for `debounce` of quiet
//! (`recv_timeout`, capped at `10 × debounce` under a constant stream of changes) and writes every
//! dirty store once. [`AppState::flush_all`] does the same synchronously.
//!
//! Screens and sources changed in memory only (Control field edits, source steps) are marked dirty
//! in their stores and written by a second long-lived thread, the content flusher: it blocks on
//! `recv`, then writes once [`CONTENT_FLUSH_DELAY`] has passed since the first change (later
//! changes ride along), so each file is written at most once per that delay. Every explicit flush
//! (`flush_all`, project open/close/switch, move, snapshot, restore, exit) writes them too; the
//! store flusher does not, so its debounce can't add writes.
//!
//! # Locking rules
//! - Data locks (`prefs`, `recents`, `project`) are held only to read or clone small structs; never
//!   across a filesystem call.
//! - `io` serializes disk writes and project switches so two writers can never race on the same
//!   file. It is always taken *before* any data lock, never while holding one.
//!
//! # Live state and the project observer
//! The `LiveHub` (what is on air) is seeded with the test screen, the template preview screen
//! (`__preview`) and every screen in the project's `screens/` (all off air) when a project opens,
//! and emptied when it closes (see `core::screen_live`). A restore re-syncs the screens by the
//! live rules.
//!
//! # Project folder watcher
//! A `ProjectWatcher` runs while a project is open: started on open, stopped and joined on close,
//! switch, move (restarted on the new folder) and around a restore. Between batches its handler
//! holds only a `Weak` to the state, and it never takes `io`, so stopping it with `io` held cannot
//! deadlock.
//!
//! The `ProjectObserver` (the output server) is told about every open, close,
//! relocation, settings change and reload, always after the change and with no `AppState` lock
//! held (not even `io`), reading the current project at call time so the last call always
//! reflects the latest state.
//!
//! # Read-only stores
//! A store file written by a newer app version (or one that could not be read) is never
//! overwritten: the session runs on defaults in memory and that store is never written.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Weak;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

use crate::core::api_poll::ApiPoller;
use crate::core::automation::Automation;
use crate::core::backup;
use crate::core::events::{names, EventSink, ProjectObserver};
use crate::core::flow_engine::FlowEngine;
use crate::core::flows::FlowStore;
use crate::core::fsx;
use crate::core::live::LiveState;
use crate::core::live_hub::LiveHub;
use crate::core::model::{
    BackupInfo, BackupPrefs, BackupReason, ControlsDoc, MidiPrefs, OscSettings, Prefs, Project,
    ProjectFile, ProjectSettings, RecentProject, UiState, BACKUP_INTERVAL_RANGE,
    BACKUP_RETENTION_RANGE, COMMAND_ID_MAX, KEYBINDINGS_CAP, KEYS_PER_COMMAND, MIDI_DEVICES_CAP,
    MIDI_NAME_MAX, OSC_ALLOW_CAP, OSC_HOST_MAX, PALETTE_RECENT_CAP, SCREEN_CROP_OVERLAYS,
    SCREEN_GRID_SIZE_RANGE, ZOOM_RANGE, ZOOM_STEP,
};
use crate::core::persist::{self, RecentsFile, Store};
use crate::core::recipes;
use crate::core::rules::Rules;
use crate::core::screens::ScreenStore;
use crate::core::sources::SourceStore;
use crate::core::templates::{is_id, TemplateStore};
use crate::core::watch::ProjectWatcher;
use crate::core::{project, recents, screen_live, templates};
use crate::error::{AppError, AppResult};

/// Quiet period before the flusher writes dirty stores.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(400);
/// The content flusher writes a changed screen or source this long after its first in-memory
/// change, so each file is written at most once per this period.
pub const CONTENT_FLUSH_DELAY: Duration = Duration::from_millis(800);

/// Allowed `UiState.control.grid_columns`.
pub const GRID_COLUMNS_RANGE: std::ops::RangeInclusive<u32> = 1..=6;
/// Allowed server port in project settings.
pub const PORT_RANGE: std::ops::RangeInclusive<u16> = 1024..=65535;
/// Allowed width/height of `ProjectSettings.default_resolution`.
pub const RESOLUTION_RANGE: std::ops::RangeInclusive<u32> = 16..=8192;

/// Single source of truth. Cheap to clone (an `Arc` inside) so commands can move it into
/// `spawn_blocking`.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    /// Handed to background workers so they never keep the state alive.
    me: Weak<Inner>,
    config_dir: PathBuf,
    events: Arc<dyn EventSink>,
    prefs: RwLock<Tracked<Prefs>>,
    recents: RwLock<Tracked<Vec<RecentProject>>>,
    project: RwLock<Option<OpenProject>>,
    move_cancel: AtomicBool,
    /// True while a project move runs (see [`AppState::begin_move`]).
    moving: AtomicBool,
    /// Backup timer of the open project. Dropping the handle stops the thread.
    backup_worker: Mutex<Option<backup::Worker>>,
    /// Serializes disk writes and project switches. See the module docs for lock order.
    io: Mutex<()>,
    /// Wakes the flusher. Dropping it (when the last `AppState` goes) ends the flusher thread.
    notify: Sender<()>,
    /// Number of store files written successfully. Observable in tests.
    writes: AtomicU64,
    /// What is on air. Shared with the output server.
    live: Arc<LiveHub>,
    /// The open project's templates (summary cache + write lock). Forgotten on every project
    /// open, close, reload and move.
    templates: TemplateStore,
    /// The open project's screens (summary cache, write lock, what the live state was built
    /// from). Replaced on every project open, close, reload and move.
    screens: ScreenStore,
    /// The open project's sources, all in memory. Replaced like `screens`.
    sources: SourceStore,
    /// Wakes the content flusher (dirty screens and sources). Dropping it ends that thread.
    content_notify: Sender<()>,
    /// The content flusher's delay, in milliseconds ([`CONTENT_FLUSH_DELAY`] unless a test
    /// changes it).
    content_delay_ms: AtomicU64,
    /// Number of screen and source files the content flusher (or a flush) wrote.
    content_writes: AtomicU64,
    /// Watches the open project's folder for external edits. Started on open, stopped (joined)
    /// on close, switch, move and restore.
    watcher: Mutex<Option<ProjectWatcher>>,
    /// Told about project changes (the output server, OSC). See the module docs.
    observers: RwLock<Vec<Arc<dyn ProjectObserver>>>,
    /// Timers aside (they live in the hub): auto-advance, macros (see `core::automation`).
    automation: Automation,
    /// Recipe instances, the schedule planner and rules (see `core::rules`).
    rules: Rules,
    /// The open project's flows (see `core::flows`).
    flows: FlowStore,
    /// The flow engine (see `core::flow_engine`).
    flow_engine: FlowEngine,
    /// API source polling and secrets (see `core::api_poll`).
    api: ApiPoller,
    /// Live polls: result streams and test simulations (see `core::polls`).
    polls: crate::core::polls::PollService,
    /// Controls: change listeners and value pacing (see `core::controls`).
    control_rt: crate::core::controls::ControlRuntime,
    /// Told after on-air changes and flow runs (OSC / MIDI feedback). See [`StateChange`].
    change_listeners: RwLock<Vec<ChangeListener>>,
    /// MIDI devices, input and LED feedback (see `core::midi`).
    midi: crate::core::midi::Midi,
    /// Connected displays and their hot-plug watcher (see `core::displays`).
    displays: crate::core::displays::Displays,
    /// Display outputs: plan, statuses, the window host (see `core::outputs`).
    outputs: crate::core::outputs::Outputs,
    /// Sign-in and licensing (see `core::cloud::service`).
    cloud: crate::core::cloud::service::Cloud,
    /// Remote control through kinetra-cloud (see `core::remote`).
    remote: crate::core::remote::Remote,
    /// The local Control API (see `core::control_api`).
    control_api: crate::core::control_api::ControlApi,
}

/// A change feedback senders follow without polling ([`AppState::add_change_listener`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateChange<'a> {
    /// After every live operation that may have changed what is on air (the `LiveHub` hook).
    Live,
    /// A flow instance started (`running`) or ended (finished, stopped or failed).
    FlowRun { id: &'a str, running: bool },
    /// A layer that went off air finished its out-animation on every output (or the fallback
    /// time passed) without any other live operation: only exact visibility changed (remote
    /// control, `LiveHub::visible`).
    Visible,
}

/// Called on the thread that made the change. For `Live` no state lock is held; `FlowRun` comes
/// from `emit_state`, which may run while the flow engine's lock is held, so a `FlowRun`
/// handler must not call back into the engine (or anything that takes its lock). Either way:
/// never block (send with `try_send` / a non-blocking socket) and never change live state.
pub type ChangeListener = Arc<dyn Fn(&AppState, StateChange<'_>) + Send + Sync>;

/// An in-memory store value plus its persistence flags.
struct Tracked<T> {
    value: T,
    dirty: bool,
    /// Never written this session (file is newer than this build, or unreadable).
    read_only: bool,
}

impl<T: Clone> Tracked<T> {
    fn clean(value: T) -> Self {
        Self {
            value,
            dirty: false,
            read_only: false,
        }
    }

    /// Clears `dirty` and returns a copy to write, unless the store is clean or read-only.
    fn take_dirty(&mut self) -> Option<T> {
        if self.dirty && !self.read_only {
            self.dirty = false;
            Some(self.value.clone())
        } else {
            None
        }
    }
}

struct OpenProject {
    /// Absolute, without a verbatim prefix.
    dir: PathBuf,
    file: Tracked<ProjectFile>,
    ui: Tracked<UiState>,
    /// `controls.json` (MIDI mappings); shared read-only with the MIDI input path.
    controls: Tracked<Arc<ControlsDoc>>,
}

impl OpenProject {
    fn to_project(&self) -> Project {
        Project {
            path: fsx::display_path(&self.dir),
            name: self.file.value.name.clone(),
            created_at: self.file.value.created_at.clone(),
            settings: self.file.value.settings.clone(),
        }
    }
}

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Named keys a key binding may use (besides A–Z, 0–9, F1–F24 and `KEY_PUNCTUATION`).
const NAMED_KEYS: &[&str] = &[
    "Escape",
    "Enter",
    "Space",
    "Tab",
    "Backspace",
    "Delete",
    "Insert",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
];
/// Punctuation keys a key binding may use, compared by the character typed.
const KEY_PUNCTUATION: &str = ";',./\\[]-=`";

/// A key binding in its one canonical spelling (src/lib/commands/keys.ts writes the same):
/// modifiers Ctrl, Alt, Shift (each at most once, in that order), then one key: A–Z, 0–9, F1–F24,
/// one of KEY_PUNCTUATION or a named key (Escape, Enter, Space, arrows…). E.g. "Ctrl+Shift+P",
/// "Shift+F3", "Escape".
pub fn is_key_combo(combo: &str) -> bool {
    let mut parts: Vec<&str> = combo.split('+').collect();
    let Some(key) = parts.pop() else {
        return false;
    };
    let mut next_mod = 0;
    for m in parts {
        match ["Ctrl", "Alt", "Shift"][next_mod..]
            .iter()
            .position(|x| *x == m)
        {
            Some(i) => next_mod += i + 1,
            None => return false,
        }
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            c.is_ascii_uppercase() || c.is_ascii_digit() || KEY_PUNCTUATION.contains(c)
        }
        _ => {
            NAMED_KEYS.contains(&key)
                || key
                    .strip_prefix('F')
                    .filter(|n| !n.starts_with('0'))
                    .and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=24).contains(&n))
        }
    }
}

/// A command id as bindings and the palette's Recent store it: 1–COMMAND_ID_MAX bytes, no
/// whitespace or control characters (e.g. "nav.control", "live.toggle:<screen>/<layer>").
pub fn is_command_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= COMMAND_ID_MAX
        && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Clamps backup settings into their allowed ranges and cleans `keybindings`: invalid ids and
/// keys are dropped, duplicate keys of one command collapse, each command keeps ≤ KEYS_PER_COMMAND
/// keys and the table ≤ KEYBINDINGS_CAP entries (the first in id order). An empty list stays (the
/// command is unbound on purpose).
pub fn clamp_prefs(mut p: Prefs) -> Prefs {
    p.zoom = p.zoom.and_then(clamp_zoom);
    p.outputs = crate::core::outputs::clamp_outputs(std::mem::take(&mut p.outputs));
    p.midi = clamp_midi_prefs(std::mem::take(&mut p.midi));
    p.keybindings = std::mem::take(&mut p.keybindings)
        .into_iter()
        .filter(|(id, _)| is_command_id(id))
        .take(KEYBINDINGS_CAP)
        .map(|(id, keys)| {
            let mut out: Vec<String> = Vec::new();
            for k in keys {
                if is_key_combo(&k) && !out.contains(&k) && out.len() < KEYS_PER_COMMAND {
                    out.push(k);
                }
            }
            (id, out)
        })
        .collect();
    p.backup.interval_minutes = p
        .backup
        .interval_minutes
        .clamp(*BACKUP_INTERVAL_RANGE.start(), *BACKUP_INTERVAL_RANGE.end());
    p.backup.retention = p.backup.retention.clamp(
        *BACKUP_RETENTION_RANGE.start(),
        *BACKUP_RETENTION_RANGE.end(),
    );
    p
}

/// `Prefs.zoom` as stored: within [`ZOOM_RANGE`], rounded to [`ZOOM_STEP`]; `None` for 1 (the
/// default) and for a value that isn't a number.
pub fn clamp_zoom(zoom: f64) -> Option<f64> {
    if !zoom.is_finite() {
        return None;
    }
    let z = zoom.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
    // Divide by the step count (not multiply by the step) so 1.2 is exactly the double "1.2".
    let steps = (1.0 / ZOOM_STEP).round();
    let z = (z * steps).round() / steps;
    ((z - 1.0).abs() > 1e-9).then_some(z)
}

/// A MIDI device (port) name as prefs store it: 1–MIDI_NAME_MAX characters, no control characters.
pub fn is_midi_name(name: &str) -> bool {
    !name.is_empty() && name.chars().count() <= MIDI_NAME_MAX && !name.chars().any(char::is_control)
}

/// Drops entries with an invalid device or output name and keeps ≤ MIDI_DEVICES_CAP per table
/// (the first in name order). Unknown profiles never get here (they fail to deserialize).
pub fn clamp_midi_prefs(mut m: MidiPrefs) -> MidiPrefs {
    m.inputs = std::mem::take(&mut m.inputs)
        .into_iter()
        .filter(|(name, _)| is_midi_name(name))
        .take(MIDI_DEVICES_CAP)
        .collect();
    m.feedback = std::mem::take(&mut m.feedback)
        .into_iter()
        .filter(|(name, f)| is_midi_name(name) && f.output.as_deref().is_none_or(is_midi_name))
        .take(MIDI_DEVICES_CAP)
        .collect();
    m
}

/// A hotkey combo: `F1`–`F12`, optionally prefixed `Shift+` (exact spelling).
pub fn is_hotkey_combo(combo: &str) -> bool {
    let key = combo.strip_prefix("Shift+").unwrap_or(combo);
    key.strip_prefix('F')
        .filter(|n| !n.starts_with('0'))
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n))
}

/// A hotkey target: `<screenId>/<layerId>` (a layer) or `macro/<macroId>` (a macro), all
/// path-safe ids. (`macro` is itself a valid id, so one rule covers both.)
pub fn is_hotkey_target(key: &str) -> bool {
    key.split_once('/')
        .is_some_and(|(screen, layer)| is_id(screen) && is_id(layer))
}

/// Clamps `control.grid_columns` into `GRID_COLUMNS_RANGE` and cleans `control.hotkeys`: entries
/// with an invalid target or combo are dropped, and when several targets share a combo only the
/// last one in key order (the map's order) keeps it. The layout editor's grid size is clamped into
/// `SCREEN_GRID_SIZE_RANGE` and an unknown crop overlay becomes `"none"`. The palette's Recent keeps
/// ≤ `PALETTE_RECENT_CAP` valid, distinct command ids (newest first, as sent).
pub fn clamp_ui_state(mut u: UiState) -> UiState {
    let mut recent: Vec<String> = Vec::new();
    for id in std::mem::take(&mut u.palette.recent) {
        if is_command_id(&id) && !recent.contains(&id) && recent.len() < PALETTE_RECENT_CAP {
            recent.push(id);
        }
    }
    u.palette.recent = recent;
    let ed = &mut u.screen_editor;
    ed.grid_size = ed.grid_size.clamp(
        *SCREEN_GRID_SIZE_RANGE.start(),
        *SCREEN_GRID_SIZE_RANGE.end(),
    );
    if !SCREEN_CROP_OVERLAYS.contains(&ed.crop.as_str()) {
        ed.crop = "none".into();
    }
    u.control.panel_width = u.control.panel_width.map(|w| {
        w.clamp(
            *crate::core::model::CONTROL_PANEL_WIDTH_RANGE.start(),
            *crate::core::model::CONTROL_PANEL_WIDTH_RANGE.end(),
        )
    });
    u.control.grid_columns = u
        .control
        .grid_columns
        .clamp(*GRID_COLUMNS_RANGE.start(), *GRID_COLUMNS_RANGE.end());
    let mut by_combo: HashMap<String, String> = HashMap::new();
    for (target, combo) in std::mem::take(&mut u.control.hotkeys) {
        if is_hotkey_target(&target) && is_hotkey_combo(&combo) {
            by_combo.insert(combo, target);
        }
    }
    u.control.hotkeys = by_combo
        .into_iter()
        .map(|(combo, target)| (target, combo))
        .collect();
    u
}

/// Rejects out-of-range project settings with `Invalid`.
pub fn validate_project_settings(s: &ProjectSettings) -> AppResult<()> {
    if !PORT_RANGE.contains(&s.port) {
        return Err(AppError::Invalid(format!(
            "Port must be between {} and {}",
            PORT_RANGE.start(),
            PORT_RANGE.end()
        )));
    }
    let r = s.default_resolution;
    if !RESOLUTION_RANGE.contains(&r.width) || !RESOLUTION_RANGE.contains(&r.height) {
        return Err(AppError::Invalid(format!(
            "Resolution must be between {} and {} pixels on each side",
            RESOLUTION_RANGE.start(),
            RESOLUTION_RANGE.end()
        )));
    }
    crate::core::styles::normalize_brand(s.brand.clone())?;
    if let Some(osc) = &s.osc {
        validate_osc(osc)?;
    }
    Ok(())
}

/// OSC settings: the port in PORT_RANGE, ≤ OSC_ALLOW_CAP allowlist entries that parse as IP
/// addresses, and a feedback target with a host (1–OSC_HOST_MAX characters, no spaces) and a port
/// in PORT_RANGE.
pub fn validate_osc(osc: &OscSettings) -> AppResult<()> {
    if !PORT_RANGE.contains(&osc.port) {
        return Err(AppError::Invalid(format!(
            "OSC port must be between {} and {}",
            PORT_RANGE.start(),
            PORT_RANGE.end()
        )));
    }
    if osc.allow.len() > OSC_ALLOW_CAP {
        return Err(AppError::Invalid(format!(
            "At most {OSC_ALLOW_CAP} allowed addresses"
        )));
    }
    if let Some(bad) = osc
        .allow
        .iter()
        .find(|a| a.trim().parse::<std::net::IpAddr>().is_err())
    {
        return Err(AppError::Invalid(format!("Not an IP address: {bad}")));
    }
    if let Some(f) = &osc.feedback {
        let host = f.host.trim();
        if host.is_empty()
            || host.len() > OSC_HOST_MAX
            || host.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(AppError::Invalid("Enter a feedback host".into()));
        }
        if !PORT_RANGE.contains(&f.port) {
            return Err(AppError::Invalid(format!(
                "OSC feedback port must be between {} and {}",
                PORT_RANGE.start(),
                PORT_RANGE.end()
            )));
        }
    }
    Ok(())
}

impl AppState {
    /// Loads app-level stores (prefs, recents) from `config_dir` (created if missing) and starts
    /// the flusher. Never fails on a corrupt or missing store file (corrupt files are set aside
    /// and `store-recovered` is emitted); only on an unusable config dir.
    pub fn load(config_dir: PathBuf, events: Arc<dyn EventSink>) -> AppResult<Self> {
        Self::load_with_debounce(config_dir, events, DEFAULT_DEBOUNCE)
    }

    /// [`AppState::load`] with a custom flusher quiet period (tests use a short one).
    pub fn load_with_debounce(
        config_dir: PathBuf,
        events: Arc<dyn EventSink>,
        debounce: Duration,
    ) -> AppResult<Self> {
        fs::create_dir_all(&config_dir)?;

        let mut prefs = load_tracked::<Prefs>(&config_dir, &*events);
        prefs.value = clamp_prefs(prefs.value);
        let recents_file = load_tracked::<RecentsFile>(&config_dir, &*events);
        let recents = Tracked {
            value: recents_file.value.projects,
            dirty: false,
            read_only: recents_file.read_only,
        };

        let (tx, rx) = mpsc::channel();
        let (content_tx, content_rx) = mpsc::channel();
        let live = Arc::new(LiveHub::default());
        live.set_notifier(events.clone());
        let inner = Arc::new_cyclic(|me| Inner {
            me: me.clone(),
            config_dir,
            events,
            prefs: RwLock::new(prefs),
            recents: RwLock::new(recents),
            project: RwLock::new(None),
            move_cancel: AtomicBool::new(false),
            moving: AtomicBool::new(false),
            backup_worker: Mutex::new(None),
            io: Mutex::new(()),
            notify: tx,
            writes: AtomicU64::new(0),
            live,
            templates: TemplateStore::default(),
            screens: ScreenStore::default(),
            sources: SourceStore::default(),
            content_notify: content_tx,
            content_delay_ms: AtomicU64::new(CONTENT_FLUSH_DELAY.as_millis() as u64),
            content_writes: AtomicU64::new(0),
            watcher: Mutex::new(None),
            observers: RwLock::new(Vec::new()),
            automation: Automation::default(),
            rules: Rules::default(),
            flows: FlowStore::default(),
            flow_engine: FlowEngine::default(),
            api: ApiPoller::default(),
            polls: Default::default(),
            control_rt: Default::default(),
            change_listeners: RwLock::new(Vec::new()),
            midi: Default::default(),
            displays: Default::default(),
            outputs: Default::default(),
            cloud: Default::default(),
            remote: Default::default(),
            control_api: Default::default(),
        });
        // Auto-advance and the `layer` / `timerZero` flow triggers follow what is on air, whichever
        // path changed it.
        let weak = Arc::downgrade(&inner);
        inner.live.set_hook(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                let st = AppState { inner };
                st.rearm();
                st.rules_live_changed();
                st.api_reconcile();
                st.notify_change(StateChange::Live);
            }
        }));
        // A fade ending (exact visibility for remote control) is a change of its own.
        let weak = Arc::downgrade(&inner);
        inner.live.set_visible_hook(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                AppState { inner }.notify_change(StateChange::Visible);
            }
        }));
        // A media layer reaching its out point fires `trigger.mediaEnded` flows.
        let weak = Arc::downgrade(&inner);
        inner
            .live
            .set_media_ended_hook(Arc::new(move |screen: &str, layer: &str| {
                if let Some(inner) = weak.upgrade() {
                    AppState { inner }.flow_media_ended(screen, layer);
                }
            }));
        // The Studio Preview's cues are kept in ui-state.json (coalesced like every store write).
        let weak = Arc::downgrade(&inner);
        inner.live.set_cues_hook(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                AppState { inner }.save_studio_cues();
            }
        }));
        let weak = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("store-flusher".into())
            .spawn(move || flusher_loop(weak, rx, debounce))?;
        let weak = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("content-flusher".into())
            .spawn(move || content_flusher_loop(weak, content_rx))?;
        let st = Self { inner };
        st.midi_init();
        st.outputs_init();
        st.remote_init();
        st.control_api_init();
        Ok(st)
    }

    /// Auto-advance and macros of the open project (see `core::automation`).
    pub fn automation(&self) -> &Automation {
        &self.inner.automation
    }

    /// Recipes, the schedule planner and rules of the open project (see `core::rules`).
    pub fn rules(&self) -> &Rules {
        &self.inner.rules
    }

    /// The open project's flow files (see `core::flows`).
    pub fn flows(&self) -> &FlowStore {
        &self.inner.flows
    }

    /// The flow engine (see `core::flow_engine`).
    pub fn flow_engine(&self) -> &FlowEngine {
        &self.inner.flow_engine
    }

    /// Sign-in and licensing (see `core::cloud::service`).
    pub fn cloud_service(&self) -> &crate::core::cloud::service::Cloud {
        &self.inner.cloud
    }

    /// API source polling and secrets (see `core::api_poll`).
    pub fn api(&self) -> &ApiPoller {
        &self.inner.api
    }

    /// Live polls: result streams and test simulations (see `core::polls`).
    pub fn poll_service(&self) -> &crate::core::polls::PollService {
        &self.inner.polls
    }

    /// Remote control (see `core::remote`).
    pub(crate) fn remote(&self) -> &crate::core::remote::Remote {
        &self.inner.remote
    }

    pub(crate) fn control_api(&self) -> &crate::core::control_api::ControlApi {
        &self.inner.control_api
    }

    /// The open project's remote control ID (`project.json` `id`), if it has one.
    pub fn project_remote_id(&self) -> Option<String> {
        read(&self.inner.project)
            .as_ref()
            .and_then(|p| p.file.value.id.clone())
    }

    /// Sets the open project's remote control ID and writes `project.json` (coalesced, like
    /// every store write). `NoProject` without one.
    pub(crate) fn set_project_remote_id(&self, id: String) -> AppResult<()> {
        {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            op.file.value.id = Some(id);
            op.file.dirty = true;
        }
        self.inner.schedule();
        Ok(())
    }

    /// A handle that does not keep the state alive (for background tasks).
    pub(crate) fn downgrade(&self) -> WeakState {
        WeakState(Arc::downgrade(&self.inner))
    }

    /// Sources of the open project (see `core::source_live` for the operations).
    pub fn sources(&self) -> &SourceStore {
        &self.inner.sources
    }

    /// Wakes the content flusher: a screen or source changed in memory only and is marked dirty.
    pub(crate) fn schedule_content_flush(&self) {
        self.inner.schedule_content();
    }

    /// Changes the content flusher's delay (tests use short or long ones). Applies from the next
    /// change on.
    pub fn set_content_flush_delay(&self, delay: Duration) {
        let ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.inner.content_delay_ms.store(ms, Ordering::Relaxed);
    }

    /// Number of screen and source files written for in-memory changes (for tests).
    pub fn content_write_count(&self) -> u64 {
        self.inner.content_writes.load(Ordering::Relaxed)
    }

    /// Writes every dirty store synchronously. Called on exit and before switching projects;
    /// Wave 2 calls it before snapshotting or copying a project.
    pub fn flush_all(&self) {
        self.inner.flush_all();
    }

    pub fn events(&self) -> &Arc<dyn EventSink> {
        &self.inner.events
    }

    pub fn config_dir(&self) -> &Path {
        &self.inner.config_dir
    }

    /// Live (on-air) state, shared with the output server. Empty while no project is open.
    pub fn live(&self) -> &Arc<LiveHub> {
        &self.inner.live
    }

    /// Templates of the open project (callers pass `project_dir()`).
    pub fn templates(&self) -> &TemplateStore {
        &self.inner.templates
    }

    /// Screens of the open project (see `core::screen_live` for the operations).
    pub fn screens(&self) -> &ScreenStore {
        &self.inner.screens
    }

    /// Whether the project folder watcher is running (for tests and diagnostics).
    pub fn watcher_running(&self) -> bool {
        lock(&self.inner.watcher).is_some()
    }

    /// Batches the open project's watcher has evaluated (0 without one); see
    /// [`ProjectWatcher::batches`].
    #[cfg(test)]
    pub(crate) fn watcher_batches(&self) -> u64 {
        lock(&self.inner.watcher)
            .as_ref()
            .map_or(0, ProjectWatcher::batches)
    }

    /// Adds an observer told about project changes (the output server, OSC; any number, told in
    /// the order they were added) and immediately reports the current project to it. It must not
    /// block or call back into `AppState` locks it could hold.
    pub fn add_project_observer(&self, observer: Arc<dyn ProjectObserver>) {
        write(&self.inner.observers).push(observer.clone());
        let project = self.current_project();
        observer.project_changed(project.as_ref());
    }

    /// Adds a listener for on-air changes and flow runs (OSC feedback, MIDI LEDs); see
    /// [`StateChange`]. Any number, told in the order they were added.
    pub fn add_change_listener(&self, listener: ChangeListener) {
        write(&self.inner.change_listeners).push(listener);
    }

    /// Tells every change listener. Costs one read lock when there is none.
    pub(crate) fn notify_change(&self, change: StateChange<'_>) {
        let listeners = {
            let g = read(&self.inner.change_listeners);
            if g.is_empty() {
                return;
            }
            g.clone()
        };
        for l in listeners {
            l(self, change);
        }
    }

    /// Tells every observer about the current project. Called after a change, with no lock held.
    /// Also emits `project-changed` (cross-window).
    fn notify_project(&self) {
        let project = self.current_project();
        self.emit_project_changed(project.as_ref());
        let observers = read(&self.inner.observers).clone();
        for o in observers {
            o.project_changed(project.as_ref());
        }
        self.outputs_request();
    }

    fn emit_project_changed(&self, project: Option<&Project>) {
        match serde_json::to_value(project) {
            Ok(v) => self.events().emit(names::PROJECT_CHANGED, v),
            Err(e) => tracing::warn!("project-changed not sent: {e}"),
        }
    }

    /// Emits `ui-state-changed` (cross-window) with the open project's UI state.
    pub(crate) fn emit_ui_state_changed(&self) {
        let Ok(ui) = self.ui_state() else {
            return;
        };
        match serde_json::to_value(&ui) {
            Ok(v) => self.events().emit(names::UI_STATE_CHANGED, v),
            Err(e) => tracing::warn!("ui-state-changed not sent: {e}"),
        }
    }

    /// Set by `cancel_move`, polled by the move worker between files.
    pub fn move_cancel(&self) -> &AtomicBool {
        &self.inner.move_cancel
    }

    /// Number of store files written so far (for tests and diagnostics).
    pub fn write_count(&self) -> u64 {
        self.inner.writes.load(Ordering::Relaxed)
    }

    // ---- Prefs ---------------------------------------------------------------------------

    pub fn prefs(&self) -> Prefs {
        read(&self.inner.prefs).value.clone()
    }

    /// Replaces prefs (clamped) and schedules a write. Returns the stored value. Emits
    /// `prefs-changed` (cross-window) when anything changed.
    pub fn set_prefs(&self, prefs: Prefs) -> Prefs {
        let prefs = clamp_prefs(prefs);
        let (any_changed, backup_changed, midi_changed, outputs_changed, capi_changed) = {
            let mut g = write(&self.inner.prefs);
            let any = g.value != prefs;
            let changed = g.value.backup != prefs.backup;
            let midi = g.value.midi != prefs.midi;
            let outputs = g.value.outputs != prefs.outputs;
            let capi = g.value.control_api != prefs.control_api;
            g.value = prefs.clone();
            g.dirty = true;
            (any, changed, midi, outputs, capi)
        };
        self.inner.schedule();
        if any_changed {
            match serde_json::to_value(&prefs) {
                Ok(v) => self.events().emit(names::PREFS_CHANGED, v),
                Err(e) => tracing::warn!("prefs-changed not sent: {e}"),
            }
        }
        if backup_changed {
            self.inner.reconfigure_backup(&prefs.backup);
        }
        if midi_changed {
            // Inputs switched on / off and LED outputs apply at once.
            self.midi_reconcile();
        }
        if outputs_changed {
            // Windows follow at once (on the blocking pool: this may run on the main thread).
            self.outputs_request();
        }
        if capi_changed {
            self.control_api_prefs_changed(prefs.control_api.enabled);
        }
        prefs
    }

    // ---- Recents -------------------------------------------------------------------------

    /// Recent projects, most recent first, with `missing: false`.
    pub fn recents(&self) -> Vec<RecentProject> {
        read(&self.inner.recents).value.clone()
    }

    /// Removes `path` (compared normalized) and returns the updated list.
    pub fn remove_recent(&self, path: &str) -> Vec<RecentProject> {
        let list = {
            let mut g = write(&self.inner.recents);
            if recents::remove(&mut g.value, path) {
                g.dirty = true;
            }
            g.value.clone()
        };
        self.inner.schedule();
        list
    }

    pub fn clear_recents(&self) {
        {
            let mut g = write(&self.inner.recents);
            g.value.clear();
            g.dirty = true;
        }
        self.inner.schedule();
    }

    // ---- Project -------------------------------------------------------------------------

    pub fn current_project(&self) -> Option<Project> {
        read(&self.inner.project)
            .as_ref()
            .map(OpenProject::to_project)
    }

    /// Directory of the open project, or `NoProject`.
    pub fn project_dir(&self) -> AppResult<PathBuf> {
        read(&self.inner.project)
            .as_ref()
            .map(|p| p.dir.clone())
            .ok_or(AppError::NoProject)
    }

    /// Opens the project at `path`. The current project is flushed first; it stays open if the
    /// new one fails to open, and is closed once the new one has loaded. Promotes the project in
    /// recents. Touches the disk: call from a blocking context.
    pub fn open_project(&self, path: &Path) -> AppResult<Project> {
        self.ensure_not_moving()?;
        let project = {
            let _io = lock(&self.inner.io);
            self.inner.flush_locked();
            let dir = fsx::absolute(path);
            let file = self.inner.open_project_file(&dir)?;
            // Schema-1 templates → variants, before anything loads them (after a `pre-migrate`
            // backup, shared with the flows migration: one per open).
            let mut backed_up = false;
            self.templates_migrate_locked(&dir, &mut backed_up);
            self.templates_rewrite_fonts_locked(&dir);
            let ui = self.inner.load_ui_state(&dir);
            let project = self.inner.install_locked(dir.clone(), file, ui);
            // Rules and macros → flows (once; after a `pre-migrate` backup).
            self.flows_migrate_locked(&dir, &mut backed_up);
            project
        };
        self.flows_project_opened();
        self.rules_project_opened();
        self.notify_project();
        self.emit_ui_state_changed();
        self.controls_announce();
        Ok(project)
    }

    /// Creates `<parent>/<slug(name)>`, initializes and opens it.
    pub fn create_project(&self, parent: &Path, name: &str) -> AppResult<Project> {
        self.ensure_not_moving()?;
        let (dir, _) = project::create(parent, name)?;
        self.open_project(&dir)
    }

    /// Initializes an existing folder (name defaults to the folder name) and opens it.
    pub fn init_project(&self, path: &Path, name: Option<&str>) -> AppResult<Project> {
        self.ensure_not_moving()?;
        let dir = fsx::absolute(path);
        let name = match name {
            Some(n) => n.to_string(),
            None => dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Project".into()),
        };
        project::init_in(&dir, &name)?;
        self.open_project(&dir)
    }

    /// Moves schema-1 template folders to the variants layout (`templates::migrate_v1`) after
    /// the `pre-migrate` backup. Caller holds `io`. Failures are reported; such a template
    /// keeps reading as before.
    fn templates_migrate_locked(&self, dir: &Path, backed_up: &mut bool) {
        if templates::needing_migration(dir).is_empty() {
            return;
        }
        if !self.pre_migrate_backup(dir, backed_up, "Templates were not converted to variants") {
            return;
        }
        for (id, e) in self.inner.templates.migrate_all(dir) {
            tracing::error!("migrating template {id} failed: {e}");
            self.events().emit(
                names::BACKGROUND_ERROR,
                serde_json::json!({
                    "message": format!("Template {id} was not converted to variants: {e}")
                }),
            );
        }
    }

    /// Rewrites retired font names in the templates (`templates::rewrite_retired_fonts`; screens
    /// follow when they load). Caller holds `io`. Failures are logged: such a template still
    /// renders the replacement font.
    fn templates_rewrite_fonts_locked(&self, dir: &Path) {
        for (id, e) in self.inner.templates.rewrite_retired_fonts(dir) {
            tracing::warn!("template {id}: retired font names not rewritten: {e}");
        }
    }

    /// Flushes and closes the open project (no-op when none is open).
    pub fn close_project(&self) {
        {
            let _io = lock(&self.inner.io);
            self.inner.flush_locked();
            self.inner.close_locked();
            self.inner.flows.forget();
        }
        self.notify_project();
        self.controls_announce();
    }

    /// Re-reads the open project's `project.json` and `ui-state.json` from disk, DISCARDING any
    /// unsaved in-memory changes. For Wave 2 after files were replaced on disk (restore, import).
    /// Does not touch recents and does not fire the open/close hooks.
    pub fn reload_project(&self) -> AppResult<Project> {
        let project = {
            let _io = lock(&self.inner.io);
            let mut backed_up = false;
            if let Ok(dir) = self.project_dir() {
                self.templates_migrate_locked(&dir, &mut backed_up);
                self.templates_rewrite_fonts_locked(&dir);
            }
            let project = self.inner.reload_locked()?;
            if let Ok(dir) = self.project_dir() {
                self.flows_migrate_locked(&dir, &mut backed_up);
            }
            project
        };
        self.flows_project_opened();
        self.rules_project_opened();
        // Settings (port, LAN) may have changed on disk.
        self.notify_project();
        self.emit_ui_state_changed();
        self.controls_announce();
        Ok(project)
    }

    /// Validates and replaces the open project's settings; schedules a `project.json` write. A
    /// changed brand kit (normalized here) re-applies every screen.
    pub fn set_project_settings(&self, mut settings: ProjectSettings) -> AppResult<Project> {
        validate_project_settings(&settings)?;
        settings.brand = crate::core::styles::normalize_brand(settings.brand)?;
        // OSC addresses and the feedback host are stored trimmed.
        if let Some(osc) = &mut settings.osc {
            for a in &mut osc.allow {
                *a = a.trim().to_string();
            }
            if let Some(f) = &mut osc.feedback {
                f.host = f.host.trim().to_string();
            }
        }
        let brand = settings.brand.clone();
        let project = {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            op.file.value.settings = settings;
            op.file.dirty = true;
            op.to_project()
        };
        self.inner.schedule();
        self.apply_brand(&brand);
        self.notify_project();
        Ok(project)
    }

    /// Changes the open project's settings in place (style library: `style`, `brand`); schedules
    /// a `project.json` write and emits `project-changed`. Nothing when `f` changes nothing.
    pub(crate) fn update_project_settings(
        &self,
        f: impl FnOnce(&mut ProjectSettings),
    ) -> AppResult<Project> {
        let (project, changed) = {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            let mut s = op.file.value.settings.clone();
            f(&mut s);
            let changed = s != op.file.value.settings;
            if changed {
                op.file.value.settings = s;
                op.file.dirty = true;
            }
            (op.to_project(), changed)
        };
        if changed {
            self.inner.schedule();
            self.emit_project_changed(Some(&project));
        }
        Ok(project)
    }

    /// The open project's UI state, or `NoProject`.
    pub fn ui_state(&self) -> AppResult<UiState> {
        read(&self.inner.project)
            .as_ref()
            .map(|p| p.ui.value.clone())
            .ok_or(AppError::NoProject)
    }

    /// Replaces the open project's UI state (clamped); schedules a write. Returns the stored value.
    /// The Studio settings and Preview cues (`control.studio`, `control.studioPreview`) are kept
    /// as stored: they change only through `set_studio` and the studio ops.
    pub fn set_ui_state(&self, ui: UiState) -> AppResult<UiState> {
        let mut ui = clamp_ui_state(ui);
        let (changed, off_changed) = {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            let was = op.ui.value.control.automation_off;
            ui.control.studio = op.ui.value.control.studio;
            ui.control.studio_preview = op.ui.value.control.studio_preview.clone();
            let changed = op.ui.value != ui;
            op.ui.value = ui.clone();
            op.ui.dirty = true;
            (changed, was != ui.control.automation_off)
        };
        self.inner.schedule();
        if changed {
            match serde_json::to_value(&ui) {
                Ok(v) => self.events().emit(names::UI_STATE_CHANGED, v),
                Err(e) => tracing::warn!("ui-state-changed not sent: {e}"),
            }
        }
        // Automation off goes through the UI state (no separate command): tell the engine.
        if off_changed {
            self.engine_set_automation_off(ui.control.automation_off);
            self.control_api_touch();
        }
        Ok(ui)
    }

    /// Changes the open project's UI state in place (server-owned parts such as the Studio
    /// settings); schedules a write only when `f` changed something.
    pub(crate) fn update_ui_state(&self, f: impl FnOnce(&mut UiState)) -> AppResult<()> {
        let changed = {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            let before = op.ui.value.clone();
            f(&mut op.ui.value);
            let changed = op.ui.value != before;
            op.ui.dirty |= changed;
            changed
        };
        if changed {
            self.inner.schedule();
        }
        Ok(())
    }

    /// For the move feature: points the open project at `new_dir` (which must exist and should
    /// already contain the copied project). The current state is flushed to the old location
    /// first, then the in-memory `project.json` / `ui-state.json` are written to `new_dir`, and
    /// the old path's recents entry is replaced by the new path at the front.
    ///
    /// `from` is the directory that was copied. If the open project is no longer that one, nothing
    /// changes and `Busy` is returned, so a copy is never attached to a different project.
    pub fn relocate_project(&self, from: &Path, new_dir: PathBuf) -> AppResult<Project> {
        let project = self.relocate_locked(from, new_dir)?;
        self.notify_project();
        Ok(project)
    }

    fn relocate_locked(&self, from: &Path, new_dir: PathBuf) -> AppResult<Project> {
        let _io = lock(&self.inner.io);
        self.inner.flush_locked();
        let new_dir = fsx::absolute(&new_dir);
        if !new_dir.is_dir() {
            return Err(AppError::NotFound(fsx::display_path(&new_dir)));
        }
        let result = self.relocate_dir_locked(from, new_dir);
        // Watches whichever folder is open now (the new one, or the old one if this failed).
        self.inner.start_watcher();
        result
    }

    /// Caller holds `io` and has flushed.
    fn relocate_dir_locked(&self, from: &Path, new_dir: PathBuf) -> AppResult<Project> {
        self.inner.stop_watcher();
        let (old_dir, project) = {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            if op.dir != fsx::absolute(from) {
                return Err(AppError::Busy(
                    "The open project changed during the move. Try again.".into(),
                ));
            }
            let old = std::mem::replace(&mut op.dir, new_dir.clone());
            op.file.dirty = true;
            op.ui.dirty = true;
            op.controls.dirty = true;
            (old, op.to_project())
        };
        self.inner.flush_locked();
        self.inner.templates.forget();
        {
            let _w = self.inner.screens.write_lock();
            self.inner.screens.relocate(&new_dir);
            self.inner.sources.relocate(&new_dir);
            self.inner.rules.relocate(&new_dir);
            self.inner.flows.relocate(&new_dir);
        }
        {
            let mut g = write(&self.inner.recents);
            recents::remove(&mut g.value, &fsx::display_path(&old_dir));
            recents::promote(
                &mut g.value,
                &project.path,
                &project.name,
                &fsx::now_rfc3339(),
            );
            g.dirty = true;
        }
        self.inner.schedule();
        Ok(project)
    }

    // ---- Controls (controls.json) ----------------------------------------------------------

    /// The open project's MIDI mappings (a cheap `Arc` clone), or `NoProject`.
    pub fn controls_doc(&self) -> AppResult<Arc<ControlsDoc>> {
        read(&self.inner.project)
            .as_ref()
            .map(|p| p.controls.value.clone())
            .ok_or(AppError::NoProject)
    }

    /// Replaces the open project's mappings in memory (already validated) and schedules a
    /// coalesced write when they changed (`write` false: a watcher reload, already on disk).
    /// Returns whether anything changed.
    pub(crate) fn replace_controls(
        &self,
        doc: Arc<ControlsDoc>,
        write_file: bool,
    ) -> AppResult<bool> {
        let changed = {
            let mut g = write(&self.inner.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            if *op.controls.value == *doc {
                false
            } else {
                op.controls.value = doc;
                op.controls.dirty |= write_file;
                true
            }
        };
        if changed && write_file {
            self.inner.schedule();
        }
        Ok(changed)
    }

    /// The MIDI runtime (see `core::midi`).
    pub(crate) fn displays_rt(&self) -> &crate::core::displays::Displays {
        &self.inner.displays
    }

    pub(crate) fn outputs_rt(&self) -> &crate::core::outputs::Outputs {
        &self.inner.outputs
    }

    /// Whether prefs configure any output (cheap: no clone).
    pub(crate) fn has_outputs(&self) -> bool {
        !read(&self.inner.prefs).value.outputs.is_empty()
    }

    pub(crate) fn midi_rt(&self) -> &crate::core::midi::Midi {
        &self.inner.midi
    }

    /// The control runtime (listeners, value pacing; see `core::controls`).
    pub(crate) fn control_runtime(&self) -> &crate::core::controls::ControlRuntime {
        &self.inner.control_rt
    }

    // ---- Backups -------------------------------------------------------------------------

    /// Takes `io`, flushes every dirty store, then runs `f` with the open project's directory.
    /// No store write can race with `f`. `f` must not call back into anything that takes `io`
    /// (`flush_all`, `open_project`, ...).
    pub(crate) fn with_flushed_io<T>(&self, f: impl FnOnce(&Path) -> AppResult<T>) -> AppResult<T> {
        let _io = lock(&self.inner.io);
        self.inner.flush_locked();
        let dir = self.project_dir()?;
        f(&dir)
    }

    fn retention(&self) -> usize {
        read(&self.inner.prefs).value.backup.retention.max(1) as usize
    }

    fn emit_backup_created(&self, info: &BackupInfo) {
        match serde_json::to_value(info) {
            Ok(v) => self.inner.events.emit(names::BACKUP_CREATED, v),
            Err(e) => tracing::warn!("backup-created payload: {e}"),
        }
    }

    /// Flushes and snapshots the open project (always written, changed or not), prunes to the
    /// retention setting and emits `backup-created`. Touches the disk.
    pub fn create_snapshot(&self, reason: BackupReason) -> AppResult<BackupInfo> {
        let keep = self.retention();
        let info = self.with_flushed_io(|dir| {
            let info = backup::snapshot(dir, reason)?;
            if let Err(e) = backup::prune(dir, keep) {
                tracing::warn!("pruning backups failed: {e}");
            }
            Ok(info)
        })?;
        self.emit_backup_created(&info);
        Ok(info)
    }

    /// Snapshots of the open project, newest first.
    pub fn list_backups(&self) -> AppResult<Vec<BackupInfo>> {
        let dir = self.project_dir()?;
        Ok(backup::list(&dir)?)
    }

    pub fn delete_backup(&self, id: &str) -> AppResult<()> {
        let _io = lock(&self.inner.io);
        let dir = self.project_dir()?;
        Ok(backup::delete(&dir, id)?)
    }

    /// Takes a `pre-restore` snapshot, makes the config set match snapshot `id` and reloads the
    /// project from disk, all under `io` so no pending store write can land in between.
    /// Prunes only afterwards, so the snapshot being restored is never pruned away first.
    pub fn restore_backup(&self, id: &str) -> AppResult<Project> {
        self.ensure_not_moving()?;
        let keep = self.retention();
        let (pre, project) = {
            let _io = lock(&self.inner.io);
            self.inner.flush_locked();
            let dir = self.project_dir()?;
            backup::backup_path(&dir, id)?;
            let pre = backup::snapshot(&dir, BackupReason::PreRestore)?;
            // The restore rewrites the project's files; they are reloaded explicitly below, so
            // the watcher sits it out.
            self.inner.stop_watcher();
            let restored = backup::restore(&dir, id).map_err(AppError::from);
            // An older snapshot may hold schema-1 templates.
            let mut backed_up = false;
            if restored.is_ok() {
                self.templates_migrate_locked(&dir, &mut backed_up);
            }
            let project = restored.and_then(|()| self.inner.reload_locked());
            self.inner.start_watcher();
            let project = project?;
            // An older snapshot may still hold rules and macros.
            self.flows_migrate_locked(&dir, &mut backed_up);
            if let Err(e) = backup::prune(&dir, keep) {
                tracing::warn!("pruning backups failed: {e}");
            }
            (pre, project)
        };
        // Flows and recipes may differ in the restored set.
        self.flows_project_opened();
        self.rules_project_opened();
        // The restored project.json may carry a different port or LAN setting.
        self.notify_project();
        self.emit_ui_state_changed();
        self.controls_announce();
        self.emit_backup_created(&pre);
        Ok(project)
    }

    /// Stops the open project's backup timer (no-op when none runs).
    pub(crate) fn stop_backup_worker(&self) {
        self.inner.stop_backup_worker();
    }

    /// (Re)starts the backup timer for the open project if backups are enabled. With `initial`,
    /// the worker first takes that snapshot (skipped when unchanged).
    pub(crate) fn start_backup_worker(&self, initial: Option<BackupReason>) {
        self.inner.start_backup_worker(initial);
    }

    #[cfg(test)]
    pub(crate) fn backup_worker_running(&self) -> bool {
        lock(&self.inner.backup_worker).is_some()
    }

    // ---- Move ----------------------------------------------------------------------------

    /// Marks a move as running and clears the cancel flag. `Busy` if one is already running.
    /// The returned guard clears the mark when dropped.
    pub(crate) fn begin_move(&self) -> AppResult<MoveGuard<'_>> {
        self.inner
            .moving
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| AppError::Busy("A project move is already running".into()))?;
        self.inner.move_cancel.store(false, Ordering::SeqCst);
        Ok(MoveGuard(&self.inner.moving))
    }

    /// `Busy` while a project move is running. Anything that switches, closes or rewrites the
    /// open project calls this first, so the move can never copy one project and attach another.
    pub fn ensure_not_moving(&self) -> AppResult<()> {
        if self.inner.moving.load(Ordering::SeqCst) {
            return Err(AppError::Busy("Wait for the project move to finish".into()));
        }
        Ok(())
    }

    /// Asks the running move (if any) to stop at the next file or chunk boundary.
    pub fn cancel_move(&self) {
        if self.inner.moving.load(Ordering::SeqCst) {
            self.inner.move_cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// A weak [`AppState`]: background tasks hold this so they never keep the state alive.
#[derive(Clone)]
pub(crate) struct WeakState(Weak<Inner>);

impl WeakState {
    pub(crate) fn upgrade(&self) -> Option<AppState> {
        self.0.upgrade().map(|inner| AppState { inner })
    }
}

/// Clears the "move running" mark on drop.
pub(crate) struct MoveGuard<'a>(&'a AtomicBool);

impl Drop for MoveGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Loads an app-config or project store, never failing: corrupt files are set aside (and
/// `store-recovered` emitted); newer or unreadable files make the store read-only on defaults.
fn load_tracked<S: Store + Clone>(dir: &Path, events: &dyn EventSink) -> Tracked<S> {
    match persist::load_or_recover::<S>(dir) {
        Ok((value, recovered)) => {
            if let Some(r) = recovered {
                events.emit(names::STORE_RECOVERED, r.payload());
            }
            Tracked::clean(value)
        }
        Err(e) => {
            tracing::warn!(
                "{}: {e}; using defaults for this session and not saving it",
                persist::path_of::<S>(dir).display()
            );
            Tracked {
                value: S::default(),
                dirty: false,
                read_only: true,
            }
        }
    }
}

impl Inner {
    /// Wakes the flusher. A send error means the flusher is gone, which only happens while the
    /// state is being dropped.
    fn schedule(&self) {
        let _ = self.notify.send(());
    }

    fn content_delay(&self) -> Duration {
        Duration::from_millis(self.content_delay_ms.load(Ordering::Relaxed))
    }

    fn flush_all(&self) {
        let _io = lock(&self.io);
        self.flush_locked();
    }

    /// Wakes the content flusher (a screen or source changed in memory only).
    fn schedule_content(&self) {
        let _ = self.content_notify.send(());
    }

    /// The store flusher's pass: the app and project stores only (screens and sources are the
    /// content flusher's, so their write rate stays bounded by its delay).
    fn flush_stores(&self) {
        let _io = lock(&self.io);
        self.flush_stores_locked();
    }

    /// The content flusher's pass: dirty screens and sources.
    fn flush_content(&self) {
        let _io = lock(&self.io);
        self.flush_content_locked();
    }

    /// Writes the screens and sources changed in memory only. Caller holds `io`.
    fn flush_content_locked(&self) {
        let n = screen_live::flush_dirty(&self.screens, &self.sources, &*self.events)
            + recipes::flush_dirty(&self.rules)
            + crate::core::flows::flush_dirty(&self.flows);
        self.content_writes.fetch_add(n, Ordering::Relaxed);
    }

    /// Writes everything dirty: content and stores. Caller holds `io`.
    fn flush_locked(&self) {
        self.flush_content_locked();
        self.flush_stores_locked();
    }

    /// Writes every dirty store once. Caller holds `io`.
    fn flush_stores_locked(&self) {
        let prefs = write(&self.prefs).take_dirty();
        if let Some(p) = prefs {
            if !self.save(&self.config_dir, &p) {
                write(&self.prefs).dirty = true;
            }
        }

        let list = write(&self.recents).take_dirty();
        if let Some(projects) = list {
            if !self.save(&self.config_dir, &RecentsFile { projects }) {
                write(&self.recents).dirty = true;
            }
        }

        let snapshot = write(&self.project).as_mut().map(|op| {
            (
                op.dir.clone(),
                op.file.take_dirty(),
                op.ui.take_dirty(),
                op.controls.take_dirty(),
            )
        });
        if let Some((dir, file, ui, controls)) = snapshot {
            let file_failed = file.is_some_and(|f| !self.save(&dir, &f));
            let ui_failed = ui.is_some_and(|u| !self.save(&dir, &u));
            let controls_failed = controls.is_some_and(|c| !self.save(&dir, &*c));
            if file_failed || ui_failed || controls_failed {
                if let Some(op) = write(&self.project).as_mut().filter(|op| op.dir == dir) {
                    op.file.dirty |= file_failed;
                    op.ui.dirty |= ui_failed;
                    op.controls.dirty |= controls_failed;
                }
            }
        }
    }

    /// Writes one store; logs and reports failures. Returns success.
    fn save<S: Store>(&self, dir: &Path, value: &S) -> bool {
        match persist::save(dir, value) {
            Ok(()) => {
                self.writes.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(e) => {
                tracing::error!("save failed: {e}");
                self.events.emit(
                    names::BACKGROUND_ERROR,
                    serde_json::json!({ "message": format!("Could not save {}: {e}", S::FILE) }),
                );
                false
            }
        }
    }

    /// `project::open`, with retired font names in the brand kit rewritten (written at once).
    fn open_project_file(&self, dir: &Path) -> AppResult<ProjectFile> {
        let mut file = project::open(dir)?;
        if crate::core::styles::normalize_brand_fonts(&mut file.settings.brand) {
            self.save(dir, &file);
        }
        Ok(file)
    }

    /// `controls.json`, sanitized (invalid mappings dropped and logged; see `core::controls`).
    fn load_controls(&self, dir: &Path) -> Tracked<Arc<ControlsDoc>> {
        let t = load_tracked::<ControlsDoc>(dir, &*self.events);
        Tracked {
            value: Arc::new(crate::core::controls::sanitize(t.value)),
            dirty: false,
            read_only: t.read_only,
        }
    }

    fn load_ui_state(&self, dir: &Path) -> Tracked<UiState> {
        let mut ui = load_tracked::<UiState>(dir, &*self.events);
        ui.value = clamp_ui_state(ui.value);
        ui
    }

    /// Closes the current project (if any) and installs a freshly loaded one. Caller holds `io`
    /// and has flushed.
    fn install_locked(&self, dir: PathBuf, file: ProjectFile, ui: Tracked<UiState>) -> Project {
        let brand = file.settings.brand.clone();
        self.close_locked();
        let controls = self.load_controls(&dir);
        let op = OpenProject {
            dir,
            file: Tracked::clean(file),
            ui,
            controls,
        };
        let project = op.to_project();
        let dir = op.dir.clone();
        let studio = op.ui.value.control.studio.unwrap_or_default().enabled;
        let cues = op.ui.value.control.studio_preview.clone();
        *write(&self.project) = Some(op);
        self.templates.forget();
        // Every screen in `screens/` goes live, off air, next to the reserved screens.
        let mut built = screen_live::build_project_live(
            &dir,
            &*self.events,
            &self.sources.clock().now(),
            &brand,
        );
        // Studio mode and its Preview cues as they were (everything starts off air).
        built.live.studio = studio;
        if studio {
            built.live.restore_cues(&cues);
        }
        {
            let _w = self.screens.write_lock();
            self.sources.install(&dir, built.sources);
            self.screens
                .install(&dir, built.docs, built.templates, brand);
            self.live.reset(built.live);
        }
        self.start_watcher();
        {
            let mut g = write(&self.recents);
            recents::promote(
                &mut g.value,
                &project.path,
                &project.name,
                &fsx::now_rfc3339(),
            );
            g.dirty = true;
        }
        self.schedule();
        // The worker takes the `open` snapshot on its own thread once `io` is released.
        self.start_backup_worker(Some(BackupReason::Open));
        project
    }

    /// Re-reads the open project's files, discarding in-memory changes. Caller holds `io`.
    fn reload_locked(&self) -> AppResult<Project> {
        let dir = read(&self.project)
            .as_ref()
            .map(|p| p.dir.clone())
            .ok_or(AppError::NoProject)?;
        let file = self.open_project_file(&dir)?;
        let ui = self.load_ui_state(&dir);
        let controls = self.load_controls(&dir);
        // Files may have been replaced on disk (restore, import).
        self.templates.forget();
        let mode = ui.value.control.live_edit;
        let studio = ui.value.control.studio.unwrap_or_default().enabled;
        let cues = ui.value.control.studio_preview.clone();
        let brand = file.settings.brand.clone();
        let project = {
            let mut g = write(&self.project);
            let op = g.as_mut().ok_or(AppError::NoProject)?;
            op.file = Tracked::clean(file);
            op.ui = ui;
            op.controls = controls;
            op.to_project()
        };
        // Running automation stops; flows and recipes are read again by the caller. API polling
        // restarts from the reloaded sources (the live reset below reconciles).
        self.automation.stop_all();
        self.api.stop_all();
        self.polls.stop_all();
        self.flow_engine.forget();
        self.rules.stop_all();
        // Studio mode and the Preview cues as the reloaded ui-state says.
        self.live.restore_studio(studio, &cues);
        screen_live::resync(
            &self.screens,
            &self.sources,
            &self.live,
            &dir,
            &*self.events,
            mode,
            &brand,
        );
        Ok(project)
    }

    /// Starts watching the open project's folder (replacing any running watcher; none without a
    /// project). A failure is logged: the app works without it, external edits just aren't seen.
    fn start_watcher(&self) {
        self.stop_watcher();
        let Some(dir) = read(&self.project).as_ref().map(|op| op.dir.clone()) else {
            return;
        };
        let weak = self.me.clone();
        let watched = dir.clone();
        let handler = move |batch| {
            if let Some(inner) = weak.upgrade() {
                AppState { inner }.handle_file_changes(&watched, batch);
            }
        };
        match ProjectWatcher::start(&dir, Box::new(handler)) {
            Ok(w) => *lock(&self.watcher) = Some(w),
            Err(e) => tracing::warn!("cannot watch {} for changes: {e}", dir.display()),
        }
    }

    /// Stops the watcher and waits for its thread. Never call with a lock its handler takes (the
    /// screen write lock, `project`).
    fn stop_watcher(&self) {
        let watcher = lock(&self.watcher).take();
        drop(watcher);
    }

    fn stop_backup_worker(&self) {
        let worker = lock(&self.backup_worker).take();
        drop(worker);
    }

    /// Replaces any running backup worker with one for the open project (none when no project
    /// is open or backups are disabled). Cheap: spawns a thread, does no I/O here.
    fn start_backup_worker(&self, initial: Option<BackupReason>) {
        let prefs = read(&self.prefs).value.backup.clone();
        let dir = read(&self.project).as_ref().map(|op| op.dir.clone());
        let mut slot = lock(&self.backup_worker);
        *slot = None;
        let Some(dir) = dir else { return };
        if !prefs.enabled {
            return;
        }
        let weak = self.me.clone();
        let worker_dir = dir.clone();
        // The worker runs each check with `io` held after a flush, and only while its project is
        // still the open one; otherwise it exits.
        let host: backup::Host = Box::new(move |f: &mut dyn FnMut()| {
            let Some(inner) = weak.upgrade() else {
                return false;
            };
            let _io = lock(&inner.io);
            let current = read(&inner.project)
                .as_ref()
                .is_some_and(|op| op.dir == worker_dir);
            if !current {
                return false;
            }
            inner.flush_locked();
            f();
            true
        });
        let cfg = backup::WorkerConfig {
            dir,
            prefs,
            unit: backup::INTERVAL_UNIT,
            initial,
        };
        match backup::Worker::spawn(cfg, self.events.clone(), host) {
            Ok(w) => *slot = Some(w),
            Err(e) => tracing::error!("could not start the backup timer: {e}"),
        }
    }

    /// Applies changed backup prefs to the running worker: new interval/retention are sent to
    /// it, disabling stops it, enabling starts one (no immediate snapshot).
    fn reconfigure_backup(&self, p: &BackupPrefs) {
        let mut slot = lock(&self.backup_worker);
        match (slot.is_some(), p.enabled) {
            (true, true) => {
                if let Some(w) = slot.as_ref() {
                    w.reconfigure(p.clone());
                }
            }
            (true, false) => *slot = None,
            (false, true) => {
                drop(slot);
                self.start_backup_worker(None);
            }
            (false, false) => {}
        }
    }

    /// Drops the open project. Caller holds `io` and has flushed.
    fn close_locked(&self) {
        let closing = write(&self.project).take();
        if let Some(op) = closing {
            // Joined: no file event of this project is handled after this.
            self.stop_watcher();
            // Signals the timer; it exits on its own without being joined.
            self.stop_backup_worker();
            // No auto-advance, flow instance or planner task outlives its project.
            self.automation.stop_all();
            self.automation.forget();
            self.api.stop_all();
            self.polls.stop_all();
            self.flow_engine.forget();
            self.rules.forget();
            {
                let _w = self.screens.write_lock();
                self.screens.forget();
                self.sources.forget();
                self.live.reset(LiveState::default());
            }
            self.templates.forget();
            tracing::info!("closed project {}", op.dir.display());
        }
    }
}

/// Body of the flusher thread. Blocks until a change arrives, waits for `debounce` of quiet
/// (bounded by `10 × debounce`), then writes everything dirty. Exits when every `AppState` is
/// dropped (the channel disconnects).
fn flusher_loop(state: Weak<Inner>, rx: Receiver<()>, debounce: Duration) {
    let max_wait = debounce.saturating_mul(10);
    while rx.recv().is_ok() {
        let deadline = Instant::now() + max_wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(debounce.min(left)) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        match state.upgrade() {
            Some(inner) => inner.flush_stores(),
            None => return,
        }
    }
}

/// Body of the content flusher thread: screens and sources changed in memory only (Control field
/// edits, source steps). Blocks until a change arrives, then writes once the delay has passed since
/// that first change (later changes in the window ride along). So each file is written at most
/// once per delay, however fast the changes come. Exits when every `AppState` is dropped.
fn content_flusher_loop(state: Weak<Inner>, rx: Receiver<()>) {
    while rx.recv().is_ok() {
        let Some(delay) = state.upgrade().map(|i| i.content_delay()) else {
            return;
        };
        let deadline = Instant::now() + delay;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        match state.upgrade() {
            Some(inner) => inner.flush_content(),
            None => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{ControlViewState, ScreenEditorView, Theme, UiMode};
    use serde_json::Value;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<(String, Value)>>);

    impl EventSink for Recorder {
        fn emit(&self, event: &str, payload: Value) {
            self.0.lock().unwrap().push((event.to_string(), payload));
        }
    }

    fn state_in(dir: &Path, debounce: Duration) -> (AppState, Arc<Recorder>) {
        let rec = Arc::new(Recorder::default());
        let st = AppState::load_with_debounce(dir.join("config"), rec.clone(), debounce).unwrap();
        (st, rec)
    }

    fn slow() -> Duration {
        Duration::from_secs(60)
    }

    #[test]
    fn open_sets_current_project() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        assert!(st.current_project().is_none());
        assert!(matches!(st.project_dir(), Err(AppError::NoProject)));
        let p = st.create_project(tmp.path(), "My Show").unwrap();
        assert_eq!(p.name, "My Show");
        assert_eq!(st.current_project(), Some(p.clone()));
        assert_eq!(st.project_dir().unwrap(), tmp.path().join("my-show"));
        assert_eq!(st.recents()[0].path, p.path);
        assert_eq!(st.ui_state().unwrap(), UiState::default());
    }

    #[test]
    fn opening_second_closes_first_and_orders_recents() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let a = st.create_project(tmp.path(), "First").unwrap();
        let ui = UiState {
            sidebar_pinned: true,
            ..Default::default()
        };
        st.set_ui_state(ui.clone()).unwrap();
        let b = st.create_project(tmp.path(), "Second").unwrap();
        assert_eq!(st.current_project().unwrap().path, b.path);
        let paths: Vec<String> = st.recents().into_iter().map(|r| r.path).collect();
        assert_eq!(paths, [b.path.clone(), a.path.clone()]);
        // First project's UI state was flushed on close.
        let saved: UiState = persist::load_strict(Path::new(&a.path)).unwrap();
        assert_eq!(saved, ui);
        // Reopening the first restores it.
        st.open_project(Path::new(&a.path)).unwrap();
        assert_eq!(st.ui_state().unwrap(), ui);
    }

    #[test]
    fn failed_open_keeps_current_project() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let a = st.create_project(tmp.path(), "A").unwrap();
        let err = st.open_project(&tmp.path().join("config")).unwrap_err();
        assert!(matches!(err, AppError::NotAProject(_)));
        assert_eq!(st.current_project().unwrap().path, a.path);
    }

    #[test]
    fn project_settings_validation_and_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let mut s = ProjectSettings::default();
        assert!(matches!(
            st.set_project_settings(s.clone()),
            Err(AppError::NoProject)
        ));
        let p = st.create_project(tmp.path(), "A").unwrap();
        s.port = 80;
        assert!(matches!(
            st.set_project_settings(s.clone()),
            Err(AppError::Invalid(_))
        ));
        s.port = 8080;
        s.default_resolution.width = 9000;
        assert!(matches!(
            st.set_project_settings(s.clone()),
            Err(AppError::Invalid(_))
        ));
        s.default_resolution.width = 1280;
        let updated = st.set_project_settings(s.clone()).unwrap();
        assert_eq!(updated.settings.port, 8080);
        st.close_project();
        assert!(st.current_project().is_none());
        let file: ProjectFile = persist::load_strict(Path::new(&p.path)).unwrap();
        assert_eq!(file.settings, s);
    }

    #[test]
    fn project_switches_refused_while_moving() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let a = st.create_project(tmp.path(), "A").unwrap();
        let b_dir = {
            let b = st.create_project(tmp.path(), "B").unwrap();
            st.open_project(Path::new(&a.path)).unwrap();
            b.path
        };
        let guard = st.begin_move().unwrap();
        assert!(matches!(
            st.open_project(Path::new(&b_dir)),
            Err(AppError::Busy(_))
        ));
        assert!(matches!(
            st.create_project(tmp.path(), "C"),
            Err(AppError::Busy(_))
        ));
        assert!(
            !tmp.path().join("c").exists(),
            "nothing created while moving"
        );
        assert!(matches!(st.ensure_not_moving(), Err(AppError::Busy(_))));
        assert_eq!(st.current_project().unwrap().path, a.path);
        drop(guard);
        st.ensure_not_moving().unwrap();
        st.open_project(Path::new(&b_dir)).unwrap();
    }

    #[test]
    fn relocate_refuses_when_open_project_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let a = st.create_project(tmp.path(), "A").unwrap();
        let b = st.create_project(tmp.path(), "B").unwrap();
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).unwrap();
        // The copy was made from A, but B is open now.
        assert!(matches!(
            st.relocate_project(Path::new(&a.path), dest.clone()),
            Err(AppError::Busy(_))
        ));
        assert_eq!(st.current_project().unwrap().path, b.path);
        st.flush_all();
        assert!(
            !dest.join("project.json").exists(),
            "B must not be written into the copy"
        );
    }

    #[test]
    fn ui_state_clamped() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        assert!(matches!(st.ui_state(), Err(AppError::NoProject)));
        st.create_project(tmp.path(), "A").unwrap();
        let mut ui = UiState::default();
        ui.control.grid_columns = 40;
        assert_eq!(st.set_ui_state(ui).unwrap().control.grid_columns, 6);
    }

    #[test]
    fn screen_editor_view_defaults_and_clamping() {
        // Old files without `screenEditor` (or with only some keys) get the defaults.
        let old: UiState = serde_json::from_str(r#"{"sidebarPinned":true}"#).unwrap();
        assert_eq!(old.screen_editor, ScreenEditorView::default());
        let d = ScreenEditorView::default();
        assert!(d.snap && d.guides && d.custom_safe);
        assert!(!d.grid && !d.action_safe && !d.title_safe && !d.center_cross);
        assert_eq!((d.grid_size, d.crop.as_str()), (16, "none"));
        let partial: UiState =
            serde_json::from_str(r#"{"screenEditor":{"grid":true,"crop":"9:16"}}"#).unwrap();
        assert!(partial.screen_editor.grid && partial.screen_editor.snap);
        assert_eq!(partial.screen_editor.crop, "9:16");
        // camelCase on the wire.
        let v = serde_json::to_value(UiState::default()).unwrap();
        assert_eq!(v["screenEditor"]["gridSize"], 16);
        assert_eq!(v["screenEditor"]["actionSafe"], false);

        for (size, want) in [(0, 2), (1, 2), (2, 2), (16, 16), (512, 512), (9000, 512)] {
            let mut u = UiState::default();
            u.screen_editor.grid_size = size;
            assert_eq!(clamp_ui_state(u).screen_editor.grid_size, want, "{size}");
        }
        for (crop, want) in [
            ("4:5", "4:5"),
            ("1:1", "1:1"),
            ("4:3", "4:3"),
            ("16:9", "none"),
            ("", "none"),
        ] {
            let mut u = UiState::default();
            u.screen_editor.crop = crop.into();
            assert_eq!(clamp_ui_state(u).screen_editor.crop, want, "{crop}");
        }

        // Through AppState: clamped and stored.
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let p = st.create_project(tmp.path(), "A").unwrap();
        let mut ui = UiState::default();
        ui.screen_editor.grid_size = 1;
        ui.screen_editor.title_safe = true;
        let stored = st.set_ui_state(ui).unwrap();
        assert_eq!(stored.screen_editor.grid_size, 2);
        st.flush_all();
        let saved: UiState = persist::load_strict(Path::new(&p.path)).unwrap();
        assert!(saved.screen_editor.title_safe);
        assert_eq!(saved.screen_editor.grid_size, 2);
    }

    #[test]
    fn hotkeys_clamped() {
        for ok in ["F1", "F9", "F12", "Shift+F1", "Shift+F12"] {
            assert!(is_hotkey_combo(ok), "{ok}");
        }
        for bad in [
            "F0",
            "F13",
            "F01",
            "f1",
            "Shift+",
            "shift+F1",
            "Ctrl+F1",
            "Shift+Shift+F1",
            "F",
            "",
            "A",
        ] {
            assert!(!is_hotkey_combo(bad), "{bad}");
        }
        let mut ui = UiState::default();
        let h = &mut ui.control.hotkeys;
        for (k, v) in [
            ("scr1/lay1", "F1"),
            ("scr1/lay2", "Shift+F1"),
            ("scr1/lay3", "F13"),
            ("layer-only", "F2"),
            ("a/b/c", "F3"),
            ("../x/l", "F4"),
            ("scr1/", "F5"),
            // Duplicate combos: the last in key order keeps it.
            ("scr2/a", "F6"),
            ("scr2/b", "F6"),
            ("scr3/z", "F7"),
            // Macros.
            ("macro/m1", "F8"),
            ("macro/a/b", "F9"),
        ] {
            h.insert(k.into(), v.into());
        }
        let got = clamp_ui_state(ui).control.hotkeys;
        let want: std::collections::BTreeMap<String, String> = [
            ("scr1/lay1", "F1"),
            ("scr1/lay2", "Shift+F1"),
            ("scr2/b", "F6"),
            ("scr3/z", "F7"),
            ("macro/m1", "F8"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn key_combos() {
        for ok in [
            "Ctrl+K",
            "Ctrl+Shift+P",
            "Ctrl+Alt+Shift+Z",
            "Alt+Shift+H",
            "Shift+F3",
            "F12",
            "F24",
            "Escape",
            "Shift+Escape",
            "Ctrl+;",
            "Ctrl+'",
            "Ctrl+,",
            "Ctrl+\\",
            "Ctrl+Enter",
            "G",
            "0",
            "ArrowUp",
        ] {
            assert!(is_key_combo(ok), "{ok}");
        }
        for bad in [
            "",
            "Ctrl+",
            "ctrl+K",
            "Ctrl+k",
            "Shift+Ctrl+K",
            "Ctrl+Ctrl+K",
            "Meta+K",
            "F0",
            "F25",
            "F01",
            "Esc",
            "escape",
            "Ctrl+Shift",
            "Ctrl+ä",
            "Ctrl++",
            "Ctrl K",
            "KK",
        ] {
            assert!(!is_key_combo(bad), "{bad}");
        }
    }

    #[test]
    fn prefs_keybindings_clamped() {
        let mut p = Prefs::default();
        let kb = &mut p.keybindings;
        kb.insert(
            "nav.control".into(),
            vec![
                "Ctrl+7".into(),
                "ctrl+7".into(),
                "Ctrl+7".into(),
                "F1".into(),
                "F2".into(),
                "F3".into(),
                "F4".into(),
            ],
        );
        kb.insert("shell.pin".into(), vec![]);
        kb.insert("bad id".into(), vec!["F5".into()]);
        kb.insert(String::new(), vec!["F6".into()]);
        kb.insert("x".repeat(COMMAND_ID_MAX + 1), vec!["F7".into()]);
        let got = clamp_prefs(p).keybindings;
        assert_eq!(got.len(), 2);
        assert_eq!(got["nav.control"], ["Ctrl+7", "F1", "F2", "F3"]);
        assert!(got["shell.pin"].is_empty());

        let mut p = Prefs::default();
        for i in 0..KEYBINDINGS_CAP + 20 {
            p.keybindings
                .insert(format!("cmd.{i:04}"), vec!["F1".into()]);
        }
        let got = clamp_prefs(p).keybindings;
        assert_eq!(got.len(), KEYBINDINGS_CAP);
        assert!(got.contains_key("cmd.0000"));

        // Older files have no keybindings: an empty table.
        let p: Prefs = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(p.keybindings.is_empty());
        let v = serde_json::to_value(Prefs::default()).unwrap();
        assert_eq!(v["keybindings"], serde_json::json!({}));
    }

    #[test]
    fn palette_recent_clamped() {
        let mut ui = UiState::default();
        ui.palette.recent = ["a", "b", "a", "bad id", "", "c", "d", "e", "f", "g"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = clamp_ui_state(ui).palette.recent;
        assert_eq!(got, ["a", "b", "c", "d", "e"]);
        // Older files have no palette section.
        let ui: UiState = serde_json::from_str(r#"{"sidebarPinned":true}"#).unwrap();
        assert!(ui.palette.recent.is_empty());
        let v = serde_json::to_value(UiState::default()).unwrap();
        assert_eq!(v["palette"]["recent"], serde_json::json!([]));
    }

    #[test]
    fn ui_state_clamps_control_panel_width() {
        for (w, want) in [
            (None, None),
            (Some(0), Some(280)),
            (Some(279), Some(280)),
            (Some(280), Some(280)),
            (Some(420), Some(420)),
            (Some(640), Some(640)),
            (Some(641), Some(640)),
            (Some(u32::MAX), Some(640)),
        ] {
            let mut u = UiState::default();
            u.control.panel_width = w;
            assert_eq!(clamp_ui_state(u).control.panel_width, want, "{w:?}");
        }
        // camelCase on the wire, left out when unset; an old file without it reads as unset.
        let mut u = UiState::default();
        assert!(serde_json::to_value(&u).unwrap()["control"]
            .get("panelWidth")
            .is_none());
        u.control.panel_width = Some(360);
        let v = serde_json::to_value(&u).unwrap();
        assert_eq!(v["control"]["panelWidth"], 360);
        let mut c = v["control"].clone();
        c.as_object_mut().unwrap().remove("panelWidth");
        let back: ControlViewState = serde_json::from_value(c).unwrap();
        assert_eq!(back.panel_width, None);
    }

    #[test]
    fn relocate_updates_dir_and_recents() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let other = st.create_project(tmp.path(), "Other").unwrap();
        let a = st.create_project(tmp.path(), "A").unwrap();
        let new_dir = tmp.path().join("moved");
        fs::create_dir(&new_dir).unwrap();
        let old_dir = st.project_dir().unwrap();
        let p = st.relocate_project(&old_dir, new_dir.clone()).unwrap();
        assert_eq!(p.name, "A");
        assert_eq!(st.project_dir().unwrap(), new_dir);
        assert!(new_dir.join("project.json").is_file());
        let paths: Vec<String> = st.recents().into_iter().map(|r| r.path).collect();
        assert_eq!(paths, [p.path.clone(), other.path.clone()]);
        assert!(!paths.contains(&a.path));
    }

    #[test]
    fn ui_mode_simple_for_new_installs_advanced_for_existing() {
        // No prefs.json: a new install.
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        assert_eq!(st.prefs().ui_mode, UiMode::Simple);
        let v = serde_json::to_value(st.prefs()).unwrap();
        assert_eq!(v["uiMode"], "simple", "always serialized");
        assert!(v.get("zoom").is_none(), "zoom omitted at 1");

        // A prefs.json from before the mode existed: an existing install.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(
            cfg.join("prefs.json"),
            r#"{"schemaVersion":1,"theme":"dark"}"#,
        )
        .unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        assert_eq!(st.prefs().ui_mode, UiMode::Advanced);
        assert_eq!(st.prefs().theme, Theme::Dark);

        // A stored mode wins either way.
        fs::write(
            cfg.join("prefs.json"),
            r#"{"schemaVersion":1,"uiMode":"simple","zoom":1.25}"#,
        )
        .unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        assert_eq!(st.prefs().ui_mode, UiMode::Simple);
        assert_eq!(st.prefs().zoom, Some(1.3), "clamped on load");
    }

    #[test]
    fn zoom_clamped_rounded_and_one_is_absent() {
        assert_eq!(clamp_zoom(1.0), None);
        assert_eq!(clamp_zoom(1.04), None);
        assert_eq!(clamp_zoom(1.2), Some(1.2));
        assert_eq!(clamp_zoom(1.1 + 0.1), Some(1.2));
        assert_eq!(clamp_zoom(0.1), Some(0.5));
        assert_eq!(clamp_zoom(9.0), Some(2.0));
        assert_eq!(clamp_zoom(0.74), Some(0.7));
        assert_eq!(clamp_zoom(f64::NAN), None);
        assert_eq!(clamp_zoom(f64::INFINITY), None);
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let p = st.set_prefs(Prefs {
            zoom: Some(1.55),
            ..st.prefs()
        });
        assert_eq!(p.zoom, Some(1.6));
        assert_eq!(serde_json::to_value(&p).unwrap()["zoom"], 1.6);
    }

    #[test]
    fn cross_window_events_follow_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, rec) = state_in(tmp.path(), slow());
        let names_of = |rec: &Recorder| -> Vec<String> {
            rec.0.lock().unwrap().drain(..).map(|(n, _)| n).collect()
        };
        let p = st.set_prefs(Prefs {
            theme: Theme::Light,
            ..st.prefs()
        });
        st.set_prefs(p.clone());
        assert_eq!(names_of(&rec), [names::PREFS_CHANGED], "only on a change");

        st.create_project(tmp.path(), "Show").unwrap();
        let got = names_of(&rec);
        assert!(got.contains(&names::PROJECT_CHANGED.to_string()), "{got:?}");
        assert!(
            got.contains(&names::UI_STATE_CHANGED.to_string()),
            "{got:?}"
        );

        let ui = UiState {
            sidebar_pinned: true,
            ..st.ui_state().unwrap()
        };
        st.set_ui_state(ui.clone()).unwrap();
        st.set_ui_state(ui).unwrap();
        let got = names_of(&rec);
        let n = got.iter().filter(|n| *n == names::UI_STATE_CHANGED).count();
        assert_eq!(n, 1, "{got:?}");

        st.update_project_settings(|s| s.style = Some("minimal".into()))
            .unwrap();
        st.update_project_settings(|s| s.style = Some("minimal".into()))
            .unwrap();
        assert_eq!(names_of(&rec), [names::PROJECT_CHANGED]);

        st.close_project();
        let events: Vec<(String, Value)> = rec.0.lock().unwrap().drain(..).collect();
        let closed = events
            .iter()
            .find(|(n, _)| n == names::PROJECT_CHANGED)
            .unwrap();
        assert_eq!(closed.1, Value::Null);
    }

    #[test]
    fn origin_is_scoped_to_the_call() {
        use crate::core::events::{origin, with_origin};
        assert_eq!(origin(), None);
        let inner = with_origin("prefs", || {
            let nested = with_origin("main", origin);
            (origin(), nested)
        });
        assert_eq!(inner, (Some("prefs".into()), Some("main".into())));
        assert_eq!(origin(), None);
    }

    #[test]
    fn corrupt_prefs_recovered_with_event() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("prefs.json"), "{{{{").unwrap();
        let (st, rec) = state_in(tmp.path(), slow());
        assert_eq!(st.prefs(), Prefs::default());
        let events = rec.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, names::STORE_RECOVERED);
        let q = events[0].1["quarantinedAs"].as_str().unwrap();
        assert!(Path::new(q).exists());
    }

    #[test]
    fn newer_prefs_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = tmp.path().join("config");
        fs::create_dir_all(&cfg).unwrap();
        let text = r#"{"schemaVersion": 99, "theme": "dark"}"#;
        fs::write(cfg.join("prefs.json"), text).unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        assert_eq!(st.prefs(), Prefs::default());
        let p = Prefs {
            theme: Theme::Light,
            ..Default::default()
        };
        st.set_prefs(p.clone());
        st.flush_all();
        assert_eq!(st.prefs(), p);
        assert_eq!(fs::read_to_string(cfg.join("prefs.json")).unwrap(), text);
    }

    #[test]
    fn rapid_sets_coalesce_into_one_write() {
        let tmp = tempfile::tempdir().unwrap();
        // A quiet period far longer than any stall of this thread between two sets, so the
        // flusher cannot run in the middle of the burst.
        let (st, _) = state_in(tmp.path(), Duration::from_millis(600));
        let mut p = Prefs::default();
        for i in 0..50 {
            p.backup.retention = i + 1;
            st.set_prefs(p.clone());
        }
        assert_eq!(st.write_count(), 0);
        wait_until("the coalesced write", || st.write_count() > 0);
        // That one write held everything: nothing is left dirty for a flush.
        st.flush_all();
        assert_eq!(st.write_count(), 1);
        let saved: Prefs = persist::load_strict(&tmp.path().join("config")).unwrap();
        assert_eq!(saved.backup.retention, 50);
    }

    #[test]
    fn flush_all_writes_immediately() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let mut p = Prefs::default();
        p.backup.interval_minutes = 10_000;
        let stored = st.set_prefs(p);
        assert_eq!(stored.backup.interval_minutes, 240);
        st.flush_all();
        assert_eq!(st.write_count(), 1);
        let saved: Prefs = persist::load_strict(&tmp.path().join("config")).unwrap();
        assert_eq!(saved, stored);
        // Nothing dirty: a second flush writes nothing.
        st.flush_all();
        assert_eq!(st.write_count(), 1);
    }

    #[test]
    fn recents_remove_and_clear_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let a = st.create_project(tmp.path(), "A").unwrap();
        let b = st.create_project(tmp.path(), "B").unwrap();
        let left = st.remove_recent(&a.path);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].path, b.path);
        st.flush_all();
        drop(st);
        let (st, _) = state_in(tmp.path(), slow());
        assert_eq!(st.recents().len(), 1);
        st.clear_recents();
        assert!(st.recents().is_empty());
    }

    fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn created_events(rec: &Recorder) -> Vec<Value> {
        rec.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == names::BACKUP_CREATED)
            .map(|(_, v)| v.clone())
            .collect()
    }

    #[test]
    fn backup_worker_follows_project_and_prefs() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, rec) = state_in(tmp.path(), slow());
        assert!(!st.backup_worker_running());
        st.create_project(tmp.path(), "A").unwrap();
        assert!(st.backup_worker_running());
        wait_until("open snapshot", || st.list_backups().unwrap().len() == 1);
        assert_eq!(st.list_backups().unwrap()[0].reason, BackupReason::Open);
        wait_until("event", || created_events(&rec).len() == 1);
        assert_eq!(created_events(&rec)[0]["reason"], "open");

        let mut p = st.prefs();
        p.backup.enabled = false;
        st.set_prefs(p.clone());
        assert!(!st.backup_worker_running());
        p.backup.enabled = true;
        st.set_prefs(p.clone());
        assert!(st.backup_worker_running());
        p.backup.interval_minutes = 5;
        st.set_prefs(p);
        assert!(st.backup_worker_running());

        st.close_project();
        assert!(!st.backup_worker_running());

        // Reopening unchanged: the open snapshot is skipped.
        let a = st.recents()[0].path.clone();
        st.open_project(Path::new(&a)).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(st.list_backups().unwrap().len(), 1);
    }

    #[test]
    fn disabled_backups_start_no_worker() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let mut p = st.prefs();
        p.backup.enabled = false;
        st.set_prefs(p);
        st.create_project(tmp.path(), "A").unwrap();
        assert!(!st.backup_worker_running());
        std::thread::sleep(Duration::from_millis(100));
        assert!(st.list_backups().unwrap().is_empty());
    }

    #[test]
    fn manual_snapshot_restore_and_retention() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, rec) = state_in(tmp.path(), slow());
        let mut p = st.prefs();
        p.backup.enabled = false;
        p.backup.retention = 2;
        st.set_prefs(p);
        assert!(matches!(
            st.create_snapshot(BackupReason::Manual),
            Err(AppError::NoProject)
        ));
        st.create_project(tmp.path(), "A").unwrap();
        let ui = UiState {
            sidebar_pinned: true,
            ..Default::default()
        };
        st.set_ui_state(ui.clone()).unwrap();
        // Manual snapshots flush first and are written even when nothing changed.
        let first = st.create_snapshot(BackupReason::Manual).unwrap();
        let second = st.create_snapshot(BackupReason::Manual).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(first.files, 2);

        st.set_ui_state(UiState::default()).unwrap();
        st.set_project_settings(ProjectSettings {
            port: 9999,
            ..Default::default()
        })
        .unwrap();
        // Retention 2: the pre-restore snapshot must not prune away `second` before restoring.
        let project = st.restore_backup(&second.id).unwrap();
        assert_eq!(project.settings.port, ProjectSettings::default().port);
        assert_eq!(st.ui_state().unwrap(), ui);
        let list = st.list_backups().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].reason, BackupReason::PreRestore);
        assert_eq!(list[1].id, second.id);
        let reasons: Vec<Value> = created_events(&rec)
            .iter()
            .map(|v| v["reason"].clone())
            .collect();
        assert_eq!(reasons, ["manual", "manual", "pre-restore"]);

        assert!(matches!(
            st.restore_backup("nope"),
            Err(AppError::NotFound(_))
        ));
        st.delete_backup(&second.id).unwrap();
        assert!(matches!(
            st.delete_backup("../x"),
            Err(AppError::Invalid(_))
        ));
        assert_eq!(st.list_backups().unwrap().len(), 1);
    }

    #[test]
    fn dropping_state_stops_flusher() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), Duration::from_millis(10));
        let weak = Arc::downgrade(&st.inner);
        st.set_prefs(Prefs::default());
        drop(st);
        // The flusher holds only a Weak; once it finishes any in-flight flush, Inner is gone.
        let deadline = Instant::now() + Duration::from_secs(5);
        while weak.upgrade().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(weak.upgrade().is_none());
    }

    /// Records `(port, lanAccess)` of every notification; `None` for no project.
    #[derive(Default)]
    struct SeenProjects(Mutex<Vec<Option<(u16, bool)>>>);

    impl ProjectObserver for SeenProjects {
        fn project_changed(&self, project: Option<&Project>) {
            let v = project.map(|p| (p.settings.port, p.settings.lan_access));
            self.0.lock().unwrap().push(v);
        }
    }

    #[test]
    fn observer_and_live_follow_the_project() {
        use crate::core::live::{TEST_LAYER_ID, TEST_SCREEN_ID};
        let tmp = tempfile::tempdir().unwrap();
        let (st, _) = state_in(tmp.path(), slow());
        let seen = Arc::new(SeenProjects::default());
        st.add_project_observer(seen.clone());
        // A second observer (OSC) sees the same sequence.
        let second = Arc::new(SeenProjects::default());
        st.add_project_observer(second.clone());
        assert!(!st.live().has_screen(TEST_SCREEN_ID));

        st.create_project(tmp.path(), "A").unwrap();
        assert!(st.live().has_screen(TEST_SCREEN_ID));
        st.live().show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();

        let s = ProjectSettings {
            port: 9001,
            lan_access: true,
            ..Default::default()
        };
        st.set_project_settings(s).unwrap();
        let old = st.project_dir().unwrap();
        let moved = tmp.path().join("moved");
        fs::create_dir(&moved).unwrap();
        st.relocate_project(&old, moved).unwrap();
        st.reload_project().unwrap();
        st.close_project();
        assert!(!st.live().has_screen(TEST_SCREEN_ID));

        // Reopening starts from a fresh, off-air live state.
        st.open_project(&tmp.path().join("moved")).unwrap();
        let view = st.live().screen_view(TEST_SCREEN_ID).unwrap();
        assert!(!view.layers[0].on_air);

        let default_port = ProjectSettings::default().port;
        assert_eq!(
            *seen.0.lock().unwrap(),
            [
                None,
                Some((default_port, false)),
                Some((9001, true)),
                Some((9001, true)),
                Some((9001, true)),
                None,
                Some((9001, true)),
            ]
        );
        assert_eq!(*second.0.lock().unwrap(), *seen.0.lock().unwrap());
    }
}
