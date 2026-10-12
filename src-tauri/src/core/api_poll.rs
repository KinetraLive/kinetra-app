//! API sources: the poller, the test request and the API source operations.
//!
//! # When a source polls
//! One tokio task (on the hub's runtime: Tauri's in the app) per *active* API source: it has a
//! URL, is not paused or held, and is in use (a screen layer binds it, a node of an armed flow
//! reads or acts on it, or an editor watches it with `watch_api_source`; counted) or has
//! `onlyWhenUsed` off. No task exists otherwise. [`AppState::api_reconcile`] decides; it runs
//! after every live change (the hub's hook: every screen apply, so bindings are covered), source
//! save / create / delete / reload, flow deploy / arm / remove / load, watch changes, pause,
//! retry and secret changes. It is cheap when the project has no API sources.
//!
//! # A poll
//! The task builds the request from the source (secret header values from the
//! [`SecretStore`](crate::core::secrets::SecretStore); a missing one holds the source in
//! `secretMissing` without a request), sends it with `If-None-Match` / `If-Modified-Since`, reads
//! at most `API_RESPONSE_MAX_BYTES`, and maps it on the blocking pool (`core::api_map`).
//! - A 304, a body whose hash equals the last one, or rows equal to the current ones change
//!   nothing: no event, no write, no live message.
//! - New rows replace the source's rows in memory (`fetchedAt` set, `index` clamped), are written
//!   coalesced by the content flusher, re-apply every bound screen with the bound fields of
//!   on-air layers forced on air (as auto-advance does; in Studio mode Program follows and the
//!   Preview is rebased), re-arm auto-advance, fire the flows' `trigger.sourceRow` (when the
//!   current row changed) and `trigger.apiChanged`, and emit `source-changed`.
//!
//! # Errors and backoff
//! Network errors, 5xx and other failing statuses, and mapping errors double the wait from the
//! interval up to [`BACKOFF_CAP`] ([`backoff`]), state `backoff`. 429 waits `Retry-After`
//! (seconds or an HTTP date, at most [`RETRY_AFTER_MAX`]) when given, else backs off. The first
//! success resets it. 401 / 403 hold the source (state `error`, no task) until Retry, a save that
//! changes its request, mapping or interval, or a secret change.
//!
//! # Status
//! [`ApiStatus`] per source, in `SourceSummary.api` and `source-changed`. An event goes out only
//! when the state, HTTP status or message changes, or the rows change; a regular poll with
//! unchanged data emits nothing (so `nextAt` / `lastAttemptAt` in the last event may be stale:
//! read them from a summary).
//!
//! Project close stops every task (abort handles); deleting a source stops its task. PANIC does
//! not stop polling (it is data, not on-air automation).
//!
//! Locking: the poller lock is a data lock, held briefly; never across I/O, the hub, the screen
//! store or the source store. Events are emitted outside it.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock};
use std::time::{Duration, SystemTime};

use tokio::sync::Notify;
use tokio::task::AbortHandle;

use crate::core::api_map::{self, Mapped};
use crate::core::fsx;
use crate::core::model::{
    ApiFormat, ApiHeader, ApiMapping, ApiMethod, ApiPoll, ApiRequest, ApiState, ApiStatus,
    ApiTestResult, SourceDoc, SourceKind, SourceSummary, API_PREVIEW_MAX, API_RESPONSE_MAX_BYTES,
    API_TEST_JSON_MAX, API_TEST_ROWS,
};
use crate::core::protocol::Fields;
use crate::core::screens;
use crate::core::secrets::{SecretError, SecretStore};
use crate::core::sources;
use crate::core::state::{AppState, WeakState};
use crate::error::{AppError, AppResult};

/// Longest wait after errors.
pub const BACKOFF_CAP: Duration = Duration::from_secs(300);
/// Longest `Retry-After` honoured.
pub const RETRY_AFTER_MAX: Duration = Duration::from_secs(3600);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(15);

const TOO_LARGE: &str = "Response is larger than 2 MB";

static GEN: AtomicU64 = AtomicU64::new(1);

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---- Pure helpers ------------------------------------------------------------------------------

/// The wait after `errors` consecutive failures: `interval × 2^errors`, capped at `cap`, never
/// shorter than the interval.
pub fn backoff(interval: Duration, errors: u32, cap: Duration) -> Duration {
    if errors == 0 {
        return interval;
    }
    let ms = u64::try_from(interval.as_millis()).unwrap_or(u64::MAX);
    let wait = Duration::from_millis(ms.saturating_mul(1u64 << errors.min(30)));
    wait.min(cap).max(interval)
}

/// `Retry-After`: delay seconds or an HTTP date (IMF-fixdate, e.g. `Sun, 06 Nov 1994 08:49:37
/// GMT`), as a wait from `now` (a past date = 0), at most [`RETRY_AFTER_MAX`].
pub fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs).min(RETRY_AFTER_MAX));
    }
    let at = http_date(v)?;
    Some(
        at.duration_since(now)
            .unwrap_or_default()
            .min(RETRY_AFTER_MAX),
    )
}

