//! API sources through `AppState` against a local axum mock server (no external network):
//! polling, conditional requests, backoff, holds, in-use tracking, secrets, the test request and
//! the `trigger.apiChanged` flow trigger. Secrets live in an in-memory store, never the keychain.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use axum::extract::{Path as UrlPath, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use super::*;
use crate::core::events::{names, EventSink};
use crate::core::model::{
    ApiCsv, ApiMappingColumn, BackupReason, ColumnKind, FlowEdge, FlowNode, NewFlow, NewScreen,
    NodeKind, PortRef, Rect, Retrigger, RowPick, ScreenDoc, ScreenLayer, ScreenLayerKind,
    SourceBinding, TemplateSource, WireKind,
};
use crate::core::secrets::MemorySecrets;

// ---- Pure helpers --------------------------------------------------------------------------

#[test]
fn backoff_doubles_from_the_interval_up_to_the_cap() {
    let s = Duration::from_secs;
    let waits: Vec<u64> = (0..8)
        .map(|n| backoff(s(10), n, BACKOFF_CAP).as_secs())
        .collect();
    assert_eq!(waits, [10, 20, 40, 80, 160, 300, 300, 300]);
    // Never shorter than the interval (an interval above the cap).
    assert_eq!(backoff(s(600), 3, BACKOFF_CAP), s(600));
    // No overflow on long runs.
    assert_eq!(backoff(s(1), u32::MAX, BACKOFF_CAP), BACKOFF_CAP);
}

#[test]
fn retry_after_seconds_and_http_dates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784_111_777); // 06 Nov 1994 08:49:37 GMT
    assert_eq!(retry_after("120", now), Some(Duration::from_secs(120)));
    assert_eq!(retry_after(" 0 ", now), Some(Duration::ZERO));
    assert_eq!(
        retry_after("Sun, 06 Nov 1994 08:50:07 GMT", now),
        Some(Duration::from_secs(30))
    );
    // A date in the past: no wait.
    assert_eq!(
        retry_after("Sun, 06 Nov 1994 08:00:00 GMT", now),
        Some(Duration::ZERO)
    );
    // Capped.
    assert_eq!(retry_after("999999", now), Some(RETRY_AFTER_MAX));
    for bad in ["soon", "-5", "Sun, 06 Nov 1994 08:50:07 CET", "06 Nov 1994"] {
        assert_eq!(retry_after(bad, now), None, "{bad}");
    }
}

#[test]
fn sensitive_headers_are_redacted() {
    for n in [
        "Authorization",
        "set-cookie",
        "Cookie",
        "X-Api-Key",
        "x-auth-token",
        "X-Client-Secret",
        "x-session-id",
    ] {
        assert!(is_sensitive_header(n), "{n}");
    }
    for n in ["content-type", "etag", "x-ratelimit-remaining", "date"] {
        assert!(!is_sensitive_header(n), "{n}");
    }
    let resp = Resp {
        status: 200,
        reason: "OK".into(),
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("set-cookie".into(), "sid=abc".into()),
            ("x-access-token".into(), "t0k3n".into()),
        ],
        cond: Cond::default(),
        retry_after: None,
        body: br#"{"items":[{"name":"Ada"},{"name":""}]}"#.to_vec(),
    };
    let mapping = ApiMapping {
        rows: "/items".into(),
        columns: vec![mcol("name", "/name"), mcol("title", "/title")],
        csv: ApiCsv::default(),
    };
    let r = test_result(resp, ApiFormat::Json, &mapping);
    assert_eq!(r.headers[0].1, "application/json");
    assert_eq!(r.headers[1].1, REDACTED);
    assert_eq!(r.headers[2].1, REDACTED);
    assert_eq!(r.hits["name"], 1);
    assert_eq!(r.hits["title"], 0);
    assert_eq!(r.rows.len(), 2);
    assert!(r.json.is_some());
    assert!(r.preview.contains("\"Ada\""));
    assert_eq!(r.error, None);
}

// ---- The mock API --------------------------------------------------------------------------

#[derive(Clone)]
struct Script {
    status: u16,
    body: String,
    content_type: &'static str,
    headers: Vec<(&'static str, String)>,
    etag: Option<String>,
}

impl Script {
    fn json(body: Value) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
            content_type: "application/json",
            headers: Vec::new(),
            etag: None,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
            content_type: "text/plain",
            headers: Vec::new(),
            etag: None,
        }
    }
}

#[derive(Default)]
struct MockState {
    scripts: HashMap<String, Script>,
    hits: HashMap<String, u64>,
    last: HashMap<String, BTreeMap<String, String>>,
}

type Mock = Arc<Mutex<MockState>>;

