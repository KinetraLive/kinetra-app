//! The connection task: ONE tokio task per switch-on, holding ONE WebSocket at a time.
//!
//! Lifecycle (§3): upgrade (Bearer, `kinetra-control.v1`) → `hello` → `welcome` → `register`
//! → `registered` → `snapshot` → diffs and commands until the switch goes off, the project
//! closes, the app quits (`end`) or the server ends / stops it. Drops reconnect with the §10.2
//! backoff (0.5 s, 1, 2, 4, 8, then 15 s, ±30 %, reset after 60 s connected) and `resume_run`;
//! nothing is ever replayed.
//!
//! The render path never waits for this task: it only wakes it (`Notify`). The task coalesces
//! wakes (at most one diff per 50 ms, i.e. ≤ 20/s; on-air changes go out within one interval),
//! builds the tree on the blocking pool and diffs it against the last one sent. A stalled socket
//! just delays the next send; whatever changed meanwhile goes out as one diff (or one snapshot
//! when that is smaller or simpler), at most one snapshot per second.
//!
//! Commands run one at a time in arrival order: the pending changes are sent first (so the stale
//! check reads an exact history), then the checks and the action run on the blocking pool, then
//! the result, then the diff it caused (`cause`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::{oneshot, Notify};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::commands;
use super::diag::{Dir, MsgRecord, Reconnect, Refusal};
use super::ids;
use super::proto::{
    self, close_action, close_code_name, stop_message, CloseAction, CmdResult, EndReason, In,
    Limits, Out, APP_PATH, DIFF_OPS_MAX, PROTOCOL, SUBPROTOCOL,
};
use super::state::{self, Model};
use super::{Ctl, Phase};
use crate::core::fsx::now_epoch_ms;
use crate::core::state::{AppState, WeakState};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub(super) type Tx = SplitSink<Ws, Message>;
pub(super) type Rx = SplitStream<Ws>;

/// Timings (tests shorten some through [`set_test_timing`]).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// Upgrade (TCP + TLS + HTTP), and each handshake step (`welcome`, `registered`).
    pub handshake: Duration,
    /// No message at all for this long: the connection is dead.
    pub silence: Duration,
    /// A send that doesn't complete in this time: the connection is dead.
    pub send: Duration,
    /// After `end`, how long to wait for the server's close.
    pub end_wait: Duration,
    /// Shortest time between two diffs (20 per second).
    pub diff_gap: Duration,
    /// Backoff steps before the steady interval.
    pub backoff: [Duration; 5],
    pub backoff_steady: Duration,
    /// Connected this long: the backoff starts over.
    pub backoff_reset: Duration,
    /// The heartbeat when the server's `heartbeat_s` is missing or 0.
    pub heartbeat: Duration,
    /// Shortest time between two snapshots.
    pub snapshot_gap: Duration,
}

impl Timing {
    pub const LIVE: Timing = Timing {
        handshake: Duration::from_secs(10),
        silence: Duration::from_secs(60),
        send: Duration::from_secs(10),
        end_wait: Duration::from_secs(2),
        diff_gap: Duration::from_millis(50),
        backoff: [
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(8),
        ],
        backoff_steady: Duration::from_secs(15),
        backoff_reset: Duration::from_secs(60),
        heartbeat: Duration::from_secs(20),
        snapshot_gap: Duration::from_secs(1),
    };
}

static TEST_TIMING: Mutex<Option<Timing>> = Mutex::new(None);

/// Tests: other timings for every connection started after this (`None`: the real ones).
#[cfg(test)]
pub fn set_test_timing(t: Option<Timing>) {
    *TEST_TIMING.lock().unwrap_or_else(PoisonError::into_inner) = t;
}

pub(super) fn timing() -> Timing {
    TEST_TIMING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .unwrap_or(Timing::LIVE)
}

/// A random number in 0..1 from the OS (jitter).
fn random_unit() -> f64 {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    f64::from(u32::from_le_bytes(b)) / f64::from(u32::MAX)
}

/// The wait before reconnect `attempt` (0-based): the §10.2 steps, then the steady interval,
/// each ±30 % (`unit` in 0..1 picks where in that range).
pub fn backoff(t: &Timing, attempt: u32, unit: f64) -> Duration {
    let base = t
        .backoff
        .get(attempt as usize)
        .copied()
        .unwrap_or(t.backoff_steady);
    base.mul_f64(0.7 + 0.6 * unit.clamp(0.0, 1.0))
}

/// How a session ended.
#[derive(Debug)]
pub(super) enum End {
    /// We sent `end` (switch off, project closed, quit, a new show).
    ByUs,
    /// A new show ID: reconnect now as a new run.
    NewShow,
    /// The run ended (close 1000 `ended`).
    Ended,
    /// Stopped for a reason the user has to handle.
    Stop { code: String, message: String },
    /// Reconnect after `after` (server's wish) or the backoff.
    Retry {
        close: Option<u16>,
        reason: String,
        after: Option<Duration>,
    },
}

/// A failed upgrade.
#[derive(Debug)]
pub(super) enum ConnectError {
    Http {
        status: u16,
        code: Option<String>,
        retry_after: Option<Duration>,
    },
    Net(String),
}

