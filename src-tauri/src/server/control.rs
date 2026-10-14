//! The Control API's HTTP side (`docs/CONTROL_API.md`): `/api/control` (WebSocket) and the REST
//! routes under `/api/*`, merged into the output server (same port and binding). The rules and
//! the state live in `core::control_api`; this module is transport only.
//!
//! # Every request (the guard, in this order)
//! 1. An `Origin` header → `403 origin`: browsers always send it (WebSockets have no CORS, so a
//!    web page could otherwise reach `127.0.0.1`); Companion, curl and scripts don't.
//! 2. The API is off → `503 disabled`.
//! 3. More than [`AUTH_FAILURES_MAX`] wrong tokens from this IP in the last minute →
//!    `429 rate_limited`.
//! 4. `Authorization: Bearer <token>` must match → else `401 token` (counted for step 3).
//!    Query-string tokens are never accepted.
//!
//! # WebSocket session
//! `hello` within [`HELLO_WAIT`] (else close 4400), attach (8 clients at most: 4408), `welcome`,
//! the snapshot, then diffs from the shared feed and command results. Commands run one at a time
//! in arrival order on a worker task (blocking pool); up to [`QUEUE_MAX`] may wait (`busy`
//! beyond), at most [`COMMANDS_PER_SEC`] per second (`rate_limited` beyond). A `resync` gets a
//! snapshot, at most one per second. The app's kicks (disconnected, token changed, API off,
//! project switched) and the server's shutdown close the socket with their code.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval_at, Instant, MissedTickBehavior};

use super::routes::{shared_text, ExtraRoutes, PING_INTERVAL};
use crate::core::control_api::commands::http_status;
use crate::core::control_api::{
    close, token, AttachError, Out, Refused, TreeView, CLIENTS_MAX, SUBPROTOCOL,
};
use crate::core::state::{AppState, WeakState};

/// The first message must arrive within this.
pub const HELLO_WAIT: Duration = Duration::from_secs(5);

/// Commands waiting per connection (one runs; `busy` beyond).
pub const QUEUE_MAX: usize = 16;

/// Commands per second per connection (and per IP over REST).
pub const COMMANDS_PER_SEC: f64 = 50.0;

/// Wrong tokens per IP per minute before `429`.
pub const AUTH_FAILURES_MAX: u32 = 10;

/// Largest message a client may send (a list value is ≤ 64 KB).
pub const CLIENT_MSG_MAX: usize = 128 * 1024;

/// A `corr` is 1–64 characters.
const CORR_MAX: usize = 64;

/// IPs tracked by the limiters before old entries are dropped.
const TRACKED_IPS_MAX: usize = 1024;

/// The Control API routes, for [`super::ServerManager::set_extra_routes`].
pub fn routes(st: &AppState) -> ExtraRoutes {
    let weak = st.downgrade();
    let limits = Arc::new(Limits::default());
    Arc::new(move |shutdown| {
        router(Ctl {
            weak: weak.clone(),
            shutdown,
            limits: limits.clone(),
        })
    })
}

#[derive(Clone)]
struct Ctl {
    weak: WeakState,
    /// `true` when this server shuts down.
    shutdown: watch::Receiver<bool>,
    limits: Arc<Limits>,
}

/// The peer's IP (the guard puts it on every request).
#[derive(Clone, Copy)]
struct Peer(IpAddr);

fn router(ctl: Ctl) -> Router {
    Router::new()
        .route("/api/control", get(ws_upgrade))
        .route("/api/state", get(get_state))
        .route("/api/command", post(command))
        .route("/api/layers/{screen}/{layer}/fields", patch(set_fields))
        .route("/api/layers/{screen}/{layer}/{action}", post(layer_action))
        .route("/api/cues/next", post(go_next))
        .route("/api/cues/{id}/run", post(run_cue))
        .route("/api/screens/{id}/clear", post(clear_screen))
        .route("/api/clear-all", post(clear_all))
        .route("/api/panic", post(panic))
        .route("/api/sources/{id}/{dir}", post(source_step))
        .route("/api/flows/{id}/run", post(run_flow))
        .layer(middleware::from_fn_with_state(ctl.clone(), guard))
        .with_state(ctl)
}

