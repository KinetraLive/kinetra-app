//! Backups end to end against a stateful fake of `/v1/cloud` and storage: it checks each single
//! upload's SHA-256 and each part's MD5 like R2, assembles multipart uploads, refuses commits
//! with blobs it doesn't have (`missing_blobs`) and marks conflicts like the server. Covers the
//! handoff's acceptance items that don't need the real dev API: nothing is sent twice, only a
//! changed file goes up, multipart resumes after a stop, nothing is sent while on air (and a
//! transfer in flight stops), a file that changes mid-upload, expired URLs, a full account,
//! names the server refuses, and `missing_blobs`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use md5::Md5;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::scan::hex;
use super::tests_service::{wait_until, ACCOUNT, MACHINE};
use crate::core::cloud::service::S_SESSION;
use crate::core::model::{CloudPhase, CloudView};
use crate::core::secrets::{MemorySecrets, SecretStore};
use crate::core::state::AppState;

const THRESHOLD: u64 = 64 * 1024;
const PART: u64 = 32 * 1024;

// ---- The fake ------------------------------------------------------------------------------------

#[derive(Default)]
struct Multi {
    sha: String,
    size: u64,
    part_count: u32,
    parts: BTreeMap<u32, Vec<u8>>,
}

#[derive(Default)]
pub(super) struct Fake {
    base: String,
    pub(super) blobs: HashMap<String, Vec<u8>>,
    uploads: HashMap<String, Multi>,
    next_id: u64,
    projects: HashMap<String, String>,
    pub(super) head: Option<String>,
    pub(super) versions: Vec<Value>,
    commits_by_key: HashMap<String, Value>,
    /// Requests: "PUT blob <sha>", "PUT part <n>", "COMMIT", "CHECK".
    pub(super) log: Vec<String>,
    /// When each single PUT was stored, with its size.
    put_times: Vec<(Instant, usize)>,
    // Knobs.
    expire_next_puts: u32,
    storage_full: bool,
    /// Checks refused with `daily_upload_limit` (when anything is missing).
    daily_limit: bool,
    force_missing_once: bool,
    refuse_path: Option<String>,
    put_delay: Duration,
    /// Called (outside the lock) after a part is stored: number.
    on_part: Option<Arc<dyn Fn(u32) + Send + Sync>>,
    /// Called before a single PUT's body is read.
    on_put: Option<Arc<dyn Fn() + Send + Sync>>,
    pub(super) quota_state: String,
    /// Commits stored but answered with 502 (the answer is lost), this many times.
    lose_commit_answers: u32,
    /// Commits refused with 429.
    commits_429: bool,
    /// Commits answered `503 commit_incomplete` (still confirming uploads), this many times.
    incomplete_commits: u32,
    /// The `Idempotency-Key` of every commit request, in order.
    commit_keys: Vec<String>,
    /// PATCHes seen (version id → body).
    patches: Vec<(String, Value)>,
    /// The `reason` of every version committed.
    reasons: Vec<String>,
    /// The project as the server holds it: name, in the trash.
    proj_name: String,
    trashed: bool,
    /// Kept versions allowed per project.
    kept_limit: usize,
    /// Downloads of a blob served with one byte wrong, this many times (restore).
    pub(super) corrupt_gets: HashMap<String, u32>,
    /// Each download waits this long before its body (restore on air).
    pub(super) get_delay: Duration,
    /// The `restored_from` of every commit.
    pub(super) restored: Vec<Value>,
}

pub(super) type F = Arc<Mutex<Fake>>;

fn uuid(n: u64) -> String {
    format!("0199e2a0-{:04x}-7000-8000-{:012x}", n % 0xffff, n)
}

fn err(status: u16, code: &str, details: Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap(),
        axum::Json(json!({"error": {"code": code, "message": code, "details": details}})),
    )
        .into_response()
}

fn ok(status: u16, v: Value) -> Response {
    (StatusCode::from_u16(status).unwrap(), axum::Json(v)).into_response()
}

fn version(f: &mut Fake, base: Option<String>, files: &[Value], new_bytes: u64) -> Value {
    f.next_id += 1;
    let id = uuid(1000 + f.next_id);
    let conflict = match (&base, &f.head) {
        (b, Some(h)) if b.as_deref() != Some(h.as_str()) => Some(h.clone()),
        _ => None,
    };
    let v = json!({"id": id, "project_id": uuid(1), "created_at": "2026-10-15T14:02:00Z",
        "base_version_id": base, "reason": "manual", "machine_name": "STUDIO-PC (b)",
        "app_version": "0.1.0", "is_head": true, "file_count": files.len(),
        "logical_bytes": 0, "new_bytes": new_bytes,
        "conflict": conflict.as_ref().map(|c| json!({"with_version_id": c, "resolved_at": null}))});
    let with = conflict.and_then(|c| f.versions.iter().find(|v| v["id"] == c.as_str()).cloned());
    f.head = Some(id);
    let mut stored = v.clone();
    stored["manifest"] = json!(files);
    f.versions.push(stored);
    json!({"version": v, "conflict": with.map(|w| json!({"with_version": w}))})
}

fn fake_project(f: &Fake, app: &str, id: &str) -> Value {
    let head = f.versions.last().map(|v| {
        let mut v = v.clone();
        v.as_object_mut().unwrap().remove("manifest");
        v
    });
    json!({"id": id, "app_project_id": app, "name": f.proj_name, "head": head,
        "versions": f.versions.len(),
        "kept": f.versions.iter().filter(|v| v["kept"] == true).count(),
        "conflict": f.versions.iter().any(|v| v["conflict"].is_object() && v["conflict"]["resolved_at"].is_null()),
        "trashed_at": if f.trashed { json!("2026-10-20T10:00:00Z") } else { Value::Null },
        "purge_at": if f.trashed { json!("2026-10-27T10:00:00Z") } else { Value::Null }})
}