/// `https://…` → `wss://…`; `http://…` (debug and tests only) → `ws://…`.
pub(super) fn ws_url(base: &str, probe: bool) -> String {
    let base = base.trim_end_matches('/');
    let ws = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws}{APP_PATH}{}", if probe { "?mode=probe" } else { "" })
}

/// Upgrades to the control endpoint. Secrets go into headers only (never logged).
pub(super) async fn connect(
    base: &str,
    session: &str,
    probe: bool,
    within: Duration,
) -> Result<(Tx, Rx), ConnectError> {
    let mut req = ws_url(base, probe)
        .into_client_request()
        .map_err(|e| ConnectError::Net(e.to_string()))?;
    let h = req.headers_mut();
    let auth = HeaderValue::from_str(&format!("Bearer {session}"))
        .map_err(|_| ConnectError::Net("The session can't be sent".into()))?;
    h.insert(header::AUTHORIZATION, auth);
    h.insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(SUBPROTOCOL),
    );
    let ua = format!(
        "Kinetra/{} ({})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    if let Ok(v) = HeaderValue::from_str(&ua) {
        h.insert(header::USER_AGENT, v);
    }
    let cfg = WebSocketConfig::default()
        .max_message_size(Some(4 * 1024 * 1024))
        .max_frame_size(Some(4 * 1024 * 1024));
    let fut = tokio_tungstenite::connect_async_with_config(req, Some(cfg), true);
    match tokio::time::timeout(within, fut).await {
        Err(_) => Err(ConnectError::Net("The connection timed out".into())),
        Ok(Ok((ws, _))) => Ok(ws.split()),
        Ok(Err(WsError::Http(resp))) => {
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let code = resp
                .body()
                .as_deref()
                .and_then(|b| serde_json::from_slice::<Value>(b).ok())
                .and_then(|v| v["error"]["code"].as_str().map(str::to_string));
            Err(ConnectError::Http {
                status,
                code,
                retry_after,
            })
        }
        Ok(Err(e)) => Err(ConnectError::Net(net_words(&e))),
    }
}

/// A network error in plain words (no addresses, no paths).
fn net_words(e: &WsError) -> String {
    match e {
        WsError::Io(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused => "The server refused the connection".into(),
            std::io::ErrorKind::TimedOut => "The connection timed out".into(),
            _ => "No connection to the server".into(),
        },
        WsError::Tls(_) => "A secure connection couldn't be made".into(),
        WsError::Url(_) => "The server address is wrong".into(),
        _ => "The connection failed".into(),
    }
}

/// One frame from the server.
pub(super) enum Got {
    Msg(In, usize),
    Closed { code: u16, reason: String },
}

/// The next protocol message (skipping WebSocket pings / pongs and unparsable text).
pub(super) async fn next(rx: &mut Rx) -> Got {
    loop {
        match rx.next().await {
            Some(Ok(Message::Text(t))) => match serde_json::from_str::<In>(t.as_str()) {
                Ok(m) => return Got::Msg(m, t.len()),
                Err(e) => tracing::debug!("remote: unreadable message ignored: {e}"),
            },
            Some(Ok(Message::Close(f))) => {
                return match f {
                    Some(f) => Got::Closed {
                        code: u16::from(f.code),
                        reason: f.reason.to_string(),
                    },
                    None => Got::Closed {
                        code: 1005,
                        reason: String::new(),
                    },
                }
            }
            Some(Ok(_)) => {}
            Some(Err(_)) | None => {
                return Got::Closed {
                    code: 1006,
                    reason: String::new(),
                }
            }
        }
    }
}

/// Sends one message (serialized once); `false` when the socket is dead or the send stalls past
/// [`Timing::send`]. Records it in the diagnostics.
pub(super) async fn send(tx: &mut Tx, weak: &WeakState, m: &Out, within: Duration) -> bool {
    let json = match serde_json::to_string(m) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("remote: {} not sent: {e}", m.kind());
            return true;
        }
    };
    let bytes = json.len();
    let ok = matches!(
        tokio::time::timeout(within, tx.send(Message::text(json))).await,
        Ok(Ok(()))
    );
    if ok {
        if let Some(st) = weak.upgrade() {
            let (cause, corr) = match m {
                Out::Diff { cause, .. } => (cause.clone(), None),
                _ => (None, m.corr().map(str::to_string)),
            };
            let ms = match m {
                Out::Result(r) => r
                    .done_ms
                    .zip(r.received_ms)
                    .map(|(d, r)| d.saturating_sub(r)),
                _ => None,
            };
            st.remote().diag().message(MsgRecord {
                at: now_epoch_ms(),
                dir: Dir::Out,
                t: m.kind(),
                corr,
                bytes,
                seq: m.seq(),
                cause,
                ms,
            });
            tracing::debug!("remote → {} {bytes} B", m.kind());
        }
    }
    ok
}

/// Closes politely (1000), ignoring errors.
pub(super) async fn close(tx: &mut Tx) {
    let frame = CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    };
    let _ =
        tokio::time::timeout(Duration::from_secs(2), tx.send(Message::Close(Some(frame)))).await;
}