/// An IMF-fixdate (RFC 9110), the only HTTP date format senders may generate.
fn http_date(v: &str) -> Option<SystemTime> {
    let mut it = v.split_whitespace();
    let _dow = it.next()?.strip_suffix(',')?;
    let day: u8 = it.next()?.parse().ok()?;
    let month = match it.next()? {
        "Jan" => time::Month::January,
        "Feb" => time::Month::February,
        "Mar" => time::Month::March,
        "Apr" => time::Month::April,
        "May" => time::Month::May,
        "Jun" => time::Month::June,
        "Jul" => time::Month::July,
        "Aug" => time::Month::August,
        "Sep" => time::Month::September,
        "Oct" => time::Month::October,
        "Nov" => time::Month::November,
        "Dec" => time::Month::December,
        _ => return None,
    };
    let year: i32 = it.next()?.parse().ok()?;
    let mut hms = it.next()?.split(':').map(|p| p.parse::<u8>().ok());
    let (h, m, s) = (hms.next()??, hms.next()??, hms.next()??);
    if it.next()? != "GMT" || it.next().is_some() {
        return None;
    }
    let date = time::Date::from_calendar_date(year, month, day).ok()?;
    let t = time::Time::from_hms(h, m, s).ok()?;
    Some(time::PrimitiveDateTime::new(date, t).assume_utc().into())
}

/// Whether a response header's value is shown redacted in the test request.
pub fn is_sensitive_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key"
    ) || n.contains("token")
        || n.contains("secret")
        || n.contains("api-key")
        || n.contains("apikey")
        || n.contains("session")
}

/// The redacted form of a sensitive header value.
pub const REDACTED: &str = "[redacted]";

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

/// The settings a running task depends on (a change restarts it with a fresh fetch).
fn settings_sig(
    request: &ApiRequest,
    format: ApiFormat,
    mapping: &ApiMapping,
    poll: &ApiPoll,
) -> u64 {
    let bytes =
        serde_json::to_vec(&(request, format, mapping, poll.interval_ms)).unwrap_or_default();
    hash_bytes(&bytes)
}

/// `s` cut to at most `max` bytes at a character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---- HTTP --------------------------------------------------------------------------------------

/// A request ready to send (secret values resolved; never logged).
#[derive(Clone)]
pub(crate) struct Req {
    url: String,
    method: ApiMethod,
    /// (name, value, is a secret).
    headers: Vec<(String, String, bool)>,
    body: Option<String>,
}

impl std::fmt::Debug for Req {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Header values (secrets among them) never appear in logs.
        f.debug_struct("Req")
            .field("url", &self.url)
            .field("method", &self.method)
            .field("headers", &self.headers.len())
            .finish()
    }
}

/// What a request needs to be conditional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Cond {
    etag: Option<String>,
    last_modified: Option<String>,
}

struct Resp {
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    cond: Cond,
    retry_after: Option<String>,
    body: Vec<u8>,
}

/// Builds the request of `request` with secret values from `secrets`. `Err((state, message))`:
/// a secret is missing (`secretMissing`) or the keychain failed (`error`).
fn build_req(request: &ApiRequest, secrets: &dyn SecretStore) -> Result<Req, (ApiState, String)> {
    let mut headers = Vec::with_capacity(request.headers.len());
    for h in &request.headers {
        match (&h.secret, &h.value) {
            (Some(id), _) => match secrets.get(id) {
                Ok(Some(v)) => headers.push((h.name.clone(), v, true)),
                Ok(None) => {
                    return Err((
                        ApiState::SecretMissing,
                        format!(
                            "Secret missing on this machine \u{2014} enter the {} value again",
                            h.name
                        ),
                    ))
                }
                Err(e) => return Err((ApiState::Error, e.to_string())),
            },
            (None, Some(v)) => headers.push((h.name.clone(), v.clone(), false)),
            (None, None) => headers.push((h.name.clone(), String::new(), false)),
        }
    }
    Ok(Req {
        url: request.url.clone(),
        method: request.method,
        headers,
        body: request.body.clone(),
    })
}

fn describe(e: &reqwest::Error, url: &str) -> String {
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();
    if e.is_timeout() {
        format!("The request timed out after {} s", TOTAL_TIMEOUT.as_secs())
    } else if e.is_connect() {
        format!("Could not connect to {host}")
    } else if e.is_redirect() {
        "Too many redirects".into()
    } else if e.is_body() || e.is_decode() {
        "The response could not be read".into()
    } else if e.is_builder() {
        "The request is not valid (check the URL and headers)".into()
    } else {
        format!("The request to {host} failed")
    }
}

/// Sends `req`. The body is read for 2xx (and for every status with `always_body`), at most
/// `API_RESPONSE_MAX_BYTES`.
async fn send(
    client: &reqwest::Client,
    req: &Req,
    cond: &Cond,
    always_body: bool,
) -> Result<Resp, String> {
    use reqwest::header::{HeaderName, HeaderValue};
    let method = match req.method {
        ApiMethod::Get => reqwest::Method::GET,
        ApiMethod::Post => reqwest::Method::POST,
    };
    let mut rb = client.request(method, &req.url);
    let mut has_type = false;
    for (name, value, secret) in &req.headers {
        let n = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("Invalid header name {name}"))?;
        let mut v = HeaderValue::from_str(value)
            .map_err(|_| format!("Header {name}: the value is not valid"))?;
        v.set_sensitive(*secret);
        has_type |= n == reqwest::header::CONTENT_TYPE;
        rb = rb.header(n, v);
    }
    if let Some(etag) = &cond.etag {
        rb = rb.header(reqwest::header::IF_NONE_MATCH, etag.as_str());
    }
    if let Some(lm) = &cond.last_modified {
        rb = rb.header(reqwest::header::IF_MODIFIED_SINCE, lm.as_str());
    }
    if let (ApiMethod::Post, Some(body)) = (req.method, &req.body) {
        if !has_type && serde_json::from_str::<serde_json::Value>(body).is_ok() {
            rb = rb.header(reqwest::header::CONTENT_TYPE, "application/json");
        }
        rb = rb.body(body.clone());
    }
    let mut resp = rb.send().await.map_err(|e| describe(&e, &req.url))?;
    let status = resp.status();
    let text = |n: reqwest::header::HeaderName| {
        resp.headers()
            .get(n)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let cond = Cond {
        etag: text(reqwest::header::ETAG),
        last_modified: text(reqwest::header::LAST_MODIFIED),
    };
    let retry_after = text(reqwest::header::RETRY_AFTER);
    let headers = resp
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect();
    let mut body = Vec::new();
    if status.is_success() || always_body {
        if resp
            .content_length()
            .is_some_and(|n| n > API_RESPONSE_MAX_BYTES as u64)
        {
            return Err(TOO_LARGE.into());
        }
        while let Some(chunk) = resp.chunk().await.map_err(|e| describe(&e, &req.url))? {
            if body.len() + chunk.len() > API_RESPONSE_MAX_BYTES {
                return Err(TOO_LARGE.into());
            }
            body.extend_from_slice(&chunk);
        }
    }
    Ok(Resp {
        status: status.as_u16(),
        reason: status.canonical_reason().unwrap_or("").to_string(),
        headers,
        cond,
        retry_after,
        body,
    })
}

