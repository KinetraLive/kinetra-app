//! Remote control through kinetra-cloud (`docs/CONTROL_PROTOCOL.md`, the contract; the plan is
//! kinetra-private `docs/CLOUD_CONTROL_PLAN.md`).
//!
//! The app keeps ONE outgoing WebSocket to `api.kinetra.live` while the switch is on for the
//! open project; browsers at `control.kinetra.live` reach it through the server. The app is the
//! authority: every command is checked again here ([`commands`]) and runs through the paths
//! local actions use.
//!
//! # Rules (§0)
//! - **Never interrupt graphics on air.** Nothing here takes anything off air or blocks local
//!   work: the render path only wakes the connection (`Notify`, never blocking); the connection
//!   builds the state on the blocking pool and sends at its own pace. A link that stalls simply
//!   sends one coalesced diff (or a snapshot) when it moves again.
//! - **Off at every app start.** The switch is per project and lives for this app session only
//!   (never saved); the app never connects by itself after a restart.
//! - **Off costs nothing:** no task, no socket; the change listener returns after one atomic
//!   load.
//! - **Commands are never queued:** executed when they arrive, one at a time, never replayed.
//!
//! # Pieces
//! [`proto`] (messages, close codes), [`state`] (tree, diff, history), [`commands`] (checks +
//! execution), [`diag`] (counters, ring buffers, export), [`ids`] (UUID v7, nonces), `conn`
//! (the connection task: lifecycle, reconnect, heartbeat, snapshots and diffs) and `probe`
//! (Test connection, §4a).

pub mod collaborators;
pub mod commands;
pub mod diag;
pub mod ids;
pub mod perms;
pub mod preview;
pub mod proto;
pub mod state;

mod conn;
mod probe;

#[cfg(test)]
mod tests;

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::{mpsc, Notify};

pub use probe::TestResult;
pub use proto::CrewMember;

use self::diag::Diag;
use self::proto::EndReason;
use self::state::RemoteAsset;
use crate::core::events::{names, ProjectObserver};
use crate::core::fsx::now_epoch_ms;
use crate::core::model::Project;
use crate::core::state::{AppState, StateChange, WeakState};
use crate::error::{AppError, AppResult};

/// Remote control is in this build (`KINETRA_REMOTE` at build time, see `build.rs`): off in
/// release builds until kinetra-cloud's relay is live, on in debug builds. Without it the core
/// refuses to turn remote control on and the UI leaves the switch out.
pub const REMOTE_BUILD: bool = cfg!(kinetra_remote);

/// The entitlement remote control needs (`kinetra_license::entitlements::PRO_CONTROL`).
pub const PRO_CONTROL: &str = kinetra_license::entitlements::PRO_CONTROL;
/// Recent remote actions kept (in memory, this run only).
pub const ACTIVITY_CAP: usize = 200;

/// Where remote control is (`remote-control`'s `state`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Phase {
    #[default]
    Off,
    Connecting,
    Live {
        run_id: String,
        resumed: bool,
    },
    Reconnecting {
        attempt: u32,
        next_ms: u64,
    },
    /// The show runs on another machine: "Move it here?" (Move it here / Make this a separate
    /// show / Cancel).
    ShowInUse {
        machine: String,
    },
    /// The run ended (the switch stays on): "Remote control ended" + Start again.
    Ended,
    /// Stopped for a reason the user has to handle (sign in, plan, update, clock, moved, the
    /// plan's limits). Start again retries.
    Stopped {
        code: String,
        message: String,
    },
}

/// A `license_warning` (§10.3): remote control continues until `ends_ms`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseWarning {
    pub ends_ms: u64,
    pub message: String,
}

