//! The OSC listener and feedback sender (UDP). The address scheme, accept rule, flow patterns and
//! the on-air diff are in `core::osc`; this module owns the sockets and the rosc codec.
//!
//! # Lifecycle
//! [`OscManager`] is a `ProjectObserver`. With no project, or OSC disabled in the project's
//! settings, there is no task and no socket. Otherwise ONE tokio task owns the UDP socket and
//! blocks on `recv_from` (bound to `127.0.0.1:<port>`, or `0.0.0.0:<port>` with the project's LAN
//! access on; IPv4 only). A port / LAN / enabled change stops that task (a `oneshot`) and spawns
//! the next one, which waits for the old one to release the port before it binds. The allowlist
//! is read per packet, so changing it never rebinds.
//!
//! Each accepted datagram is decoded (bundles flattened, in order) and run on the blocking pool
//! (`AppState::osc_message`: `run_control`, then `trigger.osc` flows); the recv loop awaits it
//! before the next datagram, so messages run in arrival order. Rejected senders are dropped
//! with a debug log (at most one per 5 s).
//!
//! # Status
//! `off` → `listening { port, lan }` or `error { portInUse | bindFailed }`, emitted as
//! `osc-status` on every change.
//!
//! # Feedback
//! With `OscSettings.feedback` set (and OSC enabled), a change listener on `AppState` sends
//! `/kinetra/layer/<screenId>/<layerId>/onair <int 0|1>` for every layer whose on-air state
//! changed (diffed in `core::osc::OnAirDiff`), and `/kinetra/flow/<flowId>/running <int 0|1>` on
//! flow instance transitions. The full on-air state goes out once when feedback is configured
//! and whenever a project opens. The socket is a non-blocking std UDP socket (a full send buffer
//! drops the message), sent from the thread that made the change: no task, no queue. The host
//! is resolved once, on the blocking pool, when the target is set.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};
use std::time::{Duration, Instant};

use rosc::{OscPacket, OscType};
use tokio::net::UdpSocket;
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::core::events::{names, EventSink, ProjectObserver};
use crate::core::model::{OscFeedback, OscStatus, Project, ServerErrorCode};
use crate::core::osc::{accepts, parse_allow, OnAirDiff, OscArg, OscMessage};
use crate::core::state::{AppState, StateChange, WeakState};

/// Receive buffer (an OSC datagram larger than this is dropped).
const RECV_BUF: usize = 16 * 1024;
/// Most messages run from one datagram (bundles included).
const MESSAGES_PER_PACKET: usize = 256;
/// Deepest bundle nesting decoded.
const BUNDLE_DEPTH: usize = 8;
/// At most one "dropped a packet" log per this long.
const DROP_LOG_EVERY: Duration = Duration::from_secs(5);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where the listener binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BindKey {
    port: u16,
    lan: bool,
}

impl BindKey {
    fn addr(self) -> SocketAddr {
        let ip = if self.lan {
            Ipv4Addr::UNSPECIFIED
        } else {
            Ipv4Addr::LOCALHOST
        };
        SocketAddr::from((ip, self.port))
    }
}

/// Who may send (read per packet).
#[derive(Debug, Default)]
struct Filter {
    lan: bool,
    allow: Vec<IpAddr>,
}

