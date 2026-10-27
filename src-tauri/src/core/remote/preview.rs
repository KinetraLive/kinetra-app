//! Live preview (protocol 2 §6): downscaled JPEG pictures of a screen's Program (and Preview in
//! Studio mode) for collaborators with `watch_preview`, only while someone watches, never at the
//! cost of the output.
//!
//! - **Demand → plan** ([`plan`], pure): every stream of the server's `preview_demand` is either
//!   captured or reported inactive in `preview_state` (`disabled`, `unsupported`, `limit`,
//!   `busy`), in the demand's order, at most the app's cap.
//! - **Capture** is the [`PreviewHost`]'s (Windows: `src-tauri/src/preview.rs`): the renderer page
//!   rendered at the picture's size (the output chunk's `bg=checker` paints the protocol's
//!   checkerboard behind transparency), read back off the output clocks, sent only when it
//!   changed, JPEG-encoded at background priority, into the [`PreviewSink`] (latest wins per
//!   stream). Nothing exists without a demand.
//! - **Rate:** at most the demand's `fps`; while anything is on air (`cloud_on_air`: a Program
//!   layer, an output showing, OBS / vMix connected) at most 1 picture per second (owner
//!   decision 11). **Busy** ([`Busy`]): an NDI output dropping frames, two late pictures in a
//!   row, or the CPU above 85 % for 3 s pause it; 10 s quiet resume it.
//! - **Socket** ([`run_socket`]): a second WebSocket opened only while a stream is active
//!   (Bearer + the demand's single-use ticket, `kinetra-control-preview.v1`), frames sent only
//!   with credit (none → the newest waits, older ones are replaced, never queued). Losing it
//!   never touches the control socket.
//!
//! Nothing from the preview is logged or stored.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{watch, Notify};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use super::proto::{DemandStream, PreviewDemand, StreamState};
use crate::core::fsx::now_epoch_ms;

/// The preview socket's subprotocol (§6.2).
pub const SUBPROTOCOL: &str = "kinetra-control-preview.v1";
/// The frame header's length (§6.2 "Frame format").
pub const HEADER_BYTES: usize = 20;
/// The picture width when the demand gives none.
pub const DEFAULT_WIDTH: u32 = 640;
/// The widest picture the app makes, whatever the demand asks.
pub const WIDTH_MAX: u32 = 1920;
/// Pictures per second while anything is on air (owner decision 11).
pub const ON_AIR_FPS: u32 = 1;

/// Inactive streams: `reason` and the words browsers show (§6.1).
pub const NOT_HERE: (&str, &str) = (
    "unsupported",
    "The live picture isn't available on this Kinetra computer.",
);
pub const DISABLED: (&str, &str) = ("disabled", "Remote preview is off on the show machine.");
pub const STUDIO_OFF: (&str, &str) = ("unsupported", "Studio mode is off on the show machine.");
pub const NO_SCREEN: (&str, &str) = ("unsupported", "This screen isn't on the show machine.");
pub const CANT_CAPTURE: (&str, &str) = ("unsupported", "This output can't be captured.");
pub const LIMIT: (&str, &str) = ("limit", "Preview limit reached on the Kinetra computer.");
pub const BUSY: (&str, &str) = ("busy", "Paused while the show machine is busy.");

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One capture the host runs: a page at a picture size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePlan {
    pub stream: u16,
    /// The renderer page (`/screen/<id>[/preview]?output=preview&bg=checker&mute=1&audio=0`).
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// What the host captures now (empty: nothing, no threads).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreviewPlan {
    pub captures: Vec<CapturePlan>,
    /// JPEG quality 1–100 (the host may go lower to stay under `max_bytes`).
    pub quality: u8,
    /// The largest frame message (header included).
    pub max_bytes: usize,
}

/// Captures pictures for the preview (Windows only, set in `lib.rs`).
pub trait PreviewHost: Send + Sync {
    /// Runs exactly `plan.captures` (a changed capture restarts; an empty plan stops everything
    /// and frees its threads, pages and GPU memory), delivering pictures to `sink`. Blocking
    /// (starting a page takes a moment): called on the blocking pool. Returns the streams that
    /// couldn't be captured.
    fn apply(&self, plan: &PreviewPlan, sink: &Arc<PreviewSink>) -> HashMap<u16, String>;
}