/// What the UI shows (`remote_view`, the `remote-control` event).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteView {
    /// The switch for the open project (this session).
    pub on: bool,
    #[serde(flatten)]
    pub phase: Phase,
    pub take_locked: bool,
    /// Who is connected (names shown to the owner only).
    pub crew: Vec<CrewMember>,
    pub show_id: Option<String>,
    pub crew_url: Option<String>,
    pub controller_url: Option<String>,
    pub server_version: Option<String>,
    pub rtt_ms: Option<u64>,
    pub license_warning: Option<LicenseWarning>,
    /// The last error code the server sent (diagnostics, shown in the panel).
    pub last_error: Option<String>,
    /// The project has a remote control ID ("Make this a separate show" is offered).
    pub has_show_id: bool,
    /// This build offers remote control (`REMOTE_BUILD`).
    pub available: bool,
    /// The protocol the server picked (1 or 2) while connected.
    pub protocol: Option<u32>,
    /// What the server has switched on (protocol 2 `welcome.features`).
    pub features: Vec<String>,
    /// The collaborators API is there (`invites`): Invite collaborator opens the app's dialog;
    /// otherwise it opens the control site's Collaborators page (`crew_url`).
    pub invites: bool,
    /// Screens in the project beyond what remote users see (the protocol shows at most
    /// [`proto::SCREENS_MAX`]): "Collaborators see the first 32 screens". 0 when all show.
    pub screens_hidden: u32,
    /// This computer can send live pictures (a preview host is set: Windows).
    pub preview: bool,
}

/// One accepted remote command (in memory only, this run; gone when the run or app ends).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub at: u64,
    pub name: String,
    pub role: String,
    /// "Kari N. updated Name on Lower third 2".
    pub text: String,
}

/// Messages from `AppState` to the connection task.
#[derive(Debug)]
pub(crate) enum Ctl {
    End(EndReason),
    Takeover,
    NewShow,
    Rename(String),
    Ping(tokio::sync::oneshot::Sender<Result<u64, String>>),
    /// The preview cap or "Allow remote preview" may have changed.
    PreviewSettings,
}

struct Conn {
    gen: u64,
    ctl: mpsc::UnboundedSender<Ctl>,
}

#[derive(Default)]
struct Inner {
    /// Project folders whose switch is on (this session).
    on_for: HashSet<PathBuf>,
    /// The open project's folder and name, as the observer last saw them.
    project: Option<PathBuf>,
    project_name: String,
    view: RemoteView,
    conn: Option<Conn>,
    /// The last cue run and its position then (session state, reset when a project opens).
    last_cue: Option<(String, usize)>,
    /// The assets remote users may pick (`None`: read again before the next build).
    assets: Option<Arc<Vec<RemoteAsset>>>,
    activity: VecDeque<Activity>,
    /// The cue position last announced (`cue-position`).
    cue_announced: Option<CuePosition>,
}

/// The cue list's position (`cue-position`; Control's "Next" mark shows `next`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CuePosition {
    /// The last cue run (a flow id).
    pub current: Option<String>,
    /// What GO runs next.
    pub next: Option<String>,
}

/// Remote control's state on `AppState`.
#[derive(Default)]
pub struct Remote {
    inner: Mutex<Inner>,
    /// A connection task exists: the change listener only wakes it then.
    active: AtomicBool,
    /// Connection details windows open (`remote_diag_watch`, counted).
    diag_watchers: std::sync::atomic::AtomicUsize,
    /// A `remote-diag` is scheduled.
    diag_pending: AtomicBool,
    /// Wakes the connection task: something it shows may have changed.
    dirty: Arc<Notify>,
    gen: AtomicU64,
    /// The newest connection task that has finished (quit waits for it).
    finished: AtomicU64,
    diag: Mutex<Diag>,
    /// Captures the live preview (Windows; `None`: this computer can't).
    preview_host: Mutex<Option<Arc<dyn preview::PreviewHost>>>,
    /// Tests: pretend this build has no remote control.
    #[cfg(test)]
    test_unavailable: AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Tells remote control about project opens, closes and renames.
struct Observer(WeakState);

impl ProjectObserver for Observer {
    fn project_changed(&self, project: Option<&Project>) {
        if let Some(st) = self.0.upgrade() {
            st.remote_project_changed(project);
        }
    }
}

impl Remote {
    pub(crate) fn diag(&self) -> MutexGuard<'_, Diag> {
        lock(&self.diag)
    }
}

impl AppState {
    /// Whether this build offers remote control ([`REMOTE_BUILD`]; tests can pretend not).
    pub fn remote_available(&self) -> bool {
        #[cfg(test)]
        if self.remote().test_unavailable.load(Ordering::Relaxed) {
            return false;
        }
        REMOTE_BUILD
    }