// ---- Limits ------------------------------------------------------------------------------------

/// A token bucket: `rate` per second, bursts up to `rate`.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new() -> Self {
        Self {
            tokens: COMMANDS_PER_SEC,
            at: Instant::now(),
        }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        let refill = now.duration_since(self.at).as_secs_f64() * COMMANDS_PER_SEC;
        self.tokens = (self.tokens + refill).min(COMMANDS_PER_SEC);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
struct Limits {
    /// Wrong tokens per IP: (window start, count).
    failures: Mutex<HashMap<IpAddr, (Instant, u32)>>,
    /// REST commands per IP.
    rest: Mutex<HashMap<IpAddr, Bucket>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Limits {
    fn locked_out(&self, ip: IpAddr) -> bool {
        lock(&self.failures).get(&ip).is_some_and(|(at, n)| {
            at.elapsed() < Duration::from_secs(60) && *n >= AUTH_FAILURES_MAX
        })
    }

    fn failed(&self, ip: IpAddr) {
        let mut g = lock(&self.failures);
        if g.len() >= TRACKED_IPS_MAX {
            g.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        }
        let e = g.entry(ip).or_insert((Instant::now(), 0));
        if e.0.elapsed() >= Duration::from_secs(60) {
            *e = (Instant::now(), 0);
        }
        e.1 += 1;
    }

    fn rest_allowed(&self, ip: IpAddr) -> bool {
        let mut g = lock(&self.rest);
        if g.len() >= TRACKED_IPS_MAX {
            g.clear();
        }
        g.entry(ip).or_insert_with(Bucket::new).take()
    }
}

// ---- Responses ---------------------------------------------------------------------------------

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(json!({"code": code, "message": message}))).into_response()
}

fn refused(r: &Refused) -> Response {
    let status =
        StatusCode::from_u16(http_status(r.code)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    error(status, r.code, &r.message)
}

fn gone() -> Response {
    error(
        StatusCode::SERVICE_UNAVAILABLE,
        "disabled",
        "Kinetra is closing.",
    )
}

// ---- The guard ---------------------------------------------------------------------------------

async fn guard(State(ctl): State<Ctl>, mut req: Request, next: Next) -> Response {
    let ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |c| c.0.ip());
    if req.headers().contains_key(header::ORIGIN) {
        return error(
            StatusCode::FORBIDDEN,
            "origin",
            "Web pages can't use the Control API.",
        );
    }
    let Some(st) = ctl.weak.upgrade() else {
        return gone();
    };
    if !st.control_api_enabled() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "disabled",
            "Turn on Control API in Kinetra › Preferences › Network & API.",
        );
    }
    if ctl.limits.locked_out(ip) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many wrong tokens. Try again in a minute.",
        );
    }
    let given = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(token::bearer)
        .map(str::to_string);
    // The first check of a session may read the keychain: off the async threads.
    let ok = match given {
        Some(t) => tokio::task::spawn_blocking(move || st.control_api_check_token(&t))
            .await
            .unwrap_or(false),
        None => false,
    };
    if !ok {
        ctl.limits.failed(ip);
        return error(
            StatusCode::UNAUTHORIZED,
            "token",
            "Wrong or missing API token.",
        );
    }
    req.extensions_mut().insert(Peer(ip));
    next.run(req).await
}

// ---- REST --------------------------------------------------------------------------------------

/// Runs a command for a REST request (rate limited per IP) and answers `{ok: true}` or the
/// refusal with its HTTP status.
async fn run(ctl: &Ctl, ip: IpAddr, action: &str, args: Value) -> Response {
    if !ctl.limits.rest_allowed(ip) {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many commands. Slow down.",
        );
    }
    let Some(st) = ctl.weak.upgrade() else {
        return gone();
    };
    let action = action.to_string();
    let result =
        tokio::task::spawn_blocking(move || st.control_api_command(None, None, &action, &args))
            .await;
    match result {
        Ok(Ok(())) => Json(json!({"ok": true})).into_response(),
        Ok(Err(r)) => refused(&r),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed",
            "Something went wrong in Kinetra.",
        ),
    }
}