/// One encoded picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub jpeg: Vec<u8>,
    pub width: u16,
    pub height: u16,
    /// Unix ms (app clock).
    pub captured_ms: u64,
}

#[derive(Default)]
struct Slot {
    /// The newest picture not sent yet.
    pending: Option<PreviewFrame>,
    /// Frames sent on this stream (the header's frame number).
    sent: u32,
}

/// Between the host's worker and the preview socket: the newest picture per stream, the pace and
/// the busy reports. Never blocks either side for long.
pub struct PreviewSink {
    slots: Mutex<BTreeMap<u16, Slot>>,
    wake: Notify,
    /// Shortest time between two pictures of one stream (ms).
    interval_ms: AtomicU32,
    /// Busy: the host does no capture work.
    paused: AtomicBool,
    /// Late pictures in a row (the host's worker fell behind its schedule).
    late_run: AtomicU32,
    /// The host saw the CPU above 85 % in its last sample.
    cpu_high: AtomicBool,
}

impl Default for PreviewSink {
    fn default() -> Self {
        Self {
            slots: Mutex::default(),
            wake: Notify::new(),
            interval_ms: AtomicU32::new(1000 / 5),
            paused: AtomicBool::new(false),
            late_run: AtomicU32::new(0),
            cpu_high: AtomicBool::new(false),
        }
    }
}

impl PreviewSink {
    /// Shortest time between two pictures of one stream.
    pub fn interval(&self) -> Duration {
        Duration::from_millis(u64::from(self.interval_ms.load(Ordering::Relaxed)))
    }

    pub fn set_interval(&self, d: Duration) {
        let ms = u32::try_from(d.as_millis()).unwrap_or(u32::MAX).max(1);
        self.interval_ms.store(ms, Ordering::Relaxed);
    }

    /// Busy: the host skips capture work until resumed.
    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub(crate) fn set_paused(&self, on: bool) {
        self.paused.store(on, Ordering::Relaxed);
    }

    /// The newest picture of `stream` (replaces an unsent one).
    pub fn put(&self, stream: u16, frame: PreviewFrame) {
        lock(&self.slots).entry(stream).or_default().pending = Some(frame);
        self.wake.notify_one();
    }

    /// The host's worker made a picture on time (`false`: it was late for its schedule).
    pub fn on_time(&self, on_time: bool) {
        if on_time {
            self.late_run.store(0, Ordering::Relaxed);
        } else {
            self.late_run.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The host's CPU sample (above 85 % or not).
    pub fn cpu(&self, high: bool) {
        self.cpu_high.store(high, Ordering::Relaxed);
    }

    /// The host's reports for [`Busy`]: late pictures in a row, CPU above 85 %.
    pub(crate) fn reports(&self) -> (u32, bool) {
        (
            self.late_run.load(Ordering::Relaxed),
            self.cpu_high.load(Ordering::Relaxed),
        )
    }

    /// Forgets the streams not in `keep` (a new demand).
    pub(crate) fn retain(&self, keep: &[u16]) {
        lock(&self.slots).retain(|s, _| keep.contains(s));
    }

    /// The next picture to send, round robin after `after`: (stream, frame number, picture).
    pub fn take_next(&self, after: u16) -> Option<(u16, u32, PreviewFrame)> {
        let mut g = lock(&self.slots);
        let pick = g
            .range(after.saturating_add(1)..)
            .chain(g.range(..=after))
            .find(|(_, s)| s.pending.is_some())
            .map(|(k, _)| *k)?;
        let slot = g.get_mut(&pick)?;
        let frame = slot.pending.take()?;
        slot.sent = slot.sent.wrapping_add(1);
        Some((pick, slot.sent, frame))
    }

    #[cfg(test)]
    fn has_pending(&self) -> bool {
        lock(&self.slots).values().any(|s| s.pending.is_some())
    }
}

/// The frame message: the 20-byte big-endian header (§6.2), then the JPEG.
pub fn frame_message(stream: u16, number: u32, f: &PreviewFrame) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_BYTES + f.jpeg.len());
    out.push(1); // format version
    out.push(1); // codec: JPEG
    out.extend_from_slice(&stream.to_be_bytes());
    out.extend_from_slice(&number.to_be_bytes());
    out.extend_from_slice(&f.captured_ms.to_be_bytes());
    out.extend_from_slice(&f.width.to_be_bytes());
    out.extend_from_slice(&f.height.to_be_bytes());
    out.extend_from_slice(&f.jpeg);
    out
}

