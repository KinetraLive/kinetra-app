//! The output server: axum on the open project's port, serving `/screen/{id}`, the embedded
//! renderer under `/renderer/*`, template fonts under `/fonts/*`, project images under
//! `/assets/*`, `/ws` and `/health`. axum types live only in this module.
//!
//! # Lifecycle
//! [`ServerManager`] follows the open project through `core::events::ProjectObserver`: the
//! observer only writes the desired `(port, lan)` into a `watch` channel. ONE long-lived
//! supervisor task (on Tauri's runtime) blocks on `watch::Receiver::changed()` and reconciles:
//! stop the old server (graceful, at most [`STOP_TIMEOUT`]), bind the new one. No project means
//! no server and no listening socket. Nothing polls.
//!
//! # Status
//! `Stopped` → `Starting { port }` → `Running { url, lanUrl }` or `Error { portInUse |
//! bindFailed }`; every change is emitted as `server-status`.

mod assets;
pub mod control;
pub mod osc;
mod routes;

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use crate::core::events::{names, EventSink, ProjectObserver};
use crate::core::live_hub::LiveHub;
use crate::core::model::{Project, ServerErrorCode, ServerStatus};

pub use assets::AssetDir;
pub use osc::OscManager;
pub use routes::{serve, serve_with, ExtraRoutes, PING_INTERVAL};

/// Longest wait for a graceful stop before the server task is aborted.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// Where the server should listen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindTarget {
    pub port: u16,
    /// `0.0.0.0` instead of `127.0.0.1`.
    pub lan: bool,
}