fn record_in(weak: &WeakState, m: &In, bytes: usize) {
    if let Some(st) = weak.upgrade() {
        st.remote().diag().message(MsgRecord {
            at: now_epoch_ms(),
            dir: Dir::In,
            t: m.kind(),
            corr: m.corr().map(str::to_string),
            bytes,
            seq: None,
            cause: None,
            ms: None,
        });
        tracing::debug!("remote ← {} {bytes} B", m.kind());
    }
}

/// Runs the connection for one switch-on until it ends for good.
pub(super) async fn run(
    weak: WeakState,
    gen: u64,
    ctl: UnboundedReceiver<Ctl>,
    dirty: Arc<Notify>,
) {
    run_inner(&weak, gen, ctl, dirty).await;
    if let Some(st) = weak.upgrade() {
        st.remote_task_finished(gen);
    }
}

async fn run_inner(
    weak: &WeakState,
    gen: u64,
    mut ctl: UnboundedReceiver<Ctl>,
    dirty: Arc<Notify>,
) {
    let t = timing();
    let mut attempt: u32 = 0;
    let mut resume: Option<String> = None;
    loop {
        let started = Instant::now();
        let end = session(weak, gen, &mut ctl, &dirty, &mut resume, &t).await;
        let Some(st) = weak.upgrade() else { return };
        if started.elapsed() >= t.backoff_reset {
            attempt = 0;
        }
        match end {
            End::ByUs => {
                st.remote_task_done(gen);
                return;
            }
            End::NewShow => {
                resume = None;
                attempt = 0;
                continue;
            }
            End::Ended => {
                st.remote_update(gen, |v| {
                    v.phase = Phase::Ended;
                    v.crew.clear();
                    v.rtt_ms = None;
                });
                st.remote_task_done(gen);
                st.live().set_fade_tracking(false);
                return;
            }
            End::Stop { code, message } => {
                tracing::info!("remote control stopped: {code}");
                st.remote_update(gen, |v| {
                    v.phase = Phase::Stopped { code, message };
                    v.crew.clear();
                    v.rtt_ms = None;
                });
                st.remote_task_done(gen);
                st.live().set_fade_tracking(false);
                return;
            }
            End::Retry {
                close,
                reason,
                after,
            } => {
                let wait = after.unwrap_or_else(|| backoff(&t, attempt, random_unit()));
                attempt = attempt.saturating_add(1);
                tracing::info!(
                    "remote control reconnecting in {} ms ({reason})",
                    wait.as_millis()
                );
                st.remote().diag().reconnect(Reconnect {
                    at: now_epoch_ms(),
                    close_code: close,
                    reason,
                    backoff_ms: wait.as_millis() as u64,
                });
                st.remote_update(gen, |v| {
                    v.phase = Phase::Reconnecting {
                        attempt,
                        next_ms: now_epoch_ms() + wait.as_millis() as u64,
                    };
                    v.crew.clear();
                    v.rtt_ms = None;
                });
                drop(st);
                let until = tokio::time::Instant::now() + wait;
                loop {
                    tokio::select! {
                        () = tokio::time::sleep_until(until) => break,
                        c = ctl.recv() => match c {
                            None | Some(Ctl::End(_)) => {
                                if let Some(st) = weak.upgrade() {
                                    st.remote_task_done(gen);
                                }
                                return;
                            }
                            Some(Ctl::Ping(reply)) => {
                                let _ = reply.send(Err("Not connected".into()));
                            }
                            Some(Ctl::NewShow) => {
                                resume = None;
                            }
                            Some(_) => {}
                        },
                    }
                }
            }
        }
    }
}

/// One connection: from the upgrade to its end.
async fn session(
    weak: &WeakState,
    gen: u64,
    ctl: &mut UnboundedReceiver<Ctl>,
    dirty: &Arc<Notify>,
    resume: &mut Option<String>,
    t: &Timing,
) -> End {
    let Some(st) = weak.upgrade() else {
        return End::ByUs;
    };
    st.remote_update(gen, |v| {
        if !matches!(v.phase, Phase::Reconnecting { .. }) {
            v.phase = Phase::Connecting;
        }
    });
    let creds = match st.remote_credentials() {
        Ok(c) => c,
        Err(_) => {
            return End::Stop {
                code: "unauthorized".into(),
                message: "Sign in to use remote control.".into(),
            }
        }
    };
    drop(st);
    let (tx, rx) = match connect(&creds.base, &creds.session, false, t.handshake).await {
        Ok(p) => p,
        Err(ConnectError::Http {
            status,
            code,
            retry_after,
        }) => {
            if let Some(st) = weak.upgrade() {
                st.remote().diag().error(
                    code.as_deref().unwrap_or(&format!("http_{status}")),
                    true,
                    None,
                );
            }
            return match status {
                401 => End::Stop {
                    code: "unauthorized".into(),
                    message: stop_message("unauthorized", "", None),
                },
                429 | 503 => End::Retry {
                    close: None,
                    reason: code.unwrap_or_else(|| format!("http_{status}")),
                    after: retry_after,
                },
                _ => End::Retry {
                    close: None,
                    reason: format!("http_{status}"),
                    after: None,
                },
            };
        }
        Err(ConnectError::Net(why)) => {
            return End::Retry {
                close: Some(1006),
                reason: why,
                after: None,
            }
        }
    };
    if let Some(st) = weak.upgrade() {
        st.remote().diag().connects += 1;
    }
    let mut s = Session {
        weak,
        gen,
        tx,
        rx,
        t: *t,
        limits: Limits::default(),
        heartbeat: t.heartbeat,
        model: Arc::new(Mutex::new(Model::default())),
        last_rx: Instant::now(),
        pings: HashMap::new(),
        recent: HashMap::new(),
        last_diff: None,
        last_snapshot: None,
        snapshot_due: None,
        fatal: None,
        restart_after: None,
        run_id: resume.take(),
    };
    let end = s.drive(ctl, dirty, &creds).await;
    *resume = s.run_id.clone();
    end
}

