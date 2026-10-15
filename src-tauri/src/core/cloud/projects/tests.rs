//! The cloud projects client against a local mock of `/v1/cloud` and storage (no external
//! network). Slice 1: request shapes and headers, error envelopes, storage requests, and that
//! nothing secret reaches the log.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use serde_json::{json, Value};

use super::r2::{R2Error, R2};
use super::wire::{
    BlobRef, IfTrashed, Manifest, NewVersion, Presigned, QuotaState, Reason, Upload,
};
use crate::core::cloud::api::{Api, CallError};

pub(crate) const SESSION: &str = "kntr_s_test-session";
pub(crate) const PID: &str = "0199e2a0-0000-7000-8000-000000000001";

// ---- Mock server ------------------------------------------------------------------------------

/// A scripted reply: status, JSON body (or none), extra headers.
#[derive(Clone)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: Option<Value>,
    pub headers: Vec<(&'static str, String)>,
}

pub(crate) fn ok(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: Some(body),
        headers: Vec::new(),
    }
}

pub(crate) fn err(status: u16, code: &str, details: Option<Value>) -> Reply {
    let mut e = json!({"error": {"code": code, "message": format!("msg {code}")}});
    if let Some(d) = details {
        e["error"]["details"] = d;
    }
    ok(status, e)
}

