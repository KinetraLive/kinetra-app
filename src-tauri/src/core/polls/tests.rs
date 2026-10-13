//! Live polls: the data source mapping, the SSE parser and input rules (pure), then the service
//! through `AppState` against a local mock of the poll API (raw TCP: JSON answers and a
//! scriptable event stream; no external network): the Pro lock, Pro-ended behaviour, live
//! results with reconnect and fallback, deletion, and test mode.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;

use super::*;
use crate::core::cloud::service::S_SESSION;
use crate::core::events::EventSink;
use crate::core::model::{PollConn, PollData, PollOption, PollStatus};
use crate::core::secrets::{MemorySecrets, SecretStore};
use crate::core::sources::Current;

const MACHINE: &str = "857a78b7a23b14cd718a9cbf9b90b0577225543ddcce5ed21854a4985f25ee27";
const PID: &str = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
const CODE: &str = "K7P2QX";

// ---- Pure ---------------------------------------------------------------------------------------

fn poll(votes: &[u64]) -> PollData {
    PollData {
        id: PID.into(),
        code: CODE.into(),
        url: format!("https://poll.kinetra.live/{CODE}"),
        question: "Who wins tonight?".into(),
        options: votes
            .iter()
            .enumerate()
            .map(|(i, v)| PollOption {
                position: i as u32,
                label: ["Home", "Away", "Draw"][i % 3].into(),
                votes: *v,
            })
            .collect(),
        status: PollStatus::Open,
        show_results: true,
        total_votes: votes.iter().sum(),
        max_votes: Some(2000),
        round: 1,
        ..PollData::default()
    }
}

#[test]
fn rows_carry_answers_and_the_poll_values() {
    let rows = rows(&poll(&[1112, 750, 556]));
    assert_eq!(rows.len(), 3);
    let r = &rows[0];
    assert_eq!(r["label"], "Home");
    assert_eq!(r["votes"], "1,112");
    assert_eq!(r["pct"], "45.99");
    assert_eq!(r["position"], "1");
    // The poll's own values are on every row.
    for r in &rows {
        assert_eq!(r["question"], "Who wins tonight?");
        assert_eq!(r["total"], "2,418");
        assert_eq!(r["total_n"], "2418");
        assert_eq!(r["status"], "open");
        assert_eq!(r["leader"], "Home");
        assert_eq!(r["short_url"], "poll.kinetra.live/K7P2QX");
        assert_eq!(r["code"], CODE);
        assert!(r["qr"].starts_with("data:image/svg+xml,"));
    }
    // Rounded shares add up to 100.
    let sum: u32 = rows
        .iter()
        .map(|r| r["pct_round"].parse::<u32>().unwrap())
        .sum();
    assert_eq!(sum, 100);
    // Only the leader carries the mark; every column is a declared one or a row mark.
    assert_eq!(rows[0].get("_leader").map(String::as_str), Some("true"));
    assert!(!rows[1].contains_key("_leader") && !rows[2].contains_key("_leader"));
    let keys: Vec<&str> = POLL_COLUMNS.iter().map(|(k, _, _)| *k).collect();
    assert!(rows[1].keys().all(|k| keys.contains(&k.as_str())));
    // Closed: no leading mark (the winner is marked per layer); full still leads.
    let mut closed = poll(&[1112, 750, 556]);
    closed.status = PollStatus::Closed;
    assert!(super::rows(&closed)
        .iter()
        .all(|r| !r.contains_key("_leader")));
    let mut full = poll(&[1112, 750, 556]);
    full.vote_limit_reached = true;
    assert_eq!(
        super::rows(&full)[0].get("_leader").map(String::as_str),
        Some("true")
    );
}