struct Session<'a> {
    weak: &'a WeakState,
    gen: u64,
    tx: Tx,
    rx: Rx,
    t: Timing,
    limits: Limits,
    heartbeat: Duration,
    model: Arc<Mutex<Model>>,
    last_rx: Instant,
    /// corr → (sent_ms, who waits for the round trip).
    pings: HashMap<String, (u64, Option<RttReply>)>,
    /// Results of the last 60 s by corr (duplicates get the same one again).
    recent: HashMap<String, (Instant, CmdResult)>,
    last_diff: Option<Instant>,
    last_snapshot: Option<Instant>,
    /// A snapshot is owed (`Some(corr)` after a resync).
    snapshot_due: Option<Option<String>>,
    /// The last fatal error (the close that follows is explained by it).
    fatal: Option<(String, String, Option<Value>)>,
    /// `server_restart`: reconnect after this.
    restart_after: Option<Duration>,
    run_id: Option<String>,
}

/// What a message handled by [`Session::common`] left for the caller.
enum Left {
    Handled,
    Msg(In),
    End(End),
}

/// Who waits for a ping's round trip (Test connection while live).
type RttReply = oneshot::Sender<Result<u64, String>>;

async fn sleep_opt(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at.into()).await,
        None => std::future::pending().await,
    }
}

