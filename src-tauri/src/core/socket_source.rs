//! Live data sources (`kind: "socket"`, shown as "Live data (WebSocket)"): a WebSocket on this
//! computer or the local network whose JSON messages are mapped into a table with the API
//! sources' mapping (`api_map::map_json_value`). Plan: kinetra-private docs/SOCKET_SOURCE_PLAN.md.
//!
//! - **One tokio task per source in use**, none otherwise (the API sources' rule): a layer bound
//!   to it, an armed deployed flow naming it, or its open editor (`watch_socket_source`,
//!   counted). Paused, held or without a URL: no task. Started and stopped by
//!   [`AppState::socket_reconcile`], which runs wherever API polling is reconciled.
//! - **Connect:** the host is resolved and every address must be on this computer or the local
//!   network ([`is_local_ip`]); the TCP connection goes to an address that was checked, then the
//!   WebSocket handshake (TLS for `wss://`). The "Send on connect" text goes out once. Refused or
//!   closed: retried after 1 s, 2 s, 5 s, then every 5 s; a connection that lasted 30 s starts the
//!   backoff over. A bad URL, an address outside the network, a refused upgrade or a message over
//!   1 MB holds the source (no task) until Retry or a save.
//! - **Messages:** text frames only; every JSON object in a frame counts, the last one that
//!   passes the filter wins. At most 200 messages a second are read; the rest are dropped and
//!   counted. Mapping runs on the task as each message arrives, into a latest-wins slot.
//! - **Pacing:** the slot is applied at most `rate` times a second through the source path
//!   ([`AppState::socket_put_rows`]: bound screens re-applied, auto-advance, flow triggers). An
//!   unchanged table does nothing. `source-changed` for new rows at most 4 times a second; status
//!   changes at once.
//! - **Rows are live:** in memory only (the file holds the settings, `sources::on_disk`); a
//!   reconnect keeps the last ones; Kinetra starts empty.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::core::api_map;
use crate::core::fsx;
use crate::core::model::{
    ApiFormat, ApiMapping, SocketFilter, SocketState, SocketStatus, SocketTestResult, SourceDoc,
    SourceKind, SourceSummary, API_PREVIEW_MAX, API_TEST_ROWS, API_URL_MAX,
    SOCKET_FILTER_VALUE_MAX, SOCKET_MESSAGE_MAX, SOCKET_RATE_DEFAULT, SOCKET_RATE_MAX,
    SOCKET_RATE_MIN, SOCKET_SEND_MAX, SOCKET_TEST_MESSAGES,
};
use crate::core::protocol::Fields;
use crate::core::screens;
use crate::core::sources;
use crate::core::state::{AppState, WeakState};
use crate::error::{AppError, AppResult};

/// Connected without a message this long: `stale`.
pub const SOCKET_STALE: Duration = Duration::from_secs(5);
const BACKOFF: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
/// A connection that lasted this long starts the backoff over.
const BACKOFF_RESET: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Messages read a second; more are dropped (and counted).
pub const SOCKET_READ_PER_SEC: u32 = 200;
/// `source-changed` for new rows at most this often.
const UI_EVERY: Duration = Duration::from_millis(250);
/// How long a test listens (it ends sooner once it has its messages).
const TEST_FOR: Duration = Duration::from_secs(10);
/// After the first matching message a test waits this long for more, then answers.
const TEST_AFTER_MATCH: Duration = Duration::from_secs(2);

const NOT_LOCAL: &str = "Live data comes from this computer or your local network.";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

static GEN: AtomicU64 = AtomicU64::new(1);

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

// ---- Settings ----------------------------------------------------------------------------------

/// An address on this computer or the local network: loopback, the private IPv4 ranges
/// (10/8, 172.16/12, 192.168/16), shared address space (100.64/10: Tailscale and similar VPNs,
/// owner 2026-10-29), link-local (169.254/16, fe80::/10) and unique local IPv6 (fc00::/7).
/// IPv4-mapped IPv6 addresses count as their IPv4 address.
pub fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (a == 100 && (64..=127).contains(&b))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// A parsed `ws://` / `wss://` URL: the host (without IPv6 brackets) and the port.
fn parse_url(url: &str) -> Result<(reqwest::Url, String, u16), String> {
    if url.len() > API_URL_MAX {
        return Err(format!(
            "The URL is too long (at most {API_URL_MAX} characters)"
        ));
    }
    let u = reqwest::Url::parse(url).map_err(|_| "The URL is not valid".to_string())?;
    let default_port = match u.scheme() {
        "ws" => 80,
        "wss" => 443,
        _ => return Err("The URL must start with ws:// or wss://".into()),
    };
    let host = match u.host_str() {
        // IPv6 literals come in brackets.
        Some(h) if !h.is_empty() => h.trim_start_matches('[').trim_end_matches(']').to_string(),
        _ => return Err("The URL is not valid".into()),
    };
    let port = u.port().unwrap_or(default_port);
    Ok((u, host, port))
}