async fn mock_api(
    State(mock): State<Mock>,
    UrlPath(name): UrlPath<String>,
    headers: HeaderMap,
) -> Response {
    let script = {
        let mut m = mock.lock().unwrap();
        *m.hits.entry(name.clone()).or_default() += 1;
        let last = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        m.last.insert(name.clone(), last);
        m.scripts.get(&name).cloned()
    };
    let Some(s) = script else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut h = HeaderMap::new();
    for (k, v) in &s.headers {
        h.insert(*k, HeaderValue::from_str(v).unwrap());
    }
    if let Some(etag) = &s.etag {
        h.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
        if headers
            .get(header::IF_NONE_MATCH)
            .is_some_and(|v| v.to_str().unwrap() == etag)
        {
            return (StatusCode::NOT_MODIFIED, h).into_response();
        }
    }
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(s.content_type),
    );
    (StatusCode::from_u16(s.status).unwrap(), h, s.body).into_response()
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

impl Recorder {
    fn named(&self, name: &str) -> Vec<Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

struct Env {
    tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    rec: Arc<Recorder>,
    dir: PathBuf,
    mock: Mock,
    base: String,
    secrets: Arc<MemorySecrets>,
}

/// An open project (backups off, content flusher slow) whose automation and polling run on the
/// test's runtime, with a mock API at `base` and fast test timing (interval 60 ms, cap 400 ms).
fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    st.set_content_flush_delay(Duration::from_secs(60));
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    let secrets = Arc::new(MemorySecrets::default());
    st.api().set_secret_store(secrets.clone());
    st.api().set_test_timing(
        Some(Duration::from_millis(60)),
        Some(Duration::from_millis(400)),
    );
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let mock: Mock = Arc::default();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = axum::Router::new()
        .route("/api/{name}", axum::routing::get(mock_api).post(mock_api))
        .with_state(mock.clone());
    rt.spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    Env {
        tmp,
        rt,
        st,
        rec,
        dir,
        mock,
        base,
        secrets,
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn mcol(key: &str, path: &str) -> ApiMappingColumn {
    ApiMappingColumn {
        key: key.into(),
        label: key.to_uppercase(),
        kind: ColumnKind::Text,
        path: path.into(),
    }
}

fn items(rows: &[(&str, &str)]) -> Value {
    json!({ "items": rows.iter().map(|(n, t)| json!({ "name": n, "title": t })).collect::<Vec<_>>() })
}

static EDGE: AtomicU64 = AtomicU64::new(0);

fn node(id: &str, kind: NodeKind) -> FlowNode {
    FlowNode {
        id: id.into(),
        position: Default::default(),
        kind,
    }
}

fn edge(from: &str, fp: &str, to: &str, tp: &str, kind: WireKind) -> FlowEdge {
    FlowEdge {
        id: format!("e{}", EDGE.fetch_add(1, AtomicOrdering::Relaxed)),
        from: PortRef {
            node: from.into(),
            port: fp.into(),
        },
        to: PortRef {
            node: to.into(),
            port: tp.into(),
        },
        kind,
    }
}

impl Env {
    fn script(&self, name: &str, s: Script) {
        self.mock
            .lock()
            .unwrap()
            .scripts
            .insert(name.to_string(), s);
    }

    fn hits(&self, name: &str) -> u64 {
        self.mock
            .lock()
            .unwrap()
            .hits
            .get(name)
            .copied()
            .unwrap_or(0)
    }

    fn last_headers(&self, name: &str) -> BTreeMap<String, String> {
        self.mock
            .lock()
            .unwrap()
            .last
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    /// An API source on mock `name`: rows `/items`, columns `name` and `title`.
    fn source(&self, name: &str, only_when_used: bool) -> SourceDoc {
        self.source_with(name, only_when_used, Vec::new())
    }

    fn source_with(&self, name: &str, only_when_used: bool, headers: Vec<ApiHeader>) -> SourceDoc {
        let mut doc = self.st.create_api_source(name).unwrap();
        doc.meta.kind = SourceKind::Api {
            request: ApiRequest {
                url: format!("{}/api/{name}", self.base),
                method: ApiMethod::Get,
                headers,
                body: None,
            },
            format: ApiFormat::Json,
            mapping: ApiMapping {
                rows: "/items".into(),
                columns: vec![mcol("name", "/name"), mcol("title", "/title")],
                csv: ApiCsv::default(),
            },
            poll: ApiPoll {
                interval_ms: 1000,
                only_when_used,
                paused: false,
            },
            columns: Vec::new(),
            rows: Vec::new(),
            index: 0,
            auto: None,
            fetched_at: None,
        };
        self.st.save_source(doc).unwrap()
    }

    fn rows(&self, id: &str) -> Vec<Fields> {
        match self.st.get_source(id).unwrap().meta.kind {
            SourceKind::Api { rows, .. } => rows,
            _ => panic!("not an API source"),
        }
    }

    fn names(&self, id: &str) -> Vec<String> {
        self.rows(id).iter().map(|r| r["name"].clone()).collect()
    }

    fn status(&self, id: &str) -> ApiStatus {
        let doc = self.st.get_source(id).unwrap();
        self.st.api_status(&doc).unwrap()
    }

    fn changed(&self, id: &str) -> Vec<Value> {
        self.rec
            .named(names::SOURCE_CHANGED)
            .into_iter()
            .filter(|v| v["id"] == id)
            .collect()
    }

    fn polling(&self, id: &str) -> bool {
        self.st.api().polling().iter().any(|p| p == id)
    }

    /// A template from the classic-bar preset (fields `name`, `title`).
    fn template(&self) -> String {
        self.st
            .templates()
            .create(
                &self.dir,
                "Bar",
                &TemplateSource::Preset {
                    preset_id: "classic-bar".into(),
                },
            )
            .unwrap()
            .id
    }

    /// A screen with layer `l1` of template `t`; field `name` bound to `source` when given.
    fn screen(&self, t: &str, source: Option<&str>) -> ScreenDoc {
        let mut doc = self
            .st
            .create_screen(&NewScreen {
                name: "Main".into(),
                width: 1920,
                height: 1080,
                background: "transparent".into(),
            })
            .unwrap()
            .doc;
        let mut bindings = BTreeMap::new();
        if let Some(src) = source {
            bindings.insert(
                "name".to_string(),
                SourceBinding {
                    source_id: src.into(),
                    column: "name".into(),
                    sort: None,
                    limit: None,
                    row: None,
                },
            );
        }
        doc.meta.layers.push(ScreenLayer {
            id: "l1".into(),
            name: "Lower third".into(),
            rect: Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            kind: ScreenLayerKind::Template {
                template_id: t.into(),
                fields: [("name", "Own"), ("title", "Own title")]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                vars: Fields::new(),
                bindings,
                variant: None,
                sizing: None,
                opacity: None,
            },
            anchor: None,
        });
        self.st.save_screen(doc).unwrap().doc
    }

    fn field(&self, screen: &str, key: &str) -> String {
        self.st
            .live()
            .with_layer(screen, "l1", |l| {
                l.fields.get(key).cloned().unwrap_or_default()
            })
            .unwrap()
    }

    fn flow(&self, name: &str, nodes: Vec<FlowNode>, edges: Vec<FlowEdge>) -> String {
        self.st
            .create_flow(NewFlow {
                name: name.into(),
                armed: true,
                retrigger: Retrigger::Restart,
                origin: None,
                nodes,
                edges,
            })
            .unwrap()
            .flow
            .id
    }

    fn flow_ends(&self, flow: &str) -> usize {
        self.rec
            .named(names::FLOW_STATE)
            .into_iter()
            .filter(|v| v["id"] == flow && (v["state"] == "finished" || v["state"] == "failed"))
            .count()
    }
}

/// Every file under `dir` (recursively) that contains `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            fs::read(e.path())
                .map(|b| String::from_utf8_lossy(&b).contains(needle))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

// ---- Polling ---------------------------------------------------------------------------------

#[test]
fn polls_maps_and_applies_rows_to_bound_screens() {
    let e = env();
    e.script(
        "guests",
        Script::json(items(&[("Ada", "Host"), ("Grace", "Guest")])),
    );
    let t = e.template();
    let src = e.source("guests", true);
    assert!(!e.polling(&src.id), "nothing uses it yet");
    let s = e.screen(&t, Some(&src.id));
    wait_until("rows", || e.names(&src.id) == ["Ada", "Grace"]);
    assert!(e.polling(&src.id));
    wait_until("bound field", || e.field(&s.id, "name") == "Ada");
    let st = e.status(&src.id);
    assert_eq!(st.state, ApiState::Ok);
    assert_eq!(st.http_status, Some(200));
    assert_eq!(st.rows, 2);
    assert!(st.last_ok_at.is_some() && st.next_at.is_some());
    // The summary carries it; the doc has fetchedAt.
    let summary =
        e.st.list_sources()
            .unwrap()
            .into_iter()
            .find(|x| x.id == src.id)
            .unwrap();
    assert_eq!(summary.api.unwrap().state, ApiState::Ok);
    assert_eq!(summary.current["name"], "Ada");
    let doc = e.st.get_source(&src.id).unwrap();
    assert!(matches!(
        doc.meta.kind,
        SourceKind::Api {
            fetched_at: Some(_),
            ..
        }
    ));
    // On air: new data goes on air directly (automation rules).
    e.st.live().show(&s.id, "l1").unwrap();
    e.script("guests", Script::json(items(&[("Linus", "Host")])));
    wait_until("on air update", || e.field(&s.id, "name") == "Linus");
    assert!(e.st.live().is_on_air(&s.id, "l1"));
    // The cache reaches the file (coalesced; here an explicit flush).
    e.st.flush_all();
    let file: Value = serde_json::from_str(
        &fs::read_to_string(e.dir.join("sources").join(format!("{}.json", src.id))).unwrap(),
    )
    .unwrap();
    assert_eq!(file["rows"][0]["name"], "Linus");
    assert_eq!(file["kind"], "api");
    assert!(file["fetchedAt"].is_string());
}

#[test]
fn unchanged_data_sends_nothing_and_writes_nothing() {
    let e = env();
    e.script("same", Script::json(items(&[("Ada", "Host")])));
    let t = e.template();
    let src = e.source("same", true);
    let s = e.screen(&t, Some(&src.id));
    e.st.live().show(&s.id, "l1").unwrap();
    wait_until("rows", || e.names(&src.id) == ["Ada"]);
    wait_until("ok", || e.status(&src.id).state == ApiState::Ok);
    e.st.flush_all();
    let writes = e.st.content_write_count();
    let events = e.changed(&src.id).len();
    let mut rx = e.st.live().subscribe();
    // Same body: the hash matches.
    let h = e.hits("same");
    wait_until("more polls", || e.hits("same") >= h + 3);
    // 304 with an ETag.
    let mut etagged = Script::json(items(&[("Ada", "Host")]));
    etagged.etag = Some("\"v1\"".into());
    e.script("same", etagged);
    let h = e.hits("same");
    wait_until("conditional polls", || e.hits("same") >= h + 4);
    assert_eq!(
        e.last_headers("same")
            .get("if-none-match")
            .map(String::as_str),
        Some("\"v1\"")
    );
    // A new body with identical rows (key order differs): no change either.
    e.script(
        "same",
        Script {
            body: r#"{"items":[{"title":"Host","name":"Ada"}],"extra":1}"#.into(),
            ..Script::json(json!({}))
        },
    );
    let h = e.hits("same");
    wait_until("polls", || e.hits("same") >= h + 2);
    assert_eq!(e.changed(&src.id).len(), events, "no source-changed");
    assert!(!e.st.sources().is_dirty(&src.id), "no write pending");
    e.st.flush_all();
    assert_eq!(e.st.content_write_count(), writes, "nothing written");
    let msgs: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|m| &*m.screen_id == s.id.as_str())
        .collect();
    assert!(msgs.is_empty(), "no live message: {}", msgs.len());
}

#[test]
fn in_use_tracking_starts_and_stops_tasks() {
    let e = env();
    e.script("u", Script::json(items(&[("Ada", "Host")])));
    let t = e.template();
    let src = e.source("u", true);
    assert_eq!(e.st.api().task_count(), 0, "0 tasks at rest");
    assert_eq!(e.status(&src.id).state, ApiState::Idle);
    // A bound layer.
    let s = e.screen(&t, Some(&src.id));
    assert!(e.polling(&src.id));
    let mut doc = s.clone();
    let ScreenLayerKind::Template { bindings, .. } = &mut doc.meta.layers[0].kind else {
        unreachable!()
    };
    bindings.clear();
    e.st.save_screen(doc).unwrap();
    assert!(!e.polling(&src.id), "unbound");
    assert_eq!(e.status(&src.id).state, ApiState::Idle);
    // A flow that reads it.
    let f = e.flow(
        "reads",
        vec![
            node("cue", NodeKind::Cue { label: None }),
            node(
                "d",
                NodeKind::SourceRow {
                    source_id: src.id.clone(),
                    row: RowPick::Current,
                },
            ),
        ],
        vec![],
    );
    assert!(e.polling(&src.id), "used by a flow");
    e.st.set_flow_armed(&f, false).unwrap();
    assert!(!e.polling(&src.id), "a disarmed flow doesn't count");
    e.st.set_flow_armed(&f, true).unwrap();
    assert!(e.polling(&src.id));
    e.st.delete_flow(&f).unwrap();
    assert!(!e.polling(&src.id));
    // The editor's watch (counted).
    e.st.watch_api_source(&src.id, true).unwrap();
    e.st.watch_api_source(&src.id, true).unwrap();
    assert!(e.polling(&src.id));
    e.st.watch_api_source(&src.id, false).unwrap();
    assert!(e.polling(&src.id), "still watched once");
    e.st.watch_api_source(&src.id, false).unwrap();
    assert!(!e.polling(&src.id));
    // onlyWhenUsed off: always.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Api { poll, .. } = &mut doc.meta.kind {
        poll.only_when_used = false;
    }
    e.st.save_source(doc).unwrap();
    assert!(e.polling(&src.id));
    // Paused: no task; resumed: polls again.
    let sum = e.st.set_api_paused(&src.id, true).unwrap();
    assert_eq!(sum.api.unwrap().state, ApiState::Paused);
    assert_eq!(e.st.api().task_count(), 0);
    let h = e.hits("u");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.hits("u"), h, "no request while paused");
    e.st.set_api_paused(&src.id, false).unwrap();
    assert!(e.polling(&src.id));
    // PANIC doesn't stop polling.
    e.st.live_panic().unwrap();
    assert!(e.polling(&src.id));
    // Deleting the source stops it.
    e.st.delete_source(&src.id).unwrap();
    assert_eq!(e.st.api().task_count(), 0);
    // Closing the project too.
    let src2 = e.source("u", false);
    assert!(e.polling(&src2.id));
    e.st.close_project();
    assert_eq!(e.st.api().task_count(), 0);
}