impl Session<'_> {
    fn st(&self) -> Option<AppState> {
        self.weak.upgrade()
    }

    async fn out(&mut self, m: &Out) -> bool {
        send(&mut self.tx, self.weak, m, self.t.send).await
    }

    /// The end a close (after an optional fatal `error`) means.
    fn closed(&mut self, code: u16, reason: &str) -> End {
        let (ecode, emsg, details) = match self.fatal.take() {
            Some(f) => f,
            None if !reason.is_empty() => (reason.to_string(), String::new(), None),
            None => (close_code_name(code).to_string(), String::new(), None),
        };
        if let Some(st) = self.st() {
            st.remote().diag().last_close = Some((code, ecode.clone()));
        }
        tracing::info!("remote control closed: {code} {ecode}");
        let machine = details
            .as_ref()
            .and_then(|d| d["machine_name"].as_str())
            .map(str::to_string);
        match close_action(&ecode) {
            CloseAction::Ended => End::Ended,
            CloseAction::Stop => End::Stop {
                message: stop_message(&ecode, &emsg, machine.as_deref()),
                code: ecode,
            },
            CloseAction::ReconnectAfter => End::Retry {
                close: Some(code),
                after: details
                    .as_ref()
                    .and_then(|d| d["retry_after_s"].as_u64())
                    .map(Duration::from_secs),
                reason: ecode,
            },
            CloseAction::Reconnect => End::Retry {
                close: Some(code),
                after: self.restart_after.take(),
                reason: ecode,
            },
        }
    }

    /// Handles what every phase handles the same way (pings, pongs, errors, crew, warnings,
    /// restarts, closes); hands anything else back.
    async fn common(&mut self, got: Got) -> Left {
        let (m, bytes) = match got {
            Got::Closed { code, reason } => return Left::End(self.closed(code, &reason)),
            Got::Msg(m, b) => (m, b),
        };
        self.last_rx = Instant::now();
        record_in(self.weak, &m, bytes);
        match m {
            In::Ping { corr, sent_ms } => {
                let received_ms = now_epoch_ms();
                let pong = Out::Pong {
                    corr,
                    ping_sent_ms: sent_ms,
                    received_ms,
                    sent_ms: now_epoch_ms(),
                };
                if !self.out(&pong).await {
                    return Left::End(End::Retry {
                        close: Some(1006),
                        reason: "send".into(),
                        after: None,
                    });
                }
            }
            In::Pong {
                corr,
                ping_sent_ms,
                received_ms,
                sent_ms,
            } => {
                if let Some((_, reply)) = self.pings.remove(&corr) {
                    let now = now_epoch_ms() as i64;
                    let rtt = ((now - ping_sent_ms as i64) - (sent_ms as i64 - received_ms as i64))
                        .max(0) as u64;
                    let offset =
                        ((received_ms as i64 - ping_sent_ms as i64) + (sent_ms as i64 - now)) / 2;
                    if let Some(st) = self.st() {
                        st.remote().diag().rtt(rtt, offset);
                        st.remote_update(self.gen, |v| v.rtt_ms = Some(rtt));
                    }
                    if let Some(r) = reply {
                        let _ = r.send(Ok(rtt));
                    }
                }
            }
            In::Error {
                code,
                message,
                fatal,
                corr,
                details,
            } => {
                tracing::info!("remote control error: {code} (fatal: {fatal})");
                if let Some(st) = self.st() {
                    st.remote().diag().error(&code, fatal, corr.clone());
                    let c = code.clone();
                    st.remote_update(self.gen, |v| v.last_error = Some(c));
                }
                if fatal {
                    self.fatal = Some((code, message, details));
                } else {
                    return Left::Msg(In::Error {
                        code,
                        message,
                        fatal,
                        corr,
                        details,
                    });
                }
            }
            In::Crew { connected } => {
                if let Some(st) = self.st() {
                    st.remote_update(self.gen, |v| v.crew = connected);
                }
            }
            In::LicenseWarning { ends_ms, message } => {
                if let Some(st) = self.st() {
                    st.remote_update(self.gen, |v| {
                        v.license_warning = Some(super::LicenseWarning {
                            ends_ms,
                            message: proto::clip(&message, proto::MESSAGE_MAX),
                        })
                    });
                }
            }
            In::ServerRestart { retry_after_ms } => {
                let ms = (retry_after_ms as f64 * random_unit()) as u64;
                self.restart_after = Some(Duration::from_millis(ms));
            }
            In::Unknown => {}
            other => return Left::Msg(other),
        }
        Left::Handled
    }

    /// Sends `end` and waits briefly for the server's close.
    async fn end(&mut self, reason: EndReason) {
        if self.out(&Out::End { reason }).await {
            let until = Instant::now() + self.t.end_wait;
            loop {
                tokio::select! {
                    got = next(&mut self.rx) => {
                        if matches!(got, Got::Closed { .. }) {
                            break;
                        }
                    }
                    () = sleep_opt(Some(until)) => break,
                }
            }
        }
        close(&mut self.tx).await;
    }

    async fn drive(
        &mut self,
        ctl: &mut UnboundedReceiver<Ctl>,
        dirty: &Arc<Notify>,
        creds: &crate::core::cloud::service::RemoteCredentials,
    ) -> End {
        // hello → welcome
        let hello = Out::Hello {
            protocol: PROTOCOL,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            machine: creds.machine.clone(),
            license_token: creds.license_token.clone(),
            nonce: ids::nonce(),
            timestamp: creds.now,
        };
        if !self.out(&hello).await {
            return End::Retry {
                close: Some(1006),
                reason: "send".into(),
                after: None,
            };
        }
        let until = Instant::now() + self.t.handshake;
        loop {
            let got = tokio::select! {
                got = next(&mut self.rx) => got,
                () = sleep_opt(Some(until)) => return End::Retry {
                    close: None, reason: "timeout".into(), after: None,
                },
                c = ctl.recv() => {
                    if let Some(e) = self.ctl_early(c).await { return e; }
                    continue;
                }
            };
            match self.common(got).await {
                Left::End(e) => return e,
                Left::Msg(In::Welcome {
                    server_version,
                    heartbeat_s,
                    limits,
                    ..
                }) => {
                    self.limits = limits;
                    if heartbeat_s > 0 {
                        self.heartbeat = Duration::from_secs(heartbeat_s);
                    }
                    if let Some(st) = self.st() {
                        let mut d = st.remote().diag();
                        d.server_version = Some(server_version.clone());
                        d.limits = serde_json::to_value(limits).ok();
                        drop(d);
                        st.remote_update(self.gen, |v| v.server_version = Some(server_version));
                    }
                    break;
                }
                _ => {}
            }
        }

        // register → registered (or "Move it here?")
        if let Some(e) = self.register(ctl, false).await {
            return e;
        }
        self.main(ctl, dirty).await
    }

    /// A control message before the run exists: `end` ends, a ping can't be answered yet.
    async fn ctl_early(&mut self, c: Option<Ctl>) -> Option<End> {
        match c {
            None => {
                self.end(EndReason::Other).await;
                Some(End::ByUs)
            }
            Some(Ctl::End(r)) => {
                self.end(r).await;
                Some(End::ByUs)
            }
            Some(Ctl::Ping(reply)) => {
                let _ = reply.send(Err("Not connected yet".into()));
                None
            }
            Some(_) => None,
        }
    }

    /// Registers (again with `takeover` after "Move it here"); `Some(end)` when the session
    /// ends instead.
    async fn register(
        &mut self,
        ctl: &mut UnboundedReceiver<Ctl>,
        mut takeover: bool,
    ) -> Option<End> {
        loop {
            let Some(st) = self.st() else {
                return Some(End::ByUs);
            };
            let Some(project_id) = st.project_remote_id() else {
                return Some(End::Stop {
                    code: "no_project".into(),
                    message: "Open a project to use remote control.".into(),
                });
            };
            let name = st.remote_project_name();
            drop(st);
            let corr = ids::uuid_v7();
            let reg = Out::Register {
                corr,
                project_id,
                name,
                resume_run: self.run_id.clone(),
                takeover,
            };
            if !self.out(&reg).await {
                return Some(End::Retry {
                    close: Some(1006),
                    reason: "send".into(),
                    after: None,
                });
            }
            let until = Instant::now() + self.t.handshake;
            loop {
                let got = tokio::select! {
                    got = next(&mut self.rx) => got,
                    () = sleep_opt(Some(until)) => return Some(End::Retry {
                        close: None, reason: "timeout".into(), after: None,
                    }),
                    c = ctl.recv() => {
                        if matches!(c, Some(Ctl::NewShow)) {
                            // The project has a new ID: register that instead.
                            takeover = false;
                            self.run_id = None;
                            break;
                        }
                        if let Some(e) = self.ctl_early(c).await { return Some(e); }
                        continue;
                    }
                };
                match self.common(got).await {
                    Left::End(e) => return Some(e),
                    Left::Handled => {}
                    Left::Msg(In::Registered {
                        show_id,
                        run_id,
                        resumed,
                        crew_url,
                        controller_url,
                        ..
                    }) => {
                        tracing::info!("remote control live (resumed: {resumed})");
                        self.run_id = Some(run_id.clone());
                        if let Some(st) = self.st() {
                            {
                                let mut d = st.remote().diag();
                                d.run_id = Some(run_id.clone());
                                d.show_id = Some(show_id.clone());
                            }
                            st.remote_update(self.gen, |v| {
                                v.phase = Phase::Live { run_id, resumed };
                                v.show_id = Some(show_id);
                                v.crew_url = Some(crew_url);
                                v.controller_url = Some(controller_url);
                            });
                        }
                        return None;
                    }
                    Left::Msg(In::Error {
                        code,
                        details,
                        message,
                        ..
                    }) => {
                        match code.as_str() {
                            "show_in_use" => {
                                let machine = details
                                    .as_ref()
                                    .and_then(|d| d["machine_name"].as_str())
                                    .unwrap_or("another computer")
                                    .to_string();
                                match self.decide(ctl, machine).await {
                                    Decision::Takeover => takeover = true,
                                    Decision::NewShow => {
                                        takeover = false;
                                        self.run_id = None;
                                    }
                                    Decision::End(e) => return Some(e),
                                }
                                if let Some(st) = self.st() {
                                    st.remote_update(self.gen, |v| v.phase = Phase::Connecting);
                                }
                                break;
                            }
                            "run_limit_reached" | "show_limit_reached" => {
                                let max = details.as_ref().and_then(|d| d["max"].as_u64());
                                let msg = limit_message(&code, max, &message);
                                close(&mut self.tx).await;
                                return Some(End::Stop { code, message: msg });
                            }
                            _ => {
                                // A field problem is a bug on our side: log it, try again later.
                                tracing::warn!("remote control register refused: {code}");
                                close(&mut self.tx).await;
                                return Some(End::Retry {
                                    close: None,
                                    reason: code,
                                    after: None,
                                });
                            }
                        }
                    }
                    Left::Msg(_) => {}
                }
            }
        }
    }

    /// "Remote control is running on <machine>. Move it here?": waits for the user's choice,
    /// answering the server meanwhile.
    async fn decide(&mut self, ctl: &mut UnboundedReceiver<Ctl>, machine: String) -> Decision {
        if let Some(st) = self.st() {
            st.remote_update(self.gen, |v| v.phase = Phase::ShowInUse { machine });
        }
        loop {
            tokio::select! {
                got = next(&mut self.rx) => {
                    if let Left::End(e) = self.common(got).await {
                        return Decision::End(e);
                    }
                }
                c = ctl.recv() => match c {
                    Some(Ctl::Takeover) => return Decision::Takeover,
                    Some(Ctl::NewShow) => return Decision::NewShow,
                    None => {
                        self.end(EndReason::Other).await;
                        return Decision::End(End::ByUs);
                    }
                    Some(Ctl::End(r)) => {
                        self.end(r).await;
                        return Decision::End(End::ByUs);
                    }
                    Some(Ctl::Ping(reply)) => {
                        let _ = reply.send(Err("Not connected yet".into()));
                    }
                    Some(Ctl::Rename(_)) => {}
                },
            }
        }
    }

    /// The run: snapshot, then diffs, commands, heartbeats and control messages.
    async fn main(&mut self, ctl: &mut UnboundedReceiver<Ctl>, dirty: &Arc<Notify>) -> End {
        if let Err(e) = self.snapshot(None).await {
            return e;
        }
        let mut heartbeat =
            tokio::time::interval_at(tokio::time::Instant::now() + self.heartbeat, self.heartbeat);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut flush_at: Option<Instant> = None;
        loop {
            let snapshot_at = self.snapshot_due.as_ref().map(|_| self.snapshot_slot());
            let silence_at = self.last_rx + self.t.silence;
            tokio::select! {
                biased;
                got = next(&mut self.rx) => match self.common(got).await {
                    Left::End(e) => return e,
                    Left::Handled => {}
                    Left::Msg(In::Command(cmd)) => {
                        if let Err(e) = self.command(cmd).await {
                            return e;
                        }
                    }
                    Left::Msg(In::Resync { corr, reason }) => {
                        tracing::info!("remote control resync: {reason}");
                        if let Some(st) = self.st() {
                            let mut d = st.remote().diag();
                            d.resyncs += 1;
                            d.last_resync = Some((reason, corr.clone()));
                        }
                        self.snapshot_due = Some(Some(corr));
                    }
                    Left::Msg(_) => {}
                },
                c = ctl.recv() => match c {
                    None => {
                        self.end(EndReason::Other).await;
                        return End::ByUs;
                    }
                    Some(Ctl::End(r)) => {
                        self.end(r).await;
                        return End::ByUs;
                    }
                    Some(Ctl::NewShow) => {
                        // `end` ends the run and the server closes (§5): the new show registers
                        // on a fresh connection, as a new run.
                        self.end(EndReason::Other).await;
                        return End::NewShow;
                    }
                    Some(Ctl::Rename(name)) => {
                        if !self.out(&Out::Rename { name }).await {
                            return self.dead();
                        }
                    }
                    Some(Ctl::Ping(reply)) => {
                        if !self.ping(Some(reply)).await {
                            return self.dead();
                        }
                    }
                    Some(Ctl::Takeover) => {}
                },
                () = sleep_opt(snapshot_at) => {
                    let corr = self.snapshot_due.take().flatten();
                    if let Err(e) = self.snapshot(corr).await {
                        return e;
                    }
                }
                () = sleep_opt(flush_at) => {
                    flush_at = None;
                    if let Err(e) = self.flush(None).await {
                        return e;
                    }
                }
                () = dirty.notified() => {
                    if flush_at.is_none() {
                        let earliest = self.last_diff.map_or_else(Instant::now, |d| d + self.t.diff_gap);
                        flush_at = Some(earliest.max(Instant::now()));
                    }
                }
                _ = heartbeat.tick() => {
                    if !self.ping(None).await {
                        return self.dead();
                    }
                }
                () = sleep_opt(Some(silence_at)) => {
                    return End::Retry { close: None, reason: "timeout".into(), after: None };
                }
            }
        }
    }

    fn dead(&self) -> End {
        End::Retry {
            close: Some(1006),
            reason: "send".into(),
            after: None,
        }
    }

    async fn ping(&mut self, reply: Option<RttReply>) -> bool {
        let corr = ids::uuid_v7();
        let sent_ms = now_epoch_ms();
        // Unanswered pings older than a minute are dropped.
        self.pings
            .retain(|_, (at, _)| sent_ms.saturating_sub(*at) < 60_000);
        self.pings.insert(corr.clone(), (sent_ms, reply));
        self.out(&Out::Ping { corr, sent_ms }).await
    }

    /// When the next snapshot may go out (≤ 1 per second).
    fn snapshot_slot(&self) -> Instant {
        self.last_snapshot
            .map_or_else(Instant::now, |s| s + self.t.snapshot_gap)
    }

    /// Builds the tree on the blocking pool.
    async fn build(&self) -> Option<Value> {
        let st = self.st()?;
        tokio::task::spawn_blocking(move || {
            let input = st.remote_build_input();
            state::build(&st, &input)
        })
        .await
        .ok()
    }

    /// Sends a snapshot (with a resync's `corr`).
    async fn snapshot(&mut self, corr: Option<String>) -> Result<(), End> {
        let Some(tree) = self.build().await else {
            return Err(End::ByUs);
        };
        let size = serde_json::to_vec(&tree).map(|v| v.len()).unwrap_or(0);
        if size > self.limits.snapshot_bytes {
            close(&mut self.tx).await;
            return Err(End::Stop {
                code: "too_large".into(),
                message: "This show is too big for remote control.".into(),
            });
        }
        let seq = self
            .model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot(tree.clone());
        if !self
            .out(&Out::Snapshot {
                seq,
                corr,
                state: tree,
            })
            .await
        {
            return Err(self.dead());
        }
        self.last_snapshot = Some(Instant::now());
        self.last_diff = Some(Instant::now());
        if let Some(st) = self.st() {
            let mut d = st.remote().diag();
            d.snapshots += 1;
            d.last_snapshot = Some((seq, size));
        }
        Ok(())
    }

    /// Sends what changed since the last tree as one diff (or owes a snapshot when the diff is
    /// over the limits).
    async fn flush(&mut self, cause: Option<String>) -> Result<(), End> {
        if self.snapshot_due.is_some() {
            return Ok(());
        }
        let Some(tree) = self.build().await else {
            return Err(End::ByUs);
        };
        let ops = {
            let m = self.model.lock().unwrap_or_else(PoisonError::into_inner);
            if !m.has_snapshot() {
                return Ok(());
            }
            state::diff(&m.tree, &tree)
        };
        if ops.is_empty() {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&ops).map(|v| v.len()).unwrap_or(0);
        if ops.len() > DIFF_OPS_MAX || bytes > self.limits.diff_bytes {
            self.snapshot_due = Some(None);
            return Ok(());
        }
        let seq = self
            .model
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .commit_diff(tree, &ops);
        if !self.out(&Out::Diff { seq, cause, ops }).await {
            return Err(self.dead());
        }
        self.last_diff = Some(Instant::now());
        Ok(())
    }

    /// One command (§8): duplicates answered again, pending changes sent first, checks and the
    /// action on the blocking pool, the result, then the diff it caused.
    async fn command(&mut self, cmd: proto::Command) -> Result<(), End> {
        let received_ms = now_epoch_ms();
        if let Some(st) = self.st() {
            st.remote().diag().commands += 1;
        }
        // 1. Duplicate corr within 60 s: the same result again, nothing done.
        self.recent
            .retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        if let Some((_, r)) = self.recent.get(&cmd.corr) {
            let r = r.clone();
            return if self.out(&Out::Result(r)).await {
                Ok(())
            } else {
                Err(self.dead())
            };
        }
        // The history must be exact: whatever changed is sent before the checks.
        self.flush(None).await?;
        let Some(st) = self.st() else {
            return Err(End::ByUs);
        };
        let model = self.model.clone();
        let c = cmd.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            let input = st.remote_build_input();
            let m = model.lock().unwrap_or_else(PoisonError::into_inner);
            let ctx = commands::Ctx {
                enabled: st.remote_enabled(),
                take_locked: input.take_locked,
                model: &m,
                assets: &input.assets,
            };
            let r = commands::run(&st, &ctx, &c);
            let labels = match (&r, c.args["layer"].as_str()) {
                (Ok(_), Some(l)) => {
                    let template = m
                        .at(&format!("/layers/{l}/template"))
                        .and_then(Value::as_str);
                    template
                        .and_then(|t| m.at(&format!("/templates/{t}/fields")))
                        .and_then(Value::as_array)
                        .map(|fs| {
                            fs.iter()
                                .filter_map(|f| {
                                    Some((f["key"].as_str()?.to_string(), f["label"].clone()))
                                })
                                .collect::<serde_json::Map<String, Value>>()
                        })
                        .unwrap_or_default()
                }
                _ => serde_json::Map::new(),
            };
            (r, labels)
        })
        .await;
        let (outcome, labels) = match outcome {
            Ok(o) => o,
            Err(_) => (
                Err(commands::Refused {
                    code: "internal",
                    message: "Something went wrong on the show machine.".into(),
                    details: None,
                }),
                serde_json::Map::new(),
            ),
        };
        let done_ms = now_epoch_ms();
        let result = match &outcome {
            Ok(_) => CmdResult {
                corr: cmd.corr.clone(),
                ok: true,
                code: None,
                message: None,
                details: None,
                received_ms: Some(received_ms),
                done_ms: Some(done_ms),
            },
            Err(r) => CmdResult {
                corr: cmd.corr.clone(),
                ok: false,
                code: Some(r.code.to_string()),
                message: Some(r.message.clone()),
                details: r.details.clone(),
                received_ms: Some(received_ms),
                done_ms: Some(done_ms),
            },
        };
        if let Some(st) = self.st() {
            match &outcome {
                Ok(done) => {
                    st.remote().diag().accepted += 1;
                    st.remote()
                        .diag()
                        .result_time(done_ms.saturating_sub(received_ms));
                    let text = commands::describe(&cmd.actor.name, done, &labels);
                    st.remote_activity_now(&cmd.actor.name, &cmd.actor.role, text);
                }
                Err(r) => {
                    st.remote().diag().refusal(Refusal {
                        at: done_ms,
                        corr: cmd.corr.clone(),
                        action: proto::clip(&cmd.action, 32),
                        role: proto::clip(&cmd.actor.role, 32),
                        code: r.code.to_string(),
                        layer: cmd.args["layer"].as_str().map(|s| proto::clip(s, 64)),
                        cue_list: cmd.args["cue_list"].as_str().map(|s| proto::clip(s, 64)),
                        cue: cmd.args["cue"].as_str().map(|s| proto::clip(s, 64)),
                    });
                }
            }
        }
        self.recent
            .insert(cmd.corr.clone(), (Instant::now(), result.clone()));
        if !self.out(&Out::Result(result)).await {
            return Err(self.dead());
        }
        if outcome.is_ok() {
            self.flush(Some(cmd.corr)).await?;
        }
        Ok(())
    }
}

