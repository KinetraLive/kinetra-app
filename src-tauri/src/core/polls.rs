//! Live polls (every request and error code is in docs/API.md "Polls").
//!
//! # Data for templates
//! A poll source (`SourceKind::Poll`) shows one poll: a linked server poll (`link` + the last
//! known `cache`) or a local test poll (`test`). Its rows are the answers in position order
//! ([`rows`]); the poll's own values (question, total, status, links, QR) are on every row, so a
//! single field binds them with the normal binding (current row = the first answer) and a list
//! field takes every answer. `pct` has two decimals (bars and the `_winner` ranking: no false
//! ties), `pct_round` whole numbers that add up to 100 (display); the leading answer gets the
//! `_leader` row mark while votes come in (not once closed: then only `_winner`).
//!
//! # Access (the core decides; the UI only displays)
//! Creating, editing and opening need `pro.polls` and full access (`require_new_pro_work`).
//! Reading, results, the stream, close, reset and delete need no entitlement on the server; the
//! app still makes **no poll request** unless the account has `pro.polls` or this machine knows
//! the poll (`polls.json` in the config dir: polls created here, or listed with "Show my
//! existing polls", which may always call). So a never-Pro Free user makes zero calls, and a
//! lapsed Pro user can still close, reset and delete their polls, and an open poll keeps
//! streaming.
//!
//! # Live results
//! One tokio task per poll **in use**: linked to a source that a screen layer binds or an armed
//! flow uses, or watched by the Polls page (`watch_poll`, counted). None otherwise. The task
//! reads the SSE stream (`snapshot` / `totals` / `status` carry the whole poll; `deleted` ends
//! it), reconnects with backoff (1 s doubling to 30 s), and after 3 failed connects in a row
//! falls back to `GET /v1/polls/{id}` every 5 s, trying the stream again every 60 s. At most
//! [`MAX_STREAMS`] streams (the server's per-account limit); more polls in use poll instead.
//! `429` waits `Retry-After`; `401` stops (sign in again); `404` / `deleted` mark the poll gone.
//! New data goes through the source path: linked sources' caches, bound screens re-applied with
//! their on-air layers forced (straight to Program; Studio rebases the Preview), `source-changed`
//! and `poll-live`. **A dropped connection never clears output:** sources keep their last data.
//!
//! # Test polls
//! Typed locally, available to everyone, no network. `simulate_test_poll` runs one sleeping task
//! per simulated source (votes added every [`Timing::sim_tick`], weighted towards a chosen
//! leader); none when paused.
//!
//! Locking: the service lock is a data lock, held briefly; never across I/O, the hub, the screen
//! store or the source store. Events go out after it is released.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::live::LiveLayerView;
use crate::core::model::{ScreenLayerKind, TakeVia, TestDataBlocked};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::task::AbortHandle;

use crate::core::cloud::api::{CallError, PollWire};
use crate::core::events::names;
use crate::core::model::{
    ColumnKind, PollConn, PollData, PollInput, PollLink, PollLive, PollOption, PollQuota,
    PollSimulation, PollSourceView, PollStatus, SourceColumn, SourceDoc, SourceKind, TestPoll,
    POLL_DURATION_MAX_S, POLL_DURATION_MIN_S, POLL_OPTIONS_MAX, POLL_OPTIONS_MIN, POLL_OPTION_MAX,
    POLL_QUESTION_MAX,
};
use crate::core::persist::{self, Store};
use crate::core::protocol::Fields;
use crate::core::qr;
use crate::core::screens;
use crate::core::sources;
use crate::core::state::{AppState, WeakState};
use crate::error::{AppError, AppResult};

#[cfg(test)]
mod tests;

/// The entitlement polls need (`kinetra_license::entitlements::PRO_POLLS`).
pub const PRO_POLLS: &str = kinetra_license::entitlements::PRO_POLLS;
/// The voting site's host (short links read `poll.kinetra.live/K7P2QX`).
pub const VOTE_HOST: &str = "poll.kinetra.live";
/// A test poll's code: not a valid code (it has a 0), so the voting site answers "No poll with
/// this code" if someone scans a rehearsal QR.
pub const TEST_CODE: &str = "TEST00";
pub const TEST_ID: &str = "test";
/// Streams at once (the server's `KINETRA_POLL_SSE_MAX_STREAMS_PER_ACCOUNT`).
pub const MAX_STREAMS: usize = 5;
/// Failed stream connects in a row before falling back to polling.
pub const FAILS_BEFORE_FALLBACK: u32 = 3;
/// Simulated votes per second, at most.
pub const SIM_RATE_MAX: u32 = 50;
/// Polls remembered per machine.
const KNOWN_MAX: usize = 500;
/// The voting code alphabet (docs/API.md: no 0/O, 1/I/L, U).
const CODE_ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The columns a poll source exposes (key, label, kind).
pub const POLL_COLUMNS: &[(&str, &str, ColumnKind)] = &[
    ("label", "Answer", ColumnKind::Text),
    ("votes", "Votes", ColumnKind::Text),
    ("pct", "Share %", ColumnKind::Text),
    ("pct_round", "Share % (rounded)", ColumnKind::Text),
    ("position", "Position", ColumnKind::Text),
    ("question", "Question", ColumnKind::Text),
    ("total", "Total votes", ColumnKind::Text),
    ("total_n", "Total votes (number)", ColumnKind::Text),
    ("status", "Status", ColumnKind::Text),
    ("leader", "Leader", ColumnKind::Text),
    ("url", "Voting link", ColumnKind::Text),
    ("short_url", "Short link", ColumnKind::Text),
    ("code", "Code", ColumnKind::Text),
    ("qr", "QR code", ColumnKind::Image),
    ("closes_at", "Closes at", ColumnKind::Text),
];

// ---- Pure: ids, validation, mapping ----------------------------------------------------------