/// A JSON object body (an empty body is `{}`); `None` for anything else.
fn body_json(body: &Bytes) -> Option<Value> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Some(json!({}));
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(v @ Value::Object(_)) => Some(v),
        _ => None,
    }
}

fn not_an_object() -> Response {
    error(
        StatusCode::BAD_REQUEST,
        "invalid",
        "The body must be a JSON object.",
    )
}

fn peer(req_ext: &axum::http::Extensions) -> IpAddr {
    req_ext
        .get::<Peer>()
        .map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |p| p.0)
}

async fn get_state(State(ctl): State<Ctl>) -> Response {
    let Some(st) = ctl.weak.upgrade() else {
        return gone();
    };
    match tokio::task::spawn_blocking(move || st.control_api_tree()).await {
        Ok(tree) => Json(json!({"state": tree})).into_response(),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed",
            "Something went wrong in Kinetra.",
        ),
    }
}

async fn command(State(ctl): State<Ctl>, req: Request) -> Response {
    let ip = peer(req.extensions());
    let body = match axum::body::to_bytes(req.into_body(), CLIENT_MSG_MAX).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "invalid", "Too large."),
    };
    let v = match body_json(&body) {
        Some(v) => v,
        None => return not_an_object(),
    };
    let Some(action) = v["action"].as_str().map(str::to_string) else {
        return error(StatusCode::BAD_REQUEST, "invalid", "Missing \"action\".");
    };
    let args = v.get("args").cloned().unwrap_or_else(|| json!({}));
    run(&ctl, ip, &action, args).await
}

fn layer_key(screen: &str, layer: &str) -> String {
    format!("{screen}.{layer}")
}

async fn layer_action(
    State(ctl): State<Ctl>,
    Path((screen, layer, action)): Path<(String, String, String)>,
    req: Request,
) -> Response {
    if !matches!(action.as_str(), "take" | "clear" | "toggle" | "update") {
        return error(StatusCode::NOT_FOUND, "not_found", "Not found");
    }
    let ip = peer(req.extensions());
    run(
        &ctl,
        ip,
        &action,
        json!({"layer": layer_key(&screen, &layer)}),
    )
    .await
}

async fn set_fields(
    State(ctl): State<Ctl>,
    Path((screen, layer)): Path<(String, String)>,
    req: Request,
) -> Response {
    let ip = peer(req.extensions());
    let body = match axum::body::to_bytes(req.into_body(), CLIENT_MSG_MAX).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "invalid", "Too large."),
    };
    let mut v = match body_json(&body) {
        Some(v) => v,
        None => return not_an_object(),
    };
    v["layer"] = json!(layer_key(&screen, &layer));
    run(&ctl, ip, "set_fields", v).await
}

async fn go_next(State(ctl): State<Ctl>, req: Request) -> Response {
    run(&ctl, peer(req.extensions()), "go_next", json!({})).await
}

async fn run_cue(State(ctl): State<Ctl>, Path(id): Path<String>, req: Request) -> Response {
    run(&ctl, peer(req.extensions()), "run_cue", json!({"cue": id})).await
}

async fn clear_screen(State(ctl): State<Ctl>, Path(id): Path<String>, req: Request) -> Response {
    run(
        &ctl,
        peer(req.extensions()),
        "clear_screen",
        json!({"screen": id}),
    )
    .await
}

async fn clear_all(State(ctl): State<Ctl>, req: Request) -> Response {
    run(&ctl, peer(req.extensions()), "clear_all", json!({})).await
}

async fn panic(State(ctl): State<Ctl>, req: Request) -> Response {
    run(&ctl, peer(req.extensions()), "panic", json!({})).await
}

async fn source_step(
    State(ctl): State<Ctl>,
    Path((id, dir)): Path<(String, String)>,
    req: Request,
) -> Response {
    if !matches!(dir.as_str(), "next" | "prev") {
        return error(StatusCode::NOT_FOUND, "not_found", "Not found");
    }
    run(
        &ctl,
        peer(req.extensions()),
        "source_step",
        json!({"source": id, "dir": dir}),
    )
    .await
}