fn status_text(code: u16, reason: &str) -> String {
    if reason.is_empty() {
        format!("HTTP {code}")
    } else {
        format!("HTTP {code} {reason}")
    }
}

// ---- The poller --------------------------------------------------------------------------------

/// A running poll task.
struct Task {
    gen: u64,
    sig: u64,
    abort: AbortHandle,
    wake: Arc<Notify>,
    /// A Retry of a source that isn't active: one fetch, then the task ends.
    once: bool,
}

/// What the poller knows about one API source.
#[derive(Default)]
struct Track {
    status: ApiStatus,
    cond: Cond,
    hash: Option<u64>,
    errors: u32,
    /// 401/403, a missing secret, a request that can't be built: no task until cleared.
    held: bool,
    /// The settings `cond`, `hash` and `held` belong to.
    sig: u64,
}

#[derive(Default)]
struct Poll {
    tasks: HashMap<String, Task>,
    tracks: HashMap<String, Track>,
    watch: HashMap<String, u32>,
}

/// The open project's API polling (see the module docs).
pub struct ApiPoller {
    inner: Mutex<Poll>,
    secrets: RwLock<Arc<dyn SecretStore>>,
    client: OnceLock<Result<reqwest::Client, String>>,
    /// Test overrides: every interval and the backoff cap (0 = none).
    test_interval_ms: AtomicU64,
    test_cap_ms: AtomicU64,
}

impl Default for ApiPoller {
    fn default() -> Self {
        #[cfg(test)]
        let secrets: Arc<dyn SecretStore> =
            Arc::new(crate::core::secrets::MemorySecrets::default());
        #[cfg(not(test))]
        let secrets: Arc<dyn SecretStore> = Arc::new(crate::core::secrets::KeyringSecrets);
        Self {
            inner: Mutex::new(Poll::default()),
            secrets: RwLock::new(secrets),
            client: OnceLock::new(),
            test_interval_ms: AtomicU64::new(0),
            test_cap_ms: AtomicU64::new(0),
        }
    }
}

/// One API source as the reconcile pass sees it (computed under the source store lock).
pub(crate) struct ApiMeta {
    pub id: String,
    pub poll: ApiPoll,
    pub has_url: bool,
    pub sig: u64,
}

impl ApiMeta {
    pub(crate) fn of(doc: &SourceDoc) -> Option<Self> {
        let SourceKind::Api {
            request,
            format,
            mapping,
            poll,
            ..
        } = &doc.meta.kind
        else {
            return None;
        };
        Some(Self {
            id: doc.id.clone(),
            poll: *poll,
            has_url: !request.url.is_empty(),
            sig: settings_sig(request, *format, mapping, poll),
        })
    }
}

impl ApiPoller {
    /// Replaces where secret values live (the e2e fixture server and tests: an in-memory store).
    pub fn set_secret_store(&self, store: Arc<dyn SecretStore>) {
        *self.secrets.write().unwrap_or_else(PoisonError::into_inner) = store;
    }

