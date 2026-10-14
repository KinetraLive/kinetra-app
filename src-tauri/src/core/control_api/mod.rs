//! The local Control API (`docs/CONTROL_API.md`): the WebSocket and REST interface other
//! programs on this computer or the LAN use to control Kinetra (Bitfocus Companion, scripts).
//! Free (no Pro gate); off until switched on in Preferences (`prefs.json` `controlApi`); every
//! request needs the token (one per machine, in the keychain).
//!
//! Its state is remote control's tree in the local flavor (`remote::state::Flavor::Local`):
//! layer keys `<screen>.<layer>`, the logical on-air state (take → clear), and what a control
//! surface shows besides (staged changes, readiness, running cues, sources, Automation off).
//!
//! # Cost
//! Off, or on with no client: no task, no work; the change listener returns after one atomic
//! load. With clients: ONE feed task ([`feed`]) shared by all of them, built on the blocking pool
//! when something changed, each diff serialized once. The server (`server::control`) runs one
//! session task per client.
//!
//! # Pieces
//! [`token`] (format, keychain id, comparison), [`feed`] (the shared tree, diffs, per-client
//! cursor), [`commands`] (checks + the core paths). The HTTP / WebSocket side lives in
//! `server::control` (axum types stay out of `core`).

pub mod commands;
pub mod feed;
pub mod token;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{watch, Notify};

pub use commands::{Refused, FEATURES};
pub use feed::{Attached, Out, TreeView};

use self::feed::FeedHandle;
use crate::core::events::{names, ProjectObserver};
use crate::core::fsx::now_epoch_ms;
use crate::core::model::{Project, CANARY_BUILD};
use crate::core::remote::state::{self, Flavor};
use crate::core::secrets::SecretStore;
use crate::core::state::{AppState, WeakState};
use crate::error::{AppError, AppResult};

/// The protocol version (`welcome.protocol`). Clients refuse a higher major.
pub const PROTOCOL: u32 = 1;

/// The WebSocket subprotocol.
pub const SUBPROTOCOL: &str = "kinetra-control-local.v1";

/// Connected WebSocket clients at most.
pub const CLIENTS_MAX: usize = 8;

/// A client's name (from its `hello`) is cut to this many characters.
pub const CLIENT_NAME_MAX: usize = 64;

/// WebSocket close codes the app sends.
pub mod close {
    /// The server is stopping or another project opened.
    pub const GOING_AWAY: u16 = 1001;
    /// A malformed message, or no `hello` in time.
    pub const BAD_MESSAGE: u16 = 4400;
    /// The token was regenerated.
    pub const TOKEN_CHANGED: u16 = 4401;
    /// Disconnected in Preferences.
    pub const DISCONNECTED: u16 = 4403;
    /// Too many clients.
    pub const CLIENT_LIMIT: u16 = 4408;
    /// The Control API was turned off.
    pub const TURNED_OFF: u16 = 4409;
}

/// A connected client as Preferences lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlClientView {
    pub id: u64,
    /// From its `hello` (e.g. "companion-module-kinetra-graphics/1.0.0").
    pub name: String,
    /// The peer's IP address.
    pub addr: String,
    /// Server epoch ms of the connection.
    pub since: u64,
}

/// What Preferences › Network & API › Control API shows (`control-api` event, on change).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlApiView {
    pub enabled: bool,
    pub clients: Vec<ControlClientView>,
}

/// Why a client can't attach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachError {
    /// The Control API is off.
    Disabled,
    /// [`CLIENTS_MAX`] clients are connected.
    Full,
    /// No project is open, or no async runtime.
    Unavailable,
}

struct Client {
    view: ControlClientView,
    kick: watch::Sender<Option<u16>>,
}

#[derive(Default)]
struct Inner {
    /// The token as last read or made (`None`: not read yet, or none stored).
    token: Option<String>,
    clients: BTreeMap<u64, Client>,
    next_id: u64,
    feed: Option<FeedHandle>,
    feed_gen: u64,
    /// A command's corr for the next diff's `cause`.
    cause: Option<String>,
    announced: Option<ControlApiView>,
    project: Option<PathBuf>,
}