/// One request as the mock saw it.
#[derive(Clone, Debug)]
pub(crate) struct Call {
    pub method: String,
    /// Path and query.
    pub target: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Call {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

#[derive(Default)]
pub(crate) struct MockState {
    /// "METHOD /path" → replies (the last one repeats).
    replies: HashMap<String, VecDeque<Reply>>,
    pub calls: Vec<Call>,
}

#[derive(Clone)]
pub(crate) struct Mock {
    pub st: Arc<Mutex<MockState>>,
    pub base: String,
}

impl Mock {
    /// Starts the mock on a free port of `rt`.
    pub fn start(rt: &tokio::runtime::Runtime) -> Self {
        let st: Arc<Mutex<MockState>> = Arc::default();
        let app = axum::Router::new().fallback(handle).with_state(st.clone());
        let listener = rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        rt.spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            st,
            base: format!("http://{addr}"),
        }
    }

    /// Scripts replies for "METHOD /path" (path without the query).
    pub fn on(&self, key: &str, replies: Vec<Reply>) {
        self.st
            .lock()
            .unwrap()
            .replies
            .insert(key.to_string(), replies.into());
    }

    pub fn calls(&self) -> Vec<Call> {
        self.st.lock().unwrap().calls.clone()
    }

    pub fn calls_to(&self, key: &str) -> Vec<Call> {
        let (m, p) = key.split_once(' ').unwrap();
        self.calls()
            .into_iter()
            .filter(|c| c.method == m && c.target.split('?').next() == Some(p))
            .collect()
    }

    /// A presigned request to this mock's storage path `/r2/<name>?X-Amz-Signature=…`.
    pub fn presigned(&self, name: &str, headers: &[(&str, &str)]) -> Presigned {
        Presigned {
            url: format!(
                "{}/r2/{name}?X-Amz-Signature=s3cr3t-sig&X-Amz-Credential=c",
                self.base
            ),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            expires_at: None,
        }
    }
}

async fn handle(
    State(st): State<Arc<Mutex<MockState>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let target = uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let key = format!("{method} {}", uri.path());
    let reply = {
        let mut s = st.lock().unwrap();
        s.calls.push(Call {
            method: method.to_string(),
            target,
            headers: headers
                .iter()
                .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
                .collect(),
            body: body.to_vec(),
        });
        match s.replies.get_mut(&key) {
            Some(q) if q.len() > 1 => q.pop_front(),
            Some(q) => q.front().cloned(),
            None => None,
        }
    };
    let Some(r) = reply else {
        return (StatusCode::NOT_FOUND, "no reply scripted").into_response();
    };
    let mut resp = match r.body {
        Some(b) => (StatusCode::from_u16(r.status).unwrap(), axum::Json(b)).into_response(),
        None => StatusCode::from_u16(r.status).unwrap().into_response(),
    };
    for (k, v) in r.headers {
        resp.headers_mut().insert(k, v.parse().unwrap());
    }
    resp
}

pub(crate) fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

pub(crate) fn quota_json(state: &str) -> Value {
    json!({
        "plan": "pro_monthly", "state": state, "used_bytes": 7340032000u64, "pending_bytes": 0,
        "limit_bytes": 20000000000u64, "percent": 36, "warn_percent": 80,
        "projects": {"used": 12, "limit": 100}, "max_file_bytes": 5000000000u64,
        "retention_days": 30, "kept_per_project": 50,
        "upload_today": {"used_bytes": 1200000000u64, "limit_bytes": 40000000000u64,
                         "resets_at": "2026-10-15T00:00:00Z"},
        "lapse": null, "retention_change": null,
        "client": {"backup_interval_s": 300, "on_air": "pause", "on_air_limit_kbps": 2000,
                   "multipart_threshold_bytes": 16777216, "part_size_bytes": 16777216},
        "added_later": {"ignored": true}
    })
}

pub(crate) fn project_json(app_id: &str, name: &str) -> Value {
    json!({"id": PID, "app_project_id": app_id, "name": name,
           "created_at": "2026-10-15T10:00:00Z", "updated_at": "2026-10-15T10:00:00Z",
           "head": null, "versions": 0, "kept": 0, "conflict": false,
           "trashed_at": null, "purge_at": null})
}

// ---- Slice 1: the client ---------------------------------------------------------------------

#[test]
fn quota_parses_and_requests_carry_the_session_and_user_agent() {
    let rt = rt();
    let m = Mock::start(&rt);
    m.on("GET /v1/cloud/quota", vec![ok(200, quota_json("warning"))]);
    let api = Api::new(m.base.clone()).unwrap();
    let q = rt.block_on(api.cloud_quota(SESSION)).unwrap();
    assert_eq!(q.state, QuotaState::Warning);
    assert_eq!(q.limit_bytes, 20_000_000_000);
    assert_eq!(q.client.backup_interval_s, 300);
    assert_eq!(
        q.upload_today.resets_at.as_deref(),
        Some("2026-10-15T00:00:00Z")
    );
    let c = &m.calls_to("GET /v1/cloud/quota")[0];
    assert_eq!(
        c.headers.get("authorization").map(String::as_str),
        Some("Bearer kntr_s_test-session")
    );
    let ua = &c.headers["user-agent"];
    assert!(
        ua.starts_with(&format!("Kinetra/{} (", env!("CARGO_PKG_VERSION"))) && ua.ends_with(')'),
        "{ua}"
    );
}

#[test]
fn a_quota_with_missing_or_unknown_values_falls_back_safely() {
    let rt = rt();
    let m = Mock::start(&rt);
    m.on(
        "GET /v1/cloud/quota",
        vec![ok(
            200,
            json!({"plan": null, "state": "not_in_plan",
                            "client": {"on_air": "something-new"}}),
        )],
    );
    let api = Api::new(m.base.clone()).unwrap();
    let q = rt.block_on(api.cloud_quota(SESSION)).unwrap();
    assert_eq!(q.state, QuotaState::NotInPlan);
    assert_eq!(q.plan, None);
    // An unknown on-air policy is the safe one; missing numbers are the contract's defaults.
    assert_eq!(q.client.on_air, super::wire::OnAir::Pause);
    assert_eq!(q.client.on_air_limit_kbps, 2000);
    assert_eq!(q.client.backup_interval_s, 300);
}

#[test]
fn register_tells_new_from_existing_and_errors_keep_their_details() {
    let rt = rt();
    let m = Mock::start(&rt);
    let app_id = "0199e2a0-1111-7000-8000-000000000002";
    m.on(
        "POST /v1/cloud/projects",
        vec![
            ok(201, project_json(app_id, "Election night")),
            ok(200, project_json(app_id, "Election night")),
            err(
                409,
                "project_in_trash",
                Some(json!({"project_id": PID, "purge_at": "2026-10-22T10:00:00Z"})),
            ),
        ],
    );
    let api = Api::new(m.base.clone()).unwrap();
    let r = rt
        .block_on(api.cloud_register(SESSION, app_id, "Election night", IfTrashed::Error))
        .unwrap();
    assert!(r.created);
    assert_eq!(r.project.id, PID);
    let r = rt
        .block_on(api.cloud_register(SESSION, app_id, "Election night", IfTrashed::Restore))
        .unwrap();
    assert!(!r.created);
    let e = rt
        .block_on(api.cloud_register(SESSION, app_id, "Election night", IfTrashed::Error))
        .unwrap_err();
    match e {
        CallError::Api {
            status,
            code,
            details,
            ..
        } => {
            assert_eq!((status, code.as_str()), (409, "project_in_trash"));
            assert_eq!(details.unwrap()["project_id"], PID);
        }
        CallError::Offline => panic!("offline"),
    }
    let bodies: Vec<Value> = m
        .calls_to("POST /v1/cloud/projects")
        .iter()
        .map(|c| c.json())
        .collect();
    assert_eq!(
        bodies[0],
        json!({"app_project_id": app_id, "name": "Election night", "if_trashed": "error"})
    );
    assert_eq!(bodies[1]["if_trashed"], "restore");
}

#[test]
fn unavailable_and_unparsable_answers_are_offline() {
    let rt = rt();
    let m = Mock::start(&rt);
    m.on(
        "GET /v1/cloud/quota",
        vec![
            err(503, "cloud_unavailable", None),
            Reply {
                status: 502,
                body: None,
                headers: Vec::new(),
            },
        ],
    );
    let api = Api::new(m.base.clone()).unwrap();
    let e = rt.block_on(api.cloud_quota(SESSION)).unwrap_err();
    assert!(e.is_offline(), "{e:?}");
    let e = rt.block_on(api.cloud_quota(SESSION)).unwrap_err();
    assert_eq!(e, CallError::Offline);
    // No server at all.
    let gone = Api::new("http://127.0.0.1:9".into()).unwrap();
    assert_eq!(
        rt.block_on(gone.cloud_quota(SESSION)).unwrap_err(),
        CallError::Offline
    );
}

#[test]
fn commits_send_an_idempotency_key_and_ids_must_be_uuids() {
    let rt = rt();
    let m = Mock::start(&rt);
    let version = json!({"id": "0199e2a0-2222-7000-8000-000000000003", "project_id": PID,
        "created_at": "2026-10-15T10:00:00Z", "reason": "manual", "machine_name": "STUDIO",
        "app_version": "0.1.0", "is_head": true, "file_count": 1, "logical_bytes": 5,
        "new_bytes": 5, "conflict": null, "expires_at": null});
    m.on(
        "POST /v1/cloud/projects/0199e2a0-0000-7000-8000-000000000001/versions",
        vec![ok(201, json!({"version": version, "conflict": null}))],
    );
    let api = Api::new(m.base.clone()).unwrap();
    let nv = NewVersion {
        base_version_id: None,
        restored_from: None,
        reason: Reason::Manual,
        label: None,
        activation_id: "act-1".into(),
        app_version: "0.1.0".into(),
        manifest: Manifest {
            format: 1,
            files: Vec::new(),
        },
    };
    let c = rt
        .block_on(api.cloud_commit(SESSION, PID, "key-1", &nv))
        .unwrap();
    assert!(c.version.is_head && c.conflict.is_none());
    let call = &m.calls()[0];
    assert_eq!(call.headers["idempotency-key"], "key-1");
    assert_eq!(call.json()["manifest"], json!({"format": 1, "files": []}));
    assert_eq!(call.json()["base_version_id"], Value::Null);
    // A server id that isn't a UUID never reaches a path.
    let e = rt
        .block_on(api.cloud_project(SESSION, "../../v1/license"))
        .unwrap_err();
    assert!(matches!(e, CallError::Api { status: 400, .. }));
    assert_eq!(m.calls().len(), 1);
    assert!(super::api::is_id(PID) && !super::api::is_id("0199e2a0-0000-7000-8000-00000000000g"));
}

#[test]
fn blob_check_reads_both_upload_kinds() {
    let rt = rt();
    let m = Mock::start(&rt);
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    m.on(
        "POST /v1/cloud/blobs/check",
        vec![ok(
            200,
            json!({"missing": [
            {"sha256": a, "size": 482113, "upload": {"kind": "put",
                "url": "https://x.r2.example/a?X-Amz-Signature=1",
                "headers": {"content-length": "482113", "x-amz-checksum-sha256": "q83v"},
                "expires_at": "2026-10-15T10:10:00Z"}},
            {"sha256": b, "size": 2147483648u64, "upload": {"kind": "multipart",
                "upload_id": "0199e2a0-3333-7000-8000-000000000004", "part_size": 16777216,
                "part_count": 128}}
        ], "reserved_until": "2026-10-16T10:00:00Z"}),
        )],
    );
    let api = Api::new(m.base.clone()).unwrap();
    let blobs = [
        BlobRef {
            sha256: a.clone(),
            size: 482113,
        },
        BlobRef {
            sha256: b.clone(),
            size: 2147483648,
        },
    ];
    let c = rt.block_on(api.cloud_check(SESSION, &blobs)).unwrap();
    match &c.missing[0].upload {
        Upload::Put { put } => {
            assert_eq!(put.headers["content-length"], "482113");
            assert_eq!(put.expires_at.as_deref(), Some("2026-10-15T10:10:00Z"));
        }
        Upload::Multipart { .. } => panic!("put expected"),
    }
    assert_eq!(
        c.missing[1].upload,
        Upload::Multipart {
            upload_id: "0199e2a0-3333-7000-8000-000000000004".into(),
            part_size: 16777216,
            part_count: 128
        }
    );
    assert_eq!(
        m.calls()[0].json()["blobs"][1],
        json!({"sha256": b, "size": 2147483648u64})
    );
}

// ---- Storage ---------------------------------------------------------------------------------

#[test]
fn storage_gets_exactly_the_given_headers_and_never_the_session() {
    let rt = rt();
    let m = Mock::start(&rt);
    m.on(
        "PUT /r2/blob",
        vec![Reply {
            status: 200,
            body: None,
            headers: vec![("etag", "\"etag-1\"".into())],
        }],
    );
    let r2 = R2::new().unwrap();
    let p = m.presigned(
        "blob",
        &[("content-length", "5"), ("x-amz-checksum-sha256", "abc=")],
    );
    let etag = rt
        .block_on(r2.put(&p, reqwest::Body::from(b"hello".to_vec())))
        .unwrap();
    assert_eq!(etag.as_deref(), Some("\"etag-1\""));
    let c = &m.calls()[0];
    assert_eq!(c.body, b"hello");
    assert_eq!(c.headers["x-amz-checksum-sha256"], "abc=");
    assert_eq!(c.headers["content-length"], "5");
    assert!(!c.headers.contains_key("authorization"));
    assert!(!c.headers.contains_key("accept-encoding"));
    // The URL arrives exactly as given (signature included).
    assert_eq!(
        c.target,
        "/r2/blob?X-Amz-Signature=s3cr3t-sig&X-Amz-Credential=c"
    );
}

#[test]
fn storage_refusals_map_to_what_the_app_does_next() {
    let rt = rt();
    let m = Mock::start(&rt);
    let xml = |code: &str| Reply {
        status: 400,
        body: Some(Value::String(format!("<Error><Code>{code}</Code></Error>"))),
        headers: Vec::new(),
    };
    m.on("PUT /r2/expired", vec![err(403, "AccessDenied", None)]);
    m.on("PUT /r2/changed", vec![xml("BadDigest")]);
    m.on("GET /r2/down", vec![err(503, "SlowDown", None)]);
    let r2 = R2::new().unwrap();
    let put = |name: &str| {
        rt.block_on(r2.put(&m.presigned(name, &[]), reqwest::Body::from(Vec::new())))
            .unwrap_err()
    };
    assert_eq!(put("expired"), R2Error::Expired);
    assert_eq!(put("changed"), R2Error::BadDigest);
    assert_eq!(
        rt.block_on(r2.get(&m.presigned("down", &[]))).unwrap_err(),
        R2Error::Offline
    );
}

// ---- Logs ------------------------------------------------------------------------------------

/// A `MakeWriter` that keeps everything written.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Captured;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn no_url_signature_path_or_hash_reaches_the_log() {
    let rt = rt();
    let m = Mock::start(&rt);
    let hash = "c0ffee".repeat(10) + "abcd";
    m.on("PUT /r2/blob", vec![err(403, "AccessDenied", None)]);
    m.on(
        "POST /v1/cloud/blobs/check",
        vec![err(403, "storage_full", Some(json!({"quota": {}})))],
    );
    let log = Captured::default();
    let sub = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    tracing::subscriber::with_default(sub, || {
        let r2 = R2::new().unwrap();
        let p = m.presigned("blob", &[]);
        let _ = rt.block_on(r2.put(&p, reqwest::Body::from(b"x".to_vec())));
        let gone = Presigned {
            url: format!("http://127.0.0.1:9/bucket/{hash}?X-Amz-Signature=s3cr3t-sig"),
            headers: Default::default(),
            expires_at: None,
        };
        let _ = rt.block_on(r2.get(&gone));
        let api = Api::new(m.base.clone()).unwrap();
        let _ = rt.block_on(api.cloud_check(
            SESSION,
            &[BlobRef {
                sha256: hash.clone(),
                size: 1,
            }],
        ));
    });
    let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(
        text.contains("storage PUT"),
        "the requests were logged: {text}"
    );
    for secret in [
        "s3cr3t-sig",
        "X-Amz",
        hash.as_str(),
        "/r2/blob",
        "/bucket/",
        SESSION,
    ] {
        assert!(!text.contains(secret), "{secret} in the log:\n{text}");
    }
}