#[test]
fn errors_back_off_and_recover() {
    let e = env();
    e.script("flaky", Script::status(500));
    let src = e.source("flaky", false);
    wait_until("backoff", || e.status(&src.id).state == ApiState::Backoff);
    let s = e.status(&src.id);
    assert_eq!(s.http_status, Some(500));
    assert!(s.message.unwrap().contains("HTTP 500"));
    // Waits grow: 120, 240, 400 (cap), 400 ms with a 60 ms interval.
    // Read the status between attempts: while a fetch runs, `nextAt` still names the old wait.
    let mut s = e.status(&src.id);
    wait_until("several failures", || {
        s = e.status(&src.id);
        e.hits("flaky") >= 4 && s.state == ApiState::Backoff && s.next_at > s.last_attempt_at
    });
    let wait = s.next_at.unwrap() - s.last_attempt_at.unwrap();
    // At least the cap; the upper bound allows a slow response under a loaded test run.
    assert!((390..=1500).contains(&wait), "capped wait {wait}");
    // Unparseable JSON is an error too.
    e.script(
        "flaky",
        Script {
            body: "{oops".into(),
            ..Script::json(json!({}))
        },
    );
    wait_until("parse error", || {
        e.status(&src.id)
            .message
            .is_some_and(|m| m.contains("not valid JSON"))
    });
    // Recovery resets the wait to the interval.
    e.script("flaky", Script::json(items(&[("Ada", "Host")])));
    wait_until("recovered", || e.status(&src.id).state == ApiState::Ok);
    let s = e.status(&src.id);
    assert_eq!(s.message, None);
    assert!(s.next_at.unwrap() - s.last_ok_at.unwrap() <= 100);
    let states = || -> Vec<String> {
        e.changed(&src.id)
            .iter()
            .map(|v| v["api"]["state"].as_str().unwrap().to_string())
            .collect()
    };
    // The status is updated before `source-changed` goes out.
    wait_until("ok announced", || {
        states().last().map(String::as_str) == Some("ok")
    });
    let states = states();
    assert_eq!(states.first().map(String::as_str), Some("fetching"));
    assert!(states.contains(&"backoff".to_string()));
    assert_eq!(states.last().map(String::as_str), Some("ok"));
}