/// A poll id we put into a URL path: 1–64 of `[0-9A-Za-z-]` (the server's are UUID v7).
pub fn is_poll_id(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A voting code: 6 characters of the code alphabet (or the test code).
pub fn is_code(s: &str) -> bool {
    s == TEST_CODE || (s.len() == 6 && s.chars().all(|c| CODE_ALPHABET.contains(c)))
}

/// The columns of a poll source.
pub fn columns() -> Vec<SourceColumn> {
    POLL_COLUMNS
        .iter()
        .map(|(k, l, kind)| SourceColumn {
            key: (*k).to_string(),
            label: (*l).to_string(),
            kind: *kind,
        })
        .collect()
}

/// Checks a poll source on save (hard checks: nothing is written when one fails). Test polls are
/// lenient while being typed (an empty question or answer is allowed); votes follow the answers.
pub fn validate(
    link: &Option<PollLink>,
    test: &mut Option<TestPoll>,
    cache: &Option<PollData>,
) -> Result<(), String> {
    if link.is_some() && test.is_some() {
        return Err("A poll source shows a poll or a test poll, not both".into());
    }
    if let Some(l) = link {
        if !is_poll_id(&l.id) || !is_code(&l.code) {
            return Err("The linked poll's id or code is not valid".into());
        }
    }
    if let Some(t) = test {
        t.question = t.question.trim().to_string();
        if t.question.chars().count() > POLL_QUESTION_MAX {
            return Err(format!(
                "The question is too long: at most {POLL_QUESTION_MAX} characters."
            ));
        }
        if t.options.len() > POLL_OPTIONS_MAX {
            return Err(format!("A poll has at most {POLL_OPTIONS_MAX} answers."));
        }
        for (i, o) in t.options.iter_mut().enumerate() {
            *o = o.trim().to_string();
            if o.chars().count() > POLL_OPTION_MAX {
                return Err(format!(
                    "Answer {} is too long: at most {POLL_OPTION_MAX} characters.",
                    i + 1
                ));
            }
        }
        t.votes.resize(t.options.len(), 0);
    }
    if let Some(c) = cache {
        if c.options.len() > POLL_OPTIONS_MAX
            || c.question.len() > POLL_QUESTION_MAX * 4
            || c.options
                .iter()
                .any(|o| o.label.len() > POLL_OPTION_MAX * 4)
            || c.url.len() > 2048
        {
            return Err("The saved poll results are not valid".into());
        }
    }
    Ok(())
}

/// The same rules the server applies to a new or edited poll (its `message` still wins), so the
/// editor can say what's wrong before sending. `max_options` is the plan's limit (8 without one).
pub fn check_input(input: &PollInput, max_options: usize) -> Result<(), String> {
    if let Some(q) = &input.question {
        let q = q.trim();
        if q.is_empty() {
            return Err("Add a question.".into());
        }
        if q.chars().count() > POLL_QUESTION_MAX {
            return Err(format!(
                "The question is too long: at most {POLL_QUESTION_MAX} characters."
            ));
        }
        if q.chars().any(char::is_control) {
            return Err("The question can't contain line breaks.".into());
        }
    }
    if let Some(opts) = &input.options {
        let max = max_options.clamp(POLL_OPTIONS_MIN, POLL_OPTIONS_MAX);
        if !(POLL_OPTIONS_MIN..=max).contains(&opts.len()) {
            return Err(format!(
                "A poll needs {POLL_OPTIONS_MIN} to {max} options on your plan."
            ));
        }
        let mut seen = HashSet::new();
        for (i, o) in opts.iter().enumerate() {
            let o = o.trim();
            if o.is_empty() {
                return Err(format!("Answer {} is empty.", i + 1));
            }
            if o.chars().count() > POLL_OPTION_MAX {
                return Err(format!(
                    "Answer {} is too long: at most {POLL_OPTION_MAX} characters.",
                    i + 1
                ));
            }
            if o.chars().any(char::is_control) {
                return Err("Answers can't contain line breaks.".into());
            }
            if !seen.insert(o.to_lowercase()) {
                return Err(format!("Two answers are the same: \u{201c}{o}\u{201d}."));
            }
        }
    }
    if let Some(d) = input.duration_seconds {
        if !(POLL_DURATION_MIN_S..=POLL_DURATION_MAX_S).contains(&d) {
            return Err("A time limit must be between 30 seconds and 24 hours.".into());
        }
    }
    Ok(())
}

/// A test poll as poll data: the test code and link, totals from its votes.
pub fn test_data(t: &TestPoll) -> PollData {
    let options: Vec<PollOption> = t
        .options
        .iter()
        .enumerate()
        .map(|(i, label)| PollOption {
            position: i as u32,
            label: label.clone(),
            votes: t.votes.get(i).copied().unwrap_or(0),
        })
        .collect();
    PollData {
        id: TEST_ID.into(),
        code: TEST_CODE.into(),
        url: format!("https://{VOTE_HOST}/{TEST_CODE}"),
        question: t.question.clone(),
        total_votes: options.iter().map(|o| o.votes).sum(),
        options,
        status: t.status,
        show_results: true,
        round: 1,
        ..PollData::default()
    }
}

/// What a poll source shows: the test poll, or the linked poll's last known data.
pub fn data_of(kind: &SourceKind) -> Option<PollData> {
    match kind {
        SourceKind::Poll { test: Some(t), .. } => Some(test_data(t)),
        SourceKind::Poll { cache, .. } => cache.clone(),
        _ => None,
    }
}

/// `draft | open | closed | full` (full: open with the vote limit reached).
pub fn state_of(d: &PollData) -> &'static str {
    match d.status {
        PollStatus::Draft => "draft",
        PollStatus::Open if d.vote_limit_reached => "full",
        PollStatus::Open => "open",
        PollStatus::Closed => "closed",
    }
}

/// `2418` → `2,418`.
pub fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Shares of `votes`: two-decimal strings (`"46.25"`, trailing zeros dropped; `"0"` with no
/// votes) and whole percentages that add up to exactly 100 (largest remainder; ties keep the
/// earlier answer).
pub fn shares(votes: &[u64]) -> (Vec<String>, Vec<u32>) {
    let total: u64 = votes.iter().sum();
    if total == 0 {
        return (vec!["0".into(); votes.len()], vec![0; votes.len()]);
    }
    let exact: Vec<f64> = votes
        .iter()
        .map(|v| *v as f64 * 100.0 / total as f64)
        .collect();
    let two = exact
        .iter()
        .map(|p| {
            let s = format!("{:.2}", (p * 100.0).round() / 100.0);
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        })
        .collect();
    let mut whole: Vec<u32> = exact.iter().map(|p| p.floor() as u32).collect();
    let left = 100u32.saturating_sub(whole.iter().sum());
    let mut order: Vec<usize> = (0..votes.len()).collect();
    order.sort_by(|a, b| {
        let (fa, fb) = (exact[*a] - exact[*a].floor(), exact[*b] - exact[*b].floor());
        fb.partial_cmp(&fa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.cmp(b))
    });
    for i in order.into_iter().take(left as usize) {
        whole[i] += 1;
    }
    (two, whole)
}

/// The leading answer's index: the most votes, `None` with no votes or a tie for first.
pub fn leader(d: &PollData) -> Option<usize> {
    let max = d.options.iter().map(|o| o.votes).max()?;
    if max == 0 {
        return None;
    }
    let mut leaders = d.options.iter().enumerate().filter(|(_, o)| o.votes == max);
    let first = leaders.next()?.0;
    leaders.next().is_none().then_some(first)
}

/// `https://poll.kinetra.live/K7P2QX` → `poll.kinetra.live/K7P2QX`.
pub fn short_url(url: &str) -> String {
    url.strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url)
        .trim_end_matches('/')
        .to_string()
}

/// The source rows of a poll: one per answer (position order), the poll's own values on every
/// row, `_leader` on the leading answer while the poll is not closed (closed: the winner is
/// marked `_winner` per layer instead, never both).
pub fn rows(d: &PollData) -> Vec<Fields> {
    let votes: Vec<u64> = d.options.iter().map(|o| o.votes).collect();
    let (pct, pct_round) = shares(&votes);
    let lead = leader(d);
    let mark_lead = if d.status == PollStatus::Closed {
        None
    } else {
        lead
    };
    let total: u64 = votes.iter().sum();
    let total = total.max(d.total_votes);
    let qr = if d.url.is_empty() {
        String::new()
    } else {
        qr::data_url(&d.url)
    };
    let shared: Vec<(&str, String)> = vec![
        ("question", d.question.clone()),
        ("total", group(total)),
        ("total_n", total.to_string()),
        ("status", state_of(d).to_string()),
        (
            "leader",
            lead.map(|i| d.options[i].label.clone()).unwrap_or_default(),
        ),
        ("url", d.url.clone()),
        ("short_url", short_url(&d.url)),
        ("code", d.code.clone()),
        ("qr", qr),
        ("closes_at", d.closes_at.clone().unwrap_or_default()),
    ];
    d.options
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let mut f = Fields::new();
            f.insert("label".into(), o.label.clone());
            f.insert("votes".into(), group(o.votes));
            f.insert("pct".into(), pct[i].clone());
            f.insert("pct_round".into(), pct_round[i].to_string());
            f.insert("position".into(), (o.position + 1).to_string());
            for (k, v) in &shared {
                f.insert((*k).into(), v.clone());
            }
            if mark_lead == Some(i) {
                f.insert("_leader".into(), "true".into());
            }
            f
        })
        .collect()
}