#[test]
fn shares_ties_no_votes_and_full() {
    let (two, whole) = shares(&[1, 1, 1]);
    assert_eq!(two, ["33.33", "33.33", "33.33"]);
    assert_eq!(
        whole,
        [34, 33, 33],
        "the remainder goes to the earliest answer"
    );
    let (two, whole) = shares(&[2, 1]);
    assert_eq!(
        (two, whole),
        (vec!["66.67".into(), "33.33".into()], vec![67, 33])
    );
    let (two, whole) = shares(&[0, 0]);
    assert_eq!(
        (two, whole),
        (vec!["0".to_string(), "0".into()], vec![0, 0])
    );
    assert_eq!(shares(&[3, 1]).0, ["75", "25"], "trailing zeros dropped");
    // Two decimals keep near-ties apart for the winner ranking.
    assert_ne!(shares(&[1001, 1000]).0[0], shares(&[1001, 1000]).0[1]);
    // A tie for first: no leader; no votes: no leader.
    assert_eq!(leader(&poll(&[5, 5, 1])), None);
    assert_eq!(leader(&poll(&[0, 0])), None);
    assert_eq!(leader(&poll(&[1, 4])), Some(1));
    let mut p = poll(&[3, 1]);
    p.vote_limit_reached = true;
    assert_eq!(state_of(&p), "full");
    p.status = PollStatus::Closed;
    assert_eq!(state_of(&p), "closed");
    assert_eq!(group(0), "0");
    assert_eq!(group(999), "999");
    assert_eq!(group(1_234_567), "1,234,567");
}

#[test]
fn a_poll_source_reads_like_any_source() {
    let doc = SourceDoc {
        id: "pollsrc001".into(),
        meta: crate::core::model::SourceMeta {
            name: "Poll".into(),
            kind: SourceKind::Poll {
                link: Some(PollLink {
                    id: PID.into(),
                    code: CODE.into(),
                }),
                cache: Some(poll(&[2, 6])),
                gone: false,
            },
            updated_at: String::new(),
        },
    };
    let now = crate::core::localtime::Clock::default().now();
    let cur = Current::of(&doc, &now);
    assert_eq!(cur.kind, crate::core::model::SourceKindName::Poll);
    assert_eq!(cur.columns.len(), POLL_COLUMNS.len());
    assert_eq!(cur.rows.len(), 2);
    assert_eq!(cur.rows[1]["_leader"], "true");
    // A single field binding reads the first answer's row without row marks.
    let row = cur.row.unwrap();
    assert_eq!(row["question"], "Who wins tonight?");
    assert!(!row.keys().any(|k| k.starts_with('_')));
    assert_eq!(
        sources::columns_of(&doc)[13].kind,
        ColumnKind::Image,
        "qr is an image"
    );
    assert_eq!(sources::position(&doc), (2, 0));
    // Without data: no rows, nothing breaks.
    let mut empty = doc.clone();
    empty.meta.kind = SourceKind::Poll {
        link: None,
        cache: None,
        gone: false,
    };
    assert!(Current::of(&empty, &now).rows.is_empty());
}

#[test]
fn sse_parser_handles_split_chunks_comments_and_multiline_data() {
    let mut p = Sse::default();
    let all = "event: snapshot\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\n:ping\n\nevent: tot";
    let mut evs = Vec::new();
    // Byte by byte, including inside a multi-byte character below.
    for b in all.as_bytes() {
        evs.extend(p.feed(&[*b]));
    }
    assert_eq!(
        evs,
        [SseEvent {
            event: "snapshot".into(),
            data: "{\"a\":\n1}".into()
        }]
    );
    let evs = p.feed("als\ndata: ø\n\n".as_bytes());
    assert_eq!(evs[0].event, "totals");
    assert_eq!(evs[0].data, "ø");
    let bytes = "data: æøå\n\n".as_bytes();
    let (a, b) = bytes.split_at(8); // splits "æ" (2 bytes) in the middle
    assert!(p.feed(a).is_empty());
    let evs = p.feed(b);
    assert_eq!(
        (evs[0].event.as_str(), evs[0].data.as_str()),
        ("message", "æøå")
    );
}

#[test]
fn input_rules_match_the_server() {
    let ok = PollInput {
        question: Some("Who wins?".into()),
        options: Some(vec!["Home".into(), "Away".into()]),
        ..PollInput::default()
    };
    assert!(check_input(&ok, 8).is_ok());
    let bad = |f: &dyn Fn(&mut PollInput)| {
        let mut i = ok.clone();
        f(&mut i);
        check_input(&i, 8).unwrap_err()
    };
    assert_eq!(bad(&|i| i.question = Some("  ".into())), "Add a question.");
    assert!(bad(&|i| i.question = Some("x".repeat(201))).contains("at most 200"));
    assert_eq!(
        bad(&|i| i.options = Some(vec!["Only".into()])),
        "A poll needs 2 to 8 options on your plan."
    );
    assert!(bad(&|i| i.options = Some(vec!["A".into(), "a ".into()])).contains("the same"));
    assert_eq!(
        bad(&|i| i.options = Some(vec!["A".into(), "".into()])),
        "Answer 2 is empty."
    );
    assert!(bad(&|i| i.options = Some(vec!["A".into(), "x".repeat(81)])).contains("at most 80"));
    assert!(bad(&|i| i.duration_seconds = Some(10)).contains("30 seconds"));
    assert!(bad(&|i| i.question = Some("a\nb".into())).contains("line breaks"));
}