async fn handle(
    State(f): State<F>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_string();
    let seg: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let h = |k: &str| {
        headers
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let json_body = || serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null);
    match (method.as_str(), seg.as_slice()) {
        ("GET", ["v1", "me"]) => ok(200, json!({"id": ACCOUNT, "email": "a@b.c"})),
        ("GET", ["v1", "cloud", "quota"]) => {
            let mut f = f.lock().unwrap();
            f.log.push("QUOTA".into());
            let used: u64 = f.blobs.values().map(|b| b.len() as u64).sum();
            ok(
                200,
                json!({"plan": "pro_monthly", "state": f.quota_state, "used_bytes": used,
                "limit_bytes": 200_000_000u64, "percent": 0, "warn_percent": 80,
                "projects": {"used": f.projects.len(), "limit": 100},
                "max_file_bytes": 5_000_000_000u64, "retention_days": 30, "kept_per_project": 50,
                "upload_today": {"used_bytes": 0, "limit_bytes": 300_000_000u64,
                                 "resets_at": "2099-01-01T00:00:00Z"},
                "client": {"backup_interval_s": 300, "on_air": "pause", "on_air_limit_kbps": 2000,
                           "multipart_threshold_bytes": THRESHOLD, "part_size_bytes": PART}}),
            )
        }
        ("POST", ["v1", "cloud", "projects"]) => {
            let b = json_body();
            let app = b["app_project_id"].as_str().unwrap_or("").to_string();
            let mut f = f.lock().unwrap();
            let created = !f.projects.contains_key(&app);
            f.proj_name = b["name"].as_str().unwrap_or("").to_string();
            let id = f
                .projects
                .entry(app.clone())
                .or_insert_with(|| uuid(1))
                .clone();
            ok(
                if created { 201 } else { 200 },
                json!({"id": id, "app_project_id": app,
                "name": b["name"], "head": null, "versions": 0, "kept": 0, "conflict": false}),
            )
        }
        ("POST", ["v1", "cloud", "blobs", "check"]) => {
            let mut f = f.lock().unwrap();
            f.log.push("CHECK".into());
            let blobs = json_body()["blobs"].as_array().cloned().unwrap_or_default();
            let mut missing = Vec::new();
            for b in blobs {
                let sha = b["sha256"].as_str().unwrap_or("").to_string();
                let size = b["size"].as_u64().unwrap_or(0);
                if f.blobs.contains_key(&sha) {
                    continue;
                }
                if f.storage_full {
                    return err(403, "storage_full", json!({"quota": {}}));
                }
                if f.daily_limit {
                    return err(403, "daily_upload_limit", json!({"quota": {}}));
                }
                let upload = if size <= THRESHOLD {
                    json!({"kind": "put", "url": format!("{}/r2/blob/{sha}?X-Amz-Signature=sig", f.base),
                           "headers": {"content-length": size.to_string(), "x-amz-checksum-sha256": sha},
                           "expires_at": "2026-10-15T14:12:00Z"})
                } else {
                    let existing = f
                        .uploads
                        .iter()
                        .find(|(_, u)| u.sha == sha)
                        .map(|(k, _)| k.clone());
                    let id = existing.unwrap_or_else(|| {
                        f.next_id += 1;
                        let id = uuid(5000 + f.next_id);
                        f.uploads.insert(
                            id.clone(),
                            Multi {
                                sha: sha.clone(),
                                size,
                                part_count: size.div_ceil(PART) as u32,
                                parts: BTreeMap::new(),
                            },
                        );
                        id
                    });
                    json!({"kind": "multipart", "upload_id": id, "part_size": PART,
                           "part_count": size.div_ceil(PART)})
                };
                missing.push(json!({"sha256": sha, "size": size, "upload": upload}));
            }
            ok(
                200,
                json!({"missing": missing, "reserved_until": "2026-10-16T14:02:00Z"}),
            )
        }
        ("POST", ["v1", "cloud", "uploads", id, "parts"]) => {
            let f = f.lock().unwrap();
            if !f.uploads.contains_key(*id) {
                return err(404, "not_found", Value::Null);
            }
            let parts: Vec<Value> = json_body()["parts"].as_array().cloned().unwrap_or_default()
                .into_iter()
                .map(|p| {
                    let n = p["number"].as_u64().unwrap_or(0);
                    json!({"number": n, "url": format!("{}/r2/part/{id}/{n}?X-Amz-Signature=sig", f.base),
                           "headers": {"content-md5": p["md5"]}, "expires_at": "2026-10-15T14:12:00Z"})
                })
                .collect();
            ok(200, json!({"parts": parts}))
        }
        ("GET", ["v1", "cloud", "uploads", id]) => {
            let f = f.lock().unwrap();
            match f.uploads.get(*id) {
                Some(u) => ok(
                    200,
                    json!({"sha256": u.sha, "size": u.size, "part_size": PART,
                    "part_count": u.part_count,
                    "parts_done": u.parts.keys().map(|n| json!({"number": n, "etag": format!("\"e{n}\"")})).collect::<Vec<_>>(),
                    "expires_at": "2026-10-16T14:02:00Z"}),
                ),
                None => err(404, "not_found", Value::Null),
            }
        }
        ("POST", ["v1", "cloud", "uploads", id, "complete"]) => {
            let mut f = f.lock().unwrap();
            let Some(u) = f.uploads.remove(*id) else {
                return err(404, "not_found", Value::Null);
            };
            let parts = json_body()["parts"].as_array().cloned().unwrap_or_default();
            let numbers: Vec<u64> = parts.iter().filter_map(|p| p["number"].as_u64()).collect();
            let all: Vec<u64> = (1..=u64::from(u.part_count)).collect();
            if numbers != all || u.parts.len() != u.part_count as usize {
                f.uploads.insert(id.to_string(), u);
                return err(409, "upload_mismatch", Value::Null);
            }
            let bytes: Vec<u8> = u.parts.values().flatten().copied().collect();
            if hex(&Sha256::digest(&bytes)) != u.sha {
                return err(409, "upload_mismatch", Value::Null);
            }
            f.blobs.insert(u.sha.clone(), bytes);
            ok(200, json!({"sha256": u.sha, "state": "stored"}))
        }
        ("DELETE", ["v1", "cloud", "uploads", id]) => {
            f.lock().unwrap().uploads.remove(*id);
            StatusCode::NO_CONTENT.into_response()
        }
        ("POST", ["v1", "cloud", "projects", _, "versions"]) => {
            let key = h("idempotency-key");
            let mut f = f.lock().unwrap();
            f.log.push("COMMIT".into());
            f.commit_keys.push(key.clone());
            if f.incomplete_commits > 0 {
                f.incomplete_commits -= 1;
                let remaining = f.incomplete_commits * 100 + 100;
                return err(503, "commit_incomplete", json!({"remaining": remaining}));
            }
            if f.commits_429 {
                return err(429, "rate_limited", Value::Null);
            }
            if let Some(prev) = f.commits_by_key.get(&key).cloned() {
                if f.lose_commit_answers > 0 {
                    f.lose_commit_answers -= 1;
                    return StatusCode::BAD_GATEWAY.into_response();
                }
                return ok(200, prev);
            }
            let b = json_body();
            let files = b["manifest"]["files"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if let Some(bad) = &f.refuse_path {
                if let Some(i) = files.iter().position(|x| x["path"] == bad.as_str()) {
                    return err(400, "invalid_path", json!({"index": i, "reason": "test"}));
                }
            }
            let missing: Vec<String> = files
                .iter()
                .filter_map(|x| x["sha256"].as_str())
                .filter(|s| !f.blobs.contains_key(*s))
                .map(str::to_string)
                .collect();
            if !missing.is_empty() {
                return err(409, "missing_blobs", json!({"missing": missing}));
            }
            if f.force_missing_once {
                f.force_missing_once = false;
                let first = files[0]["sha256"].as_str().unwrap_or("").to_string();
                f.blobs.remove(&first);
                return err(409, "missing_blobs", json!({"missing": [first]}));
            }
            let base = b["base_version_id"].as_str().map(str::to_string);
            f.reasons
                .push(b["reason"].as_str().unwrap_or("").to_string());
            f.restored.push(b["restored_from"].clone());
            let r = version(&mut f, base, &files, 0);
            f.commits_by_key.insert(key, r.clone());
            if f.lose_commit_answers > 0 {
                f.lose_commit_answers -= 1;
                return StatusCode::BAD_GATEWAY.into_response();
            }
            ok(201, r)
        }
        ("GET", ["v1", "cloud", "projects"]) => {
            let f = f.lock().unwrap();
            let include = uri.query().is_some_and(|q| q.contains("trashed=include"));
            let list: Vec<Value> = f
                .projects
                .iter()
                .filter(|_| include || !f.trashed)
                .map(|(app, id)| fake_project(&f, app, id))
                .collect();
            ok(200, json!({"projects": list}))
        }
        ("PATCH", ["v1", "cloud", "projects", pid]) => {
            let mut f = f.lock().unwrap();
            f.proj_name = json_body()["name"].as_str().unwrap_or("").to_string();
            let app = f
                .projects
                .iter()
                .find(|(_, id)| id.as_str() == *pid)
                .map(|(a, _)| a.clone())
                .unwrap_or_default();
            ok(200, fake_project(&f, &app, pid))
        }
        ("DELETE", ["v1", "cloud", "projects", pid]) => {
            let mut f = f.lock().unwrap();
            f.trashed = true;
            let app = f
                .projects
                .iter()
                .find(|(_, id)| id.as_str() == *pid)
                .map(|(a, _)| a.clone())
                .unwrap_or_default();
            ok(200, fake_project(&f, &app, pid))
        }
        ("POST", ["v1", "cloud", "projects", pid, "restore"]) => {
            let mut f = f.lock().unwrap();
            f.trashed = false;
            let app = f
                .projects
                .iter()
                .find(|(_, id)| id.as_str() == *pid)
                .map(|(a, _)| a.clone())
                .unwrap_or_default();
            ok(200, fake_project(&f, &app, pid))
        }
        ("DELETE", ["v1", "cloud", "projects", _, "versions", vid]) => {
            let mut f = f.lock().unwrap();
            if f.head.as_deref() == Some(*vid) {
                return err(409, "cannot_delete_head", Value::Null);
            }
            f.versions.retain(|v| v["id"] != *vid);
            StatusCode::NO_CONTENT.into_response()
        }
        ("GET", ["v1", "cloud", "projects", pid]) => {
            let f = f.lock().unwrap();
            let app = f
                .projects
                .iter()
                .find(|(_, id)| id.as_str() == *pid)
                .map(|(a, _)| a.clone())
                .unwrap_or_default();
            ok(200, fake_project(&f, &app, pid))
        }
        ("GET", ["v1", "cloud", "projects", _, "versions", vid, "manifest"]) => {
            let f = f.lock().unwrap();
            let Some(v) = f.versions.iter().find(|v| v["id"] == *vid) else {
                return err(404, "not_found", Value::Null);
            };
            let files = v["manifest"].clone();
            let mut version = v.clone();
            version.as_object_mut().unwrap().remove("manifest");
            ok(
                200,
                json!({"version": version, "manifest": {"format": 1, "files": files}}),
            )
        }
        ("POST", ["v1", "cloud", "blobs", "download"]) => {
            let mut f = f.lock().unwrap();
            f.log.push("DOWNLOAD URLS".into());
            let shas = json_body()["sha256s"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let (mut urls, mut missing) = (Vec::new(), Vec::new());
            for s in shas {
                let sha = s.as_str().unwrap_or("").to_string();
                if f.blobs.contains_key(&sha) {
                    urls.push(json!({"sha256": sha,
                        "url": format!("{}/r2/get/{sha}?X-Amz-Signature=sig", f.base),
                        "expires_at": "2026-10-15T14:12:00Z"}));
                } else {
                    missing.push(sha);
                }
            }
            ok(200, json!({"urls": urls, "missing": missing}))
        }
        ("GET", ["r2", "get", sha]) => {
            let delay = f.lock().unwrap().get_delay;
            tokio::time::sleep(delay).await;
            let mut f = f.lock().unwrap();
            f.log.push(format!("GET blob {sha}"));
            let Some(mut body) = f.blobs.get(*sha).cloned() else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if let Some(n) = f.corrupt_gets.get_mut(*sha).filter(|n| **n > 0) {
                *n -= 1;
                if let Some(b) = body.first_mut() {
                    *b ^= 0xff;
                }
            }
            (StatusCode::OK, body).into_response()
        }
        ("GET", ["v1", "cloud", "projects", _, "versions"]) => {
            let f = f.lock().unwrap();
            let list: Vec<Value> = f
                .versions
                .iter()
                .rev()
                .map(|v| {
                    let mut v = v.clone();
                    v.as_object_mut().unwrap().remove("manifest");
                    v
                })
                .collect();
            ok(200, json!({"versions": list, "next": null}))
        }
        ("PATCH", ["v1", "cloud", "projects", _, "versions", vid]) => {
            let mut f = f.lock().unwrap();
            let b = json_body();
            f.patches.push((vid.to_string(), b.clone()));
            let Some(v) = f.versions.iter_mut().find(|v| v["id"] == *vid) else {
                return err(404, "not_found", Value::Null);
            };
            if b["conflict_resolved"] == true && v["conflict"].is_object() {
                v["conflict"]["resolved_at"] = json!("2026-10-15T15:00:00Z");
            }
            if let Some(k) = b["kept"].as_bool() {
                v["kept"] = json!(k);
            }
            if b.get("label").is_some() {
                v["label"] = b["label"].clone();
            }
            let kept = f.versions.iter().filter(|v| v["kept"] == true).count();
            if b["kept"] == true && kept > f.kept_limit {
                let id = vid.to_string();
                if let Some(v) = f.versions.iter_mut().find(|v| v["id"] == id.as_str()) {
                    v["kept"] = json!(false);
                }
                return err(409, "too_many_kept", json!({"limit": f.kept_limit}));
            }
            let Some(v) = f.versions.iter().find(|v| v["id"] == *vid) else {
                return err(404, "not_found", Value::Null);
            };
            let mut out = v.clone();
            out.as_object_mut().unwrap().remove("manifest");
            ok(200, out)
        }
        ("PUT", ["r2", "blob", sha]) => {
            let on_put = f.lock().unwrap().on_put.clone();
            if let Some(cb) = on_put {
                cb();
            }
            let delay = f.lock().unwrap().put_delay;
            tokio::time::sleep(delay).await;
            let mut f = f.lock().unwrap();
            f.log.push(format!("PUT blob {sha}"));
            if f.expire_next_puts > 0 {
                f.expire_next_puts -= 1;
                return (
                    StatusCode::FORBIDDEN,
                    "<Error><Code>AccessDenied</Code></Error>",
                )
                    .into_response();
            }
            if hex(&Sha256::digest(&body)) != h("x-amz-checksum-sha256")
                || body.len().to_string() != h("content-length")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    "<Error><Code>BadDigest</Code></Error>",
                )
                    .into_response();
            }
            f.put_times.push((Instant::now(), body.len()));
            f.blobs.insert(sha.to_string(), body.to_vec());
            StatusCode::OK.into_response()
        }
        ("PUT", ["r2", "part", id, n]) => {
            let delay = f.lock().unwrap().put_delay;
            tokio::time::sleep(delay).await;
            let n: u32 = n.parse().unwrap_or(0);
            let cb = {
                let mut f = f.lock().unwrap();
                f.log.push(format!("PUT part {n}"));
                if hex(&Md5::digest(&body)) != h("content-md5") {
                    return (
                        StatusCode::BAD_REQUEST,
                        "<Error><Code>BadDigest</Code></Error>",
                    )
                        .into_response();
                }
                let Some(u) = f.uploads.get_mut(*id) else {
                    return StatusCode::NOT_FOUND.into_response();
                };
                u.parts.insert(n, body.to_vec());
                f.on_part.clone()
            };
            if let Some(cb) = cb {
                cb(n);
            }
            let mut r = StatusCode::OK.into_response();
            r.headers_mut()
                .insert("etag", format!("\"e{n}\"").parse().unwrap());
            r
        }
        _ => (StatusCode::NOT_FOUND, "no route").into_response(),
    }
}