// ---- Pure: server-sent events ------------------------------------------------------------------

/// One server-sent event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// An incremental `text/event-stream` parser: feed it chunks as they arrive (they may split
/// lines and UTF-8 sequences anywhere); comments (`:ping`) are skipped.
#[derive(Debug, Default)]
pub struct Sse {
    buf: Vec<u8>,
    event: String,
    data: String,
    has_data: bool,
}

/// A line longer than this is dropped (a poll event is a few KB).
const SSE_LINE_MAX: usize = 256 * 1024;

impl Sse {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=i).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            self.line(&String::from_utf8_lossy(&line), &mut out);
        }
        if self.buf.len() > SSE_LINE_MAX {
            self.buf.clear();
        }
        out
    }

    fn line(&mut self, line: &str, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if self.has_data {
                out.push(SseEvent {
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    data: std::mem::take(&mut self.data),
                });
            }
            self.event.clear();
            self.data.clear();
            self.has_data = false;
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            _ => {}
        }
    }
}

// ---- The service ----------------------------------------------------------------------------

/// Delays the client uses (tests shorten them).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// First reconnect wait; doubles per failure up to `reconnect_cap`.
    pub reconnect: Duration,
    pub reconnect_cap: Duration,
    /// Polling interval when the stream isn't available.
    pub fallback_every: Duration,
    /// While polling: how often to try the stream again.
    pub stream_retry: Duration,
    /// No bytes for this long ends the stream (the server pings every 30 s).
    pub read_timeout: Duration,
    /// Test simulation step.
    pub sim_tick: Duration,
}

impl Timing {
    pub const LIVE: Timing = Timing {
        reconnect: Duration::from_secs(1),
        reconnect_cap: Duration::from_secs(30),
        fallback_every: Duration::from_secs(5),
        stream_retry: Duration::from_secs(60),
        read_timeout: Duration::from_secs(75),
        sim_tick: Duration::from_millis(250),
    };
}

/// Polls this machine created or listed (`polls.json` in the app config dir): the ones it may
/// manage without `pro.polls`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KnownPolls {
    pub ids: Vec<String>,
}

impl Store for KnownPolls {
    const FILE: &'static str = "polls.json";
    const VERSION: u32 = 1;
}

struct Task {
    gen: u64,
    abort: AbortHandle,
    stream: bool,
}

struct Sim {
    gen: u64,
    abort: AbortHandle,
}

#[derive(Default)]
struct Svc {
    tasks: HashMap<String, Task>,
    conn: HashMap<String, PollConn>,
    watch: HashMap<String, u32>,
    /// By source id.
    sims: HashMap<String, Sim>,
    /// `None` until first read from disk.
    known: Option<Vec<String>>,
}

/// Live poll results and test simulations (see the module docs).
#[derive(Default)]
pub struct PollService {
    inner: Mutex<Svc>,
    /// The operator allowed test poll data on Program this session (`AppState::guard_test_data`).
    test_data_allowed: std::sync::atomic::AtomicBool,
    stream_client: OnceLock<Result<reqwest::Client, String>>,
    test_timing: Mutex<Option<Timing>>,
}