struct Running {
    key: BindKey,
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

/// Feedback target and what was sent.
#[derive(Default)]
struct FbOut {
    /// The configured target (as in the settings).
    target: Option<OscFeedback>,
    /// Resolved socket + address; `None` until resolved (or when off).
    sock: Option<(std::net::UdpSocket, SocketAddr)>,
    diff: OnAirDiff,
    /// Bumped on every target change (a late resolve of an old target is ignored).
    gen: u64,
}

/// Managed Tauri state and a `ProjectObserver`. Cheap to clone.
#[derive(Clone)]
pub struct OscManager {
    inner: Arc<Inner>,
}

struct Inner {
    rt: Handle,
    state: WeakState,
    events: Arc<dyn EventSink>,
    run: Mutex<Option<Running>>,
    filter: RwLock<Filter>,
    status: Mutex<OscStatus>,
    /// The project the feedback state belongs to (a different one re-sends the full state).
    project: Mutex<Option<String>>,
    /// Feedback configured and resolved (the change listener returns at once otherwise).
    fb_active: AtomicBool,
    fb: Mutex<FbOut>,
    /// Datagrams accepted / dropped by the accept rule (tests and diagnostics).
    accepted: AtomicU64,
    dropped: AtomicU64,
}

impl OscManager {
    /// Creates the manager (no task until a project with OSC enabled is reported) and adds its
    /// feedback listener to `state`. Register it with `state.add_project_observer`.
    pub fn new(state: &AppState, rt: Handle) -> Self {
        let inner = Arc::new(Inner {
            rt,
            state: state.downgrade(),
            events: state.events().clone(),
            run: Mutex::new(None),
            filter: RwLock::new(Filter::default()),
            status: Mutex::new(OscStatus::Off),
            project: Mutex::new(None),
            fb_active: AtomicBool::new(false),
            fb: Mutex::new(FbOut::default()),
            accepted: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        });
        let weak = Arc::downgrade(&inner);
        state.add_change_listener(Arc::new(move |st, change| {
            if let Some(inner) = weak.upgrade() {
                inner.feedback(st, change);
            }
        }));
        Self { inner }
    }

    /// What `osc-status` last reported.
    pub fn status(&self) -> OscStatus {
        lock(&self.inner.status).clone()
    }

    /// Whether a listener task exists (tests, budget checks).
    pub fn is_running(&self) -> bool {
        lock(&self.inner.run)
            .as_ref()
            .is_some_and(|r| !r.task.is_finished())
    }

    /// Datagrams `(accepted, dropped)` by the accept rule so far.
    pub fn packet_counts(&self) -> (u64, u64) {
        (
            self.inner.accepted.load(Ordering::Relaxed),
            self.inner.dropped.load(Ordering::Relaxed),
        )
    }

    /// Stops the listener (app exit). Later project changes may start it again.
    pub fn shutdown(&self) {
        if let Some(r) = lock(&self.inner.run).take() {
            let _ = r.stop.send(());
        }
        self.inner.set_feedback(None);
    }
}

impl ProjectObserver for OscManager {
    /// Never blocks: updates the filter, swaps the listener task (spawn + oneshot) and starts
    /// a feedback resolve on the blocking pool when the target changed.
    fn project_changed(&self, project: Option<&Project>) {
        let inner = &self.inner;
        let osc = project
            .and_then(|p| p.settings.osc.clone())
            .filter(|o| o.enabled);
        let lan = project.is_some_and(|p| p.settings.lan_access);
        {
            let mut f = inner.filter.write().unwrap_or_else(PoisonError::into_inner);
            f.lan = lan;
            f.allow = osc
                .as_ref()
                .map_or_else(Vec::new, |o| parse_allow(&o.allow));
        }
        // A different project: the next feedback update sends the full state.
        let path = project.map(|p| p.path.clone());
        let new_project = {
            let mut g = lock(&inner.project);
            let changed = *g != path;
            *g = path;
            changed
        };
        if new_project {
            lock(&inner.fb).diff.reset();
        }
        inner.reconcile(osc.as_ref().map(|o| BindKey { port: o.port, lan }));
        let target = osc.and_then(|o| o.feedback);
        let unchanged = lock(&inner.fb).target == target;
        if !unchanged {
            inner.set_feedback(target);
        } else if new_project && inner.fb_active.load(Ordering::Acquire) {
            // The full state of the new project (off this caller, which may hold `io`).
            let me = inner.clone();
            inner.rt.spawn_blocking(move || {
                if let Some(st) = me.state.upgrade() {
                    me.feedback(&st, StateChange::Live);
                }
            });
        }
    }
}

impl Inner {
    fn set_status(&self, status: OscStatus) {
        {
            let mut g = lock(&self.status);
            if *g == status {
                return;
            }
            *g = status.clone();
        }
        match serde_json::to_value(&status) {
            Ok(v) => self.events.emit(names::OSC_STATUS, v),
            Err(e) => tracing::warn!("osc-status payload: {e}"),
        }
    }