/// What [`plan`] needs to know about the machine.
#[derive(Debug, Clone, Default)]
pub struct PlanInput {
    /// A [`PreviewHost`] is set (Windows).
    pub can_capture: bool,
    /// "Allow remote preview" of the open project.
    pub allowed: bool,
    pub studio: bool,
    /// The screens remote users see, with their size.
    pub screens: HashMap<String, (u32, u32)>,
    /// The output server's port (no port: nothing to capture yet).
    pub port: Option<u16>,
    /// The app's cap (`max_streams`).
    pub cap: u8,
    pub busy: bool,
    /// Streams the host couldn't capture.
    pub failed: HashMap<u16, String>,
    /// `welcome.limits.preview_frame_bytes`.
    pub max_bytes: usize,
}

/// The picture size for a w×h screen at most `max_width` wide (aspect kept, both ≥ 16).
pub fn picture_size(w: u32, h: u32, max_width: u32) -> (u32, u32) {
    let max_w = match max_width {
        0 => DEFAULT_WIDTH,
        m => m.min(WIDTH_MAX),
    };
    let pw = w.min(max_w).max(16);
    let ph = ((u64::from(h) * u64::from(pw) + u64::from(w) / 2) / u64::from(w.max(1))) as u32;
    (pw, ph.max(16))
}

fn enc(s: &str) -> String {
    crate::core::outputs::enc(s)
}

/// The demand → what to capture and what to tell the server for every stream (in the demand's
/// order). Pure.
pub fn plan(demand: &PreviewDemand, input: &PlanInput) -> (PreviewPlan, Vec<StreamState>) {
    let mut captures = Vec::new();
    let mut states = Vec::new();
    let mut served = 0usize;
    for d in &demand.streams {
        let inactive = |(reason, message): (&'static str, &'static str)| StreamState {
            stream: d.stream,
            active: false,
            reason: Some(reason),
            message: Some(message),
        };
        let screen = input.screens.get(&d.screen);
        let state = if !input.can_capture {
            inactive(NOT_HERE)
        } else if !input.allowed {
            inactive(DISABLED)
        } else if d.kind != "program" && d.kind != "preview" {
            inactive(NOT_HERE)
        } else if d.kind == "preview" && !input.studio {
            inactive(STUDIO_OFF)
        } else if screen.is_none() || input.port.is_none() {
            inactive(NO_SCREEN)
        } else if served >= usize::from(input.cap) {
            inactive(LIMIT)
        } else if input.failed.contains_key(&d.stream) {
            served += 1;
            inactive(CANT_CAPTURE)
        } else {
            served += 1;
            if let (Some(&(w, h)), Some(port)) = (screen, input.port) {
                captures.push(capture_plan(d, w, h, port, demand.max_width));
            }
            if input.busy {
                inactive(BUSY)
            } else {
                StreamState {
                    stream: d.stream,
                    active: true,
                    reason: None,
                    message: None,
                }
            }
        };
        states.push(state);
    }
    let quality = match demand.quality {
        0 => 70,
        q => q.min(100),
    };
    (
        PreviewPlan {
            captures,
            quality,
            max_bytes: input.max_bytes,
        },
        states,
    )
}