/// Fires the flow's API call trigger; the body's `value` (optional) is its `value` output.
async fn run_flow(State(ctl): State<Ctl>, Path(id): Path<String>, req: Request) -> Response {
    let ip = peer(req.extensions());
    let body = match axum::body::to_bytes(req.into_body(), CLIENT_MSG_MAX).await {
        Ok(b) => b,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "invalid", "Too large."),
    };
    let Some(v) = body_json(&body) else {
        return not_an_object();
    };
    let mut args = json!({"flow": id});
    if let Some(value) = v.get("value") {
        args["value"] = value.clone();
    }
    run(&ctl, ip, "run_flow", args).await
}

// ---- WebSocket ---------------------------------------------------------------------------------

async fn ws_upgrade(State(ctl): State<Ctl>, ws: WebSocketUpgrade, req: Request) -> Response {
    let ip = peer(req.extensions());
    let full = ctl
        .weak
        .upgrade()
        .is_some_and(|st| st.control_api_view().clients.len() >= CLIENTS_MAX);
    if full {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "busy",
            "Too many controllers are connected.",
        );
    }
    ws.protocols([SUBPROTOCOL])
        .max_message_size(CLIENT_MSG_MAX)
        .on_upgrade(move |socket| session(socket, ctl, ip))
}

async fn send_close(socket: &mut WebSocket, code: u16, reason: &str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

fn close_reason(code: u16) -> &'static str {
    match code {
        close::GOING_AWAY => "server stopping",
        close::BAD_MESSAGE => "bad message",
        close::TOKEN_CHANGED => "token changed",
        close::DISCONNECTED => "disconnected",
        close::CLIENT_LIMIT => "too many clients",
        close::TURNED_OFF => "control api off",
        _ => "",
    }
}

/// Waits for `{"t":"hello","protocol":n,"client":"…"}`; the client's name, or `None` for
/// anything else.
async fn read_hello(socket: &mut WebSocket) -> Option<String> {
    loop {
        match socket.recv().await? {
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(t.as_str()).ok()?;
                let ok = v["t"] == "hello" && v["protocol"].as_u64().is_some_and(|p| p >= 1);
                return ok.then(|| v["client"].as_str().unwrap_or_default().to_string());
            }
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            _ => return None,
        }
    }
}

fn result_text(corr: &str, r: &Result<(), Refused>) -> String {
    match r {
        Ok(()) => json!({"t": "result", "corr": corr, "ok": true}),
        Err(e) => {
            json!({"t": "result", "corr": corr, "ok": false, "code": e.code, "message": e.message})
        }
    }
    .to_string()
}

/// One queued command.
struct Job {
    corr: String,
    action: String,
    args: Value,
}

/// Runs a session's commands one at a time, in order, on the blocking pool.
async fn worker(
    weak: WeakState,
    tree: TreeView,
    mut jobs: mpsc::Receiver<Job>,
    results: mpsc::UnboundedSender<String>,
) {
    while let Some(job) = jobs.recv().await {
        let w = weak.clone();
        let t = tree.get();
        let corr = job.corr.clone();
        let r = tokio::task::spawn_blocking(move || match w.upgrade() {
            Some(st) => st.control_api_command(Some(&corr), t, &job.action, &job.args),
            None => Err(Refused {
                code: "failed",
                message: "Kinetra is closing.".into(),
            }),
        })
        .await
        .unwrap_or_else(|_| {
            Err(Refused {
                code: "failed",
                message: "Something went wrong in Kinetra.".into(),
            })
        });
        if results.send(result_text(&job.corr, &r)).is_err() {
            return;
        }
    }
}

/// Aborts the worker when the session ends.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn sleep_until_opt(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