    /// Registers the change listener, the project observer and the flow hook. Called once by
    /// `AppState::load`; costs nothing while remote control is off. The listener and the flow
    /// hook also wake the local Control API's feed (`core::control_api`, same rule).
    pub(crate) fn remote_init(&self) {
        self.add_change_listener(Arc::new(|st: &AppState, change: StateChange<'_>| {
            if let StateChange::FlowRun { id, running: true } = change {
                st.remote_cue_ran(id);
            }
            st.remote_touch();
            st.control_api_touch();
        }));
        let weak = self.downgrade();
        self.flows().set_changed_hook(Arc::new(move || {
            if let Some(st) = weak.upgrade() {
                st.remote_touch();
                st.remote_announce_cues();
                st.control_api_touch();
            }
        }));
        self.add_project_observer(Arc::new(Observer(self.downgrade())));
    }

    /// Something remote users see may have changed: wakes the connection (never blocks).
    pub(crate) fn remote_touch(&self) {
        if self.remote().active.load(Ordering::Relaxed) {
            self.remote().dirty.notify_one();
        }
    }

    /// The media library changed: the asset list is read again before the next build.
    pub(crate) fn remote_assets_changed(&self) {
        lock(&self.remote().inner).assets = None;
        self.remote_touch();
        self.control_api_touch();
    }

    /// A flow started: a cue flow becomes the list's `current` (from any path: GO, hotkeys,
    /// MIDI / OSC, Run flow, remote).
    fn remote_cue_ran(&self, id: &str) {
        // Most flows that start aren't cues: one lookup, no list.
        let is_cue = self.flows().flow(id).is_some_and(|f| {
            f.nodes
                .iter()
                .any(|n| matches!(n.kind, crate::core::model::NodeKind::Cue { .. }))
        });
        if !is_cue {
            return;
        }
        let cues = state::cues(self);
        if let Some(pos) = cues.iter().position(|c| c.id == id) {
            lock(&self.remote().inner).last_cue = Some((id.to_string(), pos));
        }
        self.remote_announce_cues();
    }

    /// The cue list's `current` and `next` (Control's "Next" mark shows the same).
    pub fn remote_cue_position(&self) -> CuePosition {
        let order: Vec<String> = state::cues(self).into_iter().map(|c| c.id).collect();
        let last = lock(&self.remote().inner).last_cue.clone();
        let (current, next) = state::current_next(&order, last.as_ref());
        CuePosition { current, next }
    }

    /// Emits `cue-position` when it changed (a cue ran, cues were added, removed or renamed,
    /// another project opened).
    fn remote_announce_cues(&self) {
        let pos = self.remote_cue_position();
        {
            let mut g = lock(&self.remote().inner);
            if g.cue_announced.as_ref() == Some(&pos) {
                return;
            }
            g.cue_announced = Some(pos.clone());
        }
        match serde_json::to_value(&pos) {
            Ok(v) => self.events().emit(names::CUE_POSITION, v),
            Err(e) => tracing::warn!("cue-position payload: {e}"),
        }
    }

    /// The current view (`remote_view`).
    pub fn remote_view(&self) -> RemoteView {
        let mut v = lock(&self.remote().inner).view.clone();
        v.has_show_id = self.project_remote_id().is_some();
        v.available = self.remote_available();
        v.preview = self.preview_host().is_some();
        v.screens_hidden = u32::try_from(
            self.live()
                .user_screen_ids()
                .len()
                .saturating_sub(proto::SCREENS_MAX),
        )
        .unwrap_or(u32::MAX);
        v
    }

    fn remote_emit(&self) {
        let v = self.remote_view();
        match serde_json::to_value(&v) {
            Ok(p) => self.events().emit(names::REMOTE_CONTROL, p),
            Err(e) => tracing::warn!("remote-control payload: {e}"),
        }
    }

    /// Changes the view of connection `gen` (a stale task's changes are dropped) and tells the
    /// UI when anything changed.
    pub(crate) fn remote_update(&self, gen: u64, f: impl FnOnce(&mut RemoteView)) {
        let changed = {
            let mut g = lock(&self.remote().inner);
            if g.conn.as_ref().map(|c| c.gen) != Some(gen) {
                return;
            }
            let before = g.view.clone();
            f(&mut g.view);
            g.view != before
        };
        if changed {
            self.remote_emit();
        }
    }

    /// The connection task ended for good (stopped, ended, turned off): forget it.
    pub(crate) fn remote_task_done(&self, gen: u64) {
        let mut g = lock(&self.remote().inner);
        if g.conn.as_ref().map(|c| c.gen) == Some(gen) {
            g.conn = None;
            self.remote().active.store(false, Ordering::Relaxed);
        }
    }

    /// Turns remote control on or off for the open project (this session only). On: Pro
    /// (`pro.control`, full access) and a signed-in account; the project gets its ID the first
    /// time. Off: `end {disabled}` and the socket closes. Nothing on air changes either way.
    pub fn remote_set(&self, on: bool) -> AppResult<RemoteView> {
        let dir = self.project_dir()?;
        if !on {
            {
                let mut g = lock(&self.remote().inner);
                g.on_for.remove(&dir);
                g.view.on = false;
            }
            self.remote_stop(EndReason::Disabled);
            self.remote_emit();
            return Ok(self.remote_view());
        }
        if !self.remote_available() {
            return Err(AppError::Invalid(
                "Remote control isn't available in this version of Kinetra yet.".into(),
            ));
        }
        self.require_new_pro_work(PRO_CONTROL)?;
        if lock(&self.remote().inner).conn.is_some() {
            // Already on (connecting, live or reconnecting).
            return Ok(self.remote_view());
        }
        if self.project_remote_id().is_none() {
            self.set_project_remote_id(ids::uuid_v7())?;
        }
        lock(&self.remote().inner).on_for.insert(dir);
        self.remote_start()?;
        Ok(self.remote_view())
    }

    /// End remote control: every browser is told the show machine ended it; the switch stays on
    /// with Start again (§5). Nothing on air changes.
    pub fn remote_end(&self) -> AppResult<RemoteView> {
        self.project_dir()?;
        let had = lock(&self.remote().inner).conn.is_some();
        if had {
            self.remote_stop(EndReason::Disabled);
            lock(&self.remote().inner).view.phase = Phase::Ended;
            self.remote_emit();
        }
        Ok(self.remote_view())
    }

    /// "Start again" after the run ended or remote control stopped (the switch is still on).
    pub fn remote_start_again(&self) -> AppResult<RemoteView> {
        self.remote_set(true)
    }

    /// "Move it here?" → Move it here: registers again with `takeover`.
    pub fn remote_takeover(&self) -> AppResult<()> {
        self.remote_ctl(Ctl::Takeover)
    }

    /// "Make this a separate show": a new project ID (the old show stays with the other copy).
    /// While connected, the run ends and the new show registers on the same connection (empty
    /// crew). Nothing on air changes.
    pub fn remote_new_show(&self) -> AppResult<RemoteView> {
        self.project_dir()?;
        self.set_project_remote_id(ids::uuid_v7())?;
        if lock(&self.remote().inner).conn.is_some() {
            self.remote_ctl(Ctl::NewShow)?;
        } else {
            self.remote_emit();
        }
        Ok(self.remote_view())
    }

    /// Lock remote take (§9.3): remote commands that change what's on air are refused.
    pub fn remote_lock(&self, on: bool) -> AppResult<RemoteView> {
        self.project_dir()?;
        let changed = {
            let mut g = lock(&self.remote().inner);
            let changed = g.view.take_locked != on;
            g.view.take_locked = on;
            changed
        };
        if changed {
            self.remote_touch();
            self.remote_emit();
        }
        Ok(self.remote_view())
    }

    /// Whether the switch is on for the open project.
    pub(crate) fn remote_enabled(&self) -> bool {
        let g = lock(&self.remote().inner);
        g.project.as_ref().is_some_and(|p| g.on_for.contains(p))
    }

    /// Recent remote actions of this run, newest first.
    pub fn remote_activity(&self) -> Vec<Activity> {
        lock(&self.remote().inner)
            .activity
            .iter()
            .rev()
            .cloned()
            .collect()
    }

    pub(crate) fn remote_record_activity(&self, a: Activity) {
        {
            let mut g = lock(&self.remote().inner);
            if g.activity.len() == ACTIVITY_CAP {
                g.activity.pop_front();
            }
            g.activity.push_back(a.clone());
        }
        match serde_json::to_value(&a) {
            Ok(v) => self.events().emit(names::REMOTE_ACTIVITY, v),
            Err(e) => tracing::warn!("remote-activity payload: {e}"),
        }
    }

    /// The tree's inputs besides the app state: the lock, the last cue run and the assets
    /// (read from disk only after a media change). Call from a thread that may block.
    pub(crate) fn remote_build_input(&self) -> state::BuildInput {
        let (take_locked, last_cue, cached) = {
            let g = lock(&self.remote().inner);
            (g.view.take_locked, g.last_cue.clone(), g.assets.clone())
        };
        let assets = match cached {
            Some(a) => a,
            None => {
                let a = Arc::new(self.remote_read_assets());
                lock(&self.remote().inner).assets = Some(a.clone());
                a
            }
        };
        state::BuildInput {
            take_locked,
            last_cue,
            assets: assets.as_ref().clone(),
            flavor: state::Flavor::Remote,
        }
    }

    /// Images and videos in `assets/`, by name (audio files can't be field values).
    fn remote_read_assets(&self) -> Vec<RemoteAsset> {
        let Ok(dir) = self.project_dir() else {
            return Vec::new();
        };
        let root = crate::core::assets::assets_root(&dir);
        let Ok(entries) = std::fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| crate::core::assets::is_asset_name(n))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        names
            .into_iter()
            .filter_map(|name| {
                let kind = if crate::core::assets::is_image_name(&name) {
                    "image"
                } else if crate::core::assets::asset_ext(&name)
                    .is_some_and(|e| crate::core::model::MEDIA_VIDEO_EXTENSIONS.contains(&e))
                {
                    "video"
                } else {
                    return None;
                };
                Some(RemoteAsset {
                    path: crate::core::assets::url_of(&name),
                    name,
                    kind,
                })
            })
            .take(proto::ASSETS_MAX)
            .collect()
    }

    /// Starts the connection task (a fresh one; an old one is told to end).
    fn remote_start(&self) -> AppResult<()> {
        let rt = self
            .live()
            .runtime_handle()
            .ok_or_else(|| AppError::Internal("Remote control needs the app's runtime".into()))?;
        self.remote_stop(EndReason::Other);
        let gen = self.remote().gen.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::unbounded_channel();
        {
            let mut g = lock(&self.remote().inner);
            g.conn = Some(Conn { gen, ctl: tx });
            g.view.on = true;
            g.view.phase = Phase::Connecting;
            g.view.crew.clear();
            g.view.license_warning = None;
            g.activity.clear();
        }
        self.remote().active.store(true, Ordering::Relaxed);
        self.live().set_fade_tracking(true);
        self.remote_emit();
        rt.spawn(conn::run(
            self.downgrade(),
            gen,
            rx,
            self.remote().dirty.clone(),
        ));
        Ok(())
    }

    /// Tells the connection task to end with `reason` (it sends `end` and closes) and forgets
    /// it. The view goes to Off.
    fn remote_stop(&self, reason: EndReason) {
        let conn = {
            let mut g = lock(&self.remote().inner);
            g.view.phase = Phase::Off;
            g.view.crew.clear();
            g.view.rtt_ms = None;
            g.view.protocol = None;
            g.view.features.clear();
            g.view.invites = false;
            g.view.license_warning = None;
            g.conn.take()
        };
        self.remote().active.store(false, Ordering::Relaxed);
        if let Some(c) = conn {
            let _ = c.ctl.send(Ctl::End(reason));
            self.live().set_fade_tracking(false);
        }
    }

    fn remote_ctl(&self, msg: Ctl) -> AppResult<()> {
        let g = lock(&self.remote().inner);
        let c = g
            .conn
            .as_ref()
            .ok_or_else(|| AppError::Invalid("Remote control isn't connected".into()))?;
        c.ctl
            .send(msg)
            .map_err(|_| AppError::Invalid("Remote control isn't connected".into()))
    }

    /// The app is quitting: `end {quit}`, waiting up to about a second for it to go out.
    pub fn remote_quit(&self) {
        let gen = match lock(&self.remote().inner).conn.as_ref() {
            Some(c) => c.gen,
            None => return,
        };
        self.remote_stop(EndReason::Quit);
        let until = std::time::Instant::now() + std::time::Duration::from_millis(1200);
        while self.remote().finished.load(Ordering::Relaxed) < gen
            && std::time::Instant::now() < until
        {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// The connection task `gen` has returned.
    pub(crate) fn remote_task_finished(&self, gen: u64) {
        self.remote().finished.fetch_max(gen, Ordering::Relaxed);
    }

    /// A project opened, closed, was renamed or moved (the project observer).
    fn remote_project_changed(&self, project: Option<&Project>) {
        let path = project.map(|p| PathBuf::from(&p.path));
        let name = project.map(|p| p.name.clone()).unwrap_or_default();
        let (switched, renamed, start) = {
            let mut g = lock(&self.remote().inner);
            let switched = g.project != path;
            let renamed = !switched && g.project_name != name;
            if switched {
                g.project = path.clone();
                g.last_cue = None;
                g.assets = None;
                g.view.take_locked = false;
            }
            g.project_name = name.clone();
            let start = switched && path.as_ref().is_some_and(|p| g.on_for.contains(p));
            (switched, renamed, start)
        };
        if switched {
            self.remote_announce_cues();
            let had = lock(&self.remote().inner).conn.is_some();
            if had {
                self.remote_stop(EndReason::ProjectClosed);
            }
            lock(&self.remote().inner).view.on = start;
            if start {
                // The switch stays on for this project for the session.
                if let Err(e) = self.remote_start() {
                    tracing::warn!("remote control: {e}");
                }
            } else if had {
                self.remote_emit();
            }
        } else if renamed {
            let _ = self.remote_ctl(Ctl::Rename(state::name_of(&name)));
        }
    }

    pub(crate) fn remote_project_name(&self) -> String {
        state::name_of(&lock(&self.remote().inner).project_name)
    }

    /// Test connection (§4a): a probe while off; a ping on the live connection while on.
    pub async fn remote_test(&self) -> TestResult {
        if !self.remote_available() {
            return TestResult {
                code: "unavailable".into(),
                message: "Remote control isn't available in this version of Kinetra yet.".into(),
                ..Default::default()
            };
        }
        let live = {
            let g = lock(&self.remote().inner);
            matches!(g.view.phase, Phase::Live { .. })
                .then(|| g.conn.as_ref().map(|c| c.ctl.clone()))
                .flatten()
        };
        match live {
            Some(ctl) => probe::ping_live(self, ctl).await,
            None => probe::probe(self).await,
        }
    }

    /// The diagnostics export (see [`diag`]); no values, names, tokens or account IDs.
    pub fn remote_diagnostics(&self) -> Value {
        let state = serde_json::to_value(&self.remote_view().phase)
            .ok()
            .and_then(|v| v["state"].as_str().map(str::to_string))
            .unwrap_or_default();
        self.remote()
            .diag()
            .export(&state, env!("CARGO_PKG_VERSION"), std::env::consts::OS)
    }

    /// What Connection details shows: the export with the last 50 message records.
    pub fn remote_diag_view(&self) -> Value {
        let mut v = self.remote_diagnostics();
        if let Some(m) = v["messages"].as_array_mut() {
            let n = m.len().saturating_sub(50);
            m.drain(..n);
        }
        v
    }

    /// Opens (`true`) or closes a Connection details watch (counted): while one is open,
    /// `remote-diag` follows every change, at most twice a second. Returns the view now.
    pub fn remote_diag_watch(&self, on: bool) -> Value {
        let w = &self.remote().diag_watchers;
        if on {
            w.fetch_add(1, Ordering::Relaxed);
        } else {
            let _ = w.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1));
        }
        self.remote_diag_view()
    }

    /// The diagnostics changed: `remote-diag` within 500 ms while someone watches (ONE sleeping
    /// task while one is pending; nothing without a watcher).
    pub(crate) fn remote_diag_touch(&self) {
        let r = self.remote();
        if r.diag_watchers.load(Ordering::Relaxed) == 0
            || r.diag_pending.swap(true, Ordering::Relaxed)
        {
            return;
        }
        let Some(rt) = self.live().runtime_handle() else {
            r.diag_pending.store(false, Ordering::Relaxed);
            return;
        };
        let weak = self.downgrade();
        rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let Some(st) = weak.upgrade() else { return };
            st.remote().diag_pending.store(false, Ordering::Relaxed);
            let v = st.remote_diag_view();
            st.events().emit(names::REMOTE_DIAG, v);
        });
    }

    /// Writes [`AppState::remote_diagnostics`] to `path` (atomically). Touches the disk.
    pub fn remote_export_diagnostics(&self, path: &Path) -> AppResult<()> {
        let json = serde_json::to_vec_pretty(&self.remote_diagnostics())
            .map_err(|e| AppError::Internal(e.to_string()))?;
        crate::core::fsx::write_bytes_atomic(path, &json)?;
        Ok(())
    }

    /// A remote action for the activity list.
    pub(crate) fn remote_activity_now(&self, name: &str, role: &str, text: String) {
        self.remote_record_activity(Activity {
            at: now_epoch_ms(),
            name: name.to_string(),
            role: role.to_string(),
            text,
        });
    }
}