#[test]
fn rate_limits_wait_for_retry_after() {
    let e = env();
    let mut s = Script::status(429);
    s.headers = vec![("Retry-After", "1".into())];
    e.script("limited", s);
    let src = e.source("limited", false);
    wait_until("429", || e.status(&src.id).http_status == Some(429));
    let st = e.status(&src.id);
    assert_eq!(st.state, ApiState::Backoff);
    let wait = st.next_at.unwrap() - st.last_attempt_at.unwrap();
    assert!(
        // At least Retry-After; the upper bound allows a slow response under a loaded test run.
        (990..=1500).contains(&wait),
        "Retry-After respected: {wait}"
    );
    let h = e.hits("limited");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(e.hits("limited"), h, "no request before Retry-After");
    // An HTTP date. Whole seconds, 4 s ahead: the next request still waits out the previous
    // Retry-After (up to ~1.2 s after this line), so the recorded wait is 1.8..4 s whatever the
    // timing (a 2 s date made the lower bound race the previous wait).
    let at = time::OffsetDateTime::now_utc() + time::Duration::seconds(4);
    let date = format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        &at.weekday().to_string()[..3],
        at.day(),
        &at.month().to_string()[..3],
        at.year(),
        at.hour(),
        at.minute(),
        at.second()
    );
    let mut s = Script::status(429);
    s.headers = vec![("Retry-After", date)];
    e.script("limited", s);
    let h = e.hits("limited");
    wait_until("next 429", || e.hits("limited") > h);
    // Applied: the new attempt's wait is recorded.
    wait_until("recorded", || {
        let st = e.status(&src.id);
        match (st.last_attempt_at, st.next_at) {
            (Some(a), Some(n)) => a >= fsx::now_epoch_ms() - 700 && n >= a + 500,
            _ => false,
        }
    });
    let st = e.status(&src.id);
    let wait = st.next_at.unwrap() - st.last_attempt_at.unwrap();
    assert!((1500..=4100).contains(&wait), "HTTP date respected: {wait}");
    // Without Retry-After: plain backoff.
    e.script("limited", Script::status(429));
    let h = e.hits("limited");
    wait_until("plain 429", || e.hits("limited") >= h + 2);
    e.script("limited", Script::json(items(&[("Ada", "")])));
    wait_until("ok", || e.status(&src.id).state == ApiState::Ok);
}