/// A URL (or empty: not configured yet) that may be saved: `ws://` or `wss://`, and an IP
/// address on this computer or the local network (host names are checked at every connect).
pub fn check_url(url: &str) -> Result<(), String> {
    if url.is_empty() {
        return Ok(());
    }
    let (_, host, _) = parse_url(url)?;
    match host.parse::<IpAddr>() {
        Ok(ip) if !is_local_ip(ip) => Err(NOT_LOCAL.into()),
        _ => Ok(()),
    }
}

/// Hard checks of a live data source's settings; normalizes in place: the URL and the filter
/// pointer are trimmed, an empty filter or "Send on connect" becomes null, labels are trimmed.
pub fn validate_settings(
    url: &mut String,
    mapping: &mut ApiMapping,
    filter: &mut Option<SocketFilter>,
    send_on_connect: &mut Option<String>,
    rate: u32,
) -> Result<(), String> {
    *url = url.trim().to_string();
    check_url(url)?;
    api_map::validate_mapping(ApiFormat::Json, mapping)?;
    if let Some(f) = filter {
        f.pointer = f.pointer.trim().to_string();
        if f.pointer.is_empty() && f.equals.is_empty() {
            *filter = None;
        } else if f.pointer.is_empty() || !api_map::is_pointer(&f.pointer) {
            return Err(format!(
                "Filter: \"{}\" is not a JSON Pointer (e.g. /type)",
                f.pointer
            ));
        } else if f.equals.len() > SOCKET_FILTER_VALUE_MAX {
            return Err(format!(
                "Filter: the value is too long (at most {SOCKET_FILTER_VALUE_MAX} characters)"
            ));
        }
    }
    if send_on_connect.as_deref().is_some_and(str::is_empty) {
        *send_on_connect = None;
    }
    if send_on_connect
        .as_deref()
        .is_some_and(|s| s.len() > SOCKET_SEND_MAX)
    {
        return Err(format!(
            "\"Send on connect\" is too long (at most {} KB)",
            SOCKET_SEND_MAX / 1024
        ));
    }
    if !(SOCKET_RATE_MIN..=SOCKET_RATE_MAX).contains(&rate) {
        return Err(format!(
            "Updates must be {SOCKET_RATE_MIN} to {SOCKET_RATE_MAX} a second"
        ));
    }
    Ok(())
}

/// The value `filter` reads, as text (strings as they are, other values as JSON).
fn filter_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Whether a message passes the filter (no filter: every message).
pub fn passes(filter: Option<&SocketFilter>, msg: &Value) -> bool {
    match filter {
        None => true,
        Some(f) => api_map::lookup(msg, &f.pointer, &mut api_map::Unpacked::default())
            .is_some_and(|v| filter_text(&v) == f.equals),
    }
}

/// Every JSON object or value in a frame, in order (a frame may hold several). `None` when the
/// frame isn't JSON at all.
pub fn frame_values(text: &str) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    for v in serde_json::Deserializer::from_str(text).into_iter::<Value>() {
        match v {
            Ok(v) => out.push(v),
            Err(_) if out.is_empty() => return None,
            Err(_) => break,
        }
    }
    (!out.is_empty()).then_some(out)
}

/// A live data source's rows and index come from its connection: a client save or a file reload
/// keeps those in memory.
pub fn keep_live(doc: &mut SourceDoc, old: &SourceDoc) {
    if let (
        SourceKind::Socket { rows, index, .. },
        SourceKind::Socket {
            rows: have,
            index: at,
            ..
        },
    ) = (&mut doc.meta.kind, &old.meta.kind)
    {
        rows.clone_from(have);
        *index = *at;
    }
}

/// The settings a connection runs with.
#[derive(Clone)]
struct Settings {
    url: String,
    mapping: ApiMapping,
    filter: Option<SocketFilter>,
    send_on_connect: Option<String>,
    rate: u32,
}

fn settings_of(doc: &SourceDoc) -> Option<Settings> {
    let SourceKind::Socket {
        url,
        mapping,
        filter,
        send_on_connect,
        rate,
        ..
    } = &doc.meta.kind
    else {
        return None;
    };
    Some(Settings {
        url: url.clone(),
        mapping: mapping.clone(),
        filter: filter.clone(),
        send_on_connect: send_on_connect.clone(),
        rate: (*rate).clamp(SOCKET_RATE_MIN, SOCKET_RATE_MAX),
    })
}