pub(super) struct Env {
    pub(super) _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    pub(super) st: AppState,
    pub(super) fake: F,
    pub(super) dir: PathBuf,
}

pub(super) fn env() -> Env {
    let tmp = tempfile::tempdir().unwrap();
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        Arc::new(crate::core::events::NullSink),
        Duration::from_secs(60),
    )
    .unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    st.live().set_runtime(rt.handle().clone());
    let fake: F = Arc::new(Mutex::new(Fake {
        quota_state: "ok".into(),
        kept_limit: 50,
        ..Default::default()
    }));
    let app = axum::Router::new()
        .fallback(handle)
        .with_state(fake.clone());
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    fake.lock().unwrap().base = base.clone();
    rt.spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let secrets = Arc::new(MemorySecrets::default());
    secrets.set(S_SESSION, "kntr_s_test").unwrap();
    st.cloud_service().set_test_env(&base, secrets, MACHINE);
    st.cloud_init();
    st.cloud_service().set_test_license(None, Some("act-1"));
    st.cloud_service()
        .set_test_entitlements(&["core.app", super::service::PRO_CLOUD], true);
    let parent = tmp.path().join("shows");
    std::fs::create_dir_all(&parent).unwrap();
    let st2 = st.clone();
    let dir = rt.block_on(async move {
        tokio::task::spawn_blocking(move || st2.create_project(&parent, "Gala").unwrap().path)
            .await
            .unwrap()
    });
    let dir = PathBuf::from(dir);
    rt.block_on(st.cloud_refresh(false)).unwrap();
    Env {
        _tmp: tmp,
        _rt: rt,
        st,
        fake,
        dir,
    }
}

