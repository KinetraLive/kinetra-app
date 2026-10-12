//! Lets `core` push events to the UI without depending on Tauri. `lib.rs` implements `EventSink`
//! for the app handle; tests use `NullSink` or a recording sink.

use serde_json::Value;

/// Event names emitted to the UI. Keep in sync with `EVENTS` in `src/lib/api.ts`.
pub mod names {
    /// Payload: `RecentProject[]` with `missing` filled in.
    pub const RECENTS_CHECKED: &str = "recents-checked";
    /// Payload: `MoveProgress`. Throttled to at most 10 per second.
    pub const MOVE_PROGRESS: &str = "project-move-progress";
    /// Payload: `{ file, quarantinedAs }` when a corrupt store file was set aside.
    pub const STORE_RECOVERED: &str = "store-recovered";
    /// Payload: `BackupInfo` for every snapshot written, including timer snapshots.
    pub const BACKUP_CREATED: &str = "backup-created";
    /// Payload: `{ message }` when a background task fails (e.g. a timer snapshot).
    pub const BACKGROUND_ERROR: &str = "background-error";
    /// Payload: `ServerStatus` whenever the output server's state changes.
    pub const SERVER_STATUS: &str = "server-status";
    /// Payload: `FilesChanged` when a template or screen changed on disk outside Kinetra
    /// (debounced; Kinetra's own writes are ignored).
    pub const PROJECT_FILES_CHANGED: &str = "project-files-changed";
    /// Payload: `LiveScreenView` of one screen whose view (on air, pending, fields, layers, info)
    /// changed, from any path (commands, auto-out, saves, watcher). Not emitted when nothing in
    /// the view changed (e.g. a rect drag). Reserved `__*` screens are never emitted.
    pub const LIVE_CHANGED: &str = "live-changed";
    /// Payload: `PreviewView` of one screen whose Preview (Studio mode: cues, staged content, the
    /// layers as Preview shows them, the T-bar position) changed, from any path. Only while Studio
    /// mode is on; never at rest.
    pub const PREVIEW_CHANGED: &str = "preview-changed";
    /// Payload: `SourceSummary` when a source changes without a command reply to carry it
    /// (auto-advance step, arming/disarming: `nextAt`; API sources: new rows or a change of the
    /// fetch status `api`, never at rest).
    pub const SOURCE_CHANGED: &str = "source-changed";
    /// Payload: `PollLive`: a poll's latest data (at most 4 per second, the server's pace) or
    /// a change of how it arrives. Only for polls with a stream, a watcher or a test simulation.
    pub const POLL_LIVE: &str = "poll-live";
    /// Payload: `TestDataBlocked`: a take of test poll data to Program was refused (a project that
    /// also shows a real poll, test data not allowed on air this session).
    pub const TEST_DATA_BLOCKED: &str = "test-data-blocked";
    /// Payload: `bool`: whether test data is allowed on air this session (changed in the app).
    pub const TEST_DATA_ALLOWED: &str = "test-data-allowed";
    /// Payload: `ScheduleView` whenever a schedule's statuses change (a boundary passed, an
    /// operator action, an edit or a watcher reload).
    pub const SCHEDULE_CHANGED: &str = "schedule-changed";
    /// Payload: `FlowState` when a flow instance starts, finishes, is stopped or fails (
    /// replaces `macro-state` and `recipe-state`).
    pub const FLOW_STATE: &str = "flow-state";
    /// Payload: `FlowLog` (`{ id, node, message, at }`) when a flow's Log action asks for a toast (at most
    /// one per 2 s per flow; every Log also goes to `kinetra.log`).
    pub const FLOW_LOG: &str = "flow-log";
    /// Payload: `FlowDebug`, batched (≤ 10 per second per flow), only for flows an editor
    /// subscribed with `subscribe_flow_debug`.
    pub const FLOW_DEBUG: &str = "flow-debug";
    /// Payload: `MidiDevice[]` when the MIDI devices (or which inputs listen) change; never at
    /// rest.
    pub const MIDI_DEVICES: &str = "midi-devices";
    /// Payload: `MidiLearned`, only while a learn session is open (`midi_learn(true)`).
    pub const MIDI_LEARN: &str = "midi-learn";
    /// Payload: `ControlsDoc` after `controls.json` was saved, reloaded from disk (watcher,
    /// restore, import) or a project opened or closed (closed: no mappings).
    pub const CONTROLS_CHANGED: &str = "controls-changed";
    /// Payload: `{ source: MidiSource, mappingIds: string[] }` when an input matches conflicting
    /// mappings (neither runs); at most once per 2 s per source.
    pub const CONTROL_CONFLICT: &str = "control-conflict";
    /// Payload: `OscStatus` whenever the OSC listener's state changes (off, listening, port in
    /// use).
    pub const OSC_STATUS: &str = "osc-status";
    /// Payload: `LicenseView` whenever the license, sign-in progress or account changed (to
    /// every app window). Display only: the core gates on its own state.
    pub const LICENSE_CHANGED: &str = "license-changed";
    /// Payload: `AiJobView` when an AI job starts, ends (succeeded with the imported template id,
    /// failed with the API's code) or is canceled. Never while it is just pending.
    pub const AI_JOB: &str = "ai-job";
    /// Payload: `Prefs` after prefs changed (set_prefs, set_zoom, reset, import). Cross-window.
    pub const PREFS_CHANGED: &str = "prefs-changed";
    /// Payload: `UiState` after the open project's UI state changed (set_ui_state, Studio
    /// settings, open / reload: the new project's state; not the Studio cue-ins). Cross-window.
    pub const UI_STATE_CHANGED: &str = "ui-state-changed";
    /// Payload: `Project | null` after a project opened, closed, moved, reloaded or its settings
    /// (incl. style and brand kit) changed. Cross-window.
    pub const PROJECT_CHANGED: &str = "project-changed";
    /// Main window only. Payload: `{ id }` of a clicked native menu item.
    pub const MENU: &str = "menu";
    /// Main window only. Payload: null. The window's close button was pressed with a project
    /// open: the page runs File › Exit (asking first when needed) after `close_request_ack`.
    pub const CLOSE_REQUESTED: &str = "close-requested";
    /// Preferences window only. Payload: `{ section }` asked for while it was open.
    pub const PREFS_SECTION: &str = "prefs-section";
    /// Payload: `UpdateProgress` while installing an update (≤ 10 per second).
    pub const UPDATE_PROGRESS: &str = "update-progress";
    /// Payload: `DisplayInfo[]` when the connected displays changed (plugged, unplugged,
    /// rearranged, mode changed); never at rest.
    pub const DISPLAYS_CHANGED: &str = "displays-changed";
    /// Payload: `OutputStatus[]` (every configured output) when any output's status changed.
    pub const OUTPUTS_CHANGED: &str = "outputs-changed";
    /// Payload: `AssetImportProgress` per file of an import job (≤ 10/s per file, plus its
    /// `done` event).
    pub const ASSET_IMPORT: &str = "asset-import";