/// The Control API's state on `AppState`.
pub struct ControlApi {
    inner: Mutex<Inner>,
    /// A feed task exists: the change listener only wakes it then.
    active: AtomicBool,
    /// Wakes the feed: something clients see may have changed.
    dirty: Arc<Notify>,
    secrets: RwLock<Arc<dyn SecretStore>>,
    /// Failed token checks (diagnostics; the server limits per IP).
    rejected: AtomicU64,
    /// Feed tasks running (0 without clients; tested).
    tasks: Arc<AtomicUsize>,
}

impl Default for ControlApi {
    fn default() -> Self {
        #[cfg(test)]
        let secrets: Arc<dyn SecretStore> =
            Arc::new(crate::core::secrets::MemorySecrets::default());
        #[cfg(not(test))]
        let secrets: Arc<dyn SecretStore> = Arc::new(crate::core::secrets::KeyringSecrets);
        Self {
            inner: Mutex::default(),
            active: AtomicBool::new(false),
            dirty: Arc::new(Notify::new()),
            secrets: RwLock::new(secrets),
            rejected: AtomicU64::new(0),
            tasks: Arc::default(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Detaches its client when dropped (the connection ended, however).
pub struct DetachGuard {
    weak: WeakState,
    id: u64,
}

impl Drop for DetachGuard {
    fn drop(&mut self) {
        if let Some(st) = self.weak.upgrade() {
            st.control_api_detach(self.id);
        }
    }
}

struct Observer(WeakState);

impl ProjectObserver for Observer {
    fn project_changed(&self, project: Option<&Project>) {
        if let Some(st) = self.0.upgrade() {
            st.control_api_project_changed(project);
        }
    }
}

impl AppState {
    fn capi(&self) -> &ControlApi {
        self.control_api()
    }

    fn secret_store(&self) -> Arc<dyn SecretStore> {
        self.capi()
            .secrets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Tests: a keychain of their own.
    #[cfg(test)]
    pub fn set_control_api_secrets(&self, store: Arc<dyn SecretStore>) {
        *self
            .capi()
            .secrets
            .write()
            .unwrap_or_else(PoisonError::into_inner) = store;
        lock(&self.capi().inner).token = None;
    }

    /// Registers the project observer. Called once by `AppState::load`; the change listener
    /// (shared with remote control) calls [`AppState::control_api_touch`].
    pub(crate) fn control_api_init(&self) {
        self.add_project_observer(Arc::new(Observer(self.downgrade())));
    }

    /// Something clients see may have changed: wakes the feed (never blocks; one atomic load
    /// when no client is connected).
    pub(crate) fn control_api_touch(&self) {
        if self.capi().active.load(Ordering::Relaxed) {
            self.capi().dirty.notify_one();
        }
    }

    /// The Control API's state tree, built from memory (the asset list is read from disk only
    /// after a media change). Call from a thread that may block briefly.
    pub fn control_api_tree(&self) -> Value {
        let mut input = self.remote_build_input();
        input.flavor = Flavor::Local;
        input.take_locked = false;
        state::build(self, &input)
    }

    /// Whether the Control API is on (`prefs.json` `controlApi.enabled`).
    pub fn control_api_enabled(&self) -> bool {
        self.prefs().control_api.enabled
    }

    /// Turns the Control API on or off (a token is made the first time it goes on). Off
    /// disconnects every client.
    pub fn control_api_set_enabled(&self, on: bool) -> AppResult<ControlApiView> {
        if on {
            self.control_api_token()?;
        }
        let mut prefs = self.prefs();
        prefs.control_api.enabled = on;
        self.set_prefs(prefs);
        Ok(self.control_api_view())
    }

    /// `set_prefs` saw `controlApi` change (Preferences, a settings import, a reset).
    pub(crate) fn control_api_prefs_changed(&self, enabled: bool) {
        if !enabled {
            self.control_api_kick_all(close::TURNED_OFF);
        }
        self.control_api_announce();
    }

    /// The token, made and stored the first time (or when the stored value isn't one of ours).
    /// Reads the keychain only once per session.
    pub fn control_api_token(&self) -> AppResult<String> {
        if let Some(t) = lock(&self.capi().inner).token.clone() {
            return Ok(t);
        }
        let store = self.secret_store();
        let stored = store.get(token::SECRET_ID).map_err(AppError::from)?;
        let t = match stored.filter(|t| token::is_well_formed(t)) {
            Some(t) => t,
            None => {
                let t = token::generate()
                    .ok_or_else(|| AppError::Internal("No secure random source".into()))?;
                store.set(token::SECRET_ID, &t).map_err(AppError::from)?;
                t
            }
        };
        lock(&self.capi().inner).token = Some(t.clone());
        Ok(t)
    }

    /// Replaces the token; every connected client is disconnected (they need the new one).
    pub fn control_api_regenerate_token(&self) -> AppResult<String> {
        let t = token::generate()
            .ok_or_else(|| AppError::Internal("No secure random source".into()))?;
        self.secret_store()
            .set(token::SECRET_ID, &t)
            .map_err(AppError::from)?;
        lock(&self.capi().inner).token = Some(t.clone());
        self.control_api_kick_all(close::TOKEN_CHANGED);
        Ok(t)
    }

    /// Whether `given` is the token. Never makes one: without a stored token nothing matches.
    pub fn control_api_check_token(&self, given: &str) -> bool {
        let cached = lock(&self.capi().inner).token.clone();
        let expected = match cached {
            Some(t) => Some(t),
            None => match self.secret_store().get(token::SECRET_ID) {
                Ok(Some(t)) if token::is_well_formed(&t) => {
                    lock(&self.capi().inner).token = Some(t.clone());
                    Some(t)
                }
                _ => None,
            },
        };
        let ok = expected.is_some_and(|t| token::matches(&t, given));
        if !ok {
            self.capi().rejected.fetch_add(1, Ordering::Relaxed);
        }
        ok
    }

    /// Attaches a WebSocket client: registers it (shown in Preferences) and starts the feed if
    /// it's the first. Dropping the returned [`Attached`] detaches it.
    pub fn control_api_attach(&self, name: &str, addr: &str) -> Result<Attached, AttachError> {
        if !self.control_api_enabled() {
            return Err(AttachError::Disabled);
        }
        if self.project_dir().is_err() {
            return Err(AttachError::Unavailable);
        }
        let rt = self
            .live()
            .runtime_handle()
            .ok_or(AttachError::Unavailable)?;
        let name: String = name
            .chars()
            .filter(|c| !c.is_control())
            .take(CLIENT_NAME_MAX)
            .collect();
        let (kick_tx, kick_rx) = watch::channel(None);
        let attached = {
            let mut g = lock(&self.capi().inner);
            if g.clients.len() >= CLIENTS_MAX {
                return Err(AttachError::Full);
            }
            g.next_id += 1;
            let id = g.next_id;
            g.clients.insert(
                id,
                Client {
                    view: ControlClientView {
                        id,
                        name: if name.trim().is_empty() {
                            "Unknown".into()
                        } else {
                            name.trim().to_string()
                        },
                        addr: addr.to_string(),
                        since: now_epoch_ms(),
                    },
                    kick: kick_tx,
                },
            );
            if g.feed.is_none() {
                g.feed_gen += 1;
                let gen = g.feed_gen;
                g.feed = Some(FeedHandle::start(
                    &rt,
                    self.downgrade(),
                    self.capi().dirty.clone(),
                    gen,
                    self.capi().tasks.clone(),
                ));
                self.capi().active.store(true, Ordering::Relaxed);
            }
            let Some(feed) = g.feed.as_ref() else {
                return Err(AttachError::Unavailable);
            };
            Attached::new(
                id,
                feed,
                kick_rx,
                DetachGuard {
                    weak: self.downgrade(),
                    id,
                },
            )
        };
        self.control_api_announce();
        Ok(attached)
    }

    fn control_api_detach(&self, id: u64) {
        let last = {
            let mut g = lock(&self.capi().inner);
            g.clients.remove(&id);
            g.clients.is_empty()
        };
        if last {
            // The feed sees no client on this wake and ends.
            self.capi().dirty.notify_one();
        }
        self.control_api_announce();
    }

    /// The feed asks after a wake whether it should end (no client left, or it was replaced).
    pub(crate) fn control_api_feed_done(&self, gen: u64) -> bool {
        let mut g = lock(&self.capi().inner);
        if g.feed.as_ref().is_none_or(|f| f.gen != gen) {
            return true;
        }
        if g.clients.is_empty() {
            g.feed = None;
            g.cause = None;
            self.capi().active.store(false, Ordering::Relaxed);
            return true;
        }
        false
    }

    pub(crate) fn control_api_take_cause(&self) -> Option<String> {
        lock(&self.capi().inner).cause.take()
    }

    /// Feed tasks running (tests: none without clients).
    #[cfg(test)]
    pub fn control_api_tasks(&self) -> usize {
        self.capi().tasks.load(Ordering::Relaxed)
    }

    /// Disconnects one client (Preferences).
    pub fn control_api_disconnect(&self, id: u64) {
        if let Some(c) = lock(&self.capi().inner).clients.get(&id) {
            c.kick.send_replace(Some(close::DISCONNECTED));
        }
    }

    fn control_api_kick_all(&self, code: u16) {
        for c in lock(&self.capi().inner).clients.values() {
            c.kick.send_replace(Some(code));
        }
    }

    /// Another project opened (or none): every client is disconnected (the server restarts).
    fn control_api_project_changed(&self, project: Option<&Project>) {
        let path = project.map(|p| PathBuf::from(&p.path));
        let switched = {
            let mut g = lock(&self.capi().inner);
            let switched = g.project != path;
            g.project = path;
            switched
        };
        if switched {
            self.control_api_kick_all(close::GOING_AWAY);
        }
    }

    pub fn control_api_view(&self) -> ControlApiView {
        let enabled = self.control_api_enabled();
        let clients = lock(&self.capi().inner)
            .clients
            .values()
            .map(|c| c.view.clone())
            .collect();
        ControlApiView { enabled, clients }
    }

    /// Emits `control-api` when the view changed.
    fn control_api_announce(&self) {
        let view = self.control_api_view();
        {
            let mut g = lock(&self.capi().inner);
            if g.announced.as_ref() == Some(&view) {
                return;
            }
            g.announced = Some(view.clone());
        }
        match serde_json::to_value(&view) {
            Ok(v) => self.events().emit(names::CONTROL_API, v),
            Err(e) => tracing::warn!("control-api payload: {e}"),
        }
    }

    /// The `welcome` message.
    pub fn control_api_welcome(&self) -> Value {
        json!({
            "t": "welcome",
            "protocol": PROTOCOL,
            "app": "Kinetra",
            "version": env!("CARGO_PKG_VERSION"),
            "channel": if CANARY_BUILD { "canary" } else { "stable" },
            "now": now_epoch_ms(),
            "project": {"name": self.current_project().map(|p| p.name).unwrap_or_default()},
            "features": FEATURES,
        })
    }

    /// Runs one command (WebSocket or REST) against `tree` (an attached client's current tree)
    /// or a freshly built one. `corr` becomes the `cause` of the diff it produces. Call from the
    /// blocking pool.
    pub fn control_api_command(
        &self,
        corr: Option<&str>,
        tree: Option<Arc<Value>>,
        action: &str,
        args: &Value,
    ) -> Result<(), Refused> {
        if !self.control_api_enabled() {
            return Err(Refused {
                code: "disabled",
                message: "The Control API is off in Kinetra.".into(),
            });
        }
        let tree = tree.unwrap_or_else(|| Arc::new(self.control_api_tree()));
        if let Some(c) = corr {
            lock(&self.capi().inner).cause = Some(c.to_string());
        }
        let result = commands::run(self, &tree, action, args);
        if result.is_err() && corr.is_some() {
            let mut g = lock(&self.capi().inner);
            if g.cause.as_deref() == corr {
                g.cause = None;
            }
        }
        result
    }
}