impl Env {
    pub(super) fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    pub(super) fn rt_block<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self._rt.block_on(f)
    }

    pub(super) fn log(&self) -> Vec<String> {
        self.fake.lock().unwrap().log.clone()
    }

    pub(super) fn clear_log(&self) {
        self.fake.lock().unwrap().log.clear();
    }

    fn puts(&self) -> usize {
        self.log().iter().filter(|l| l.starts_with("PUT")).count()
    }

    pub(super) fn view(&self) -> CloudView {
        self.st.cloud_view()
    }

    /// Runs "Back up now" and waits for it to end.
    pub(super) fn backup(&self) -> CloudView {
        self.st.cloud_backup_now().unwrap();
        self.wait_done();
        self.view()
    }

    fn wait_done(&self) {
        let t = Instant::now();
        while self.st.cloud_backup_running() {
            assert!(t.elapsed() < Duration::from_secs(20), "backup didn't end");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn last_manifest(&self) -> Vec<Value> {
        let f = self.fake.lock().unwrap();
        f.versions.last().unwrap()["manifest"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn paths(&self) -> Vec<String> {
        self.last_manifest()
            .iter()
            .map(|x| x["path"].as_str().unwrap().to_string())
            .collect()
    }
}

pub(super) fn bytes(n: usize, seed: u8) -> Vec<u8> {
    (0..n)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

// ---- Tests ----------------------------------------------------------------------------------------

#[test]
fn a_project_backs_up_and_an_unchanged_one_sends_nothing() {
    let e = env();
    e.write("assets/logo.png", &bytes(20_000, 1));
    e.write("assets/intro.mov", &bytes(150_000, 2)); // multipart: 5 parts of 32 KiB
    e.write(".backups/x/project.json", b"{}");
    let v = e.backup();
    let p = v.project.unwrap();
    assert_eq!(p.error, None);
    assert!(p.last_backup_at.is_some());
    assert!(p.run.is_none());
    let paths = e.paths();
    assert!(paths.contains(&"assets/intro.mov".to_string()));
    assert!(paths.contains(&"project.json".to_string()));
    assert!(!paths.iter().any(|p| p.starts_with(".backups")));
    // The big file went up in parts and storage has it byte for byte.
    let log = e.log();
    assert_eq!(log.iter().filter(|l| l.starts_with("PUT part")).count(), 5);
    let mov = std::fs::read(e.dir.join("assets/intro.mov")).unwrap();
    assert!(e
        .fake
        .lock()
        .unwrap()
        .blobs
        .contains_key(&hex(&Sha256::digest(&mov))));
    // The manifest has the file's hash, size and mtime.
    let entry = e
        .last_manifest()
        .into_iter()
        .find(|x| x["path"] == "assets/intro.mov")
        .unwrap();
    assert_eq!(entry["size"], 150_000);
    assert!(entry["modified_at"].as_str().unwrap().ends_with('Z'));

    // Again with no changes: nothing is sent, a version is still committed on the last head.
    e.clear_log();
    let v = e.backup();
    assert_eq!(v.project.unwrap().error, None);
    assert_eq!(e.puts(), 0, "{:?}", e.log());
    let f = e.fake.lock().unwrap();
    assert_eq!(f.versions.len(), 2);
    assert_eq!(f.versions[1]["base_version_id"], f.versions[0]["id"]);
    assert!(f.versions[1]["conflict"].is_null());
}

#[test]
fn changing_one_file_sends_only_that_file() {
    let e = env();
    e.write("a.txt", b"one");
    e.write("b.txt", b"two");
    e.backup();
    e.clear_log();
    std::thread::sleep(Duration::from_millis(20));
    e.write("b.txt", b"two, changed");
    e.backup();
    let sha = hex(&Sha256::digest(b"two, changed"));
    let puts: Vec<String> = e
        .log()
        .into_iter()
        .filter(|l| l.starts_with("PUT"))
        .collect();
    assert_eq!(puts, [format!("PUT blob {sha}")]);
}

#[test]
fn a_multipart_upload_resumes_where_it_stopped() {
    let e = env();
    let data = bytes(200_000, 3); // 7 parts
    e.write("assets/big.mov", &data);
    // Stop the backup right after part 3 is stored (the app quits mid-upload).
    let st = e.st.clone();
    e.fake.lock().unwrap().on_part = Some(Arc::new(move |n| {
        if n == 3 {
            st.cloud_backup_cancel();
        }
    }));
    e.st.cloud_backup_now().unwrap();
    e.wait_done();
    std::thread::sleep(Duration::from_millis(100));
    let first: Vec<String> = e
        .log()
        .into_iter()
        .filter(|l| l.starts_with("PUT part"))
        .collect();
    assert!(first.len() >= 3 && first.len() < 7, "{first:?}");
    assert!(
        e.fake.lock().unwrap().versions.is_empty(),
        "nothing committed"
    );
    // The upload id and its parts are remembered on this computer.
    let file = e.st.cloud_file();
    let uploads = &file.accounts[ACCOUNT]
        .projects
        .values()
        .next()
        .unwrap()
        .uploads;
    assert_eq!(uploads.len(), 1);
    // Again: only the parts storage doesn't have yet.
    e.fake.lock().unwrap().on_part = None;
    e.clear_log();
    let v = e.backup();
    assert_eq!(v.project.unwrap().error, None);
    let again: Vec<String> = e
        .log()
        .into_iter()
        .filter(|l| l.starts_with("PUT part"))
        .collect();
    let done_before: HashSet<String> = first.into_iter().collect();
    assert!(
        !again.iter().any(|p| p == "PUT part 1" || p == "PUT part 2"),
        "{again:?}"
    );
    assert_eq!(
        done_before.len() + again.len(),
        7,
        "{done_before:?} + {again:?}"
    );
    let sha = hex(&Sha256::digest(&data));
    assert_eq!(e.fake.lock().unwrap().blobs[&sha], data, "byte for byte");
    let file = e.st.cloud_file();
    assert!(file.accounts[ACCOUNT]
        .projects
        .values()
        .next()
        .unwrap()
        .uploads
        .is_empty());
}

#[test]
fn nothing_is_sent_while_on_air() {
    let e = env();
    e.write("a.txt", b"hello");
    // OBS has the browser source loaded: on air for cloud backup.
    let obs = e.st.live().renderer_attach("main", true);
    assert!(e.st.cloud_on_air());
    e.st.cloud_backup_now().unwrap();
    wait_until("paused", || {
        e.view()
            .project
            .and_then(|p| p.run)
            .is_some_and(|r| r.paused && r.phase == CloudPhase::Uploading)
    });
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(e.puts(), 0);
    drop(obs);
    e.wait_done();
    assert!(e.puts() >= 1);
    assert_eq!(e.view().project.unwrap().error, None);
}

#[test]
fn going_on_air_stops_a_transfer_within_a_second() {
    let e = env();
    e.write("a.txt", &bytes(30_000, 4));
    e.fake.lock().unwrap().put_delay = Duration::from_millis(1500);
    let obs: Arc<Mutex<Option<crate::core::live_hub::RendererReg>>> = Arc::default();
    let (st, slot) = (e.st.clone(), obs.clone());
    // The moment the PUT arrives, OBS connects.
    e.fake.lock().unwrap().on_put = Some(Arc::new(move || {
        let mut g = slot.lock().unwrap();
        if g.is_none() {
            *g = Some(st.live().renderer_attach("main", true));
        }
    }));
    e.st.cloud_backup_now().unwrap();
    wait_until("on air", || obs.lock().unwrap().is_some());
    let t = Instant::now();
    wait_until("paused", || {
        e.view()
            .project
            .and_then(|p| p.run)
            .is_some_and(|r| r.paused)
    });
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    e.fake.lock().unwrap().on_put = None;
    e.fake.lock().unwrap().put_delay = Duration::ZERO;
    obs.lock().unwrap().take();
    e.wait_done();
    assert_eq!(e.view().project.unwrap().error, None);
    assert_eq!(e.fake.lock().unwrap().versions.len(), 1);
}

#[test]
fn an_expired_url_is_asked_for_again() {
    let e = env();
    e.write("a.txt", b"x");
    e.fake.lock().unwrap().expire_next_puts = 1;
    let v = e.backup();
    assert_eq!(v.project.unwrap().error, None);
    assert_eq!(
        e.log().iter().filter(|l| *l == "CHECK").count(),
        2,
        "{:?}",
        e.log()
    );
}

#[test]
fn a_file_that_changes_during_its_upload_is_hashed_and_sent_again() {
    let e = env();
    e.write("live.txt", b"before");
    let path = e.dir.join("live.txt");
    let once = Arc::new(Mutex::new(false));
    e.fake.lock().unwrap().on_put = Some(Arc::new(move || {
        let mut done = once.lock().unwrap();
        if !*done {
            *done = true;
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(&path, b"after, longer").unwrap();
        }
    }));
    let v = e.backup();
    assert_eq!(v.project.unwrap().error, None);
    let entry = e
        .last_manifest()
        .into_iter()
        .find(|x| x["path"] == "live.txt")
        .unwrap();
    assert_eq!(entry["sha256"], hex(&Sha256::digest(b"after, longer")));
}

#[test]
fn a_full_account_commits_what_storage_has_and_lists_the_rest() {
    let e = env();
    e.write("a.txt", b"stored before");
    e.backup();
    e.write("b.txt", b"new, won't fit");
    e.fake.lock().unwrap().storage_full = true;
    let v = e.backup();
    let p = v.project.unwrap();
    assert!(p.error.unwrap().starts_with("Cloud storage is full."));
    assert!(p.report.iter().any(|s| s.path == "b.txt"));
    // A version without new bytes still went in.
    assert_eq!(e.fake.lock().unwrap().versions.len(), 2);
    assert!(!e.paths().contains(&"b.txt".to_string()));
    assert!(e.paths().contains(&"a.txt".to_string()));
}

#[test]
fn a_name_the_server_refuses_is_dropped_and_listed() {
    let e = env();
    e.write("ok.txt", b"1");
    e.write("odd-name.txt", b"2");
    e.fake.lock().unwrap().refuse_path = Some("odd-name.txt".into());
    let v = e.backup();
    let p = v.project.unwrap();
    assert_eq!(p.error, None);
    assert_eq!(p.skipped, 1);
    assert_eq!(p.report[0].path, "odd-name.txt");
    assert!(e.paths().contains(&"ok.txt".to_string()));
}

#[test]
fn missing_blobs_are_sent_and_the_commit_retried_with_the_same_key() {
    let e = env();
    e.write("a.txt", b"x");
    e.fake.lock().unwrap().force_missing_once = true;
    let v = e.backup();
    assert_eq!(v.project.unwrap().error, None);
    let log = e.log();
    assert_eq!(log.iter().filter(|l| *l == "COMMIT").count(), 2, "{log:?}");
    assert_eq!(e.fake.lock().unwrap().versions.len(), 1);
}

#[test]
fn another_computers_newer_backup_is_a_conflict_not_a_failure() {
    let e = env();
    e.write("a.txt", b"x");
    e.backup();
    // Another computer committed in between.
    {
        let mut f = e.fake.lock().unwrap();
        let head = f.head.clone();
        let _ = version(&mut f, head, &[], 0);
    }
    let v = e.backup();
    let p = v.project.unwrap();
    assert_eq!(p.error, None);
    assert_eq!(
        p.conflict.unwrap().machine.as_deref(),
        Some("STUDIO-PC (b)")
    );
}

#[test]
fn backup_needs_the_project_switch_on() {
    let e = env();
    e.st.cloud_set_backup(false).unwrap();
    assert!(e.st.cloud_backup_now().is_err());
    assert!(!e.st.cloud_backup_running());
}

fn set_on_air_policy(st: &AppState, on_air: crate::core::model::CloudOnAir, kbps: u32) {
    let mut p = st.prefs();
    p.cloud.on_air = Some(on_air);
    p.cloud.limit_kbps = Some(kbps);
    st.set_prefs(p);
}

#[test]
fn limit_mode_keeps_sending_on_air_at_the_set_speed() {
    let e = env();
    // 400 kbit/s = 50 000 B/s: 60 000 B take at least 1.2 s while on air.
    set_on_air_policy(&e.st, crate::core::model::CloudOnAir::Limit, 400);
    e.write("clip.bin", &bytes(60_000, 5));
    let obs = e.st.live().renderer_attach("main", true);
    let t = Instant::now();
    e.st.cloud_backup_now().unwrap();
    wait_until("limited", || {
        e.view()
            .project
            .and_then(|p| p.run)
            .is_some_and(|r| r.limited && !r.paused)
    });
    e.wait_done();
    let took = t.elapsed();
    assert!(took >= Duration::from_millis(1100), "{took:?}");
    assert_eq!(e.view().project.unwrap().error, None);
    assert_eq!(e.fake.lock().unwrap().versions.len(), 1);
    drop(obs);
    // Off air: the same amount goes at full speed.
    e.write("clip2.bin", &bytes(60_000, 6));
    let t = Instant::now();
    e.backup();
    assert!(
        t.elapsed() < Duration::from_millis(1000),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn switching_to_pause_mid_backup_pauses_at_once() {
    use crate::core::model::{CloudOnAir, CloudPhase};
    let e = env();
    // The project's own files go up first, so the clip is the next backup's only transfer.
    e.backup();
    let before = e.puts();
    set_on_air_policy(&e.st, CloudOnAir::Limit, 64);
    e.write("clip.bin", &bytes(60_000, 7)); // one chunk, held ~7.5 s at 64 kbit/s
    let obs = e.st.live().renderer_attach("main", true);
    e.st.cloud_backup_now().unwrap();
    wait_until("uploading, limited", || {
        e.view()
            .project
            .and_then(|p| p.run)
            .is_some_and(|r| r.limited && r.phase == CloudPhase::Uploading)
    });
    // In flight: the request is open and the pacer holds its body.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(e.puts(), before, "held by the pacer: {:?}", e.log());
    let t = Instant::now();
    set_on_air_policy(&e.st, CloudOnAir::Pause, 64);
    wait_until("paused", || {
        e.view()
            .project
            .and_then(|p| p.run)
            .is_some_and(|r| r.paused)
    });
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    assert_eq!(
        e.puts(),
        before,
        "the paced request was dropped before it finished: {:?}",
        e.log()
    );
    drop(obs);
    e.wait_done();
    assert_eq!(e.view().project.unwrap().error, None);
    assert_eq!(e.puts(), before + 1, "sent once off air");
}

#[test]
fn nothing_goes_out_unpaced_while_on_air_first_chunk_included() {
    let e = env();
    set_on_air_policy(&e.st, crate::core::model::CloudOnAir::Limit, 64); // 8,000 bytes/s
    e.write("a.bin", &bytes(4_000, 1)); // single chunks: ~0.5 s and ~0.25 s
    e.write("b.bin", &bytes(2_000, 2));
    let obs = e.st.live().renderer_attach("main", true);
    let t0 = Instant::now();
    e.st.cloud_backup_now().unwrap();
    e.wait_done();
    assert_eq!(e.view().project.unwrap().error, None);
    let puts = e.fake.lock().unwrap().put_times.clone();
    assert!(puts.len() >= 2, "{:?}", e.log());
    // Every upload, the project's own small files and each first chunk included: by the time
    // a PUT is stored, the bytes sent so far fit 64 kbit/s since the backup started.
    let mut sent = 0usize;
    for (at, n) in &puts {
        sent += n;
        let due = Duration::from_secs_f64(sent as f64 / 8_000.0).mul_f64(0.95);
        assert!(
            at.duration_since(t0) >= due,
            "{sent} bytes stored after {:?}, before {due:?}",
            at.duration_since(t0)
        );
    }
    drop(obs);
}

#[test]
fn commit_incomplete_commits_again_at_once_with_the_same_key_and_is_not_offline() {
    let e = env();
    e.write("clip.bin", &bytes(10_000, 3));
    // More answers than the commit's tries (4) and offline retries (2) together: none of
    // them counts.
    e.fake.lock().unwrap().incomplete_commits = 6;
    let v = e.backup();
    let p = v.project.unwrap();
    assert_eq!(p.error, None, "not offline, not failed");
    assert!(p.last_backup_at.is_some());
    let f = e.fake.lock().unwrap();
    assert_eq!(f.versions.len(), 1, "one version");
    assert_eq!(
        f.commit_keys.len(),
        7,
        "6 incomplete, then stored: {:?}",
        f.log
    );
    assert!(
        f.commit_keys.iter().all(|k| *k == f.commit_keys[0]),
        "the same key every time: {:?}",
        f.commit_keys
    );
    assert_eq!(f.incomplete_commits, 0);
    drop(f);
    assert_eq!(
        e.st.cloud_record().unwrap().pending_commit,
        None,
        "confirmed"
    );
}

#[test]
fn commit_incomplete_past_50_rounds_takes_the_normal_retries_then_offline() {
    let commits = |e: &Env| e.fake.lock().unwrap().commit_keys.len();
    // 50 paced rounds, then the normal schedule's 2 retries: the 53rd commit is stored.
    let e = env();
    e.write("clip.bin", &bytes(10_000, 3));
    e.fake.lock().unwrap().incomplete_commits = 52;
    let p = e.backup().project.unwrap();
    assert_eq!(p.error, None, "recovered within the normal retries");
    assert_eq!(commits(&e), 53);
    assert_eq!(e.fake.lock().unwrap().versions.len(), 1);
    // One more: the normal retries run out too, and only then it's offline.
    let e = env();
    e.write("clip.bin", &bytes(10_000, 3));
    e.fake.lock().unwrap().incomplete_commits = 53;
    let p = e.backup().project.unwrap();
    assert_eq!(
        p.error.as_deref(),
        Some("Cloud backup: offline, will retry.")
    );
    assert_eq!(commits(&e), 53);
    let keys = e.fake.lock().unwrap().commit_keys.clone();
    assert!(
        keys.iter().all(|k| *k == keys[0]),
        "the same key throughout"
    );
    assert!(
        e.st.cloud_record().unwrap().pending_commit.is_some(),
        "kept for the next backup"
    );
}

#[test]
fn a_lost_commit_answer_is_retried_with_the_same_key_and_makes_one_version() {
    let e = env();
    e.write("a.txt", b"x");
    e.backup();
    e.write("b.txt", b"y");
    // The server stores the commit, but the answer never arrives (twice).
    e.fake.lock().unwrap().lose_commit_answers = 2;
    let v = e.backup();
    let p = v.project.unwrap();
    assert_eq!(p.error, None);
    assert!(p.conflict.is_none(), "not a conflict with ourselves");
    assert_eq!(e.fake.lock().unwrap().versions.len(), 2);
    // Answers lost on every try: offline now, and the next backup reuses the key.
    e.write("c.txt", b"z");
    e.fake.lock().unwrap().lose_commit_answers = 3;
    let v = e.backup();
    assert_eq!(
        v.project.unwrap().error.as_deref(),
        Some("Cloud backup: offline, will retry.")
    );
    assert_eq!(e.fake.lock().unwrap().versions.len(), 3);
    let v = e.backup();
    let p = v.project.unwrap();
    assert_eq!(p.error, None);
    assert!(p.conflict.is_none());
    assert_eq!(
        e.fake.lock().unwrap().versions.len(),
        3,
        "the stored commit was confirmed"
    );
    // Confirmed: the key is forgotten, the next backup makes a new version.
    e.backup();
    assert_eq!(e.fake.lock().unwrap().versions.len(), 4);
}

#[test]
fn a_conflict_comes_back_after_reopening_and_dismiss_resolves_it() {
    let e = env();
    e.write("a.txt", b"x");
    e.backup();
    {
        let mut f = e.fake.lock().unwrap();
        let head = f.head.clone();
        let _ = version(&mut f, head, &[], 0);
    }
    let v = e.backup();
    let c = v.project.unwrap().conflict.unwrap();
    // Reopened (or after a restart): the notice is read from the server.
    e.st.close_project();
    e.st.open_project(&e.dir).unwrap();
    wait_until("conflict loaded", || {
        e.view().project.is_some_and(|p| p.conflict.is_some())
    });
    assert_eq!(
        e.view().project.unwrap().conflict.unwrap().version_id,
        c.version_id
    );
    // Dismiss: the server marks it resolved, the notice goes.
    e.rt_block(e.st.cloud_conflict_dismiss()).unwrap();
    assert!(e.view().project.unwrap().conflict.is_none());
    let patches = e.fake.lock().unwrap().patches.clone();
    assert_eq!(
        patches,
        [(c.version_id.clone(), json!({"conflict_resolved": true}))]
    );
    e.st.close_project();
    e.st.open_project(&e.dir).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    assert!(e.view().project.unwrap().conflict.is_none());
}

#[test]
fn too_many_commits_says_so_instead_of_offline() {
    let e = env();
    e.write("a.txt", b"x");
    e.fake.lock().unwrap().commits_429 = true;
    let v = e.backup();
    assert_eq!(
        v.project.unwrap().error.as_deref(),
        Some("Cloud backup is busy for this account. Kinetra will try again later.")
    );
}

// ---- Slice 8: by itself ---------------------------------------------------------------------------

fn reasons(e: &Env) -> Vec<String> {
    e.fake.lock().unwrap().reasons.clone()
}

/// A change Kinetra makes in the project (through `fsx`, like every save).
fn own_write(e: &Env, rel: &str, bytes: &[u8]) {
    std::fs::create_dir_all(e.dir.join(rel).parent().unwrap()).unwrap();
    crate::core::fsx::write_bytes_atomic(&e.dir.join(rel), bytes).unwrap();
}

#[test]
fn a_change_backs_up_after_the_interval_and_nothing_waits_at_rest() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(150)));
    // The project opened before the quota arrived: refresh once more so it's armed.
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    assert!(!e.st.cloud_timer_waiting(), "no timer without changes");
    own_write(&e, "screens/x.json", b"{}");
    wait_until("timer", || e.st.cloud_timer_waiting());
    wait_until("auto backup", || reasons(&e) == ["auto"]);
    e.wait_done();
    // A change during the backup (its flush) armed a timer the backup then covered: it stops.
    let t = Instant::now();
    while e.st.cloud_timer_waiting() {
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "timed out: nothing waits; due in {:?}, backups {:?}, pending {}, log {:?}",
            e.st.cloud_timer_due()
                .map(|d| d.saturating_duration_since(Instant::now())),
            reasons(&e),
            e.st.cloud_record().unwrap().pending,
            e.log()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!e.st.cloud_record().unwrap().pending);
    // Excluded paths (our snapshots) aren't changes.
    own_write(&e, ".backups/x/project.json", b"{}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(reasons(&e), ["auto"]);
    assert!(!e.st.cloud_timer_waiting());
}

#[test]
fn many_changes_make_one_backup_after_the_last() {
    let e = env();
    // Long enough that no write of the burst (a disk flush can stall under load) lets it fire.
    e.st.set_test_cloud_interval(Some(Duration::from_secs(60)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    let mut due = None;
    for i in 0..5 {
        own_write(&e, &format!("assets/{i}.txt"), b"x");
        wait_until("timer", || e.st.cloud_timer_due() > due);
        // Each change moves the deadline: the backup waits for the last one.
        due = e.st.cloud_timer_due();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(reasons(&e).is_empty());
    // Due now: the waiting timer starts again with the short interval and fires.
    e.st.set_test_cloud_interval(Some(Duration::from_millis(50)));
    wait_until("auto backup", || reasons(&e).len() == 1);
    e.wait_done();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(reasons(&e), ["auto"], "one backup for the burst");
    assert!(!e.st.cloud_timer_waiting());
    let paths = e.paths();
    assert!((0..5).all(|i| paths.contains(&format!("assets/{i}.txt"))));
}

#[test]
fn an_outside_edit_is_a_change_too() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    // Written by another program (not through fsx): the watcher reports it.
    std::fs::create_dir_all(e.dir.join("assets")).unwrap();
    std::fs::write(e.dir.join("assets/from-outside.png"), b"png").unwrap();
    wait_until("auto backup", || reasons(&e) == ["auto"]);
    e.wait_done();
    assert!(e.paths().contains(&"assets/from-outside.png".to_string()));
}

#[test]
fn closing_with_changes_backs_up_the_closed_project() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_secs(600)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    e.backup(); // registered, a first version
    own_write(&e, "assets/late.txt", b"made just before closing");
    wait_until("pending", || e.st.cloud_record().is_some_and(|r| r.pending));
    let cid = e.st.project_cloud_id().unwrap().1.unwrap();
    e.st.close_project();
    wait_until("close backup", || reasons(&e).len() == 2);
    wait_until("done", || e.st.cloud_closed_backups() == 0);
    assert_eq!(reasons(&e)[1], "close");
    assert!(e.paths().contains(&"assets/late.txt".to_string()));
    assert!(!e.st.cloud_record_of(&cid).unwrap().pending);
}

#[test]
fn changes_left_when_backup_was_off_go_up_once_it_is_on_again() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    e.backup();
    e.st.set_test_cloud_interval(Some(Duration::from_secs(600)));
    own_write(&e, "assets/waiting.txt", b"x");
    wait_until("pending", || e.st.cloud_record().is_some_and(|r| r.pending));
    // Off, closed and opened again (a later session): nothing runs while off.
    e.st.cloud_set_backup(false).unwrap();
    e.st.close_project();
    e.st.open_project(&e.dir).unwrap();
    assert!(e.st.cloud_record().unwrap().pending, "remembered");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(reasons(&e).len(), 1);
    // On again: the waiting changes go up.
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.st.cloud_set_backup(true).unwrap();
    wait_until("auto backup", || reasons(&e).len() == 2);
    e.wait_done();
    assert!(e.paths().contains(&"assets/waiting.txt".to_string()));
}

#[test]
fn a_large_first_automatic_backup_asks_once() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    // Over the (test) threshold of 100 KB.
    own_write(&e, "assets/big.mov", &bytes(150_000, 9));
    wait_until("asked", || {
        e.view().project.is_some_and(|p| {
            matches!(p.question, Some(crate::core::model::CloudQuestion::FirstBackup { bytes }) if bytes >= 150_000)
        })
    });
    assert!(reasons(&e).is_empty(), "nothing uploaded before the answer");
    assert_eq!(e.puts(), 0);
    e.rt_block(e.st.cloud_answer(crate::core::model::CloudAnswer::BackUp))
        .unwrap();
    wait_until("backed up", || reasons(&e) == ["auto"]);
    e.wait_done();
    assert!(e.st.cloud_record().unwrap().big_ok);
    assert!(e.view().project.unwrap().question.is_none());
}

#[test]
fn quitting_asks_only_with_changes_and_nothing_on_air() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_secs(600)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    e.backup();
    assert!(!e.st.cloud_exit_should_ask(), "everything is in the cloud");
    own_write(&e, "assets/new.txt", b"x");
    wait_until("pending", || e.st.cloud_record().is_some_and(|r| r.pending));
    assert!(e.st.cloud_exit_should_ask());
    let obs = e.st.live().renderer_attach("main", true);
    assert!(!e.st.cloud_exit_should_ask(), "never while on air");
    drop(obs);
    // "Back up and quit".
    e.st.cloud_backup_before_exit().unwrap();
    e.wait_done();
    assert_eq!(reasons(&e).last().map(String::as_str), Some("close"));
    assert!(!e.st.cloud_exit_should_ask());
}

// ---- Slice 9: history and the account's projects -------------------------------------------------

#[test]
fn the_history_pages_keeps_labels_and_deletes() {
    let e = env();
    for i in 0..3 {
        e.write(&format!("v{i}.txt"), b"x");
        e.backup();
    }
    let page = e.rt_block(e.st.cloud_versions(None)).unwrap();
    assert_eq!(page.versions.len(), 3);
    assert!(page.versions[0].is_head, "newest first");
    assert_eq!(page.versions[0].machine.as_deref(), Some("STUDIO-PC (b)"));
    let middle = page.versions[1].id.clone();
    let oldest = page.versions[2].id.clone();
    let head = page.versions[0].id.clone();
    // Keep and label.
    let v = e.rt_block(e.st.cloud_version_keep(&middle, true)).unwrap();
    assert!(v.kept);
    let v = e
        .rt_block(e.st.cloud_version_label(&middle, Some("  Before the show ".into())))
        .unwrap();
    assert_eq!(v.label.as_deref(), Some("Before the show"));
    let v = e
        .rt_block(e.st.cloud_version_label(&middle, Some(" ".into())))
        .unwrap();
    assert_eq!(v.label, None, "blank clears it");
    // The plan's limit, in words.
    e.fake.lock().unwrap().kept_limit = 1;
    let er = e
        .rt_block(e.st.cloud_version_keep(&oldest, true))
        .unwrap_err();
    assert_eq!(
        er.to_string(),
        "Your plan keeps up to 1 version per project."
    );
    // The newest can't be deleted; another can.
    let er = e.rt_block(e.st.cloud_version_delete(&head)).unwrap_err();
    assert!(
        er.to_string()
            .starts_with("The newest version can't be deleted."),
        "{er}"
    );
    e.rt_block(e.st.cloud_version_delete(&oldest)).unwrap();
    let page = e.rt_block(e.st.cloud_versions(None)).unwrap();
    assert_eq!(page.versions.len(), 2);
}

#[test]
fn cloud_projects_rename_trash_and_restore() {
    let e = env();
    e.write("a.txt", b"x");
    e.backup();
    let list = e.rt_block(e.st.cloud_projects_list(false)).unwrap();
    assert_eq!(list.len(), 1);
    let p = &list[0];
    assert!(p.open && p.here);
    assert_eq!(p.versions, 1);
    assert!(p.last_at.is_some());
    let p = e
        .rt_block(e.st.cloud_project_rename(&p.id, "Gala 2026"))
        .unwrap();
    assert_eq!(p.name, "Gala 2026");
    // To the trash: the open project's backup asks what to do.
    let t = e.rt_block(e.st.cloud_project_trash(&p.id)).unwrap();
    assert_eq!(t.purge_at.as_deref(), Some("2026-10-27T10:00:00Z"));
    assert!(matches!(
        e.view().project.unwrap().question,
        Some(crate::core::model::CloudQuestion::InTrash { .. })
    ));
    assert!(e
        .rt_block(e.st.cloud_projects_list(false))
        .unwrap()
        .is_empty());
    assert_eq!(e.rt_block(e.st.cloud_projects_list(true)).unwrap().len(), 1);
    // Back from the trash: the question goes.
    let r = e.rt_block(e.st.cloud_project_untrash(&p.id)).unwrap();
    assert!(r.trashed_at.is_none());
    assert!(e.view().project.unwrap().question.is_none());
}

#[test]
fn the_history_needs_a_backed_up_project() {
    let e = env();
    // The project registers in the background after opening: either order can win.
    let registered = || e.view().project.is_some_and(|p| p.registered);
    match e.rt_block(e.st.cloud_versions(None)) {
        Err(er) => assert_eq!(er.to_string(), "This project has no cloud backup yet."),
        Ok(page) => assert!(registered() && page.versions.is_empty()),
    }
    // Registered with nothing backed up: an empty history.
    let t = Instant::now();
    while !registered() {
        assert!(t.elapsed() < Duration::from_secs(10), "never registered");
        std::thread::sleep(Duration::from_millis(10));
    }
    let page = e.rt_block(e.st.cloud_versions(None)).unwrap();
    assert!(page.versions.is_empty());
}

fn commits(e: &Env) -> usize {
    e.fake.lock().unwrap().versions.len()
}

#[test]
fn a_full_account_pauses_automatic_backups_and_back_up_now_still_commits() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.write("a.txt", b"x");
    e.backup();
    e.fake.lock().unwrap().quota_state = "full".into();
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    // A change while full: nothing runs by itself (no version every interval).
    own_write(&e, "screens/x.json", b"{}");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(commits(&e), 1);
    assert!(!reasons(&e).contains(&"auto".to_string()));
    // "Back up now" still commits what storage has, and says why the rest waits.
    e.fake.lock().unwrap().storage_full = true;
    e.write("big.bin", &bytes(10_000, 5));
    let v = e.backup();
    assert_eq!(commits(&e), 2);
    assert!(v
        .project
        .unwrap()
        .error
        .unwrap()
        .starts_with("Cloud storage is full"));
    // Room again (versions deleted, or a bigger plan): the waiting changes back up by themselves.
    {
        let mut f = e.fake.lock().unwrap();
        f.storage_full = false;
        f.quota_state = "ok".into();
    }
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    wait_until("auto backup", || {
        reasons(&e).last().map(String::as_str) == Some("auto")
    });
    e.wait_done();
    assert!(e.paths().contains(&"big.bin".to_string()));
}

#[test]
fn a_used_up_daily_allowance_holds_automatic_backups_until_it_resets() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    // The reset comes 800 ms after the refusal here.
    e.st.cloud_projects()
        .set_test_retry(Some(Duration::from_millis(800)));
    e.fake.lock().unwrap().daily_limit = true;
    own_write(&e, "assets/a.bin", &bytes(1_000, 3));
    wait_until("held", || e.st.cloud_held());
    e.wait_done();
    let autos = |e: &Env| reasons(e).iter().filter(|r| *r == "auto").count();
    assert_eq!(
        autos(&e),
        1,
        "the refused backup committed what storage had"
    );
    assert_eq!(e.view().held_until.as_deref(), Some("2099-01-01T00:00:00Z"));
    // No retry every interval while held.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(autos(&e), 1);
    // A new day: the file goes up by itself.
    e.fake.lock().unwrap().daily_limit = false;
    wait_until("released", || !e.st.cloud_held());
    wait_until("backed up", || autos(&e) == 2);
    e.wait_done();
    assert!(e.paths().contains(&"assets/a.bin".to_string()));
    assert_eq!(e.view().held_until, None);
}