#[test]
fn auth_errors_hold_until_retry() {
    let e = env();
    e.script("auth", Script::status(401));
    let src = e.source("auth", false);
    wait_until("held", || e.status(&src.id).state == ApiState::Error);
    assert_eq!(e.st.api().task_count(), 0, "no task while held");
    let h = e.hits("auth");
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(e.hits("auth"), h, "no retries");
    assert!(e.status(&src.id).message.unwrap().contains("HTTP 401"));
    // Live changes don't restart it.
    e.st.live_panic().unwrap();
    assert_eq!(e.st.api().task_count(), 0);
    // Retry: fetched again; recovered.
    e.script("auth", Script::json(items(&[("Ada", "Host")])));
    e.st.retry_api_source(&src.id).unwrap();
    wait_until("ok", || e.status(&src.id).state == ApiState::Ok);
    assert!(e.polling(&src.id));
    // 403 holds too; a save with new settings clears the hold.
    e.script("auth", Script::status(403));
    wait_until("held again", || e.status(&src.id).state == ApiState::Error);
    e.script("auth", Script::json(items(&[("Grace", "Host")])));
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Api { request, .. } = &mut doc.meta.kind {
        request.url.push_str("?v=2");
    }
    e.st.save_source(doc).unwrap();
    wait_until("new rows", || e.names(&src.id) == ["Grace"]);
}