fn settings_sig(s: &Settings) -> u64 {
    let mut h = DefaultHasher::new();
    s.url.hash(&mut h);
    serde_json::to_string(&s.mapping)
        .unwrap_or_default()
        .hash(&mut h);
    s.filter
        .as_ref()
        .map(|f| (&f.pointer, &f.equals))
        .hash(&mut h);
    s.send_on_connect.hash(&mut h);
    s.rate.hash(&mut h);
    h.finish()
}

/// One live data source as the reconcile pass sees it (computed under the source store lock).
pub(crate) struct SocketMeta {
    id: String,
    has_url: bool,
    paused: bool,
    sig: u64,
}

impl SocketMeta {
    pub(crate) fn of(doc: &SourceDoc) -> Option<Self> {
        let s = settings_of(doc)?;
        let SourceKind::Socket { paused, .. } = &doc.meta.kind else {
            return None;
        };
        Some(Self {
            id: doc.id.clone(),
            has_url: !s.url.is_empty(),
            paused: *paused,
            sig: settings_sig(&s),
        })
    }
}

// ---- The service -------------------------------------------------------------------------------

struct Task {
    gen: u64,
    sig: u64,
    abort: AbortHandle,
    wake: Arc<Notify>,
}

#[derive(Default)]
struct Track {
    status: SocketStatus,
    /// A bad URL, an address outside the network, a refused upgrade, a message too big: no task
    /// until Retry or a save.
    held: bool,
    sig: u64,
}

#[derive(Default)]
struct Inner {
    tasks: HashMap<String, Task>,
    tracks: HashMap<String, Track>,
    watch: HashMap<String, u32>,
}

/// The open project's live data connections (see the module docs).
#[derive(Default)]
pub struct SocketSources {
    inner: Mutex<Inner>,
    /// Tests: every backoff step, in ms (0 = the real ones).
    test_backoff_ms: AtomicU64,
    /// Row updates applied (all sources), for tests and measurements.
    applied: AtomicU64,
}

impl SocketSources {
    /// Row updates applied so far (all sources).
    pub fn applied(&self) -> u64 {
        self.applied.load(Ordering::Relaxed)
    }

    /// Number of connection tasks alive.
    pub fn task_count(&self) -> usize {
        lock(&self.inner).tasks.len()
    }

    /// Test timing: every reconnect wait.
    #[doc(hidden)]
    pub fn set_test_backoff(&self, d: Option<Duration>) {
        self.test_backoff_ms
            .store(d.map_or(0, |d| d.as_millis() as u64), Ordering::Relaxed);
    }

    fn backoff(&self, attempt: usize) -> Duration {
        match self.test_backoff_ms.load(Ordering::Relaxed) {
            0 => BACKOFF[attempt.min(BACKOFF.len() - 1)],
            ms => Duration::from_millis(ms),
        }
    }

    /// Stops every connection and forgets everything (project closed or reloaded).
    pub(crate) fn stop_all(&self) {
        let mut g = lock(&self.inner);
        for (_, t) in g.tasks.drain() {
            t.abort.abort();
        }
        g.tracks.clear();
        g.watch.clear();
    }

    fn is_current(&self, id: &str, gen: u64) -> bool {
        lock(&self.inner)
            .tasks
            .get(id)
            .is_some_and(|t| t.gen == gen)
    }

    fn watched(&self, id: &str) -> bool {
        lock(&self.inner).watch.get(id).is_some_and(|n| *n > 0)
    }
}

/// Why a connection ended or didn't start.
enum Fail {
    /// Try again after the backoff, with this reason.
    Retry(String),
    /// Held until Retry or a save.
    Hold(String),
    /// The task is no longer current (stopped, settings changed): end quietly.
    Gone,
}

// ---- Connecting --------------------------------------------------------------------------------