#[test]
fn a_lapsed_plan_is_read_only_and_the_history_still_loads() {
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.write("a.txt", b"x");
    e.backup();
    e.fake.lock().unwrap().quota_state = "lapsed".into();
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    // No automatic backup for a change; "Back up now" says why and sends nothing.
    own_write(&e, "screens/x.json", b"{}");
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(commits(&e), 1);
    let v = e.backup();
    assert_eq!(commits(&e), 1);
    assert!(v
        .project
        .unwrap()
        .error
        .unwrap()
        .starts_with("Your plan no longer includes cloud backup"));
    // Reading still works (and downloads: see tests_restore).
    let page = e.rt_block(e.st.cloud_versions(None)).unwrap();
    assert_eq!(page.versions.len(), 1);
    // The plan is back: the change made meanwhile goes up by itself.
    e.fake.lock().unwrap().quota_state = "ok".into();
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    wait_until("auto backup", || commits(&e) == 2);
}

#[test]
fn a_build_without_cloud_backup_asks_nothing_and_runs_nothing() {
    // Debug builds and tests have it (KINETRA_CLOUD unset = on in debug).
    const { assert!(super::CLOUD_BUILD) };
    let e = env();
    e.st.set_test_cloud_interval(Some(Duration::from_millis(100)));
    e.st.cloud_projects().set_test_unavailable(true);
    e.clear_log();
    let v = e.rt_block(e.st.cloud_refresh(true)).unwrap();
    assert_eq!(v.access, crate::core::model::CloudAccess::Unavailable);
    assert!(v.project.is_none() && v.quota.is_none());
    // A change: nothing is recorded or scheduled; actions refuse.
    own_write(&e, "screens/x.json", b"{}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(!e.st.cloud_timer_waiting());
    assert!(e.st.cloud_backup_now().is_err());
    assert!(e.st.cloud_set_backup(false).is_err());
    assert!(e.rt_block(e.st.cloud_versions(None)).is_err());
    assert!(e.rt_block(e.st.cloud_projects_list(false)).is_err());
    assert!(e
        .st
        .cloud_restore_start(crate::core::model::CloudRestoreRequest {
            project_id: "0199e2a0-0001-7000-8000-000000000001".into(),
            version_id: None,
            mode: crate::core::model::CloudRestoreMode::NewFolder,
            folder: Some(
                e.dir
                    .parent()
                    .unwrap()
                    .join("x")
                    .to_string_lossy()
                    .into_owned()
            ),
        })
        .is_err());
    // Not a single request.
    assert!(e.log().is_empty(), "{:?}", e.log());
}