#[test]
fn retry_fetches_once_when_not_in_use() {
    let e = env();
    e.script("once", Script::json(items(&[("Ada", "Host")])));
    let src = e.source("once", true);
    assert_eq!(e.st.api().task_count(), 0);
    e.st.retry_api_source(&src.id).unwrap();
    wait_until("rows", || e.names(&src.id) == ["Ada"]);
    wait_until("one-off done", || e.st.api().task_count() == 0);
    assert_eq!(e.hits("once"), 1);
    assert_eq!(e.status(&src.id).state, ApiState::Ok);
}

// ---- Secrets ---------------------------------------------------------------------------------

const SECRET: &str = "s3cr3t-VALUE-never-on-disk";

#[test]
fn secret_missing_holds_without_a_request() {
    let e = env();
    e.script("key", Script::json(items(&[("Ada", "Host")])));
    let src = e.source_with(
        "key",
        false,
        vec![ApiHeader {
            name: "X-Api-Key".into(),
            secret: Some("notInTheKeychain1".into()),
            ..Default::default()
        }],
    );
    wait_until("secretMissing", || {
        e.status(&src.id).state == ApiState::SecretMissing
    });
    assert_eq!(e.hits("key"), 0, "no request");
    assert_eq!(e.st.api().task_count(), 0);
    assert!(e
        .status(&src.id)
        .message
        .unwrap()
        .contains("Secret missing on this machine"));
    let doc = e.st.get_source(&src.id).unwrap();
    let SourceKind::Api { request, .. } = &doc.meta.kind else {
        panic!()
    };
    assert_eq!(request.headers[0].stored, Some(false));
    // Entering it resumes polling with the header.
    let doc =
        e.st.set_source_secret(&src.id, "X-Api-Key", SECRET)
            .unwrap();
    let SourceKind::Api { request, .. } = &doc.meta.kind else {
        panic!()
    };
    assert_eq!(request.headers[0].stored, Some(true));
    assert_eq!(request.headers[0].value, None);
    wait_until("rows", || e.names(&src.id) == ["Ada"]);
    assert_eq!(
        e.last_headers("key").get("x-api-key").map(String::as_str),
        Some(SECRET)
    );
    assert_eq!(
        e.last_headers("key").get("user-agent").map(String::as_str),
        Some(concat!("Kinetra/", env!("CARGO_PKG_VERSION")))
    );
}

#[test]
fn secrets_are_never_written_to_project_files() {
    let e = env();
    e.script("sec", Script::json(items(&[("Ada", "Host")])));
    let src = e.source("sec", false);
    e.st.set_source_secret(&src.id, "Authorization", SECRET)
        .unwrap();
    wait_until("rows", || e.names(&src.id) == ["Ada"]);
    wait_until("the secret header sent", || {
        e.last_headers("sec")
            .get("authorization")
            .map(String::as_str)
            == Some(SECRET)
    });
    // A UI save that still carries a value next to the secret id: dropped.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Api { request, .. } = &mut doc.meta.kind {
        request.headers[0].value = Some(SECRET.into());
    }
    let saved = e.st.save_source(doc).unwrap();
    let SourceKind::Api { request, .. } = &saved.meta.kind else {
        panic!()
    };
    assert_eq!(request.headers[0].value, None);
    // Status and events never carry it.
    e.script("sec", Script::status(500));
    wait_until("error", || e.status(&src.id).state == ApiState::Backoff);
    e.st.flush_all();
    e.st.create_snapshot(BackupReason::Manual).unwrap();
    crate::core::settings_io::export(&e.st, &e.tmp.path().join("export.json")).unwrap();
    assert!(e.tmp.path().join("export.json").exists());
    let found = files_containing(e.tmp.path(), SECRET);
    assert!(found.is_empty(), "secret found in {found:?}");
    let events = serde_json::to_string(&*e.rec.0.lock().unwrap()).unwrap();
    assert!(!events.contains(SECRET), "secret in an event");
    // The secret id is in the file, the value in the store.
    let file = fs::read_to_string(e.dir.join("sources").join(format!("{}.json", src.id))).unwrap();
    assert!(file.contains("\"secret\""));
    assert!(!file.contains("\"stored\""), "stored is never written");
    assert_eq!(e.secrets.len(), 1);
}