async fn session(mut socket: WebSocket, ctl: Ctl, ip: IpAddr) {
    let Ctl {
        weak, mut shutdown, ..
    } = ctl;
    let name = match tokio::time::timeout(HELLO_WAIT, read_hello(&mut socket)).await {
        Ok(Some(n)) => n,
        _ => return send_close(&mut socket, close::BAD_MESSAGE, "hello expected").await,
    };
    let Some(st) = weak.upgrade() else {
        return send_close(&mut socket, close::GOING_AWAY, "server stopping").await;
    };
    let mut attached = match st.control_api_attach(&name, &ip.to_string()) {
        Ok(a) => a,
        Err(e) => {
            let code = match e {
                AttachError::Full => close::CLIENT_LIMIT,
                AttachError::Disabled => close::TURNED_OFF,
                AttachError::Unavailable => close::GOING_AWAY,
            };
            return send_close(&mut socket, code, close_reason(code)).await;
        }
    };
    let welcome = st.control_api_welcome().to_string();
    drop(st);
    if socket.send(Message::Text(welcome.into())).await.is_err() {
        return;
    }

    let (job_tx, job_rx) = mpsc::channel::<Job>(QUEUE_MAX);
    let (res_tx, mut res_rx) = mpsc::unbounded_channel::<String>();
    let _worker = AbortOnDrop(tokio::spawn(worker(
        weak.clone(),
        attached.tree_view(),
        job_rx,
        res_tx,
    )));
    let mut kick = attached.kick.clone();
    let mut bucket = Bucket::new();
    let mut last_resync: Option<Instant> = None;
    let mut resync_at: Option<Instant> = None;
    let mut ping = interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let code = loop {
        if let Some(code) = *kick.borrow() {
            break code;
        }
        tokio::select! {
            () = async { let _ = shutdown.wait_for(|&down| down).await; } => break close::GOING_AWAY,
            _ = kick.changed() => continue,
            out = attached.next() => {
                let Some(out) = out else { break close::GOING_AWAY };
                let msg = match out {
                    Out::Shared(s) => shared_text(&s),
                    Out::Own(s) => Message::Text(s.into()),
                };
                if socket.send(msg).await.is_err() {
                    return;
                }
            }
            Some(text) = res_rx.recv() => {
                if socket.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
            }
            () = sleep_until_opt(resync_at) => {
                resync_at = None;
                last_resync = Some(Instant::now());
                if let Some(out) = attached.resync() {
                    if socket.send(Message::Text(out.as_str().to_string().into())).await.is_err() {
                        return;
                    }
                }
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                    Some(Ok(Message::Binary(_))) => break close::BAD_MESSAGE,
                };
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    break close::BAD_MESSAGE;
                };
                match v["t"].as_str() {
                    Some("command") => {
                        let corr = v["corr"].as_str().unwrap_or_default();
                        let action = v["action"].as_str();
                        let (true, Some(action)) = (
                            !corr.is_empty() && corr.chars().count() <= CORR_MAX,
                            action,
                        ) else {
                            break close::BAD_MESSAGE;
                        };
                        let job = Job {
                            corr: corr.to_string(),
                            action: action.to_string(),
                            args: v.get("args").cloned().unwrap_or_else(|| json!({})),
                        };
                        let refusal = if !bucket.take() {
                            Some(("rate_limited", "Too many commands. Slow down."))
                        } else {
                            match job_tx.try_send(job) {
                                Ok(()) => None,
                                Err(_) => Some(("busy", "Too many commands waiting.")),
                            }
                        };
                        if let Some((code, message)) = refusal {
                            let r = Err(Refused { code, message: message.into() });
                            if socket.send(Message::Text(result_text(corr, &r).into())).await.is_err() {
                                return;
                            }
                        }
                    }
                    Some("resync") => {
                        // At most one snapshot per second; a second request waits for its slot.
                        let slot = last_resync.map_or_else(Instant::now, |t| t + Duration::from_secs(1));
                        if resync_at.is_none() {
                            resync_at = Some(slot.max(Instant::now()));
                        }
                    }
                    Some("hello") => {}
                    _ => break close::BAD_MESSAGE,
                }
            }
        }
    };
    send_close(&mut socket, code, close_reason(code)).await;
}

#[cfg(test)]
mod tests;