    /// Makes the listener match `want` (see the module docs).
    fn reconcile(self: &Arc<Self>, want: Option<BindKey>) {
        let mut run = lock(&self.run);
        if run.as_ref().map(|r| r.key) == want
            && run.as_ref().is_some_and(|r| !r.task.is_finished())
        {
            return;
        }
        let old = run.take().map(|r| {
            let _ = r.stop.send(());
            r.task
        });
        match want {
            None => {
                drop(run);
                self.set_status(OscStatus::Off);
            }
            Some(key) => {
                let (stop, stop_rx) = oneshot::channel();
                let task = self.rt.spawn(listen(self.clone(), key, old, stop_rx));
                *run = Some(Running { key, stop, task });
            }
        }
    }

    /// Whether a datagram from `src` is accepted.
    fn accepts(&self, src: IpAddr) -> bool {
        let f = self.filter.read().unwrap_or_else(PoisonError::into_inner);
        accepts(src, f.lan, &f.allow)
    }

    // ---- Feedback -----------------------------------------------------------------------------

    /// Sets (or clears) the feedback target; resolving happens on the blocking pool.
    fn set_feedback(self: &Arc<Self>, target: Option<OscFeedback>) {
        let gen = {
            let mut fb = lock(&self.fb);
            fb.target = target.clone();
            fb.sock = None;
            fb.diff.reset();
            fb.gen += 1;
            self.fb_active.store(false, Ordering::Release);
            fb.gen
        };
        let Some(target) = target else { return };
        let me = self.clone();
        self.rt.spawn_blocking(move || {
            let sock = match open_feedback(&target) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("OSC feedback to {}:{}: {e}", target.host, target.port);
                    return;
                }
            };
            {
                let mut fb = lock(&me.fb);
                if fb.gen != gen {
                    return;
                }
                fb.sock = Some(sock);
                fb.diff.reset();
                me.fb_active.store(true, Ordering::Release);
            }
            // The full state once.
            if let Some(st) = me.state.upgrade() {
                me.feedback(&st, StateChange::Live);
            }
        });
    }

    /// The change listener: sends what changed. Returns at once without feedback.
    fn feedback(&self, st: &AppState, change: StateChange<'_>) {
        if !self.fb_active.load(Ordering::Acquire) {
            return;
        }
        let mut fb = lock(&self.fb);
        match change {
            StateChange::Live => {
                // Read under the feedback lock so concurrent changes go out in order.
                let now = st.live().user_layers_on_air();
                let changes = fb.diff.update(now);
                let Some((sock, to)) = fb.sock.as_ref() else {
                    return;
                };
                for (s, l, on) in changes {
                    send(sock, *to, format!("/kinetra/layer/{s}/{l}/onair"), on);
                }
            }
            StateChange::FlowRun { id, running } => {
                if let Some((sock, to)) = fb.sock.as_ref() {
                    send(sock, *to, format!("/kinetra/flow/{id}/running"), running);
                }
            }
            // Feedback follows on air in Program, not the out-animation.
            StateChange::Visible | StateChange::Preview => {}
        }
    }
}

/// A non-blocking UDP socket for feedback to `target` (resolved here: may block on DNS). Bound
/// to loopback when the target is on this computer (no firewall prompt).
fn open_feedback(target: &OscFeedback) -> std::io::Result<(std::net::UdpSocket, SocketAddr)> {
    // IPv4 first: "localhost" often resolves to ::1 first, and most OSC apps listen on IPv4.
    let addrs: Vec<SocketAddr> = (target.host.as_str(), target.port)
        .to_socket_addrs()?
        .collect();
    let to = addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| std::io::Error::other("no address"))?;
    let local: SocketAddr = match (to.ip().is_loopback(), to.is_ipv4()) {
        (true, true) => (Ipv4Addr::LOCALHOST, 0).into(),
        (true, false) => (std::net::Ipv6Addr::LOCALHOST, 0).into(),
        (false, true) => (Ipv4Addr::UNSPECIFIED, 0).into(),
        (false, false) => (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
    };
    let sock = std::net::UdpSocket::bind(local)?;
    sock.set_nonblocking(true)?;
    Ok((sock, to))
}

/// Sends `<address> <int 0|1>`; a full buffer or an error drops it (debug log).
fn send(sock: &std::net::UdpSocket, to: SocketAddr, address: String, on: bool) {
    let packet = OscPacket::Message(rosc::OscMessage {
        addr: address,
        args: vec![OscType::Int(i32::from(on))],
    });
    match rosc::encoder::encode(&packet) {
        Ok(bytes) => {
            if let Err(e) = sock.send_to(&bytes, to) {
                tracing::debug!("OSC feedback not sent: {e}");
            }
        }
        Err(e) => tracing::debug!("OSC feedback encode: {e}"),
    }
}