/// Resolves the URL's host, checks every address, connects TCP to the first that answers, then
/// the WebSocket handshake.
async fn connect(url: &str) -> Result<Ws, Fail> {
    let (u, host, port) = parse_url(url).map_err(Fail::Hold)?;
    let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => match tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        {
            Ok(Ok(a)) => a.collect(),
            Ok(Err(_)) => return Err(Fail::Retry(format!("Kinetra can't find {host}"))),
            Err(_) => return Err(Fail::Retry(format!("Looking up {host} timed out"))),
        },
    };
    if addrs.is_empty() {
        return Err(Fail::Retry(format!("Kinetra can't find {host}")));
    }
    // Every address the name points to must be local, so no answer can lead outside.
    if addrs.iter().any(|a| !is_local_ip(a.ip())) {
        return Err(Fail::Hold(NOT_LOCAL.into()));
    }
    let mut last = format!("Nothing answers at {host}:{port}");
    let mut stream = None;
    for a in &addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(a)).await {
            Ok(Ok(s)) => {
                stream = Some(s);
                break;
            }
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionRefused => {}
            Ok(Err(e)) => last = format!("Can't connect to {host}:{port}: {e}"),
            Err(_) => last = format!("Connecting to {host}:{port} timed out"),
        }
    }
    let Some(stream) = stream else {
        return Err(Fail::Retry(last));
    };
    let _ = stream.set_nodelay(true);
    let cfg = WebSocketConfig::default()
        .max_message_size(Some(SOCKET_MESSAGE_MAX))
        .max_frame_size(Some(SOCKET_MESSAGE_MAX));
    let hs = tokio_tungstenite::client_async_tls_with_config(u.as_str(), stream, Some(cfg), None);
    match tokio::time::timeout(CONNECT_TIMEOUT, hs).await {
        Err(_) => Err(Fail::Retry("The WebSocket handshake timed out".into())),
        Ok(Ok((ws, _))) => Ok(ws),
        Ok(Err(WsError::Http(resp))) => Err(Fail::Hold(format!(
            "The server answered HTTP {} instead of a WebSocket",
            resp.status().as_u16()
        ))),
        Ok(Err(WsError::Tls(e))) => Err(Fail::Hold(format!("Secure connection failed: {e}"))),
        Ok(Err(e)) => Err(Fail::Retry(format!("The WebSocket handshake failed: {e}"))),
    }
}

/// A read error, as the status says it.
fn read_fail(e: WsError) -> Fail {
    match e {
        WsError::Capacity(_) => Fail::Hold(format!(
            "A message was larger than {} MB",
            SOCKET_MESSAGE_MAX / (1024 * 1024)
        )),
        WsError::ConnectionClosed | WsError::AlreadyClosed => {
            Fail::Retry("The server closed the connection".into())
        }
        e => Fail::Retry(format!("The connection dropped: {e}")),
    }
}

// ---- The task ----------------------------------------------------------------------------------

/// A connection task: connect, read until it ends, wait the backoff (or a Retry), repeat; ends
/// when stopped, held or no longer current.
async fn socket_task(weak: WeakState, id: String, gen: u64, wake: Arc<Notify>) {
    let mut attempt = 0usize;
    loop {
        let Some(settings) = weak.upgrade().and_then(|st| st.socket_settings(&id, gen)) else {
            return;
        };
        if let Some(st) = weak.upgrade() {
            st.socket_state(&id, gen, SocketState::Connecting, None, None);
        }
        let started = Instant::now();
        let fail = match connect(&settings.url).await {
            Err(f) => f,
            Ok(ws) => session(&weak, &id, gen, ws, &settings, &wake).await,
        };
        if started.elapsed() >= BACKOFF_RESET {
            attempt = 0;
        }
        let Some(st) = weak.upgrade() else { return };
        match fail {
            Fail::Gone => return,
            Fail::Hold(message) => {
                st.socket_hold(&id, gen, message);
                return;
            }
            Fail::Retry(message) => {
                let wait = st.socket_sources().backoff(attempt);
                attempt += 1;
                let at = fsx::now_epoch_ms() + wait.as_millis() as u64;
                if !st.socket_state(&id, gen, SocketState::Waiting, Some(message), Some(at)) {
                    return;
                }
                drop(st);
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = wake.notified() => {}
                }
            }
        }
    }
}