fn capture_plan(d: &DemandStream, w: u32, h: u32, port: u16, max_width: u32) -> CapturePlan {
    let (width, height) = picture_size(w, h, max_width);
    let bus = if d.kind == "preview" { "/preview" } else { "" };
    CapturePlan {
        stream: d.stream,
        url: format!(
            "http://127.0.0.1:{port}/screen/{}{bus}?output=preview&bg=checker&mute=1&audio=0",
            enc(&d.screen)
        ),
        width,
        height,
    }
}

/// The time between two pictures of a stream: the demand's rate (≤ the server's limit), at
/// most [`ON_AIR_FPS`] while anything is on air.
pub fn interval(demand_fps: u32, limit_fps: u32, on_air: bool) -> Duration {
    let mut fps = match demand_fps {
        0 => limit_fps.max(1),
        f => f.min(limit_fps.max(1)),
    };
    if on_air {
        fps = fps.min(ON_AIR_FPS);
    }
    Duration::from_millis(1000 / u64::from(fps.max(1)))
}

/// The busy rule (owner decision 6): pause when an NDI output drops frames, a picture is late
/// twice in a row, or the CPU stays above 85 % for 3 s; resume after 10 s without any of them.
#[derive(Debug, Default)]
pub struct Busy {
    busy: bool,
    cpu_since: Option<Instant>,
    quiet_since: Option<Instant>,
}

pub const CPU_FOR: Duration = Duration::from_secs(3);
pub const QUIET_FOR: Duration = Duration::from_secs(10);

impl Busy {
    /// One sample (about every second); returns whether preview is paused now.
    pub fn sample(
        &mut self,
        now: Instant,
        ndi_dropping: bool,
        late_run: u32,
        cpu_high: bool,
    ) -> bool {
        self.cpu_since = if cpu_high {
            Some(self.cpu_since.unwrap_or(now))
        } else {
            None
        };
        let cpu = self
            .cpu_since
            .is_some_and(|t| now.duration_since(t) >= CPU_FOR);
        let trouble = ndi_dropping || late_run >= 2 || cpu;
        if trouble {
            self.busy = true;
            self.quiet_since = None;
        } else if self.busy {
            let since = *self.quiet_since.get_or_insert(now);
            if now.duration_since(since) >= QUIET_FOR {
                self.busy = false;
                self.quiet_since = None;
            }
        }
        self.busy
    }

    pub fn is_busy(&self) -> bool {
        self.busy
    }
}

/// The preview socket's URL is only ever on the API's own host (the session token goes there):
/// `wss://` on the base's host, or `ws://` when the base itself is `http://` (debug and tests).
pub fn url_allowed(url: &str, api_base: &str) -> bool {
    fn split(u: &str) -> Option<(&str, &str)> {
        let (scheme, rest) = u.split_once("://")?;
        let host = rest.split(['/', '?', '#']).next()?;
        (!host.is_empty() && !host.contains('@')).then_some((scheme, host))
    }
    let (Some((scheme, host)), Some((base_scheme, base_host))) = (split(url), split(api_base))
    else {
        return false;
    };
    let scheme_ok = match base_scheme {
        "https" => scheme == "wss",
        "http" => scheme == "ws" || scheme == "wss",
        _ => false,
    };
    scheme_ok && host.eq_ignore_ascii_case(base_host)
}

/// What the preview socket task needs; the ticket arrives (and is replaced) through `ticket`.
pub(crate) struct SocketJob {
    pub url: String,
    pub session: String,
    pub run_id: String,
    pub ticket: watch::Receiver<Option<String>>,
    pub sink: Arc<PreviewSink>,
    pub max_bytes: usize,
    pub heartbeat: Duration,
    pub silence: Duration,
    pub handshake: Duration,
    pub backoff: fn(u32) -> Duration,
    /// Frames sent / dropped for the diagnostics.
    pub sent: Arc<std::sync::atomic::AtomicU64>,
}

/// How one preview connection ended.
#[derive(Debug, PartialEq, Eq)]
enum SocketEnd {
    /// The ticket was refused (4401) or used: wait for a new one.
    NeedTicket,
    /// Anything else: reconnect after the backoff.
    Retry,
}