#[test]
fn validation_of_poll_sources() {
    let link = Some(PollLink {
        id: PID.into(),
        code: CODE.into(),
    });
    validate(&link, &Some(poll(&[1, 2]))).unwrap();
    validate(&None, &None).unwrap();
    let bad = Some(PollLink {
        id: "../x".into(),
        code: CODE.into(),
    });
    assert!(validate(&bad, &None).is_err());
    assert!(is_code("K7P2QX") && !is_code("TEST00") && !is_code("K7P2Q0") && !is_code("k7p2qx"));
}

#[test]
fn reconnect_waits_double_up_to_the_cap() {
    let t = Timing::LIVE;
    let w: Vec<u64> = (1..=7).map(|f| reconnect_delay(&t, f).as_secs()).collect();
    assert_eq!(w, [1, 2, 4, 8, 16, 30, 30]);
}

// ---- Mock poll API (raw TCP) ------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Push {
    Event(String, Value),
    Raw(String),
    Drop,
}

#[derive(Default)]
struct MockState {
    /// (method, path) → replies (the last one repeats).
    replies: HashMap<(String, String), VecDeque<(u16, Value)>>,
    /// Statuses for the next stream connects (empty: 200).
    stream_status: VecDeque<u16>,
    calls: Vec<(String, String, Value)>,
    stream_connects: usize,
}

struct Mock {
    st: Arc<Mutex<MockState>>,
    push: broadcast::Sender<Push>,
}

async fn handle(
    mut sock: tokio::net::TcpStream,
    st: Arc<Mutex<MockState>>,
    push: broadcast::Sender<Push>,
) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let head_end = loop {
        let Ok(n) = sock.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let len: usize = lines
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse().ok())
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let Ok(n) = sock.read(&mut tmp).await else {
            return;
        };
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let body: Value = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
    if method == "GET" && path.ends_with("/events") {
        let (status, mut rx) = {
            let mut g = st.lock().unwrap();
            g.calls.push((method.clone(), path.clone(), body));
            g.stream_connects += 1;
            (g.stream_status.pop_front().unwrap_or(200), push.subscribe())
        };
        if status != 200 {
            let b = json!({"error": {"code": "service_unavailable", "message": "-"}}).to_string();
            let _ = sock
                .write_all(
                    format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
                        b.len()
                    )
                    .as_bytes(),
                )
                .await;
            return;
        }
        if sock
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            )
            .await
            .is_err()
        {
            return;
        }
        while let Ok(p) = rx.recv().await {
            let text = match p {
                Push::Event(e, v) => format!("event: {e}\ndata: {v}\n\n"),
                Push::Raw(s) => s,
                Push::Drop => return,
            };
            if sock.write_all(text.as_bytes()).await.is_err() {
                return;
            }
        }
        return;
    }
    let reply = {
        let mut g = st.lock().unwrap();
        g.calls.push((method.clone(), path.clone(), body));
        match g.replies.get_mut(&(method.clone(), path.clone())) {
            Some(q) if q.len() > 1 => q.pop_front(),
            Some(q) => q.front().cloned(),
            None => None,
        }
    };
    let (status, v) = reply.unwrap_or((
        404,
        json!({"error": {"code": "not_found", "message": "No such poll."}}),
    ));
    let b = if status == 204 {
        String::new()
    } else {
        v.to_string()
    };
    let _ = sock
        .write_all(
            format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
                b.len()
            )
            .as_bytes(),
        )
        .await;
}

#[derive(Default)]
struct Rec(Mutex<Vec<(String, Value)>>);

impl EventSink for Rec {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    rt: tokio::runtime::Runtime,
    st: AppState,
    mock: Mock,
    rec: Arc<Rec>,
    secrets: Arc<MemorySecrets>,
}

fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Rec::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (push, _) = broadcast::channel(64);
    let mock = Mock {
        st: Arc::default(),
        push: push.clone(),
    };
    let ms = mock.st.clone();
    rt.spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(handle(sock, ms.clone(), push.clone()));
        }
    });
    let secrets = Arc::new(MemorySecrets::default());
    st.cloud_service()
        .set_test_env(&base, secrets.clone(), MACHINE);
    st.cloud_init();
    st.poll_service().set_test_timing(Some(Timing {
        reconnect: Duration::from_millis(30),
        reconnect_cap: Duration::from_millis(200),
        fallback_every: Duration::from_millis(60),
        stream_retry: Duration::from_millis(400),
        read_timeout: Duration::from_secs(3),
    }));
    Env {
        _tmp: tmp,
        rt,
        st,
        mock,
        rec,
        secrets,
    }
}

impl Env {
    fn signed_in(&self) {
        self.secrets.set(S_SESSION, "kntr_s_test").unwrap();
    }

    fn pro(&self, on: bool) {
        let ent: &[&str] = if on {
            &["core.app", PRO_POLLS]
        } else {
            &["core.app"]
        };
        self.st.cloud_service().set_test_entitlements(ent, true);
    }

    fn reply(&self, method: &str, path: &str, replies: Vec<(u16, Value)>) {
        self.mock
            .st
            .lock()
            .unwrap()
            .replies
            .insert((method.into(), path.into()), replies.into());
    }

    fn calls(&self) -> Vec<(String, String, Value)> {
        self.mock.st.lock().unwrap().calls.clone()
    }

    fn push(&self, p: Push) {
        let _ = self.mock.push.send(p);
    }

    fn project(&self) {
        self.st.create_project(self._tmp.path(), "Show").unwrap();
    }

    fn cache(&self, source: &str) -> Option<PollData> {
        match self.st.sources().doc(source)?.meta.kind {
            SourceKind::Poll { cache, .. } => cache,
            _ => None,
        }
    }