#[test]
fn duplicate_copies_and_delete_removes_secrets() {
    let e = env();
    let src = e.source("dup", true);
    e.st.set_source_secret(&src.id, "X-Key", SECRET).unwrap();
    let dup = e.st.duplicate_source(&src.id, "Copy").unwrap();
    let id_of = |d: &SourceDoc| match &d.meta.kind {
        SourceKind::Api { request, .. } => request.headers[0].secret.clone().unwrap(),
        _ => panic!(),
    };
    let (a, b) = (id_of(&e.st.get_source(&src.id).unwrap()), id_of(&dup));
    assert_ne!(a, b, "a new secret id");
    let store = e.st.api().secrets();
    assert_eq!(store.get(&b).unwrap().as_deref(), Some(SECRET));
    assert_eq!(e.secrets.len(), 2);
    // Clearing one keeps the header.
    let cleared = e.st.clear_source_secret(&dup.id, "x-key").unwrap();
    let SourceKind::Api { request, .. } = &cleared.meta.kind else {
        panic!()
    };
    assert_eq!(request.headers[0].name, "X-Key");
    assert_eq!(request.headers[0].secret, None);
    assert_eq!(store.get(&b).unwrap(), None);
    // Removing the header in a save deletes its value.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Api { request, .. } = &mut doc.meta.kind {
        request.headers.clear();
    }
    e.st.save_source(doc).unwrap();
    assert!(e.secrets.is_empty());
    // Deleting a source deletes its secrets.
    e.st.set_source_secret(&src.id, "X-Key", SECRET).unwrap();
    assert_eq!(e.secrets.len(), 1);
    e.st.delete_source(&src.id).unwrap();
    assert!(e.secrets.is_empty());
}

// ---- Test request ----------------------------------------------------------------------------

#[test]
fn test_request_returns_preview_rows_and_redacted_headers() {
    let e = env();
    let mut s = Script::json(items(&[("Ada", "Host"), ("Grace", "")]));
    s.headers = vec![("Set-Cookie", "sid=xyz".into()), ("X-Rate", "9".into())];
    e.script("t", s);
    let src = e.source("t", true);
    e.st.set_source_secret(&src.id, "Authorization", SECRET)
        .unwrap();
    let draft = e.st.get_source(&src.id).unwrap();
    let r = e.rt.block_on(e.st.test_api_source(draft.clone())).unwrap();
    assert_eq!(r.error, None);
    assert_eq!(r.http_status, Some(200));
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.hits["name"], 2);
    assert_eq!(r.hits["title"], 1);
    assert!(r.json.is_some() && r.csv_columns.is_none());
    let cookie = r.headers.iter().find(|(k, _)| k == "set-cookie").unwrap();
    assert_eq!(cookie.1, REDACTED);
    assert!(r.headers.iter().any(|(k, v)| k == "x-rate" && v == "9"));
    assert_eq!(
        e.last_headers("t").get("authorization").map(String::as_str),
        Some(SECRET)
    );
    assert!(!serde_json::to_string(&r).unwrap().contains(SECRET));
    // Nothing saved, nothing polled.
    assert!(e.rows(&src.id).is_empty());
    assert_eq!(e.st.api().task_count(), 0);
    // CSV draft.
    e.script(
        "c",
        Script {
            body: "name;title\nAda;Host\n".into(),
            content_type: "text/csv",
            ..Script::status(200)
        },
    );
    let mut csv = draft.clone();
    if let SourceKind::Api {
        request,
        format,
        mapping,
        ..
    } = &mut csv.meta.kind
    {
        request.url = format!("{}/api/c", e.base);
        request.headers.clear();
        *format = ApiFormat::Csv;
        mapping.columns = vec![mcol("name", "name"), mcol("t", "title")];
    }
    let r = e.rt.block_on(e.st.test_api_source(csv)).unwrap();
    assert_eq!(r.csv_columns.as_deref().unwrap(), ["name", "title"]);
    assert_eq!(r.rows[0]["t"], "Host");
    assert!(r.json.is_none());
    // Errors are in the result.
    let mut bad = draft.clone();
    if let SourceKind::Api { request, .. } = &mut bad.meta.kind {
        request.url = format!("{}/api/missing", e.base);
    }
    let r = e.rt.block_on(e.st.test_api_source(bad)).unwrap();
    assert_eq!(r.http_status, Some(404));
    assert!(r.error.unwrap().contains("404"));
    let mut bad = draft;
    if let SourceKind::Api { request, .. } = &mut bad.meta.kind {
        request.url = "ftp://x".into();
    }
    let r = e.rt.block_on(e.st.test_api_source(bad)).unwrap();
    assert!(r.error.unwrap().contains("http"));
}

#[test]
fn responses_over_2_mb_are_errors() {
    let e = env();
    let big = format!(
        "{{\"items\":[{{\"name\":\"{}\"}}]}}",
        "x".repeat(API_RESPONSE_MAX_BYTES)
    );
    e.script(
        "big",
        Script {
            body: big,
            ..Script::json(json!({}))
        },
    );
    let src = e.source("big", false);
    wait_until("too large", || {
        e.status(&src.id)
            .message
            .is_some_and(|m| m == "Response is larger than 2 MB")
    });
    assert!(e.rows(&src.id).is_empty());
}