/// The preview socket: opened with the newest unused ticket, reopened with the backoff while the
/// task lives (the session stops it when no stream is active). Ends when `ticket` closes.
pub(crate) async fn run_socket(mut job: SocketJob) {
    let mut attempt = 0u32;
    let mut used: Option<String> = None;
    loop {
        let ticket = loop {
            let t = job.ticket.borrow_and_update().clone();
            match t {
                Some(t) if used.as_ref() != Some(&t) => break t,
                _ => {
                    if job.ticket.changed().await.is_err() {
                        return;
                    }
                }
            }
        };
        used = Some(ticket.clone());
        let started = Instant::now();
        let end = connection(&mut job, &ticket).await;
        if started.elapsed() >= Duration::from_secs(60) {
            attempt = 0;
        }
        if end == SocketEnd::Retry {
            let wait = (job.backoff)(attempt);
            attempt = attempt.saturating_add(1);
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                r = job.ticket.changed() => if r.is_err() { return },
            }
            // Single-use tickets: a retry needs a new one unless the server sent it meanwhile.
        }
    }
}

async fn connection(job: &mut SocketJob, ticket: &str) -> SocketEnd {
    let Ok(mut req) = job.url.as_str().into_client_request() else {
        return SocketEnd::NeedTicket;
    };
    let h = req.headers_mut();
    let Ok(auth) = HeaderValue::from_str(&format!("Bearer {}", job.session)) else {
        return SocketEnd::NeedTicket;
    };
    h.insert(header::AUTHORIZATION, auth);
    h.insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(SUBPROTOCOL),
    );
    let cfg = WebSocketConfig::default()
        .max_message_size(Some(64 * 1024))
        .max_frame_size(Some(64 * 1024));
    let fut = tokio_tungstenite::connect_async_with_config(req, Some(cfg), true);
    let ws = match tokio::time::timeout(job.handshake, fut).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(resp))) if resp.status() == 401 => {
            return SocketEnd::NeedTicket
        }
        _ => return SocketEnd::Retry,
    };
    let (mut tx, mut rx) = ws.split();
    let hello = json!({"t": "preview_hello", "run_id": job.run_id, "ticket": ticket});
    if tx.send(Message::text(hello.to_string())).await.is_err() {
        return SocketEnd::Retry;
    }
    // preview_welcome {credit}
    let mut credit: u64 = loop {
        let got = tokio::time::timeout(job.handshake, rx.next()).await;
        match got {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
                if v["t"] == "preview_welcome" {
                    break v["credit"].as_u64().unwrap_or(0);
                }
            }
            Ok(Some(Ok(Message::Close(f)))) => {
                return match f.map(|f| u16::from(f.code)) {
                    Some(4401) => SocketEnd::NeedTicket,
                    _ => SocketEnd::Retry,
                }
            }
            Ok(Some(Ok(_))) => {}
            _ => return SocketEnd::Retry,
        }
    };
    let mut last_stream = u16::MAX;
    let mut last_rx = Instant::now();
    let mut heartbeat =
        tokio::time::interval_at(tokio::time::Instant::now() + job.heartbeat, job.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        // Send what credit allows before waiting.
        while credit > 0 {
            let Some((stream, number, frame)) = job.sink.take_next(last_stream) else {
                break;
            };
            last_stream = stream;
            let msg = frame_message(stream, number, &frame);
            if msg.len() > job.max_bytes {
                continue;
            }
            if tx.send(Message::binary(msg)).await.is_err() {
                return SocketEnd::Retry;
            }
            credit -= 1;
            job.sent.fetch_add(1, Ordering::Relaxed);
        }
        let silence_at = last_rx + job.silence;
        tokio::select! {
            got = rx.next() => {
                last_rx = Instant::now();
                match got {
                    Some(Ok(Message::Text(t))) => {
                        let v: Value = serde_json::from_str(t.as_str()).unwrap_or(Value::Null);
                        match v["t"].as_str() {
                            Some("credit") => credit = credit.saturating_add(v["n"].as_u64().unwrap_or(0)),
                            Some("ping") => {
                                let now = now_epoch_ms();
                                let pong = json!({"t": "pong", "corr": v["corr"],
                                    "ping_sent_ms": v["sent_ms"], "received_ms": now, "sent_ms": now});
                                if tx.send(Message::text(pong.to_string())).await.is_err() {
                                    return SocketEnd::Retry;
                                }
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(f))) => {
                        return match f.map(|f| u16::from(f.code)) {
                            // The demand became empty or the run ended: wait for a new ticket.
                            Some(1000) | Some(4401) => SocketEnd::NeedTicket,
                            _ => SocketEnd::Retry,
                        };
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return SocketEnd::Retry,
                }
            }
            // Without credit the newest pictures wait in their slots (replaced, never queued).
            () = job.sink.wake.notified(), if credit > 0 => {}
            _ = heartbeat.tick() => {
                let ping = json!({"t": "ping", "corr": super::ids::uuid_v7(), "sent_ms": now_epoch_ms()});
                if tx.send(Message::text(ping.to_string())).await.is_err() {
                    return SocketEnd::Retry;
                }
            }
            () = tokio::time::sleep_until(silence_at.into()) => return SocketEnd::Retry,
            // A new ticket while open is ignored (§6.1); the session dropping it ends the socket.
            r = job.ticket.changed() => {
                if r.is_err() {
                    let _ = tx.close().await;
                    return SocketEnd::NeedTicket;
                }
            }
        }
    }
}

impl crate::core::state::AppState {
    /// Sets the preview capture (Windows, `lib.rs`); `None`: this computer can't capture, so
    /// `hello` offers no preview and every demanded stream is `unsupported`.
    pub fn set_preview_host(&self, host: Option<Arc<dyn PreviewHost>>) {
        *lock(&self.remote().preview_host) = host;
    }

    pub(crate) fn preview_host(&self) -> Option<Arc<dyn PreviewHost>> {
        lock(&self.remote().preview_host).clone()
    }

    /// `hello.preview.max_streams` / `preview_caps`: this machine's cap (Preferences › Remote
    /// control › Preview streams), `None` without a capture host.
    pub(crate) fn remote_preview_caps(&self) -> Option<u8> {
        self.preview_host()
            .is_some()
            .then(|| self.prefs().remote.preview_streams)
    }

    /// The open project allows remote preview ("Allow remote preview", on by default).
    pub(crate) fn remote_preview_allowed(&self) -> bool {
        self.project_remote_preview()
    }

    /// Prefs or the project's settings changed: a live session re-reads the cap and "Allow
    /// remote preview" (never blocks; nothing when remote control is off).
    pub(crate) fn remote_preview_settings_changed(&self) {
        if self.remote().active.load(Ordering::Relaxed) {
            let _ = self.remote_ctl(super::Ctl::PreviewSettings);
        }
    }

    /// What [`plan`] reads from the machine now.
    pub(crate) fn preview_plan_input(
        &self,
        cap: u8,
        busy: bool,
        failed: &HashMap<u16, String>,
        max_bytes: usize,
    ) -> PlanInput {
        let live = self.live();
        let screens = live
            .user_screen_ids()
            .into_iter()
            .take(super::proto::SCREENS_MAX)
            .filter_map(|id| live.screen_size(&id).map(|wh| (id, wh)))
            .collect();
        PlanInput {
            can_capture: self.preview_host().is_some(),
            allowed: self.remote_preview_allowed(),
            studio: live.is_studio(),
            screens,
            port: self.output_server_port(),
            cap,
            busy,
            failed: failed.clone(),
            max_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demand(streams: &[(u16, &str, &str)]) -> PreviewDemand {
        PreviewDemand {
            streams: streams
                .iter()
                .map(|(s, sc, k)| DemandStream {
                    stream: *s,
                    screen: sc.to_string(),
                    kind: k.to_string(),
                })
                .collect(),
            fps: 5,
            max_width: 640,
            quality: 70,
            url: "wss://api.kinetra.live/v1/control/app/preview".into(),
            ticket: Some("t".into()),
        }
    }

    fn input() -> PlanInput {
        PlanInput {
            can_capture: true,
            allowed: true,
            studio: false,
            screens: [
                ("s1".to_string(), (1920, 1080)),
                ("s2".to_string(), (1080, 1920)),
            ]
            .into_iter()
            .collect(),
            port: Some(7770),
            cap: 2,
            busy: false,
            failed: HashMap::new(),
            max_bytes: 256 * 1024,
        }
    }

    fn reasons(states: &[StreamState]) -> Vec<(u16, Option<&str>)> {
        states.iter().map(|s| (s.stream, s.reason)).collect()
    }

    #[test]
    fn a_demand_becomes_captures_and_states_in_its_order() {
        let (p, st) = plan(
            &demand(&[(1, "s1", "program"), (2, "s2", "program")]),
            &input(),
        );
        assert_eq!(reasons(&st), vec![(1, None), (2, None)]);
        assert_eq!(p.captures.len(), 2);
        assert_eq!(
            p.captures[0].url,
            "http://127.0.0.1:7770/screen/s1?output=preview&bg=checker&mute=1&audio=0"
        );
        assert_eq!((p.captures[0].width, p.captures[0].height), (640, 360));
        // A portrait screen: 640 wide too.
        assert_eq!((p.captures[1].width, p.captures[1].height), (640, 1138));
        assert_eq!(p.quality, 70);
    }

    #[test]
    fn inactive_streams_say_why() {
        let d = demand(&[
            (1, "s1", "preview"),
            (2, "gone", "program"),
            (3, "s1", "program"),
            (4, "s2", "program"),
            (5, "s2", "program"),
        ]);
        let (p, st) = plan(&d, &input());
        // Preview without Studio mode; a screen that isn't there; then the cap of 2.
        assert_eq!(
            reasons(&st),
            vec![
                (1, Some("unsupported")),
                (2, Some("unsupported")),
                (3, None),
                (4, None),
                (5, Some("limit"))
            ]
        );
        assert_eq!(st[0].message, Some(STUDIO_OFF.1));
        assert_eq!(
            p.captures.iter().map(|c| c.stream).collect::<Vec<_>>(),
            [3, 4]
        );

        let mut i = input();
        i.studio = true;
        let (p, _) = plan(&demand(&[(1, "s1", "preview")]), &i);
        assert!(p.captures[0].url.contains("/screen/s1/preview?"));

        i.allowed = false;
        let (p, st) = plan(&d, &i);
        assert!(p.captures.is_empty());
        assert!(st.iter().all(|s| s.reason == Some("disabled")));

        let mut i = input();
        i.can_capture = false;
        let (_, st) = plan(&d, &i);
        assert!(st.iter().all(|s| s.message == Some(NOT_HERE.1)));
    }

    #[test]
    fn busy_keeps_the_captures_but_reports_every_stream() {
        let mut i = input();
        i.busy = true;
        let (p, st) = plan(&demand(&[(1, "s1", "program")]), &i);
        assert_eq!(p.captures.len(), 1);
        assert_eq!(reasons(&st), vec![(1, Some("busy"))]);
        let mut i = input();
        i.failed.insert(1, "no".into());
        let (p, st) = plan(&demand(&[(1, "s1", "program"), (2, "s2", "program")]), &i);
        // A failed capture still counts toward the cap.
        assert_eq!(reasons(&st), vec![(1, Some("unsupported")), (2, None)]);
        assert_eq!(p.captures.len(), 1);
    }

    #[test]
    fn sizes_rates_and_quality() {
        assert_eq!(picture_size(1920, 1080, 0), (640, 360));
        assert_eq!(picture_size(320, 180, 640), (320, 180));
        assert_eq!(picture_size(7680, 4320, 4000), (1920, 1080));
        assert_eq!(picture_size(16, 9000, 640), (16, 9000));
        assert_eq!(interval(5, 5, false), Duration::from_millis(200));
        assert_eq!(interval(30, 5, false), Duration::from_millis(200));
        assert_eq!(interval(0, 5, false), Duration::from_millis(200));
        assert_eq!(interval(5, 5, true), Duration::from_millis(1000));
        let mut d = demand(&[]);
        d.quality = 0;
        assert_eq!(plan(&d, &input()).0.quality, 70);
    }

    #[test]
    fn busy_pauses_and_resumes_after_quiet() {
        let t0 = Instant::now();
        let s = |secs: u64| t0 + Duration::from_secs(secs);
        let mut b = Busy::default();
        assert!(!b.sample(s(0), false, 0, false));
        // One late picture is fine, two in a row pause.
        assert!(!b.sample(s(1), false, 1, false));
        assert!(b.sample(s(2), false, 2, false));
        // Quiet from 3 s: still paused at 12 s, resumed at 13 s.
        assert!(b.sample(s(3), false, 0, false));
        assert!(b.sample(s(12), false, 0, false));
        assert!(!b.sample(s(13), false, 0, false));
        // CPU: needs 3 s in a row.
        assert!(!b.sample(s(20), false, 0, true));
        assert!(!b.sample(s(22), false, 0, true));
        assert!(b.sample(s(23), false, 0, true));
        let mut b = Busy::default();
        assert!(!b.sample(s(0), false, 0, true));
        assert!(!b.sample(s(1), false, 0, false));
        assert!(!b.sample(s(3), false, 0, true));
        // NDI dropping: at once.
        assert!(b.sample(s(4), true, 0, false));
    }

    #[test]
    fn frames_carry_the_header() {
        let f = PreviewFrame {
            jpeg: vec![0xff, 0xd8, 0xff, 0xd9],
            width: 640,
            height: 360,
            captured_ms: 0x0102_0304_0506_0708,
        };
        let m = frame_message(0x0a0b, 7, &f);
        assert_eq!(m.len(), HEADER_BYTES + 4);
        assert_eq!(
            &m[..HEADER_BYTES],
            &[1, 1, 0x0a, 0x0b, 0, 0, 0, 7, 1, 2, 3, 4, 5, 6, 7, 8, 0x02, 0x80, 0x01, 0x68]
        );
        assert_eq!(&m[HEADER_BYTES..], &f.jpeg[..]);
    }

    #[test]
    fn the_sink_keeps_the_newest_and_takes_round_robin() {
        let s = PreviewSink::default();
        let f = |n: u8| PreviewFrame {
            jpeg: vec![n],
            width: 1,
            height: 1,
            captured_ms: 0,
        };
        s.put(1, f(1));
        s.put(1, f(2));
        s.put(2, f(3));
        let (a, an, af) = s.take_next(u16::MAX).unwrap();
        assert_eq!((a, an, af.jpeg[0]), (1, 1, 2));
        let (b, _, bf) = s.take_next(a).unwrap();
        assert_eq!((b, bf.jpeg[0]), (2, 3));
        assert!(s.take_next(b).is_none());
        s.put(1, f(4));
        assert_eq!(s.take_next(2).unwrap().1, 2);
        s.retain(&[2]);
        s.put(1, f(5));
        s.retain(&[2]);
        assert!(!s.has_pending());
    }

    #[test]
    fn the_socket_only_goes_to_the_api_host() {
        let base = "https://api.kinetra.live";
        assert!(url_allowed(
            "wss://api.kinetra.live/v1/control/app/preview",
            base
        ));
        assert!(url_allowed("wss://API.kinetra.live/x", base));
        assert!(!url_allowed("ws://api.kinetra.live/x", base));
        assert!(!url_allowed("wss://evil.example/x", base));
        assert!(!url_allowed("wss://api.kinetra.live.evil.example/x", base));
        assert!(!url_allowed("wss://user@api.kinetra.live/x", base));
        assert!(!url_allowed("api.kinetra.live/x", base));
        assert!(url_allowed(
            "ws://127.0.0.1:5000/p",
            "http://127.0.0.1:5000"
        ));
        assert!(!url_allowed(
            "ws://127.0.0.1:5001/p",
            "http://127.0.0.1:5000"
        ));
    }
}