impl BindTarget {
    pub fn addr(self) -> SocketAddr {
        let ip = if self.lan {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        SocketAddr::from((ip, self.port))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Managed Tauri state and the app's `ProjectObserver`. Cheap to clone.
#[derive(Clone)]
pub struct ServerManager {
    inner: Arc<Inner>,
}

struct Inner {
    shared: Arc<Shared>,
    /// Desired target. `None` = no project. Dropped by [`ServerManager::shutdown`], which ends the
    /// supervisor.
    desired: Mutex<Option<watch::Sender<Option<BindTarget>>>>,
    /// Disconnects when the supervisor task ends.
    done: Mutex<Option<mpsc::Receiver<()>>>,
}

/// State the supervisor shares with the manager.
struct Shared {
    status: Mutex<ServerStatus>,
    events: Arc<dyn EventSink>,
    hub: Arc<LiveHub>,
    /// The open project for `/assets/*` (set by the observer, read per request).
    assets: AssetDir,
    /// Told about every status change (display outputs follow the port).
    listeners: Mutex<Vec<StatusListener>>,
    /// Routes merged into every server started (the Control API).
    extra: Mutex<Option<ExtraRoutes>>,
}

/// Called with the new status after each change, on the supervisor task. Must not block.
pub type StatusListener = Arc<dyn Fn(&ServerStatus) + Send + Sync>;

impl Shared {
    /// Stores and emits `status` if it changed.
    fn set_status(&self, status: ServerStatus) {
        {
            let mut g = lock(&self.status);
            if *g == status {
                return;
            }
            *g = status.clone();
        }
        match serde_json::to_value(&status) {
            Ok(v) => self.events.emit(names::SERVER_STATUS, v),
            Err(e) => tracing::warn!("server-status payload: {e}"),
        }
        let listeners = lock(&self.listeners).clone();
        for l in listeners {
            l(&status);
        }
    }
}

impl ServerManager {
    /// Starts the supervisor task (idle until a project opens). Call where Tauri's async runtime
    /// is available (it is created on first use).
    pub fn new(events: Arc<dyn EventSink>, hub: Arc<LiveHub>) -> Self {
        let shared = Arc::new(Shared {
            status: Mutex::new(ServerStatus::Stopped),
            events,
            hub,
            assets: AssetDir::default(),
            listeners: Mutex::new(Vec::new()),
            extra: Mutex::new(None),
        });
        let (tx, rx) = watch::channel(None);
        let (done_tx, done_rx) = mpsc::channel();
        tauri::async_runtime::spawn(supervise(shared.clone(), rx, done_tx));
        Self {
            inner: Arc::new(Inner {
                shared,
                desired: Mutex::new(Some(tx)),
                done: Mutex::new(Some(done_rx)),
            }),
        }
    }

    /// Routes merged into every server started from now on (the app adds the Control API).
    pub fn set_extra_routes(&self, extra: ExtraRoutes) {
        *lock(&self.inner.shared.extra) = Some(extra);
    }

    /// Current status (what the `server-status` event last reported).
    pub fn status(&self) -> ServerStatus {
        lock(&self.inner.shared.status).clone()
    }

    /// Adds a status listener and tells it the current status.
    pub fn on_status(&self, listener: StatusListener) {
        lock(&self.inner.shared.listeners).push(listener.clone());
        listener(&self.status());
    }

    /// Sets where the server should listen (`None` stops it). Never blocks; the supervisor
    /// applies it. Does nothing if the target is unchanged.
    pub fn set_target(&self, target: Option<BindTarget>) {
        if let Some(tx) = lock(&self.inner.desired).as_ref() {
            tx.send_if_modified(|cur| {
                if *cur == target {
                    false
                } else {
                    *cur = target;
                    true
                }
            });
        }
    }

    /// Stops the server and the supervisor, waiting at most about [`STOP_TIMEOUT`]. For app
    /// exit; call from a blocking (non-async) context. Later project changes are ignored.
    pub fn shutdown(&self) {
        drop(lock(&self.inner.desired).take());
        let done = lock(&self.inner.done).take();
        if let Some(done) = done {
            // Disconnected once the supervisor has stopped the server and exited.
            let _ = done.recv_timeout(STOP_TIMEOUT + Duration::from_millis(100));
        }
    }
}

/// Called by `AppState` after every project change, possibly while its `io` mutex is held by a
/// caller further up. That is safe because this only writes into a `watch` channel: it never
/// blocks, never awaits and never calls back into `AppState`.
impl ProjectObserver for ServerManager {
    fn project_changed(&self, project: Option<&Project>) {
        self.inner
            .shared
            .assets
            .set_project(project.map(|p| std::path::PathBuf::from(&p.path)));
        self.set_target(project.map(|p| BindTarget {
            port: p.settings.port,
            lan: p.settings.lan_access,
        }));
    }
}

/// A started server.
struct Running {
    target: BindTarget,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Running {
    /// Graceful stop, bounded by [`STOP_TIMEOUT`]; the task is aborted after that.
    async fn stop(self) {
        let Running {
            target,
            stop,
            mut task,
        } = self;
        let _ = stop.send(());
        if tokio::time::timeout(STOP_TIMEOUT, &mut task).await.is_err() {
            tracing::warn!("output server on port {} did not stop in time", target.port);
            task.abort();
            let _ = task.await;
        }
    }
}

/// The supervisor: blocks on `changed()` and reconciles the running server to the target.
/// `_done` is dropped when it returns, which wakes [`ServerManager::shutdown`].
async fn supervise(
    shared: Arc<Shared>,
    mut rx: watch::Receiver<Option<BindTarget>>,
    _done: mpsc::Sender<()>,
) {
    let mut running: Option<Running> = None;
    loop {
        let want = *rx.borrow_and_update();
        if let Some(r) = running.take() {
            if Some(r.target) == want {
                running = Some(r);
            } else {
                r.stop().await;
            }
        }
        match want {
            None => shared.set_status(ServerStatus::Stopped),
            Some(_) if running.is_some() => {}
            Some(target) => running = start(&shared, target).await,
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
    if let Some(r) = running {
        r.stop().await;
    }
    shared.set_status(ServerStatus::Stopped);
}

async fn start(shared: &Shared, target: BindTarget) -> Option<Running> {
    let port = target.port;
    shared.set_status(ServerStatus::Starting { port });
    let listener = match TcpListener::bind(target.addr()).await {
        Ok(l) => l,
        Err(e) => {
            let (code, message) = if e.kind() == std::io::ErrorKind::AddrInUse {
                (
                    ServerErrorCode::PortInUse,
                    format!("Port {port} is already in use"),
                )
            } else {
                (
                    ServerErrorCode::BindFailed,
                    format!("Could not start the output server on port {port}: {e}"),
                )
            };
            tracing::warn!("{message}");
            shared.set_status(ServerStatus::Error {
                code,
                port,
                message,
            });
            return None;
        }
    };
    let (stop, stop_rx) = oneshot::channel::<()>();
    let hub = shared.hub.clone();
    let assets = shared.assets.clone();
    let extra = lock(&shared.extra).clone();
    let task = tokio::spawn(async move {
        let shutdown = async move {
            let _ = stop_rx.await;
        };
        if let Err(e) = serve_with(listener, hub, assets, extra, shutdown).await {
            tracing::error!("output server failed: {e}");
        }
    });
    tracing::info!("output server listening on {}", target.addr());
    shared.set_status(ServerStatus::Running {
        port,
        url: format!("http://127.0.0.1:{port}"),
        lan_url: target
            .lan
            .then(lan_ipv4)
            .flatten()
            .map(|ip| format!("http://{ip}:{port}")),
    });
    Some(Running { target, stop, task })
}

/// This machine's LAN IPv4: the source address the OS would use to reach a TEST-NET address.
/// A UDP `connect` sends nothing. `None` without a usable route.
pub fn lan_ipv4() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