    /// Events every app window mirrors: sent to all windows except the one whose command caused
    /// the change (see [`super::with_origin`]), so a window never echoes its own optimistic write.
    pub const CROSS_WINDOW: [&str; 3] = [PREFS_CHANGED, UI_STATE_CHANGED, PROJECT_CHANGED];
}

thread_local! {
    static ORIGIN: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Runs `f` with `label` as the origin window of the changes it makes: cross-window events
/// ([`names::CROSS_WINDOW`]) emitted on this thread during `f` skip that window. Core emits those
/// events synchronously from the changing call, so a command wraps its core call (inside its
/// `spawn_blocking` closure when it has one). Nested calls restore the outer origin.
pub fn with_origin<T>(label: &str, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0.take();
            ORIGIN.with(|o| *o.borrow_mut() = prev);
        }
    }
    let prev = ORIGIN.with(|o| o.borrow_mut().replace(label.to_string()));
    let _restore = Restore(prev);
    f()
}

/// The origin window set by [`with_origin`] on this thread, if any.
pub fn origin() -> Option<String> {
    ORIGIN.with(|o| o.borrow().clone())
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

/// Told when the open project changes: opened, closed, relocated, or its settings changed. The
/// output server uses it to start, stop or rebind. Called after the change is in place, without
/// any `AppState` lock held, so implementations may read `AppState`. Must not block: hand real
/// work to a thread or the async runtime.
pub trait ProjectObserver: Send + Sync {
    fn project_changed(&self, project: Option<&crate::core::model::Project>);
}

/// Discards everything. For tests and for code paths with no UI attached.
pub struct NullSink;

impl EventSink for NullSink {
    fn emit(&self, _event: &str, _payload: Value) {}
}