// ---- Flows: trigger.apiChanged ---------------------------------------------------------------

#[test]
fn api_changed_fires_on_column_and_any_row_changes() {
    let e = env();
    e.script(
        "score",
        Script::json(items(&[("Home", "0"), ("Away", "0")])),
    );
    let t = e.template();
    let s = e.screen(&t, None);
    let src = e.source("score", true);
    // Paused while the flows are made, so both see the first rows.
    e.st.set_api_paused(&src.id, true).unwrap();
    let log = |id: &str| {
        node(
            id,
            NodeKind::Log {
                message: id.into(),
                toast: false,
            },
        )
    };
    // Column `title` of the current row → the layer's `title` field.
    let col_flow = e.flow(
        "column",
        vec![
            node(
                "t",
                NodeKind::ApiChanged {
                    source_id: src.id.clone(),
                    column: Some("title".into()),
                },
            ),
            node(
                "set",
                NodeKind::SetFields {
                    screen_id: s.id.clone(),
                    layer_id: "l1".into(),
                    fields: [("title".to_string(), String::new())].into(),
                },
            ),
        ],
        vec![
            edge("t", "fired", "set", "in", WireKind::Event),
            edge("t", "value", "set", "title", WireKind::Data),
        ],
    );
    let any_flow = e.flow(
        "any",
        vec![
            node(
                "t",
                NodeKind::ApiChanged {
                    source_id: src.id.clone(),
                    column: None,
                },
            ),
            log("l"),
        ],
        vec![edge("t", "fired", "l", "in", WireKind::Event)],
    );
    // The flows use the source: it polls.
    e.st.set_api_paused(&src.id, false).unwrap();
    wait_until("first rows", || e.names(&src.id) == ["Home", "Away"]);
    wait_until("first firing", || e.flow_ends(&any_flow) == 1);
    wait_until("column firing", || e.field(&s.id, "title") == "0");
    // The field is set before the flow reports `finished`.
    wait_until("column flow finished", || e.flow_ends(&col_flow) == 1);
    let col_runs = 1;
    // Another row changes: any-row fires, the column trigger (current row) doesn't.
    e.script(
        "score",
        Script::json(items(&[("Home", "0"), ("Away", "1")])),
    );
    wait_until("any-row firing", || e.flow_ends(&any_flow) == 2);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        e.flow_ends(&col_flow),
        col_runs,
        "current row's title unchanged"
    );
    // The current row's column changes: both fire, with the new value.
    e.script(
        "score",
        Script::json(items(&[("Home", "2"), ("Away", "1")])),
    );
    wait_until("column value", || e.field(&s.id, "title") == "2");
    wait_until("any-row again", || e.flow_ends(&any_flow) == 3);
    wait_until("column flow again", || {
        e.flow_ends(&col_flow) == col_runs + 1
    });
    // Unchanged data: nothing fires.
    let h = e.hits("score");
    wait_until("polls", || e.hits("score") >= h + 3);
    assert_eq!(e.flow_ends(&any_flow), 3);
    // Warnings: not an API source.
    let text = e.st.create_source("Plain").unwrap();
    let check =
        e.st.check_flow_of(
            None,
            vec![node(
                "t",
                NodeKind::ApiChanged {
                    source_id: text.id.clone(),
                    column: Some("nope".into()),
                },
            )],
            vec![],
        )
        .unwrap();
    assert!(check
        .warnings
        .iter()
        .any(|w| w.message.contains("not an API source")));
    let check =
        e.st.check_flow_of(
            None,
            vec![node(
                "t",
                NodeKind::ApiChanged {
                    source_id: src.id.clone(),
                    column: Some("nope".into()),
                },
            )],
            vec![],
        )
        .unwrap();
    assert!(check.warnings.iter().any(|w| w.message.contains("nope")));
}

// ---- Rest ------------------------------------------------------------------------------------

#[test]
fn no_tasks_at_rest_and_cache_survives_reopen() {
    let e = env();
    e.script("rest", Script::json(items(&[("Ada", "Host")])));
    let src = e.source("rest", false);
    wait_until("rows", || e.names(&src.id) == ["Ada"]);
    // Not in use + paused or onlyWhenUsed: nothing runs.
    let mut doc = e.st.get_source(&src.id).unwrap();
    if let SourceKind::Api { poll, .. } = &mut doc.meta.kind {
        poll.only_when_used = true;
    }
    e.st.save_source(doc).unwrap();
    assert_eq!(e.st.api().task_count(), 0);
    let h = e.hits("rest");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(e.hits("rest"), h);
    // Reopen: the cached rows are there before any fetch.
    e.st.flush_all();
    let dir = e.dir.clone();
    e.st.close_project();
    e.st.open_project(&dir).unwrap();
    assert_eq!(e.names(&src.id), ["Ada"]);
    assert_eq!(e.st.api().task_count(), 0);
    assert_eq!(e.status(&src.id).state, ApiState::Idle);
}