    pub(crate) fn secrets(&self) -> Arc<dyn SecretStore> {
        self.secrets
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The shared HTTP client (built on first use: gzip, timeouts, User-Agent).
    fn client(&self) -> Result<reqwest::Client, String> {
        self.client
            .get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(concat!("Kinetra/", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(CONNECT_TIMEOUT)
                    .timeout(TOTAL_TIMEOUT)
                    .build()
                    .map_err(|_| "The HTTP client could not start".to_string())
            })
            .clone()
    }

    /// Test timing: every poll interval and the backoff cap.
    #[doc(hidden)]
    pub fn set_test_timing(&self, interval: Option<Duration>, cap: Option<Duration>) {
        let ms = |d: Option<Duration>| d.map_or(0, |d| d.as_millis() as u64);
        self.test_interval_ms.store(ms(interval), Ordering::Relaxed);
        self.test_cap_ms.store(ms(cap), Ordering::Relaxed);
    }

    fn interval(&self, poll: &ApiPoll) -> Duration {
        match self.test_interval_ms.load(Ordering::Relaxed) {
            0 => Duration::from_millis(u64::from(poll.interval_ms)),
            ms => Duration::from_millis(ms),
        }
    }

    fn cap(&self) -> Duration {
        match self.test_cap_ms.load(Ordering::Relaxed) {
            0 => BACKOFF_CAP,
            ms => Duration::from_millis(ms),
        }
    }

    /// Number of poll tasks alive.
    pub fn task_count(&self) -> usize {
        lock(&self.inner).tasks.len()
    }

    /// Ids of the sources with a poll task (sorted).
    pub fn polling(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.inner).tasks.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// The source's status as last recorded (`Idle` when never seen).
    pub fn status(&self, id: &str) -> ApiStatus {
        lock(&self.inner)
            .tracks
            .get(id)
            .map(|t| t.status.clone())
            .unwrap_or_default()
    }

    /// Stops every task and forgets everything (project closed or reloaded).
    pub(crate) fn stop_all(&self) {
        let mut p = lock(&self.inner);
        for (_, t) in p.tasks.drain() {
            t.abort.abort();
        }
        p.tracks.clear();
        p.watch.clear();
    }

    fn is_current(&self, id: &str, gen: u64) -> bool {
        lock(&self.inner)
            .tasks
            .get(id)
            .is_some_and(|t| t.gen == gen)
    }

    /// Whether the editor watch counter of `id` is positive.
    fn watched(&self, id: &str) -> bool {
        lock(&self.inner).watch.get(id).is_some_and(|n| *n > 0)
    }
}

/// What a fetch came back with.
enum Outcome {
    /// 304, or the same body as last time.
    Unchanged { cond: Cond, http: u16 },
    Rows {
        mapped: Mapped,
        hash: u64,
        cond: Cond,
        http: u16,
    },
    Failed {
        http: Option<u16>,
        message: String,
        /// `Some`: wait this long (429 with Retry-After); `None`: back off.
        wait: Option<Duration>,
        /// 401 / 403 / an invalid request: hold until Retry or a save.
        hold: bool,
    },
}

/// The request settings of a task's fetch (read on the blocking pool).
struct Go {
    req: Req,
    format: ApiFormat,
    mapping: ApiMapping,
    cond: Cond,
    hash: Option<u64>,
}

enum Prep {
    Go(Box<Go>),
    Stop,
}

/// A poll task: fetch, apply, sleep (or wake on Retry), repeat; ends when stopped.
async fn poll_task(weak: WeakState, id: String, gen: u64, wake: Arc<Notify>) {
    loop {
        let Some(st) = weak.upgrade() else { return };
        let (s, i) = (st.clone(), id.clone());
        drop(st);
        let prep = match tokio::task::spawn_blocking(move || s.api_prepare(&i, gen)).await {
            Ok(p) => p,
            Err(_) => return,
        };
        let Prep::Go(go) = prep else {
            return;
        };
        let Go {
            req,
            format,
            mapping,
            cond,
            hash,
        } = *go;
        let Some(client) = weak.upgrade().map(|st| st.api().client()) else {
            return;
        };
        let outcome = match client {
            Ok(c) => fetch(&c, &req, format, mapping, &cond, hash).await,
            Err(message) => Outcome::Failed {
                http: None,
                message,
                wait: None,
                hold: false,
            },
        };
        drop(req);
        let Some(st) = weak.upgrade() else { return };
        let i = id.clone();
        let next = match tokio::task::spawn_blocking(move || st.api_apply(&i, gen, outcome)).await {
            Ok(n) => n,
            Err(_) => return,
        };
        let Some(delay) = next else { return };
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = wake.notified() => {}
        }
    }
}

/// One conditional fetch and its mapping (on the blocking pool).
async fn fetch(
    client: &reqwest::Client,
    req: &Req,
    format: ApiFormat,
    mapping: ApiMapping,
    cond: &Cond,
    last_hash: Option<u64>,
) -> Outcome {
    let failed = |http, message, wait, hold| Outcome::Failed {
        http,
        message,
        wait,
        hold,
    };
    let resp = match send(client, req, cond, false).await {
        Ok(r) => r,
        Err(m) => return failed(None, m, None, false),
    };
    let code = resp.status;
    if code == 304 {
        // Keep validators the server didn't repeat.
        let mut c = cond.clone();
        if resp.cond.etag.is_some() {
            c.etag = resp.cond.etag;
        }
        if resp.cond.last_modified.is_some() {
            c.last_modified = resp.cond.last_modified;
        }
        return Outcome::Unchanged { cond: c, http: 304 };
    }
    if code == 429 {
        let wait = resp
            .retry_after
            .as_deref()
            .and_then(|v| retry_after(v, SystemTime::now()));
        return failed(Some(code), status_text(code, &resp.reason), wait, false);
    }
    if code == 401 || code == 403 {
        let m = format!(
            "{}: check the URL and the key (polling paused until Retry)",
            status_text(code, &resp.reason)
        );
        return failed(Some(code), m, None, true);
    }
    if !(200..300).contains(&code) {
        return failed(Some(code), status_text(code, &resp.reason), None, false);
    }
    let hash = hash_bytes(&resp.body);
    if last_hash == Some(hash) {
        return Outcome::Unchanged {
            cond: resp.cond,
            http: code,
        };
    }
    let body = resp.body;
    let mapped =
        tokio::task::spawn_blocking(move || api_map::map_body(format, &body, &mapping)).await;
    match mapped {
        Ok(Ok(mapped)) => Outcome::Rows {
            mapped,
            hash,
            cond: resp.cond,
            http: code,
        },
        Ok(Err(m)) => failed(Some(code), m, None, false),
        Err(_) => failed(
            Some(code),
            "The response could not be read".into(),
            None,
            false,
        ),
    }
}

impl AppState {
    /// API polling of the open project (see `core::api_poll`).
    fn poller(&self) -> &ApiPoller {
        self.api()
    }