static GEN: AtomicU64 = AtomicU64::new(1);

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PollService {
    /// Tests: shorter delays.
    #[doc(hidden)]
    pub fn set_test_timing(&self, t: Option<Timing>) {
        *lock(&self.test_timing) = t;
    }

    pub fn timing(&self) -> Timing {
        lock(&self.test_timing).unwrap_or(Timing::LIVE)
    }

    /// The streaming client: a connect timeout, no total timeout (the stream lasts hours).
    fn stream_client(&self) -> Result<reqwest::Client, String> {
        self.stream_client
            .get_or_init(|| {
                reqwest::Client::builder()
                    .user_agent(concat!("Kinetra/", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(Duration::from_secs(8))
                    .build()
                    .map_err(|_| "The HTTP client could not start".to_string())
            })
            .clone()
    }

    /// Number of live-result tasks.
    pub fn task_count(&self) -> usize {
        lock(&self.inner).tasks.len()
    }

    /// Number of running test simulations.
    pub fn sim_count(&self) -> usize {
        lock(&self.inner).sims.len()
    }

    /// How `id`'s data arrives (`Idle` when not in use).
    pub fn conn(&self, id: &str) -> PollConn {
        lock(&self.inner).conn.get(id).copied().unwrap_or_default()
    }

    /// Whether `id`'s task is a stream (vs polling the totals).
    pub fn is_streaming(&self, id: &str) -> bool {
        lock(&self.inner).tasks.get(id).is_some_and(|t| t.stream)
    }

    fn is_current(&self, id: &str, gen: u64) -> bool {
        lock(&self.inner)
            .tasks
            .get(id)
            .is_some_and(|t| t.gen == gen)
    }

    fn sim_current(&self, source: &str, gen: u64) -> bool {
        lock(&self.inner)
            .sims
            .get(source)
            .is_some_and(|s| s.gen == gen)
    }

    /// Stops every task and simulation (project closed or reloaded). The remembered polls stay.
    pub(crate) fn stop_all(&self) {
        let mut s = lock(&self.inner);
        for (_, t) in s.tasks.drain() {
            t.abort.abort();
        }
        for (_, m) in s.sims.drain() {
            m.abort.abort();
        }
        s.conn.clear();
        s.watch.clear();
    }
}

/// How a stream ended.
enum StreamEnd {
    /// Closed by the server, a network error or silence: reconnect.
    Dropped { got_data: bool },
    /// `deleted`: the poll is gone.
    Deleted,
    /// The task was replaced or stopped.
    Stale,
}

/// A small xorshift generator for simulated votes (no crate; quality doesn't matter here).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// In `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The wait after `fails` failed connects in a row: `base` doubling up to `cap`.
pub fn reconnect_delay(t: &Timing, fails: u32) -> Duration {
    let exp = fails.saturating_sub(1).min(16);
    t.reconnect.saturating_mul(1u32 << exp).min(t.reconnect_cap)
}

impl AppState {
    // ---- Access ----

    /// Whether the account has `pro.polls` (effective entitlements; never from the UI).
    pub fn has_polls_pro(&self) -> bool {
        self.require(PRO_POLLS).is_ok()
    }

    fn known_ids(&self) -> Vec<String> {
        let svc = self.poll_service();
        {
            let s = lock(&svc.inner);
            if let Some(k) = &s.known {
                return k.clone();
            }
        }
        let ids = persist::load_or_recover::<KnownPolls>(self.config_dir())
            .map(|(k, _)| k.ids)
            .unwrap_or_default();
        let mut s = lock(&svc.inner);
        s.known.get_or_insert(ids).clone()
    }

    /// Whether this machine knows the poll (created or listed here).
    pub fn knows_poll(&self, id: &str) -> bool {
        self.known_ids().iter().any(|k| k == id)
    }

    /// Whether the app may call the poll API (for `id`, or at all): `pro.polls`, or a poll this
    /// machine knows.
    pub fn may_call_polls(&self, id: Option<&str>) -> bool {
        self.has_polls_pro()
            || match id {
                Some(id) => self.knows_poll(id),
                None => !self.known_ids().is_empty(),
            }
    }

    fn save_known(&self, ids: Vec<String>) {
        let dir = self.config_dir().to_path_buf();
        let save = move || {
            if let Err(e) = persist::save(&dir, &KnownPolls { ids }) {
                tracing::warn!("polls.json: {e}");
            }
        };
        match self.live().runtime_handle() {
            Some(h) => drop(h.spawn_blocking(save)),
            None => save(),
        }
    }

    /// Adds `ids` to the remembered polls (newest last, at most `KNOWN_MAX`).
    fn remember_polls(&self, ids: impl IntoIterator<Item = String>) {
        let mut known = self.known_ids();
        let before = known.len();
        for id in ids {
            if !known.contains(&id) {
                known.push(id);
            }
        }
        if known.len() == before {
            return;
        }
        if known.len() > KNOWN_MAX {
            let cut = known.len() - KNOWN_MAX;
            known.drain(..cut);
        }
        lock(&self.poll_service().inner).known = Some(known.clone());
        self.save_known(known);
    }

    /// Replaces the remembered polls with the account's list (what the server says exists).
    fn set_known(&self, ids: Vec<String>) {
        if self.known_ids() == ids {
            return;
        }
        lock(&self.poll_service().inner).known = Some(ids.clone());
        self.save_known(ids);
    }

    fn forget_poll(&self, id: &str) {
        let mut known = self.known_ids();
        let before = known.len();
        known.retain(|k| k != id);
        if known.len() != before {
            lock(&self.poll_service().inner).known = Some(known.clone());
            self.save_known(known);
        }
    }

    /// A poll the app may manage without creating anything new (close, reset, delete, read).
    fn gate_existing(&self, id: &str) -> AppResult<()> {
        if !is_poll_id(id) {
            return Err(AppError::Invalid("Unknown poll".into()));
        }
        if self.may_call_polls(Some(id)) {
            Ok(())
        } else {
            Err(AppError::ProRequired)
        }
    }

    fn poll_session(&self) -> AppResult<(crate::core::cloud::api::Api, String)> {
        self.cloud_session_parts().map(|(api, s, _)| (api, s))
    }

    // ---- Operations (commands) ----

    /// The plan's limits and this month's usage; `None` (no request made) when the app may not
    /// call the poll API (Free, nothing to manage).
    pub async fn polls_quota(&self) -> AppResult<Option<PollQuota>> {
        if !self.may_call_polls(None) {
            return Ok(None);
        }
        let (api, s) = self.poll_session()?;
        Ok(Some(api.polls_quota(&s).await?))
    }

    /// The account's polls, newest first. Without `pro.polls` this makes no request unless the
    /// machine knows polls, or `on_demand` ("Show my existing polls": always asks, once). The
    /// result becomes the remembered list, so a lapsed plan can still manage them.
    pub async fn list_polls(&self, on_demand: bool) -> AppResult<Vec<PollData>> {
        if !on_demand && !self.may_call_polls(None) {
            return Ok(Vec::new());
        }
        let (api, s) = self.poll_session()?;
        let list: Vec<PollData> = api
            .polls_list(&s)
            .await?
            .into_iter()
            .map(PollData::from)
            .collect();
        self.set_known(list.iter().map(|p| p.id.clone()).collect());
        for p in &list {
            self.poll_apply(p.clone());
        }
        self.polls_reconcile();
        Ok(list)
    }

    /// Creates a draft poll (Pro).
    pub async fn create_poll(&self, input: PollInput) -> AppResult<PollData> {
        self.require_new_pro_work(PRO_POLLS)?;
        if input.question.is_none() || input.options.is_none() {
            return Err(AppError::Invalid(
                "A poll needs a question and answers".into(),
            ));
        }
        check_input(&input, POLL_OPTIONS_MAX).map_err(AppError::Invalid)?;
        let mut body = json!({
            "question": input.question.as_deref().unwrap_or_default().trim(),
            "options": input.options.unwrap_or_default().iter().map(|o| o.trim()).collect::<Vec<_>>(),
        });
        if let Some(d) = input.duration_seconds {
            body["duration_seconds"] = json!(d);
        }
        if let Some(sr) = input.show_results {
            body["show_results"] = json!(sr);
        }
        let (api, s) = self.poll_session()?;
        let poll = PollData::from(api.poll_create(&s, body).await?);
        self.remember_polls([poll.id.clone()]);
        Ok(poll)
    }

    /// Edits a poll (Pro): question / answers / duration only while not open (and without
    /// votes, for the text); "show results to voters" any time.
    pub async fn update_poll(&self, id: &str, input: PollInput) -> AppResult<PollData> {
        self.require_new_pro_work(PRO_POLLS)?;
        self.gate_existing(id)?;
        check_input(&input, POLL_OPTIONS_MAX).map_err(AppError::Invalid)?;
        let mut body = serde_json::Map::new();
        if let Some(q) = &input.question {
            body.insert("question".into(), json!(q.trim()));
        }
        if let Some(o) = &input.options {
            body.insert(
                "options".into(),
                json!(o.iter().map(|o| o.trim()).collect::<Vec<_>>()),
            );
        }
        if input.clear_duration {
            body.insert("duration_seconds".into(), Value::Null);
        } else if let Some(d) = input.duration_seconds {
            body.insert("duration_seconds".into(), json!(d));
        }
        if let Some(sr) = input.show_results {
            body.insert("show_results".into(), json!(sr));
        }
        let (api, s) = self.poll_session()?;
        let poll = self.poll_result(id, api.poll_patch(&s, id, Value::Object(body)).await)?;
        Ok(poll)
    }

    /// Opens a poll (Pro).
    pub async fn open_poll(&self, id: &str) -> AppResult<PollData> {
        self.require_new_pro_work(PRO_POLLS)?;
        self.poll_action(id, "open").await
    }

    /// Closes a poll (also without Pro: a poll is never stranded).
    pub async fn close_poll(&self, id: &str) -> AppResult<PollData> {
        self.poll_action(id, "close").await
    }

    /// Deletes this round's votes and starts a new round (also without Pro).
    pub async fn reset_poll(&self, id: &str) -> AppResult<PollData> {
        self.poll_action(id, "reset").await
    }

    /// One poll's current data (also without Pro).
    pub async fn get_poll(&self, id: &str) -> AppResult<PollData> {
        self.gate_existing(id)?;
        let (api, s) = self.poll_session()?;
        self.poll_result(id, api.poll_get(&s, id).await)
    }

    /// Deletes a poll and its votes now (also without Pro). Linked sources keep their last
    /// results on screen.
    pub async fn delete_poll(&self, id: &str) -> AppResult<()> {
        self.gate_existing(id)?;
        let (api, s) = self.poll_session()?;
        match api.poll_delete(&s, id).await {
            Ok(()) => {
                self.poll_gone(id);
                Ok(())
            }
            Err(CallError::Api { status: 404, .. }) => {
                self.poll_gone(id);
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn poll_action(&self, id: &str, action: &str) -> AppResult<PollData> {
        self.gate_existing(id)?;
        let (api, s) = self.poll_session()?;
        self.poll_result(id, api.poll_action(&s, id, action).await)
    }

    /// A call that answered with a poll: applied to linked sources; a 404 marks it gone.
    fn poll_result(&self, id: &str, r: Result<PollWire, CallError>) -> AppResult<PollData> {
        match r {
            Ok(w) => {
                let poll = PollData::from(w);
                self.poll_apply(poll.clone());
                Ok(poll)
            }
            Err(e) => {
                if matches!(e, CallError::Api { status: 404, .. }) {
                    self.poll_gone(id);
                }
                Err(e.into())
            }
        }
    }

    /// The Polls page (or a source editor) shows `id` live: counted; streams while > 0.
    pub fn watch_poll(&self, id: &str, on: bool) -> AppResult<()> {
        if !is_poll_id(id) {
            return Err(AppError::Invalid("Unknown poll".into()));
        }
        {
            let mut s = lock(&self.poll_service().inner);
            let n = s.watch.entry(id.to_string()).or_insert(0);
            *n = if on {
                n.saturating_add(1)
            } else {
                n.saturating_sub(1)
            };
            if *n == 0 {
                s.watch.remove(id);
            }
        }
        self.polls_reconcile();
        Ok(())
    }

    // ---- Poll sources ----

    /// A new poll source showing `poll` (linked, its data as the first cache) or a test poll.
    pub fn create_poll_source(
        &self,
        name: &str,
        poll: Option<PollData>,
        test: Option<TestPoll>,
    ) -> AppResult<SourceDoc> {
        let kind = match (poll, test) {
            (Some(p), None) => {
                if !is_poll_id(&p.id) || !is_code(&p.code) {
                    return Err(AppError::Invalid("Unknown poll".into()));
                }
                SourceKind::Poll {
                    link: Some(PollLink {
                        id: p.id.clone(),
                        code: p.code.clone(),
                    }),
                    test: None,
                    cache: Some(p),
                    gone: false,
                }
            }
            (None, Some(t)) => SourceKind::Poll {
                link: None,
                test: Some(t),
                cache: None,
                gone: false,
            },
            _ => {
                return Err(AppError::Invalid(
                    "A poll source shows a poll or a test poll".into(),
                ))
            }
        };
        let doc = {
            let (dir, _w) = self.screen_guard()?;
            let doc = sources::create_with(&dir, name, kind)?;
            self.sources().put(doc.clone());
            doc
        };
        self.polls_reconcile();
        Ok(doc)
    }

    /// The first poll source linked to `poll_id`, if any ("Add to screen" reuses it).
    pub fn poll_source_for(&self, poll_id: &str) -> Option<String> {
        let mut ids: Vec<String> = self
            .sources()
            .poll_links()
            .into_iter()
            .filter(|(_, l, _)| l.as_deref() == Some(poll_id))
            .map(|(s, _, _)| s)
            .collect();
        ids.sort();
        ids.into_iter().next()
    }

    /// Whether the open project has a poll source linked to a real poll (for the test-data
    /// confirmation before Program).
    pub fn project_uses_real_polls(&self) -> bool {
        self.sources()
            .poll_links()
            .iter()
            .any(|(_, l, test)| l.is_some() && !test)
    }

    /// The poll part of a source summary.
    pub(crate) fn poll_view(&self, doc: &SourceDoc) -> Option<PollSourceView> {
        let SourceKind::Poll {
            link, test, gone, ..
        } = &doc.meta.kind
        else {
            return None;
        };
        let data = data_of(&doc.meta.kind);
        let svc = self.poll_service();
        let (conn, simulating) = {
            let s = lock(&svc.inner);
            let conn = if test.is_some() {
                PollConn::Test
            } else if *gone {
                PollConn::Gone
            } else {
                link.as_ref()
                    .and_then(|l| s.conn.get(&l.id).copied())
                    .unwrap_or_default()
            };
            (conn, s.sims.contains_key(&doc.id))
        };
        Some(PollSourceView {
            test: test.is_some(),
            poll_id: link.as_ref().map(|l| l.id.clone()),
            code: link
                .as_ref()
                .map(|l| l.code.clone())
                .or_else(|| test.as_ref().map(|_| TEST_CODE.to_string())),
            conn,
            state: data.as_ref().map(|d| state_of(d).to_string()),
            total_votes: data.as_ref().map_or(0, |d| {
                d.total_votes.max(d.options.iter().map(|o| o.votes).sum())
            }),
            simulating,
        })
    }

    // ---- Applying data ----

    /// New data for a poll: every linked source's cache (written coalesced), bound screens
    /// re-applied with their on-air layers forced, `source-changed` per changed source and
    /// `poll-live` for watchers.
    pub(crate) fn poll_apply(&self, data: PollData) {
        let changed = self.poll_put(&data.id, |kind| {
            let SourceKind::Poll { cache, gone, .. } = kind else {
                return false;
            };
            if cache.as_ref() == Some(&data) && !*gone {
                return false;
            }
            *cache = Some(data.clone());
            *gone = false;
            true
        });
        for id in &changed {
            self.emit_source_changed(id);
        }
        let conn = self.poll_service().conn(&data.id);
        self.emit_poll_live(PollLive {
            id: data.id.clone(),
            conn,
            poll: Some(data),
        });
    }

    /// Applies `f` to every source linked to `poll_id` (also gone ones) under the screen lock;
    /// changed ones are put dirty and their screens re-applied. Returns the changed source ids.
    fn poll_put(&self, poll_id: &str, mut f: impl FnMut(&mut SourceKind) -> bool) -> Vec<String> {
        let Ok((dir, _w)) = self.screen_guard() else {
            return Vec::new();
        };
        let mut changed = Vec::new();
        for doc in self.sources().docs() {
            let linked = matches!(&doc.meta.kind,
                SourceKind::Poll { link: Some(l), .. } if l.id == poll_id);
            if !linked {
                continue;
            }
            let mut doc = doc;
            if !f(&mut doc.meta.kind) {
                continue;
            }
            let id = doc.id.clone();
            self.sources().put_dirty(doc);
            for screen in self.screens().source_users(&id) {
                let force = screens::bound_layers(&screen, &id);
                if let Err(e) = self.apply_screen_doc_forcing(&dir, &screen, &force) {
                    tracing::warn!("poll source {id}: applying screen {}: {e}", screen.id);
                }
            }
            changed.push(id);
        }
        if !changed.is_empty() {
            self.schedule_content_flush();
        }
        changed
    }

    /// The poll was deleted on the server: forgotten here, linked sources keep their last data
    /// (marked gone), its task ends.
    pub(crate) fn poll_gone(&self, id: &str) {
        self.forget_poll(id);
        let changed = self.poll_put(id, |kind| {
            let SourceKind::Poll { gone, .. } = kind else {
                return false;
            };
            let was = *gone;
            *gone = true;
            !was
        });
        {
            let mut s = lock(&self.poll_service().inner);
            if let Some(t) = s.tasks.remove(id) {
                t.abort.abort();
            }
            s.conn.insert(id.to_string(), PollConn::Gone);
        }
        for sid in &changed {
            self.emit_source_changed(sid);
        }
        self.emit_poll_live(PollLive {
            id: id.to_string(),
            conn: PollConn::Gone,
            poll: None,
        });
    }

    fn emit_poll_live(&self, live: PollLive) {
        match serde_json::to_value(&live) {
            Ok(v) => self.events().emit(names::POLL_LIVE, v),
            Err(e) => tracing::warn!("poll-live payload: {e}"),
        }
    }

    /// Records how `id`'s data arrives; tells watchers and linked sources when it changed. With
    /// `gen`, only for the current task.
    fn poll_set_conn(&self, id: &str, gen: Option<u64>, conn: PollConn) {
        let changed = {
            let mut s = lock(&self.poll_service().inner);
            if let Some(g) = gen {
                if !s.tasks.get(id).is_some_and(|t| t.gen == g) {
                    return;
                }
            }
            s.conn.insert(id.to_string(), conn) != Some(conn)
        };
        if !changed {
            return;
        }
        self.emit_poll_live(PollLive {
            id: id.to_string(),
            conn,
            poll: None,
        });
        for (sid, l, _) in self.sources().poll_links() {
            if l.as_deref() == Some(id) {
                self.emit_source_changed(&sid);
            }
        }
    }

    // ---- Reconcile ----

    /// Starts and stops live-result tasks (see the module docs) and drops simulations of sources
    /// that are gone. Cheap when nothing involves polls; runs after every live change, source
    /// change, flow deploy and license change.
    pub(crate) fn polls_reconcile(&self) {
        let links = if self.project_dir().is_ok() {
            self.sources().poll_links()
        } else {
            Vec::new()
        };
        let svc = self.poll_service();
        let watched: Vec<String> = {
            let s = lock(&svc.inner);
            if links.is_empty() && s.watch.is_empty() && s.tasks.is_empty() && s.sims.is_empty() {
                return;
            }
            s.watch.keys().cloned().collect()
        };
        // Simulations of deleted or no-longer-test sources stop.
        {
            let tests: HashSet<&str> = links
                .iter()
                .filter(|(_, _, t)| *t)
                .map(|(s, _, _)| s.as_str())
                .collect();
            let mut s = lock(&svc.inner);
            s.sims.retain(|id, m| {
                let keep = tests.contains(id.as_str());
                if !keep {
                    m.abort.abort();
                }
                keep
            });
        }
        let mut wanted: HashSet<String> = watched.into_iter().collect();
        let linked: Vec<(&String, &String)> = links
            .iter()
            .filter_map(|(s, l, _)| l.as_ref().map(|l| (s, l)))
            .collect();
        if !linked.is_empty() {
            let usage = self.screens().source_usage();
            let index = self.flow_engine().index();
            for (sid, pid) in &linked {
                if usage.contains_key(*sid) || index.uses_source(sid) {
                    wanted.insert((*pid).clone());
                }
            }
        }
        // Allowed: the account may call for this poll. Signed in: checked only when starting.
        wanted.retain(|id| self.may_call_polls(Some(id)));
        let mut stopped = Vec::new();
        let start: Vec<String> = {
            let mut s = lock(&svc.inner);
            let s = &mut *s;
            s.tasks.retain(|id, t| {
                let keep = wanted.contains(id);
                if !keep {
                    t.abort.abort();
                    stopped.push(id.clone());
                }
                keep
            });
            let mut start: Vec<String> = wanted
                .iter()
                .filter(|id| !s.tasks.contains_key(*id))
                .cloned()
                .collect();
            start.sort();
            start
        };
        for id in stopped {
            if self.poll_service().conn(&id) != PollConn::Gone {
                self.poll_set_conn(&id, None, PollConn::Idle);
            }
        }
        if start.is_empty() {
            return;
        }
        if self.poll_session().is_err() {
            for id in start {
                self.poll_set_conn(&id, None, PollConn::SignedOut);
            }
            return;
        }
        let Some(h) = self.live().runtime_handle() else {
            tracing::warn!("no async runtime: polls not streamed");
            return;
        };
        for id in start {
            let gen = next_gen();
            let stream = {
                let mut s = lock(&svc.inner);
                let streams = s.tasks.values().filter(|t| t.stream).count();
                let stream = streams < MAX_STREAMS;
                let task = h.spawn(poll_task(self.downgrade(), id.clone(), gen, stream));
                s.tasks.insert(
                    id.clone(),
                    Task {
                        gen,
                        abort: task.abort_handle(),
                        stream,
                    },
                );
                stream
            };
            let conn = if stream {
                PollConn::Connecting
            } else {
                PollConn::Polling
            };
            self.poll_set_conn(&id, Some(gen), conn);
        }
    }

    // ---- Test polls ----

    fn test_doc(&self, source: &str) -> AppResult<SourceDoc> {
        let doc = self
            .sources()
            .doc(source)
            .ok_or_else(|| AppError::NotFound(format!("source {source}")))?;
        if !matches!(doc.meta.kind, SourceKind::Poll { test: Some(_), .. }) {
            return Err(AppError::Invalid("Not a test poll".into()));
        }
        Ok(doc)
    }

    /// Changes a test poll in memory (written coalesced) and re-applies its screens.
    fn test_put(&self, source: &str, f: impl FnOnce(&mut TestPoll) -> bool) -> AppResult<bool> {
        let changed = {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.test_doc(source)?;
            let SourceKind::Poll { test: Some(t), .. } = &mut doc.meta.kind else {
                return Ok(false);
            };
            if !f(t) {
                return Ok(false);
            }
            self.sources().put_dirty(doc);
            for screen in self.screens().source_users(source) {
                let force = screens::bound_layers(&screen, source);
                if let Err(e) = self.apply_screen_doc_forcing(&dir, &screen, &force) {
                    tracing::warn!("test poll {source}: applying screen {}: {e}", screen.id);
                }
            }
            true
        };
        self.schedule_content_flush();
        self.emit_source_changed(source);
        Ok(changed)
    }

    /// Opens, closes or re-drafts a test poll. Closing stops a simulation.
    pub fn set_test_poll_status(&self, source: &str, status: PollStatus) -> AppResult<()> {
        if status != PollStatus::Open {
            self.stop_sim(source);
        }
        self.test_put(source, |t| {
            let changed = t.status != status;
            t.status = status;
            changed
        })?;
        Ok(())
    }

    /// Zeroes a test poll's votes.
    pub fn reset_test_poll(&self, source: &str) -> AppResult<()> {
        self.test_put(source, |t| {
            let changed = t.votes.iter().any(|v| *v > 0);
            t.votes.iter_mut().for_each(|v| *v = 0);
            changed
        })?;
        Ok(())
    }

    /// Starts (opening the poll) or stops simulated votes.
    pub fn simulate_test_poll(&self, source: &str, sim: PollSimulation) -> AppResult<()> {
        self.test_doc(source)?;
        if !sim.run {
            self.stop_sim(source);
            self.emit_source_changed(source);
            return Ok(());
        }
        let rate = sim.rate.clamp(1, SIM_RATE_MAX);
        self.test_put(source, |t| {
            let changed = t.status != PollStatus::Open;
            t.status = PollStatus::Open;
            changed
        })?;
        let Some(h) = self.live().runtime_handle() else {
            return Err(AppError::Internal("No async runtime".into()));
        };
        let gen = next_gen();
        {
            let mut s = lock(&self.poll_service().inner);
            if let Some(old) = s.sims.remove(source) {
                old.abort.abort();
            }
            let task = h.spawn(sim_task(
                self.downgrade(),
                source.to_string(),
                gen,
                rate,
                sim.lead,
            ));
            s.sims.insert(
                source.to_string(),
                Sim {
                    gen,
                    abort: task.abort_handle(),
                },
            );
        }
        self.emit_source_changed(source);
        Ok(())
    }

    fn stop_sim(&self, source: &str) {
        if let Some(m) = lock(&self.poll_service().inner).sims.remove(source) {
            m.abort.abort();
        }
    }

    /// One simulation step: `adds` votes per answer. `false` ends the task (stale, not a test
    /// poll any more, or not open).
    fn sim_step(&self, source: &str, gen: u64, adds: &[u64]) -> bool {
        if !self.poll_service().sim_current(source, gen) {
            return false;
        }
        let mut open = true;
        let r = self.test_put(source, |t| {
            if t.status != PollStatus::Open {
                open = false;
                return false;
            }
            t.votes.resize(t.options.len(), 0);
            let mut any = false;
            for (v, a) in t.votes.iter_mut().zip(adds) {
                if *a > 0 {
                    *v = v.saturating_add(*a);
                    any = true;
                }
            }
            any
        });
        if r.is_err() || !open {
            let mut s = lock(&self.poll_service().inner);
            if s.sims.get(source).is_some_and(|m| m.gen == gen) {
                s.sims.remove(source);
            }
            drop(s);
            self.emit_source_changed(source);
            return false;
        }
        true
    }
}

/// A live-results task (see the module docs).
async fn poll_task(weak: WeakState, id: String, gen: u64, stream_first: bool) {
    let mut stream = stream_first;
    let mut fails: u32 = 0;
    let mut retry_stream_at: Option<Instant> = None;
    loop {
        let Some(st) = weak.upgrade() else { return };
        if !st.poll_service().is_current(&id, gen) {
            return;
        }
        let t = st.poll_service().timing();
        let session = {
            let s = st.clone();
            tokio::task::spawn_blocking(move || s.poll_session()).await
        };
        let Ok(Ok((api, session))) = session else {
            st.poll_set_conn(&id, Some(gen), PollConn::SignedOut);
            return;
        };
        let client = st.poll_service().stream_client();
        drop(st);
        let use_stream = stream || retry_stream_at.is_some_and(|at| Instant::now() >= at);
        if use_stream {
            let Ok(client) = client else { return };
            match api.poll_events(&client, &session, &id).await {
                Ok(resp) => match read_stream(&weak, &id, gen, resp, t.read_timeout).await {
                    StreamEnd::Deleted => {
                        if let Some(st) = weak.upgrade() {
                            spawn_blocking_with(st, move |st| st.poll_gone(&id)).await;
                        }
                        return;
                    }
                    StreamEnd::Stale => return,
                    StreamEnd::Dropped { got_data } => {
                        stream = true;
                        retry_stream_at = None;
                        fails = if got_data { 1 } else { fails.saturating_add(1) };
                        let Some(st) = weak.upgrade() else { return };
                        st.poll_set_conn(&id, Some(gen), PollConn::Reconnecting);
                        drop(st);
                        tokio::time::sleep(reconnect_delay(&t, fails)).await;
                        continue;
                    }
                },
                Err(e) => {
                    let Some(st) = weak.upgrade() else { return };
                    match e {
                        CallError::Api { status: 404, .. } => {
                            spawn_blocking_with(st, move |st| st.poll_gone(&id)).await;
                            return;
                        }
                        CallError::Api { status: 401, .. } => {
                            st.poll_set_conn(&id, Some(gen), PollConn::SignedOut);
                            return;
                        }
                        CallError::Api {
                            status: 429,
                            retry_after,
                            ..
                        } => {
                            drop(st);
                            let wait = retry_after
                                .map(Duration::from_secs)
                                .unwrap_or(t.reconnect_cap)
                                .min(Duration::from_secs(3600));
                            tokio::time::sleep(wait).await;
                            continue;
                        }
                        _ => {
                            fails = fails.saturating_add(1);
                            if fails >= FAILS_BEFORE_FALLBACK {
                                stream = false;
                                retry_stream_at = Some(Instant::now() + t.stream_retry);
                                // One more failure after a retry returns to polling at once.
                                fails = FAILS_BEFORE_FALLBACK - 1;
                                st.poll_set_conn(&id, Some(gen), PollConn::Polling);
                                drop(st);
                                // Poll right away (below, next round).
                                continue;
                            }
                            st.poll_set_conn(&id, Some(gen), PollConn::Reconnecting);
                            drop(st);
                            tokio::time::sleep(reconnect_delay(&t, fails)).await;
                            continue;
                        }
                    }
                }
            }
        }
        // Polling the totals.
        match api.poll_get(&session, &id).await {
            Ok(w) => {
                let Some(st) = weak.upgrade() else { return };
                if !st.poll_service().is_current(&id, gen) {
                    return;
                }
                let data = PollData::from(w);
                spawn_blocking_with(st, move |st| st.poll_apply(data)).await;
            }
            Err(CallError::Api { status: 404, .. }) => {
                if let Some(st) = weak.upgrade() {
                    spawn_blocking_with(st, move |st| st.poll_gone(&id)).await;
                }
                return;
            }
            Err(CallError::Api { status: 401, .. }) => {
                if let Some(st) = weak.upgrade() {
                    st.poll_set_conn(&id, Some(gen), PollConn::SignedOut);
                }
                return;
            }
            Err(_) => {}
        }
        if retry_stream_at.is_none() {
            retry_stream_at = Some(Instant::now() + t.stream_retry);
        }
        tokio::time::sleep(t.fallback_every).await;
    }
}

/// Runs `f` on the blocking pool (source and screen work does file-system-adjacent locking).
async fn spawn_blocking_with(st: AppState, f: impl FnOnce(&AppState) + Send + 'static) {
    let _ = tokio::task::spawn_blocking(move || f(&st)).await;
}

/// Reads one stream until it ends. Every `snapshot` / `totals` / `status` event carries the whole
/// poll and is applied as it comes; the first one sets the connection live.
async fn read_stream(
    weak: &WeakState,
    id: &str,
    gen: u64,
    mut resp: reqwest::Response,
    read_timeout: Duration,
) -> StreamEnd {
    let mut sse = Sse::default();
    let mut got_data = false;
    loop {
        let chunk = match tokio::time::timeout(read_timeout, resp.chunk()).await {
            Ok(Ok(Some(c))) => c,
            _ => return StreamEnd::Dropped { got_data },
        };
        for ev in sse.feed(&chunk) {
            match ev.event.as_str() {
                "deleted" => return StreamEnd::Deleted,
                "snapshot" | "totals" | "status" => {
                    let Ok(w) = serde_json::from_str::<PollWire>(&ev.data) else {
                        tracing::debug!("poll {id}: an event didn't parse");
                        continue;
                    };
                    if w.id != id {
                        continue;
                    }
                    let Some(st) = weak.upgrade() else {
                        return StreamEnd::Stale;
                    };
                    if !st.poll_service().is_current(id, gen) {
                        return StreamEnd::Stale;
                    }
                    if !got_data {
                        got_data = true;
                        st.poll_set_conn(id, Some(gen), PollConn::Live);
                    }
                    let data = PollData::from(w);
                    spawn_blocking_with(st, move |st| st.poll_apply(data)).await;
                }
                _ => {}
            }
        }
        let Some(st) = weak.upgrade() else {
            return StreamEnd::Stale;
        };
        if !st.poll_service().is_current(id, gen) {
            return StreamEnd::Stale;
        }
    }
}

/// A test simulation: every tick, `rate × tick` votes (fractions carried over), weighted three
/// to one towards `lead` (or even, with a little drift).
async fn sim_task(weak: WeakState, source: String, gen: u64, rate: u32, lead: Option<u32>) {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(gen, |d| d.as_nanos() as u64 ^ gen);
    let mut rng = Rng::new(seed);
    let mut carry = 0.0f64;
    loop {
        let Some(st) = weak.upgrade() else { return };
        let tick = st.poll_service().timing().sim_tick;
        let n = st
            .sources()
            .doc(&source)
            .and_then(|d| match d.meta.kind {
                SourceKind::Poll { test: Some(t), .. } => Some(t.options.len()),
                _ => None,
            })
            .unwrap_or(0);
        drop(st);
        tokio::time::sleep(tick).await;
        if n == 0 {
            continue;
        }
        carry += f64::from(rate) * tick.as_secs_f64();
        let votes = carry.floor() as u64;
        carry -= votes as f64;
        let weights: Vec<f64> = (0..n)
            .map(|i| match lead {
                Some(l) if l as usize == i => 3.0,
                Some(_) => 1.0,
                None => 0.8 + 0.4 * rng.unit(),
            })
            .collect();
        let sum: f64 = weights.iter().sum();
        let mut adds = vec![0u64; n];
        for _ in 0..votes {
            let mut r = rng.unit() * sum;
            for (i, w) in weights.iter().enumerate() {
                if r < *w || i == n - 1 {
                    adds[i] += 1;
                    break;
                }
                r -= w;
            }
        }
        let Some(st) = weak.upgrade() else { return };
        let s = source.clone();
        let go = tokio::task::spawn_blocking(move || st.sim_step(&s, gen, &adds)).await;
        if !matches!(go, Ok(true)) {
            return;
        }
    }
}

// ---- The test-data safeguard ---------------------------------------------------------------------

impl AppState {
    /// Whether the operator allowed test poll data on Program this session.
    pub fn test_data_allowed(&self) -> bool {
        self.poll_service()
            .test_data_allowed
            .load(Ordering::Relaxed)
    }

    /// The app's "Allow test data on air" (for this session; the app asks first). Tells every
    /// window when it changes.
    pub fn allow_test_data_on_air(&self, on: bool) {
        let was = self
            .poll_service()
            .test_data_allowed
            .swap(on, Ordering::Relaxed);
        if was != on {
            self.events()
                .emit(names::TEST_DATA_ALLOWED, Value::Bool(on));
        }
    }

    /// Names of the layers among `layer_ids` (of `screen_id`) bound to a test poll source.
    fn test_data_layers(&self, screen_id: &str, layer_ids: &[&str]) -> Vec<String> {
        let tests: HashSet<String> = self
            .sources()
            .poll_links()
            .into_iter()
            .filter(|(_, _, test)| *test)
            .map(|(id, _, _)| id)
            .collect();
        if tests.is_empty() {
            return Vec::new();
        }
        let Some(doc) = self.screens().doc(screen_id) else {
            return Vec::new();
        };
        doc.meta
            .layers
            .iter()
            .filter(|l| layer_ids.contains(&l.id.as_str()))
            .filter(|l| match &l.kind {
                ScreenLayerKind::Template { bindings, .. } => {
                    bindings.values().any(|b| tests.contains(&b.source_id))
                }
                _ => false,
            })
            .map(|l| l.name.clone())
            .collect()
    }

    /// The safeguard every path to Program goes through (the app, MIDI, OSC, the API / Companion,
    /// flows): when the project also shows a real poll and test data isn't allowed on air this
    /// session, a take that would put test poll data on Program is refused, with a log entry and a
    /// `test-data-blocked` event (Control shows a notice with "Allow test data on air"). Cheap
    /// when the project has no test polls.
    pub fn guard_test_data(
        &self,
        screen_id: &str,
        layer_ids: &[&str],
        via: TakeVia,
    ) -> AppResult<()> {
        if layer_ids.is_empty() || self.test_data_allowed() || !self.project_uses_real_polls() {
            return Ok(());
        }
        let layers = self.test_data_layers(screen_id, layer_ids);
        if layers.is_empty() {
            return Ok(());
        }
        let list = layers.join(", ");
        tracing::warn!(
            "refused a {via:?} take of {list} on screen {screen_id}: it shows test poll data and \
             the project also shows a real poll (allow test data on air in Control)"
        );
        let blocked = TestDataBlocked {
            screen_id: screen_id.to_string(),
            layers,
            via,
            at: crate::core::fsx::now_epoch_ms(),
        };
        match serde_json::to_value(&blocked) {
            Ok(v) => self.events().emit(names::TEST_DATA_BLOCKED, v),
            Err(e) => tracing::warn!("test-data-blocked payload: {e}"),
        }
        Err(AppError::TestData(list))
    }

    /// TAKE of one layer from any path: the safeguard (only when it isn't on air yet: an update of
    /// an on-air layer changes nothing about what is on Program), then the live rules.
    pub fn live_take(
        &self,
        screen_id: &str,
        layer_id: &str,
        via: TakeVia,
    ) -> AppResult<LiveLayerView> {
        let on_air = self.live().layer_view(screen_id, layer_id)?.on_air;
        if !on_air {
            self.guard_test_data(screen_id, &[layer_id], via)?;
        }
        Ok(self.live().show(screen_id, layer_id)?)
    }

    /// The layers a Studio take of `screen_id` would bring to Program (cued in).
    pub(crate) fn studio_incoming(&self, screen_id: &str) -> Vec<String> {
        self.live()
            .preview_view(screen_id)
            .map(|v| {
                v.cues
                    .into_iter()
                    .filter(|c| c.visible == Some(true))
                    .map(|c| c.layer_id)
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ---- Add to screen ---------------------------------------------------------------------------------

/// The Poll preset's field → the poll source column it shows (`""`: the list of every answer).
pub const POLL_PRESET_BINDINGS: &[(&str, &str)] = &[
    ("question", "question"),
    ("status", "status"),
    ("total_votes", "total"),
    ("options", ""),
    ("qr", "qr"),
    ("short_url", "short_url"),
];

impl AppState {
    /// "Add to screen": the Poll graphic on `screen_id`, every field bound to the poll source
    /// `source_id`. Uses the project's template made from the Poll preset, or makes one; the layer
    /// gets the preset's design box like a layer added in the layout editor, off air. Touches disk.
    pub fn add_poll_to_screen(
        &self,
        source_id: &str,
        screen_id: &str,
    ) -> AppResult<crate::core::model::ScreenState> {
        use crate::core::model::{ScreenLayer, SourceBinding, TemplateSource};
        let dir = self.project_dir()?;
        if crate::core::live::is_reserved_screen(screen_id) {
            return Err(AppError::NotFound(format!("Screen not found: {screen_id}")));
        }
        let src = self
            .sources()
            .doc(source_id)
            .ok_or_else(|| AppError::NotFound(format!("Source not found: {source_id}")))?;
        if !sources::is_poll(&src) {
            return Err(AppError::Invalid("That source isn't a poll".into()));
        }
        let mut screen = self
            .screens()
            .doc(screen_id)
            .ok_or_else(|| AppError::NotFound(format!("Screen not found: {screen_id}")))?;
        let existing = self
            .templates()
            .list(&dir)?
            .into_iter()
            .find(|t| t.preset.as_ref().is_some_and(|p| p.id == "poll"));
        let template = match existing {
            Some(t) => self.templates().get(&dir, &t.id)?,
            None => self.templates().create(
                &dir,
                "Poll",
                &TemplateSource::Preset {
                    preset_id: "poll".into(),
                },
            )?,
        };
        let have: HashSet<String> = template.meta.fields.iter().map(|f| f.key.clone()).collect();
        let bindings = POLL_PRESET_BINDINGS
            .iter()
            .filter(|(field, _)| have.contains(*field))
            .map(|(field, column)| {
                (
                    (*field).to_string(),
                    SourceBinding {
                        source_id: source_id.to_string(),
                        column: (*column).to_string(),
                        sort: None,
                        limit: None,
                        row: None,
                    },
                )
            })
            .collect();
        let (rect, sizing) = screens::place_layer(
            template.meta.design.as_ref(),
            screen.meta.width,
            screen.meta.height,
        );
        let taken: HashSet<String> = screen.meta.layers.iter().map(|l| l.id.clone()).collect();
        screen.meta.layers.push(ScreenLayer {
            id: screens::new_layer_id(&taken),
            name: "Poll".into(),
            rect,
            kind: ScreenLayerKind::Template {
                template_id: template.id.clone(),
                fields: Fields::new(),
                vars: Fields::new(),
                bindings,
                variant: None,
                sizing,
                opacity: None,
            },
            anchor: None,
        });
        let state = self.save_screen(screen)?;
        self.polls_reconcile();
        Ok(state)
    }
}

impl AppState {
    /// Test seam (the e2e fixture, tests): sets a test poll's votes exactly, like a simulation step
    /// (the answers keep their order; missing ones are zero). Applies through the source path.
    pub fn set_test_poll_votes(&self, source: &str, votes: &[u64]) -> AppResult<()> {
        self.test_put(source, |t| {
            let mut next = votes.to_vec();
            next.resize(t.options.len(), 0);
            let changed = t.votes != next;
            t.votes = next;
            changed
        })?;
        Ok(())
    }
}