/// Reads one connection until it ends (see the module docs for pacing).
async fn session(
    weak: &WeakState,
    id: &str,
    gen: u64,
    mut ws: Ws,
    settings: &Settings,
    wake: &Notify,
) -> Fail {
    if let Some(text) = &settings.send_on_connect {
        if let Err(e) = ws.send(Message::text(text.clone())).await {
            return read_fail(e);
        }
    }
    {
        let Some(st) = weak.upgrade() else {
            return Fail::Gone;
        };
        st.socket_connected(id, gen);
        if !st.socket_state(id, gen, SocketState::Connected, None, None) {
            return Fail::Gone;
        }
    }
    let gap = Duration::from_secs(1) / settings.rate;
    let far = || Instant::now() + Duration::from_secs(3600);
    // Latest-wins rows waiting to be applied, and when they may be.
    let mut slot: Option<Vec<Fields>> = None;
    let mut next_apply = Instant::now();
    // New rows not yet announced to the UI, and when they may be.
    let mut ui_due: Option<Instant> = None;
    let mut last_ui: Option<Instant> = None;
    let mut last_msg: Option<Instant> = None;
    let mut live = false;
    // Reading budget: messages in the current second.
    let mut window = Instant::now();
    let mut in_window = 0u32;
    let mut mapping_error: Option<String> = None;
    loop {
        let stale_at = last_msg.filter(|_| live).map(|t| t + SOCKET_STALE);
        let deadline = [slot.as_ref().map(|_| next_apply), ui_due, stale_at]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or_else(far);
        tokio::select! {
            m = ws.next() => {
                let msg = match m {
                    None => return Fail::Retry("The server closed the connection".into()),
                    Some(Err(e)) => return read_fail(e),
                    Some(Ok(m)) => m,
                };
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => {
                        return Fail::Retry("The server closed the connection".into())
                    }
                    // Binary frames, pings (answered by the library) and pongs.
                    _ => continue,
                };
                let now = Instant::now();
                last_msg = Some(now);
                if now.duration_since(window) >= Duration::from_secs(1) {
                    window = now;
                    in_window = 0;
                }
                in_window += 1;
                let Some(st) = weak.upgrade() else {
                    return Fail::Gone;
                };
                if in_window > SOCKET_READ_PER_SEC {
                    st.socket_dropped(id, gen);
                    continue;
                }
                if !live {
                    live = true;
                    if !st.socket_state(id, gen, SocketState::Live, mapping_error.clone(), None) {
                        return Fail::Gone;
                    }
                }
                st.socket_seen(id, gen);
                drop(st);
                let Some(values) = frame_values(text.as_str()) else {
                    continue;
                };
                let Some(msg) = values
                    .into_iter()
                    .rev()
                    .find(|v| passes(settings.filter.as_ref(), v))
                else {
                    continue;
                };
                let error = match api_map::map_json_value(&msg, &settings.mapping) {
                    Ok(m) => {
                        slot = Some(m.rows);
                        None
                    }
                    Err(e) => Some(e),
                };
                if error != mapping_error {
                    mapping_error = error;
                    let Some(st) = weak.upgrade() else {
                        return Fail::Gone;
                    };
                    if !st.socket_state(id, gen, SocketState::Live, mapping_error.clone(), None) {
                        return Fail::Gone;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let now = Instant::now();
                if let (Some(rows), true) = (slot.as_ref(), now >= next_apply) {
                    let rows = rows.clone();
                    slot = None;
                    next_apply = now + gap;
                    let Some(st) = weak.upgrade() else {
                        return Fail::Gone;
                    };
                    let i = id.to_string();
                    let changed = tokio::task::spawn_blocking(move || {
                        st.socket_put_rows(&i, gen, rows)
                    })
                    .await
                    .unwrap_or(false);
                    if changed {
                        let earliest = last_ui.map_or(now, |t| t + UI_EVERY);
                        ui_due = Some(ui_due.unwrap_or(earliest).max(now));
                    }
                }
                if ui_due.is_some_and(|t| now >= t) {
                    ui_due = None;
                    last_ui = Some(now);
                    let Some(st) = weak.upgrade() else {
                        return Fail::Gone;
                    };
                    st.emit_source_changed(id);
                }
                if live && stale_at.is_some_and(|t| now >= t) {
                    live = false;
                    let Some(st) = weak.upgrade() else {
                        return Fail::Gone;
                    };
                    if !st.socket_state(id, gen, SocketState::Stale, mapping_error.clone(), None) {
                        return Fail::Gone;
                    }
                }
            }
            // A Retry while connected: nothing to do.
            _ = wake.notified() => {}
        }
    }
}

// ---- AppState ----------------------------------------------------------------------------------

impl AppState {
    /// The source's status for a summary (`None` for other kinds).
    pub(crate) fn socket_status(&self, doc: &SourceDoc) -> Option<SocketStatus> {
        let SourceKind::Socket { paused, rows, .. } = &doc.meta.kind else {
            return None;
        };
        let g = lock(&self.socket_sources().inner);
        let mut s = g
            .tracks
            .get(&doc.id)
            .map(|t| t.status.clone())
            .unwrap_or_default();
        if !g.tasks.contains_key(&doc.id) && *paused {
            s.state = SocketState::Paused;
            s.next_at = None;
        }
        s.rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        Some(s)
    }

    /// Starts and stops connection tasks (see the module docs). Cheap without live data
    /// sources.
    pub(crate) fn socket_reconcile(&self) {
        let metas = if self.project_dir().is_ok() {
            self.sources().socket_metas()
        } else {
            Vec::new()
        };
        {
            let g = lock(&self.socket_sources().inner);
            if metas.is_empty() && g.tasks.is_empty() && g.tracks.is_empty() {
                return;
            }
        }
        let used: HashSet<String> = if metas.is_empty() {
            HashSet::new()
        } else {
            let index = self.flow_engine().index();
            let mut used: HashSet<String> = self.screens().source_usage().into_keys().collect();
            used.retain(|id| metas.iter().any(|m| m.id == *id));
            for m in &metas {
                if index.uses_source(&m.id) || self.socket_sources().watched(&m.id) {
                    used.insert(m.id.clone());
                }
            }
            used
        };
        let handle = self.live().runtime_handle();
        let mut changed = Vec::new();
        {
            let mut g = lock(&self.socket_sources().inner);
            let g = &mut *g;
            let ids: HashSet<&str> = metas.iter().map(|m| m.id.as_str()).collect();
            g.tasks.retain(|id, t| {
                let keep = ids.contains(id.as_str());
                if !keep {
                    t.abort.abort();
                }
                keep
            });
            g.tracks.retain(|id, _| ids.contains(id.as_str()));
            g.watch.retain(|id, _| ids.contains(id.as_str()));
            for m in &metas {
                let track = g.tracks.entry(m.id.clone()).or_default();
                if track.sig != m.sig {
                    // New settings: no hold, a fresh connection.
                    track.sig = m.sig;
                    track.held = false;
                    track.status.message = None;
                }
                let wanted = m.has_url && !m.paused && !track.held && used.contains(&m.id);
                let before = (track.status.state, track.status.message.clone());
                let keep = g.tasks.get(&m.id).is_some_and(|t| wanted && t.sig == m.sig);
                if !keep {
                    if let Some(t) = g.tasks.remove(&m.id) {
                        t.abort.abort();
                    }
                }
                if wanted && !keep {
                    match &handle {
                        Some(h) => {
                            let gen = next_gen();
                            let wake = Arc::new(Notify::new());
                            let task = h.spawn(socket_task(
                                self.downgrade(),
                                m.id.clone(),
                                gen,
                                wake.clone(),
                            ));
                            g.tasks.insert(
                                m.id.clone(),
                                Task {
                                    gen,
                                    sig: m.sig,
                                    abort: task.abort_handle(),
                                    wake,
                                },
                            );
                            track.status.state = SocketState::Connecting;
                            track.status.next_at = None;
                        }
                        None => tracing::warn!("no async runtime: live data source {} idle", m.id),
                    }
                }
                if !g.tasks.contains_key(&m.id) && !track.held {
                    track.status.state = if m.paused {
                        SocketState::Paused
                    } else {
                        SocketState::Idle
                    };
                    track.status.next_at = None;
                    track.status.message = None;
                }
                if (track.status.state, track.status.message.clone()) != before {
                    changed.push(m.id.clone());
                }
            }
        }
        for id in changed {
            self.emit_source_changed(&id);
        }
    }

    /// The settings of a current task's source (`None`: stop).
    fn socket_settings(&self, id: &str, gen: u64) -> Option<Settings> {
        if !self.socket_sources().is_current(id, gen) {
            return None;
        }
        self.sources().doc(id).as_ref().and_then(settings_of)
    }

    /// Sets the state of a current task's source; emits when the state or message changed.
    /// `false` when the task is no longer current.
    fn socket_state(
        &self,
        id: &str,
        gen: u64,
        state: SocketState,
        message: Option<String>,
        next_at: Option<u64>,
    ) -> bool {
        let changed = {
            let mut g = lock(&self.socket_sources().inner);
            if !g.tasks.get(id).is_some_and(|t| t.gen == gen) {
                return false;
            }
            let t = g.tracks.entry(id.to_string()).or_default();
            let before = (t.status.state, t.status.message.clone());
            t.status.state = state;
            t.status.message = message;
            t.status.next_at = next_at;
            (t.status.state, t.status.message.clone()) != before
        };
        if changed {
            self.emit_source_changed(id);
        }
        true
    }

    /// A connection opened: the drop counter starts over.
    fn socket_connected(&self, id: &str, gen: u64) {
        let mut g = lock(&self.socket_sources().inner);
        if g.tasks.get(id).is_some_and(|t| t.gen == gen) {
            g.tracks.entry(id.to_string()).or_default().status.dropped = 0;
        }
    }

    /// A message arrived (no event: the next announcement carries the time).
    fn socket_seen(&self, id: &str, gen: u64) {
        let mut g = lock(&self.socket_sources().inner);
        if g.tasks.get(id).is_some_and(|t| t.gen == gen) {
            g.tracks
                .entry(id.to_string())
                .or_default()
                .status
                .last_message_at = Some(fsx::now_epoch_ms());
        }
    }

    /// A message dropped by the reading budget (counted, no event).
    fn socket_dropped(&self, id: &str, gen: u64) {
        let mut g = lock(&self.socket_sources().inner);
        if g.tasks.get(id).is_some_and(|t| t.gen == gen) {
            let s = &mut g.tracks.entry(id.to_string()).or_default().status;
            s.dropped = s.dropped.saturating_add(1);
            s.last_message_at = Some(fsx::now_epoch_ms());
        }
    }

    /// Holds the source: no task until Retry or a save.
    fn socket_hold(&self, id: &str, gen: u64, message: String) {
        {
            let mut g = lock(&self.socket_sources().inner);
            if !g.tasks.get(id).is_some_and(|t| t.gen == gen) {
                return;
            }
            g.tasks.remove(id);
            let t = g.tracks.entry(id.to_string()).or_default();
            t.held = true;
            t.status.state = SocketState::Error;
            t.status.message = Some(message);
            t.status.next_at = None;
        }
        self.emit_source_changed(id);
    }

    /// New rows (blocking pool): in memory only, bound screens re-applied with their on-air
    /// fields forced, auto-advance re-armed, flow triggers. `false` when nothing changed.
    fn socket_put_rows(&self, id: &str, gen: u64, rows: Vec<Fields>) -> bool {
        let (before, after, index_before, index_after) = {
            let Ok((dir, _w)) = self.screen_guard() else {
                return false;
            };
            if !self.socket_sources().is_current(id, gen) {
                return false;
            }
            let Some(mut doc) = self.sources().doc(id) else {
                return false;
            };
            let now = self.sources().clock().now();
            let before = sources::current_row(&doc, &now);
            let SourceKind::Socket {
                rows: have, index, ..
            } = &mut doc.meta.kind
            else {
                return false;
            };
            if *have == rows {
                return false;
            }
            let index_before = *index;
            *index = sources::clamp_index(*index, rows.len());
            let index_after = *index;
            *have = rows;
            let after = sources::current_row(&doc, &now);
            // Live rows are never written (the file holds the settings).
            self.sources().put(doc);
            self.socket_sources()
                .applied
                .fetch_add(1, Ordering::Relaxed);
            for screen in self.screens().source_users(id) {
                let force = screens::bound_layers(&screen, id);
                if let Err(e) = self.apply_screen_doc_forcing(&dir, &screen, &force) {
                    tracing::warn!("live data source {id}: applying screen {}: {e}", screen.id);
                }
            }
            (before, after, index_before, index_after)
        };
        self.rearm();
        if before != after || index_before != index_after {
            self.flow_source_row_changed(id);
        }
        self.flow_api_changed(id, &before, &after);
        true
    }

    // ---- Operations (commands) -------------------------------------------------------------

    /// A new live data source: no URL yet, the whole message as one row, no columns, 10 updates
    /// a second.
    pub fn create_socket_source(&self, name: &str) -> AppResult<SourceDoc> {
        let doc = {
            let (dir, _w) = self.screen_guard()?;
            let kind = SourceKind::Socket {
                url: String::new(),
                mapping: ApiMapping::default(),
                filter: None,
                send_on_connect: None,
                rate: SOCKET_RATE_DEFAULT,
                paused: false,
                columns: Vec::new(),
                rows: Vec::new(),
                index: 0,
                auto: None,
            };
            let doc = sources::create_with(&dir, name, kind)?;
            self.sources().put(doc.clone());
            doc
        };
        self.api_reconcile();
        Ok(doc)
    }

    /// Connects now: clears a hold and the backoff wait. Only while the source is in use.
    pub fn retry_socket_source(&self, id: &str) -> AppResult<SourceSummary> {
        self.project_dir()?;
        let doc = self.loaded_source(id)?;
        if !sources::is_socket(&doc) {
            return Err(AppError::Invalid("Only live data sources connect".into()));
        }
        {
            let mut g = lock(&self.socket_sources().inner);
            if let Some(t) = g.tracks.get_mut(id) {
                t.held = false;
            }
            if let Some(t) = g.tasks.get(id) {
                t.wake.notify_one();
            }
        }
        self.api_reconcile();
        Ok(self.source_summary(&self.loaded_source(id)?))
    }

    /// Pauses (no connection) or resumes; saved in the source file.
    pub fn set_socket_paused(&self, id: &str, paused: bool) -> AppResult<SourceSummary> {
        let saved = {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.loaded_source(id)?;
            let SourceKind::Socket { paused: p, .. } = &mut doc.meta.kind else {
                return Err(AppError::Invalid(
                    "Only live data sources can be paused here".into(),
                ));
            };
            if *p == paused {
                doc
            } else {
                *p = paused;
                let saved = sources::save(&dir, doc, false)?;
                self.sources().put(saved.clone());
                saved
            }
        };
        self.api_reconcile();
        Ok(self.source_summary(&saved))
    }

    /// The editor shows the source (`on`) or no longer does: counted; a watched source is in
    /// use.
    pub fn watch_socket_source(&self, id: &str, on: bool) -> AppResult<()> {
        self.project_dir()?;
        sources::validate_id(id)?;
        {
            let mut g = lock(&self.socket_sources().inner);
            let n = g.watch.entry(id.to_string()).or_default();
            *n = if on {
                n.saturating_add(1)
            } else {
                n.saturating_sub(1)
            };
            if *n == 0 {
                g.watch.remove(id);
            }
        }
        self.api_reconcile();
        Ok(())
    }

    /// Connects with the draft for up to 10 s (sooner once it has its messages); nothing is
    /// saved. Problems are in `error`.
    pub async fn test_socket_source(&self, doc: SourceDoc) -> AppResult<SocketTestResult> {
        self.project_dir()?;
        let SourceKind::Socket {
            mut url,
            mut mapping,
            mut filter,
            mut send_on_connect,
            rate,
            ..
        } = doc.meta.kind
        else {
            return Err(AppError::Invalid(
                "Only live data sources can be tested here".into(),
            ));
        };
        let mut r = SocketTestResult::default();
        if let Err(m) = validate_settings(
            &mut url,
            &mut mapping,
            &mut filter,
            &mut send_on_connect,
            rate.clamp(SOCKET_RATE_MIN, SOCKET_RATE_MAX),
        ) {
            r.error = Some(m);
            return Ok(r);
        }
        if url.is_empty() {
            r.error = Some("Enter a URL first".into());
            return Ok(r);
        }
        let until = Instant::now() + TEST_FOR;
        let mut ws = match tokio::time::timeout_at(until, connect(&url)).await {
            Err(_) => {
                r.error = Some("Connecting timed out".into());
                return Ok(r);
            }
            Ok(Err(Fail::Retry(m) | Fail::Hold(m))) => {
                r.error = Some(m);
                return Ok(r);
            }
            Ok(Err(Fail::Gone)) => return Ok(r),
            Ok(Ok(ws)) => ws,
        };
        if let Some(text) = &send_on_connect {
            if let Err(e) = ws.send(Message::text(text.clone())).await {
                r.error = Some(format!("Sending on connect failed: {e}"));
                return Ok(r);
            }
        }
        let mut budget = API_PREVIEW_MAX;
        let mut end = until;
        let mut last: Option<Value> = None;
        while r.seen < SOCKET_TEST_MESSAGES as u32 {
            let text = match tokio::time::timeout_at(end, ws.next()).await {
                Err(_) | Ok(None) => break,
                Ok(Some(Err(e))) => {
                    if let Fail::Retry(m) | Fail::Hold(m) = read_fail(e) {
                        r.error = Some(m);
                    }
                    break;
                }
                Ok(Some(Ok(Message::Text(t)))) => t,
                Ok(Some(Ok(Message::Close(_)))) => {
                    r.error = Some("The server closed the connection".into());
                    break;
                }
                Ok(Some(Ok(_))) => continue,
            };
            r.seen += 1;
            let values = frame_values(text.as_str());
            let shown = match &values {
                Some(v) if v.len() == 1 => v[0].clone(),
                Some(v) => Value::Array(v.clone()),
                None => Value::String(text.as_str().to_string()),
            };
            let size = shown.to_string().len();
            if size <= budget {
                budget -= size;
                r.messages.push(shown);
            }
            if let Some(m) =
                values.and_then(|v| v.into_iter().rev().find(|v| passes(filter.as_ref(), v)))
            {
                if r.matched == 0 {
                    end = end.min(Instant::now() + TEST_AFTER_MATCH);
                }
                r.matched += 1;
                last = Some(m);
            }
        }
        let _ = ws.close(None).await;
        if let Some(msg) = last {
            match api_map::map_json_value(&msg, &mapping) {
                Ok(m) => {
                    for c in &mapping.columns {
                        let n = m
                            .rows
                            .iter()
                            .filter(|row| row.get(&c.key).is_some_and(|v| !v.is_empty()))
                            .count();
                        r.hits
                            .insert(c.key.clone(), u32::try_from(n).unwrap_or(u32::MAX));
                    }
                    r.rows = m.rows.into_iter().take(API_TEST_ROWS).collect();
                }
                Err(e) => r.error = r.error.take().or(Some(e)),
            }
            if msg.to_string().len() <= API_PREVIEW_MAX {
                r.json = Some(msg);
            }
        } else if r.error.is_none() {
            r.error = Some(if r.seen == 0 {
                "Connected, but no message came within 10 seconds".into()
            } else {
                "No message matched the filter".into()
            });
        }
        Ok(r)
    }
}

#[cfg(test)]
mod tests;