enum Decision {
    Takeover,
    NewShow,
    End(End),
}

/// "All N of your plan's live shows are in use." and the like.
fn limit_message(code: &str, max: Option<u64>, server: &str) -> String {
    if !server.trim().is_empty() {
        return proto::clip(server.trim(), proto::MESSAGE_MAX);
    }
    match (code, max) {
        ("run_limit_reached", Some(n)) => {
            format!("Your plan allows {n} live remote shows at a time, and they're all in use.")
        }
        ("run_limit_reached", None) => "Your plan's live remote shows are all in use.".into(),
        (_, Some(n)) => {
            format!("Your plan allows {n} remote shows. Delete one on the control site.")
        }
        _ => "Your plan's remote shows are all used. Delete one on the control site.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_follows_the_steps_with_jitter_bounds() {
        let t = Timing::LIVE;
        let ms = |a, u| backoff(&t, a, u).as_millis();
        assert_eq!(ms(0, 0.0), 350);
        assert_eq!(ms(0, 1.0), 650);
        assert_eq!(ms(1, 0.5), 1000);
        assert_eq!(ms(4, 0.5), 8000);
        assert_eq!(ms(5, 0.5), 15000);
        assert_eq!(ms(40, 0.0), 10500);
        assert_eq!(ms(40, 1.0), 19500);
    }

    #[test]
    fn ws_urls() {
        assert_eq!(
            ws_url("https://api.kinetra.live", false),
            "wss://api.kinetra.live/v1/control/app"
        );
        assert_eq!(
            ws_url("http://127.0.0.1:9/", true),
            "ws://127.0.0.1:9/v1/control/app?mode=probe"
        );
    }
}