    /// The source's status for a summary (`None` for other kinds).
    pub(crate) fn api_status(&self, doc: &SourceDoc) -> Option<ApiStatus> {
        let SourceKind::Api { poll, rows, .. } = &doc.meta.kind else {
            return None;
        };
        let mut s = self.poller().status(&doc.id);
        let running = lock(&self.poller().inner).tasks.contains_key(&doc.id);
        if !running && poll.paused {
            s.state = ApiState::Paused;
            s.next_at = None;
        }
        s.rows = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        Some(s)
    }

    /// Fills `stored` of every secret header (whether the keychain has its value).
    pub(crate) fn with_stored(&self, mut doc: SourceDoc) -> SourceDoc {
        if let SourceKind::Api { request, .. } = &mut doc.meta.kind {
            let secrets = self.poller().secrets();
            for h in &mut request.headers {
                h.stored = h
                    .secret
                    .as_deref()
                    .map(|id| secrets.get(id).ok().flatten().is_some());
            }
        }
        doc
    }

    /// Starts and stops poll tasks (see the module docs). Cheap without API sources.
    pub(crate) fn api_reconcile(&self) {
        // Poll sources follow the same triggers (bindings, flows, source changes).
        self.polls_reconcile();
        let metas = if self.project_dir().is_ok() {
            self.sources().api_metas()
        } else {
            Vec::new()
        };
        {
            let p = lock(&self.poller().inner);
            if metas.is_empty() && p.tasks.is_empty() && p.tracks.is_empty() {
                return;
            }
        }
        let used: HashSet<String> = if metas.iter().any(|m| m.poll.only_when_used) {
            let index = self.flow_engine().index();
            let mut used: HashSet<String> = self.screens().source_usage().into_keys().collect();
            for m in &metas {
                if index.uses_source(&m.id) || self.poller().watched(&m.id) {
                    used.insert(m.id.clone());
                }
            }
            used
        } else {
            HashSet::new()
        };
        let handle = self.live().runtime_handle();
        let mut changed = Vec::new();
        {
            let poller = self.poller();
            let mut p = lock(&poller.inner);
            let p = &mut *p;
            // Deleted sources.
            let ids: HashSet<&str> = metas.iter().map(|m| m.id.as_str()).collect();
            p.tasks.retain(|id, t| {
                let keep = ids.contains(id.as_str());
                if !keep {
                    t.abort.abort();
                }
                keep
            });
            p.tracks.retain(|id, _| ids.contains(id.as_str()));
            p.watch.retain(|id, _| ids.contains(id.as_str()));
            for m in &metas {
                let track = p.tracks.entry(m.id.clone()).or_default();
                if track.sig != m.sig {
                    // New settings: fresh fetch, no hold, no backoff.
                    track.sig = m.sig;
                    track.cond = Cond::default();
                    track.hash = None;
                    track.held = false;
                    track.errors = 0;
                }
                let wanted = m.has_url
                    && !m.poll.paused
                    && !track.held
                    && (!m.poll.only_when_used || used.contains(&m.id));
                let before = (track.status.state, track.status.message.clone());
                match p.tasks.get_mut(&m.id) {
                    Some(t) if wanted && t.sig == m.sig => t.once = false,
                    Some(t) if !wanted && t.once && t.sig == m.sig && !m.poll.paused => {}
                    Some(_) => {
                        if let Some(t) = p.tasks.remove(&m.id) {
                            t.abort.abort();
                        }
                    }
                    None => {}
                }
                if wanted && !p.tasks.contains_key(&m.id) {
                    if let Some(h) = &handle {
                        let gen = next_gen();
                        let wake = Arc::new(Notify::new());
                        let task =
                            h.spawn(poll_task(self.downgrade(), m.id.clone(), gen, wake.clone()));
                        p.tasks.insert(
                            m.id.clone(),
                            Task {
                                gen,
                                sig: m.sig,
                                abort: task.abort_handle(),
                                wake,
                                once: false,
                            },
                        );
                        if matches!(
                            track.status.state,
                            ApiState::Idle
                                | ApiState::Paused
                                | ApiState::Error
                                | ApiState::SecretMissing
                        ) {
                            track.status.state = ApiState::Fetching;
                            track.status.message = None;
                        }
                    } else {
                        tracing::warn!("no async runtime: API source {} not polled", m.id);
                    }
                }
                if !p.tasks.contains_key(&m.id) && !track.held {
                    track.status.state = if m.poll.paused {
                        ApiState::Paused
                    } else {
                        ApiState::Idle
                    };
                    track.status.next_at = None;
                    if !m.has_url {
                        track.status.message = None;
                    }
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

    /// A task's request settings (blocking pool). A missing secret or a keychain failure holds
    /// the source and ends the task.
    fn api_prepare(&self, id: &str, gen: u64) -> Prep {
        if !self.poller().is_current(id, gen) {
            return Prep::Stop;
        }
        let Some(doc) = self.sources().doc(id) else {
            return Prep::Stop;
        };
        let SourceKind::Api {
            request,
            format,
            mapping,
            ..
        } = doc.meta.kind
        else {
            return Prep::Stop;
        };
        let secrets = self.poller().secrets();
        match build_req(&request, &*secrets) {
            Ok(req) => {
                let (cond, hash) = {
                    let mut p = lock(&self.poller().inner);
                    let now = fsx::now_epoch_ms();
                    let t = p.tracks.entry(id.to_string()).or_default();
                    t.status.last_attempt_at = Some(now);
                    (t.cond.clone(), t.hash)
                };
                Prep::Go(Box::new(Go {
                    req,
                    format,
                    mapping,
                    cond,
                    hash,
                }))
            }
            Err((state, message)) => {
                self.api_hold(id, gen, state, None, message);
                Prep::Stop
            }
        }
    }

    /// Holds the source (no task until Retry, a save or a secret change).
    fn api_hold(&self, id: &str, gen: u64, state: ApiState, http: Option<u16>, message: String) {
        let changed = {
            let mut p = lock(&self.poller().inner);
            if !p.tasks.get(id).is_some_and(|t| t.gen == gen) {
                return;
            }
            p.tasks.remove(id);
            let t = p.tracks.entry(id.to_string()).or_default();
            t.held = true;
            let before = t.status.clone();
            t.status.state = state;
            t.status.http_status = http;
            t.status.message = Some(message);
            t.status.next_at = None;
            t.status.state != before.state
                || t.status.http_status != before.http_status
                || t.status.message != before.message
        };
        if changed {
            self.emit_source_changed(id);
        }
    }

    /// Applies a fetch (blocking pool). Returns the wait before the next one, or `None` when the
    /// task should end.
    fn api_apply(&self, id: &str, gen: u64, outcome: Outcome) -> Option<Duration> {
        let doc = self.sources().doc(id)?;
        let SourceKind::Api { poll, .. } = &doc.meta.kind else {
            return None;
        };
        let interval = self.poller().interval(poll);
        let cap = self.poller().cap();
        let now = fsx::now_epoch_ms();
        match outcome {
            Outcome::Unchanged { cond, http } => {
                let (changed, once) = self.api_track(id, gen, |t| {
                    t.cond = cond;
                    t.errors = 0;
                    t.status.state = ApiState::Ok;
                    t.status.last_ok_at = Some(now);
                    // A 304 keeps the status of the response that carried the data.
                    if http != 304 || t.status.http_status.is_none() {
                        t.status.http_status = Some(http);
                    }
                    // A truncation note stays while the rows are unchanged.
                    if t.status
                        .message
                        .as_deref()
                        .is_some_and(|m| !m.starts_with("Showing the first"))
                    {
                        t.status.message = None;
                    }
                    t.status.next_at = Some(now + interval.as_millis() as u64);
                })?;
                if changed {
                    self.emit_source_changed(id);
                }
                (!once).then_some(interval)
            }
            Outcome::Rows {
                mapped,
                hash,
                cond,
                http,
            } => {
                let note = mapped.note();
                let rows_changed = self.api_put_rows(id, gen, mapped.rows);
                let (changed, once) = self.api_track(id, gen, |t| {
                    t.cond = cond;
                    t.hash = Some(hash);
                    t.errors = 0;
                    t.status.state = ApiState::Ok;
                    t.status.last_ok_at = Some(now);
                    t.status.http_status = Some(http);
                    t.status.message = note;
                    t.status.next_at = Some(now + interval.as_millis() as u64);
                })?;
                if changed || rows_changed {
                    self.emit_source_changed(id);
                }
                (!once).then_some(interval)
            }
            Outcome::Failed {
                http,
                message,
                wait,
                hold,
            } => {
                if hold {
                    self.api_hold(id, gen, ApiState::Error, http, message);
                    return None;
                }
                let mut delay = interval;
                let (changed, once) = self.api_track(id, gen, |t| {
                    t.errors = t.errors.saturating_add(1);
                    delay = wait
                        .map(|w| w.max(interval))
                        .unwrap_or_else(|| backoff(interval, t.errors, cap));
                    t.status.state = ApiState::Backoff;
                    t.status.http_status = http;
                    t.status.message = Some(message);
                    t.status.next_at = Some(now + delay.as_millis() as u64);
                })?;
                if changed {
                    self.emit_source_changed(id);
                }
                (!once).then_some(delay)
            }
        }
    }

    /// Updates the track of a current task. `None` when the task is stale. Returns whether the
    /// reported status changed and whether the task was a one-off (removed now).
    fn api_track(&self, id: &str, gen: u64, f: impl FnOnce(&mut Track)) -> Option<(bool, bool)> {
        let mut p = lock(&self.poller().inner);
        let once = p.tasks.get(id).filter(|t| t.gen == gen)?.once;
        if once {
            p.tasks.remove(id);
        }
        let t = p.tracks.entry(id.to_string()).or_default();
        let before = (
            t.status.state,
            t.status.http_status,
            t.status.message.clone(),
        );
        f(t);
        if once && t.status.state == ApiState::Ok {
            t.status.next_at = None;
        }
        let after = (
            t.status.state,
            t.status.http_status,
            t.status.message.clone(),
        );
        Some((before != after, once))
    }

    /// New rows: in memory (written coalesced), bound screens re-applied with their on-air
    /// fields forced, auto-advance re-armed, flow triggers. `false` when the rows are the same.
    fn api_put_rows(&self, id: &str, gen: u64, rows: Vec<Fields>) -> bool {
        let (before, after, index_before, index_after) = {
            let Ok((dir, _w)) = self.screen_guard() else {
                return false;
            };
            if !self.poller().is_current(id, gen) {
                return false;
            }
            let Some(mut doc) = self.sources().doc(id) else {
                return false;
            };
            let now = self.sources().clock().now();
            let before = sources::current_row(&doc, &now);
            let SourceKind::Api {
                rows: have,
                index,
                fetched_at,
                ..
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
            *fetched_at = Some(fsx::now_rfc3339());
            let after = sources::current_row(&doc, &now);
            self.sources().put_dirty(doc);
            self.schedule_content_flush();
            for screen in self.screens().source_users(id) {
                let force = screens::bound_layers(&screen, id);
                if let Err(e) = self.apply_screen_doc_forcing(&dir, &screen, &force) {
                    tracing::warn!("API source {id}: applying screen {}: {e}", screen.id);
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

    /// A new API source: GET, no URL yet, JSON with the root as rows, no columns, polled every
    /// 10 s while used.
    pub fn create_api_source(&self, name: &str) -> AppResult<SourceDoc> {
        let doc = {
            let (dir, _w) = self.screen_guard()?;
            let kind = SourceKind::Api {
                request: ApiRequest::default(),
                format: ApiFormat::Json,
                mapping: ApiMapping::default(),
                poll: ApiPoll::default(),
                columns: Vec::new(),
                rows: Vec::new(),
                index: 0,
                auto: None,
                fetched_at: None,
            };
            let doc = sources::create_with(&dir, name, kind)?;
            self.sources().put(doc.clone());
            doc
        };
        self.api_reconcile();
        Ok(doc)
    }

    /// Stores a header's value in the keychain (write-only). The header is added when the
    /// source has none of that name; a new secret id is made when it had none. Returns the doc.
    pub fn set_source_secret(&self, id: &str, header: &str, value: &str) -> AppResult<SourceDoc> {
        let header = header.trim();
        if !api_map::is_token(header) {
            return Err(AppError::Invalid(format!(
                "Invalid header name \"{header}\""
            )));
        }
        if value.is_empty() || value.chars().any(|c| c.is_control() && c != '\t') {
            return Err(AppError::Invalid(
                "The secret value must be one line of text".into(),
            ));
        }
        let saved = {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.loaded_source(id)?;
            let SourceKind::Api { request, .. } = &mut doc.meta.kind else {
                return Err(AppError::Invalid("Only API sources have secrets".into()));
            };
            let pos = match request
                .headers
                .iter()
                .position(|h| h.name.eq_ignore_ascii_case(header))
            {
                Some(p) => p,
                None => {
                    request.headers.push(ApiHeader {
                        name: header.to_string(),
                        ..Default::default()
                    });
                    request.headers.len() - 1
                }
            };
            let h = &mut request.headers[pos];
            let secret_id = h.secret.clone().unwrap_or_else(|| nanoid::nanoid!());
            self.poller()
                .secrets()
                .set(&secret_id, value)
                .map_err(AppError::from)?;
            h.secret = Some(secret_id);
            h.value = None;
            let saved = sources::save(&dir, doc, false)?;
            self.sources().put(saved.clone());
            saved
        };
        self.api_clear_hold(id);
        self.api_reconcile();
        self.emit_source_changed(id);
        Ok(self.with_stored(saved))
    }

    /// Deletes a header's keychain value; the header stays, without a value.
    pub fn clear_source_secret(&self, id: &str, header: &str) -> AppResult<SourceDoc> {
        let saved = {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.loaded_source(id)?;
            let SourceKind::Api { request, .. } = &mut doc.meta.kind else {
                return Err(AppError::Invalid("Only API sources have secrets".into()));
            };
            let Some(h) = request
                .headers
                .iter_mut()
                .find(|h| h.name.eq_ignore_ascii_case(header.trim()))
            else {
                return Err(AppError::NotFound(format!("Header {}", header.trim())));
            };
            if let Some(secret) = h.secret.take() {
                self.poller()
                    .secrets()
                    .delete(&secret)
                    .map_err(AppError::from)?;
            }
            h.value = None;
            let saved = sources::save(&dir, doc, false)?;
            self.sources().put(saved.clone());
            saved
        };
        self.api_reconcile();
        Ok(self.with_stored(saved))
    }

    fn api_clear_hold(&self, id: &str) {
        let mut p = lock(&self.poller().inner);
        if let Some(t) = p.tracks.get_mut(id) {
            t.held = false;
            t.errors = 0;
        }
    }

    /// Fetches now: clears a hold and the backoff; a running task wakes, a source that isn't
    /// polling (not used) gets one fetch. Not while paused or without a URL.
    pub fn retry_api_source(&self, id: &str) -> AppResult<SourceSummary> {
        self.project_dir()?;
        let doc = self.loaded_source(id)?;
        let Some(meta) = ApiMeta::of(&doc) else {
            return Err(AppError::Invalid("Only API sources can be fetched".into()));
        };
        self.api_clear_hold(id);
        self.api_reconcile();
        if meta.has_url && !meta.poll.paused {
            let handle = self.live().runtime_handle();
            let spawned = {
                let mut p = lock(&self.poller().inner);
                match p.tasks.get(id) {
                    Some(t) => {
                        t.wake.notify_one();
                        false
                    }
                    None => match handle {
                        Some(h) => {
                            let gen = next_gen();
                            let wake = Arc::new(Notify::new());
                            let task = h.spawn(poll_task(
                                self.downgrade(),
                                id.to_string(),
                                gen,
                                wake.clone(),
                            ));
                            p.tasks.insert(
                                id.to_string(),
                                Task {
                                    gen,
                                    sig: meta.sig,
                                    abort: task.abort_handle(),
                                    wake,
                                    once: true,
                                },
                            );
                            let t = p.tracks.entry(id.to_string()).or_default();
                            t.status.state = ApiState::Fetching;
                            t.status.message = None;
                            true
                        }
                        None => false,
                    },
                }
            };
            if spawned {
                self.emit_source_changed(id);
            }
        }
        Ok(self.source_summary(&self.loaded_source(id)?))
    }

    /// Pauses (no task) or resumes polling; saved in the source file.
    pub fn set_api_paused(&self, id: &str, paused: bool) -> AppResult<SourceSummary> {
        let saved = {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.loaded_source(id)?;
            let SourceKind::Api { poll, .. } = &mut doc.meta.kind else {
                return Err(AppError::Invalid("Only API sources can be paused".into()));
            };
            if poll.paused == paused {
                doc
            } else {
                poll.paused = paused;
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
    pub fn watch_api_source(&self, id: &str, on: bool) -> AppResult<()> {
        self.project_dir()?;
        sources::validate_id(id)?;
        {
            let mut p = lock(&self.poller().inner);
            let n = p.watch.entry(id.to_string()).or_default();
            *n = if on {
                n.saturating_add(1)
            } else {
                n.saturating_sub(1)
            };
            if *n == 0 {
                p.watch.remove(id);
            }
        }
        self.api_reconcile();
        Ok(())
    }

    /// The secret ids of a source.
    pub(crate) fn secret_ids(doc: &SourceDoc) -> Vec<String> {
        match &doc.meta.kind {
            SourceKind::Api { request, .. } => request
                .headers
                .iter()
                .filter_map(|h| h.secret.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Deletes keychain values (logged on failure; never the value).
    pub(crate) fn delete_secrets(&self, ids: &[String]) {
        let secrets = self.poller().secrets();
        for id in ids {
            if let Err(e) = secrets.delete(id) {
                tracing::warn!("deleting a secret of an API source failed: {e}");
            }
        }
    }

    /// A duplicate's secret headers get new ids holding copies of the values (a missing value
    /// stays missing).
    pub(crate) fn copy_secrets(&self, doc: &mut SourceDoc) -> Result<(), SecretError> {
        let SourceKind::Api { request, .. } = &mut doc.meta.kind else {
            return Ok(());
        };
        let secrets = self.poller().secrets();
        for h in &mut request.headers {
            let Some(old) = h.secret.clone() else {
                continue;
            };
            let new = nanoid::nanoid!();
            if let Some(v) = secrets.get(&old)? {
                secrets.set(&new, &v)?;
            }
            h.secret = Some(new);
        }
        Ok(())
    }

    /// One fetch with the draft (secrets by id from the keychain); nothing is saved. Errors
    /// (invalid settings, network, mapping) are in `error`.
    pub async fn test_api_source(&self, doc: SourceDoc) -> AppResult<ApiTestResult> {
        self.project_dir()?;
        let SourceKind::Api {
            mut request,
            format,
            mut mapping,
            poll,
            ..
        } = doc.meta.kind
        else {
            return Err(AppError::Invalid("Only API sources can be tested".into()));
        };
        let mut result = ApiTestResult::default();
        if let Err(m) = api_map::validate_settings(&mut request, format, &mut mapping, &poll) {
            result.error = Some(m);
            return Ok(result);
        }
        if request.url.is_empty() {
            result.error = Some("Enter a URL first".into());
            return Ok(result);
        }
        let secrets = self.poller().secrets();
        let req = match build_req(&request, &*secrets) {
            Ok(r) => r,
            Err((_, m)) => {
                result.error = Some(m);
                return Ok(result);
            }
        };
        let client = match self.poller().client() {
            Ok(c) => c,
            Err(m) => {
                result.error = Some(m);
                return Ok(result);
            }
        };
        let resp = match send(&client, &req, &Cond::default(), true).await {
            Ok(r) => r,
            Err(m) => {
                result.error = Some(m);
                return Ok(result);
            }
        };
        drop(req);
        let built = tokio::task::spawn_blocking(move || test_result(resp, format, &mapping))
            .await
            .map_err(|_| AppError::Internal("The test request failed".into()))?;
        Ok(built)
    }
}

/// The test request's result from a response (blocking pool).
fn test_result(resp: Resp, format: ApiFormat, mapping: &ApiMapping) -> ApiTestResult {
    let mut r = ApiTestResult {
        http_status: Some(resp.status),
        headers: resp
            .headers
            .into_iter()
            .map(|(k, v)| {
                let v = if is_sensitive_header(&k) {
                    REDACTED.to_string()
                } else {
                    v
                };
                (k, v)
            })
            .collect(),
        ..Default::default()
    };
    let ok = (200..300).contains(&resp.status);
    let text = api_map::body_text(&resp.body);
    let mapped = match format {
        ApiFormat::Json => match api_map::parse_json(&resp.body) {
            Ok(v) => {
                let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
                r.preview = cut(&pretty, API_PREVIEW_MAX).to_string();
                let mapped = api_map::map_json_value(&v, mapping);
                if resp.body.len() <= API_TEST_JSON_MAX {
                    r.json = Some(v);
                }
                mapped
            }
            Err(e) => {
                r.preview = cut(&text, API_PREVIEW_MAX).to_string();
                Err(e)
            }
        },
        ApiFormat::Csv => {
            r.preview = cut(&text, API_PREVIEW_MAX).to_string();
            api_map::map_csv(&resp.body, mapping)
        }
    };
    match mapped {
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
            r.csv_columns = m.csv_columns;
            r.rows = m.rows.into_iter().take(API_TEST_ROWS).collect();
            if !ok {
                r.error = Some(status_text(resp.status, &resp.reason));
            }
        }
        Err(e) => {
            r.error = Some(if ok {
                e
            } else {
                status_text(resp.status, &resp.reason)
            });
        }
    }
    r
}

#[cfg(test)]
mod tests;