// ---- Listener ----------------------------------------------------------------------------------

async fn listen(
    inner: Arc<Inner>,
    key: BindKey,
    old: Option<JoinHandle<()>>,
    mut stop: oneshot::Receiver<()>,
) {
    if let Some(old) = old {
        let _ = old.await;
    }
    // Replaced or stopped while the old listener was shutting down.
    if !matches!(stop.try_recv(), Err(oneshot::error::TryRecvError::Empty)) {
        return;
    }
    let sock = match UdpSocket::bind(key.addr()).await {
        Ok(s) => s,
        Err(e) => {
            let port = key.port;
            let (code, message) = if e.kind() == std::io::ErrorKind::AddrInUse {
                (
                    ServerErrorCode::PortInUse,
                    format!("Port {port} is already in use"),
                )
            } else {
                (
                    ServerErrorCode::BindFailed,
                    format!("Could not listen for OSC on port {port}: {e}"),
                )
            };
            tracing::warn!("{message}");
            inner.set_status(OscStatus::Error {
                code,
                port,
                message,
            });
            return;
        }
    };
    tracing::info!("OSC listening on udp {}", key.addr());
    inner.set_status(OscStatus::Listening {
        port: key.port,
        lan: key.lan,
    });
    let mut buf = vec![0u8; RECV_BUF];
    let mut drop_logged: Option<Instant> = None;
    loop {
        let (n, from) = tokio::select! {
            _ = &mut stop => break,
            r = sock.recv_from(&mut buf) => match r {
                Ok(x) => x,
                Err(e) => {
                    // Windows reports an ICMP "port unreachable" or a datagram larger than the
                    // buffer as an error on the next receive; neither ends the listener.
                    tracing::debug!("OSC receive: {e}");
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
            },
        };
        if !inner.accepts(from.ip()) {
            inner.dropped.fetch_add(1, Ordering::Relaxed);
            if drop_logged.is_none_or(|t| t.elapsed() >= DROP_LOG_EVERY) {
                drop_logged = Some(Instant::now());
                tracing::debug!("OSC packet from {} dropped (not allowed)", from.ip());
            }
            continue;
        }
        inner.accepted.fetch_add(1, Ordering::Relaxed);
        let messages = decode(&buf[..n]);
        if messages.is_empty() {
            continue;
        }
        let Some(st) = inner.state.upgrade() else {
            break;
        };
        // Off the async loop; awaited so messages run in arrival order.
        let _ = tokio::task::spawn_blocking(move || {
            for m in &messages {
                st.osc_message(m);
            }
        })
        .await;
    }
}

/// The messages of a datagram, bundles flattened in order (≤ [`MESSAGES_PER_PACKET`]). An
/// undecodable datagram gives none.
pub fn decode(bytes: &[u8]) -> Vec<OscMessage> {
    let mut out = Vec::new();
    match rosc::decoder::decode_udp(bytes) {
        Ok((_, packet)) => flatten(packet, 0, &mut out),
        Err(e) => tracing::debug!("OSC decode: {e}"),
    }
    out
}

fn flatten(packet: OscPacket, depth: usize, out: &mut Vec<OscMessage>) {
    match packet {
        OscPacket::Message(m) => {
            if out.len() < MESSAGES_PER_PACKET {
                out.push(OscMessage {
                    address: m.addr,
                    args: m.args.into_iter().map(arg).collect(),
                });
            }
        }
        OscPacket::Bundle(b) if depth < BUNDLE_DEPTH => {
            for p in b.content {
                flatten(p, depth + 1, out);
            }
        }
        OscPacket::Bundle(_) => {}
    }
}

fn arg(t: OscType) -> OscArg {
    match t {
        OscType::Int(i) => OscArg::Int(i64::from(i)),
        OscType::Long(i) => OscArg::Int(i),
        OscType::Float(f) => OscArg::Float(f),
        OscType::Double(d) => OscArg::Double(d),
        OscType::String(s) => OscArg::Text(s),
        OscType::Char(c) => OscArg::Text(c.to_string()),
        OscType::Bool(b) => OscArg::Bool(b),
        _ => OscArg::Other,
    }
}

#[cfg(test)]
mod tests;