    fn events(&self, name: &str) -> Vec<Value> {
        self.rec
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

fn wire(votes: &[u64], status: &str) -> Value {
    let labels = ["Home", "Away", "Draw"];
    let options: Vec<Value> = votes
        .iter()
        .enumerate()
        .map(|(i, v)| json!({"position": i, "label": labels[i % 3], "votes": v}))
        .collect();
    json!({
        "id": PID, "code": CODE, "url": format!("https://poll.kinetra.live/{CODE}"),
        "question": "Who wins tonight?",
        "options": options,
        "status": status, "show_results": true, "duration_seconds": null,
        "total_votes": votes.iter().sum::<u64>(), "max_votes": 2000, "vote_limit_reached": false, "round": 1,
        "created_at": "2026-10-12T10:00:00Z", "updated_at": "2026-10-12T10:00:00Z",
        "opened_at": null, "closes_at": null, "closed_at": null,
        "delete_after": "2026-10-19T10:00:00Z", "expires_at": "2026-10-19T10:00:00Z", "future_field": 1
    })
}

fn wait_until(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn api_code(e: &AppError) -> Option<&str> {
    match e {
        AppError::Cloud { api_code, .. } => Some(api_code),
        _ => None,
    }
}

// ---- Access ---------------------------------------------------------------------------------

#[test]
fn free_users_make_no_poll_calls() {
    let e = env();
    e.signed_in();
    e.pro(false);
    assert_eq!(e.rt.block_on(e.st.polls_quota()).unwrap(), None);
    assert!(e.rt.block_on(e.st.list_polls(false)).unwrap().is_empty());
    let input = PollInput {
        question: Some("Q?".into()),
        options: Some(vec!["A".into(), "B".into()]),
        ..PollInput::default()
    };
    assert!(matches!(
        e.rt.block_on(e.st.create_poll(input)),
        Err(AppError::ProRequired)
    ));
    assert!(matches!(
        e.rt.block_on(e.st.open_poll(PID)),
        Err(AppError::ProRequired)
    ));
    assert!(matches!(
        e.rt.block_on(e.st.close_poll(PID)),
        Err(AppError::ProRequired)
    ));
    assert!(matches!(
        e.rt.block_on(e.st.delete_poll(PID)),
        Err(AppError::ProRequired)
    ));
    e.st.watch_poll(PID, true).unwrap();
    assert_eq!(
        e.st.poll_service().task_count(),
        0,
        "no stream for a poll we may not call"
    );
    assert!(e.calls().is_empty(), "not one request: {:?}", e.calls());
}

#[test]
fn pro_creates_and_reads_the_quota() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.reply(
        "GET",
        "/v1/polls/quota",
        vec![(200, json!({"plan": "pro",
            "limits": {"open_polls": 3, "polls_per_month": 30, "votes_per_poll": 2000, "options_per_poll": 8,
                       "max_duration_seconds": 86400, "lifetime_seconds": 604800, "delete_after_close_seconds": 43200},
            "usage": {"open_polls": 1, "polls_this_month": 4, "polls": 6},
            "month_resets_at": "2026-11-01T00:00:00Z"}))],
    );
    e.reply("POST", "/v1/polls", vec![(201, wire(&[0, 0], "draft"))]);
    let q = e.rt.block_on(e.st.polls_quota()).unwrap().unwrap();
    assert_eq!(q.plan.as_deref(), Some("pro"));
    let l = q.limits.unwrap();
    assert_eq!(
        (l.open_polls, l.polls_per_month, l.votes_per_poll),
        (3, Some(30), 2000)
    );
    assert_eq!((q.usage.open_polls, q.usage.polls_this_month), (1, 4));
    // Unlimited months come as null.
    let v = serde_json::to_value(PollQuota {
        plan: Some("enterprise".into()),
        plan_name: Some("Kinetra Enterprise".into()),
        limits: Some(crate::core::model::PollLimits {
            polls_per_month: None,
            ..l
        }),
        usage: q.usage.clone(),
        month_resets_at: None,
    })
    .unwrap();
    assert!(
        v["limits"]["pollsPerMonth"].is_null(),
        "camelCase to the UI"
    );

    let p =
        e.rt.block_on(e.st.create_poll(PollInput {
            question: Some(" Who wins tonight? ".into()),
            options: Some(vec!["Home".into(), " Away".into()]),
            duration_seconds: Some(120),
            ..PollInput::default()
        }))
        .unwrap();
    assert_eq!((p.id.as_str(), p.status), (PID, PollStatus::Draft));
    let (_, _, body) = e.calls().into_iter().find(|c| c.0 == "POST").unwrap();
    assert_eq!(
        body,
        json!({"question": "Who wins tonight?", "options": ["Home", "Away"], "duration_seconds": 120})
    );
    // Remembered on this machine (polls.json), so it stays manageable after Pro.
    assert!(e.st.knows_poll(PID));
    wait_until("polls.json", || {
        e.st.config_dir().join("polls.json").exists()
    });
    // Reduced access (Pro ending, existing work only): nothing new starts.
    e.st.cloud_service()
        .set_test_entitlements(&["core.app", PRO_POLLS], false);
    assert!(matches!(
        e.rt.block_on(e.st.open_poll(PID)),
        Err(AppError::ProLimited)
    ));
}

#[test]
fn after_pro_ends_existing_polls_still_work() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.reply("POST", "/v1/polls", vec![(201, wire(&[0, 0], "draft"))]);
    let input = PollInput {
        question: Some("Who wins tonight?".into()),
        options: Some(vec!["Home".into(), "Away".into()]),
        ..PollInput::default()
    };
    e.rt.block_on(e.st.create_poll(input.clone())).unwrap();
    e.pro(false);
    let n = e.calls().len();
    // New work is refused before any request.
    assert!(matches!(
        e.rt.block_on(e.st.create_poll(input)),
        Err(AppError::ProRequired)
    ));
    assert!(matches!(
        e.rt.block_on(e.st.open_poll(PID)),
        Err(AppError::ProRequired)
    ));
    assert!(matches!(
        e.rt.block_on(e.st.update_poll(
            PID,
            PollInput {
                show_results: Some(false),
                ..PollInput::default()
            }
        )),
        Err(AppError::ProRequired)
    ));
    assert_eq!(e.calls().len(), n);
    // Close, reset, read and delete still work.
    e.reply(
        "POST",
        &format!("/v1/polls/{PID}/close"),
        vec![(200, wire(&[4, 2], "closed"))],
    );
    e.reply(
        "POST",
        &format!("/v1/polls/{PID}/reset"),
        vec![(200, wire(&[0, 0], "closed"))],
    );
    e.reply(
        "GET",
        &format!("/v1/polls/{PID}"),
        vec![(200, wire(&[0, 0], "closed"))],
    );
    e.reply(
        "DELETE",
        &format!("/v1/polls/{PID}"),
        vec![(204, Value::Null)],
    );
    assert_eq!(
        e.rt.block_on(e.st.close_poll(PID)).unwrap().status,
        PollStatus::Closed
    );
    assert_eq!(e.rt.block_on(e.st.reset_poll(PID)).unwrap().total_votes, 0);
    e.rt.block_on(e.st.get_poll(PID)).unwrap();
    // The quota and the list are still asked (the machine has polls to manage).
    e.reply(
        "GET",
        "/v1/polls",
        vec![(200, json!({"polls": [wire(&[0, 0], "closed")]}))],
    );
    assert_eq!(e.rt.block_on(e.st.list_polls(false)).unwrap().len(), 1);
    e.rt.block_on(e.st.delete_poll(PID)).unwrap();
    assert!(!e.st.knows_poll(PID), "a deleted poll is forgotten");
    // A poll this machine never knew stays locked.
    let other = "0199ffff-0000-7000-8000-000000000000";
    assert!(matches!(
        e.rt.block_on(e.st.close_poll(other)),
        Err(AppError::ProRequired)
    ));
}

#[test]
fn show_my_existing_polls_works_on_any_computer() {
    let e = env();
    e.signed_in();
    e.pro(false);
    e.reply(
        "GET",
        "/v1/polls",
        vec![(200, json!({"polls": [wire(&[3, 1], "open")]}))],
    );
    // On demand: one request even without Pro; the polls become manageable here.
    let list = e.rt.block_on(e.st.list_polls(true)).unwrap();
    assert_eq!((list.len(), list[0].status), (1, PollStatus::Open));
    assert!(e.st.knows_poll(PID));
    e.reply(
        "POST",
        &format!("/v1/polls/{PID}/close"),
        vec![(200, wire(&[3, 1], "closed"))],
    );
    e.rt.block_on(e.st.close_poll(PID)).unwrap();
}

#[test]
fn server_errors_reach_the_ui_with_their_details() {
    let e = env();
    e.signed_in();
    e.pro(true);
    e.reply("POST", "/v1/polls", vec![(201, wire(&[0, 0], "draft"))]);
    e.rt.block_on(e.st.create_poll(PollInput {
        question: Some("Q?".into()),
        options: Some(vec!["A".into(), "B".into()]),
        ..PollInput::default()
    }))
    .unwrap();
    e.reply(
        "POST",
        &format!("/v1/polls/{PID}/open"),
        vec![(409, json!({"error": {"code": "poll_limit_reached", "message": "3 polls are open.", "details": {"max": 3}}}))],
    );
    let err = e.rt.block_on(e.st.open_poll(PID)).unwrap_err();
    assert_eq!(api_code(&err), Some("poll_limit_reached"));
    let v = serde_json::to_value(&err).unwrap();
    assert_eq!(v["details"]["max"], 3);
    // 404: the poll is gone (forgotten).
    e.reply(
        "POST",
        &format!("/v1/polls/{PID}/close"),
        vec![(404, json!({"error": {"code": "not_found", "message": "-"}}))],
    );
    assert_eq!(
        api_code(&e.rt.block_on(e.st.close_poll(PID)).unwrap_err()),
        Some("not_found")
    );
    assert!(!e.st.knows_poll(PID));
}

// ---- Live results ---------------------------------------------------------------------------

/// A project with a poll source linked to the poll, the poll known (created here).
fn linked(e: &Env) -> String {
    e.signed_in();
    e.pro(true);
    e.project();
    e.reply("POST", "/v1/polls", vec![(201, wire(&[0, 0], "draft"))]);
    let p =
        e.rt.block_on(e.st.create_poll(PollInput {
            question: Some("Who wins tonight?".into()),
            options: Some(vec!["Home".into(), "Away".into()]),
            ..PollInput::default()
        }))
        .unwrap();
    e.st.create_poll_source("Poll", p).unwrap().id
}

#[test]
fn live_results_stream_and_reconnect_without_losing_data() {
    let e = env();
    let src = linked(&e);
    assert_eq!(e.st.poll_service().task_count(), 0, "not in use: no task");
    e.st.watch_poll(PID, true).unwrap();
    wait_until("connected", || {
        e.mock.st.lock().unwrap().stream_connects == 1
    });
    // The receiver subscribes on connect; give the handler a moment, then push.
    std::thread::sleep(Duration::from_millis(50));
    e.push(Push::Event("snapshot".into(), wire(&[1, 0], "open")));
    wait_until("live", || e.st.poll_service().conn(PID) == PollConn::Live);
    wait_until("snapshot applied", || {
        e.cache(&src).is_some_and(|c| c.total_votes == 1)
    });
    e.push(Push::Raw(":ping\n\n".into()));
    e.push(Push::Event("totals".into(), wire(&[5, 3], "open")));
    wait_until("totals applied", || {
        e.cache(&src).is_some_and(|c| c.total_votes == 8)
    });
    let live = e.events(names::POLL_LIVE);
    assert!(live.iter().any(|v| v["poll"]["totalVotes"] == 8));
    let summary = e.st.source_summary(&e.st.sources().doc(&src).unwrap());
    let view = summary.poll.unwrap();
    assert_eq!(
        (view.conn, view.state.as_deref(), view.total_votes),
        (PollConn::Live, Some("open"), 8)
    );
    // The stream drops: the source keeps its last results, the task reconnects.
    e.push(Push::Drop);
    wait_until("reconnected", || {
        e.mock.st.lock().unwrap().stream_connects == 2
    });
    assert_eq!(e.cache(&src).unwrap().total_votes, 8, "last results stay");
    std::thread::sleep(Duration::from_millis(50));
    e.push(Push::Event("snapshot".into(), wire(&[6, 3], "open")));
    wait_until("live again", || {
        e.cache(&src).is_some_and(|c| c.total_votes == 9)
    });
    assert_eq!(e.st.poll_service().conn(PID), PollConn::Live);
    // Not watched any more and not bound: the task stops.
    e.st.watch_poll(PID, false).unwrap();
    assert_eq!(e.st.poll_service().task_count(), 0);
    assert_eq!(e.st.poll_service().conn(PID), PollConn::Idle);
}

#[test]
fn falls_back_to_polling_when_the_stream_fails() {
    let e = env();
    let src = linked(&e);
    e.mock.st.lock().unwrap().stream_status = vec![503; 50].into();
    e.reply(
        "GET",
        &format!("/v1/polls/{PID}"),
        vec![(200, wire(&[2, 2], "open"))],
    );
    e.st.watch_poll(PID, true).unwrap();
    wait_until("polling", || {
        e.st.poll_service().conn(PID) == PollConn::Polling
    });
    assert!(e.mock.st.lock().unwrap().stream_connects >= FAILS_BEFORE_FALLBACK as usize);
    wait_until("polled data", || {
        e.cache(&src).is_some_and(|c| c.total_votes == 4)
    });
    // The stream comes back: after the retry interval it's live again.
    e.mock.st.lock().unwrap().stream_status.clear();
    let before = e.mock.st.lock().unwrap().stream_connects;
    wait_until("stream retried", || {
        e.mock.st.lock().unwrap().stream_connects > before
    });
    std::thread::sleep(Duration::from_millis(50));
    e.push(Push::Event("snapshot".into(), wire(&[3, 2], "open")));
    wait_until("live", || e.st.poll_service().conn(PID) == PollConn::Live);
    e.st.watch_poll(PID, false).unwrap();
}

#[test]
fn a_deleted_poll_keeps_its_last_results_on_screen() {
    let e = env();
    let src = linked(&e);
    e.st.watch_poll(PID, true).unwrap();
    wait_until("connected", || {
        e.mock.st.lock().unwrap().stream_connects == 1
    });
    std::thread::sleep(Duration::from_millis(50));
    e.push(Push::Event("snapshot".into(), wire(&[7, 2], "closed")));
    wait_until("applied", || {
        e.cache(&src).is_some_and(|c| c.total_votes == 9)
    });
    e.push(Push::Event("deleted".into(), json!({"id": PID})));
    wait_until("gone", || e.st.poll_service().conn(PID) == PollConn::Gone);
    let doc = e.st.sources().doc(&src).unwrap();
    assert!(
        matches!(doc.meta.kind, SourceKind::Poll { gone: true, cache: Some(ref c), .. } if c.total_votes == 9)
    );
    assert_eq!(e.st.poll_service().task_count(), 0);
    assert!(!e.st.knows_poll(PID));
    assert_eq!(e.st.source_summary(&doc).poll.unwrap().conn, PollConn::Gone);
}

#[test]
fn signed_out_streams_nothing() {
    let e = env();
    e.pro(true);
    e.project();
    e.st.watch_poll(PID, true).unwrap();
    // Pro but not signed in: no task, the status says why.
    assert_eq!(e.st.poll_service().task_count(), 0);
    assert!(e.calls().is_empty());
}

#[test]
fn add_to_screen_binds_the_poll_graphic_to_the_poll() {
    use crate::core::model::NewScreen;
    let e = env();
    e.project();
    let src = e.st.create_poll_source("Poll", poll(&[1, 3])).unwrap().id;
    let screen =
        e.st.create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc
        .id;
    let state = e.st.add_poll_to_screen(&src, &screen).unwrap();
    let layer = state.doc.meta.layers.last().unwrap().clone();
    assert_eq!(layer.name, "Poll");
    // The preset's design box, not the full screen.
    assert!(layer.rect.w < 1920 && layer.rect.h < 1080);
    let field = |key: &str| {
        e.st.live()
            .with_layer(&screen, &layer.id, |l| {
                l.fields.get(key).cloned().unwrap_or_default()
            })
            .unwrap()
    };
    assert_eq!(field("question"), "Who wins tonight?");
    assert_eq!(field("status"), "open");
    assert_eq!(field("total_votes"), "4");
    assert_eq!(field("short_url"), format!("poll.kinetra.live/{CODE}"));
    assert!(field("qr").starts_with("data:image/svg+xml,"));
    let rows: Vec<Value> = serde_json::from_str(&field("options")).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[1]["label"].as_str(), rows[1]["pct_round"].as_str()),
        (Some("Away"), Some("75"))
    );
    assert_eq!(rows[1]["_leader"].as_str(), Some("true"));
    // A second add reuses the project's Poll template.
    e.st.add_poll_to_screen(&src, &screen).unwrap();
    let templates = e.st.templates().list(&e.st.project_dir().unwrap()).unwrap();
    assert_eq!(
        templates
            .iter()
            .filter(|t| t.preset.as_ref().is_some_and(|p| p.id == "poll"))
            .count(),
        1
    );
    // New results reach the layer; a closed poll shows as closed.
    e.st.poll_apply(poll(&[0, 0]));
    assert_eq!(field("total_votes"), "0");
    e.st.poll_apply(PollData {
        status: PollStatus::Closed,
        ..poll(&[2, 5])
    });
    assert_eq!(field("status"), "closed");
    // Not a poll source: refused.
    let text = e.st.create_source("Plain").unwrap();
    assert!(matches!(
        e.st.add_poll_to_screen(&text.id, &screen),
        Err(AppError::Invalid(_))
    ));
}

/// Budget: parsing a stream event and building the rows < 0.2 ms. Measured on
/// an 8-answer poll: SSE framing, the JSON, the mapping and the rows (QR included), per event.
#[test]
fn an_event_parses_into_rows_within_budget() {
    let votes = [412, 388, 301, 255, 190, 120, 77, 31];
    let frame = format!("event: totals\ndata: {}\n\n", wire(&votes, "open"));
    let mut sse = Sse::default();
    let run = |sse: &mut Sse| {
        for ev in sse.feed(frame.as_bytes()) {
            let w: PollWire = serde_json::from_str(&ev.data).expect("event");
            let rows = rows(&PollData::from(w));
            assert_eq!(rows.len(), 8);
        }
    };
    run(&mut sse); // warm the QR cache, as a running poll has
    let n = 1000;
    let t = Instant::now();
    for _ in 0..n {
        run(&mut sse);
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
    println!("poll event → rows: {per:.4} ms per event (8 answers)");
    // The budget is for release builds; debug ones (~0.1 ms here) get room for a busy CI runner.
    let limit = if cfg!(debug_assertions) { 1.0 } else { 0.2 };
    assert!(per < limit, "{per:.4} ms per event");
}
