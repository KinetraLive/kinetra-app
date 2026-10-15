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
struct Fake {
    base: String,
    blobs: HashMap<String, Vec<u8>>,
    uploads: HashMap<String, Multi>,
    next_id: u64,
    projects: HashMap<String, String>,
    head: Option<String>,
    versions: Vec<Value>,
    commits_by_key: HashMap<String, Value>,
    /// Requests: "PUT blob <sha>", "PUT part <n>", "COMMIT", "CHECK".
    log: Vec<String>,
    // Knobs.
    expire_next_puts: u32,
    storage_full: bool,
    force_missing_once: bool,
    refuse_path: Option<String>,
    put_delay: Duration,
    /// Called (outside the lock) after a part is stored: number.
    on_part: Option<Arc<dyn Fn(u32) + Send + Sync>>,
    /// Called before a single PUT's body is read.
    on_put: Option<Arc<dyn Fn() + Send + Sync>>,
    quota_state: String,
}

type F = Arc<Mutex<Fake>>;

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
            let f = f.lock().unwrap();
            let used: u64 = f.blobs.values().map(|b| b.len() as u64).sum();
            ok(
                200,
                json!({"plan": "pro_monthly", "state": f.quota_state, "used_bytes": used,
                "limit_bytes": 200_000_000u64, "percent": 0, "warn_percent": 80,
                "projects": {"used": f.projects.len(), "limit": 100},
                "max_file_bytes": 5_000_000_000u64, "retention_days": 30, "kept_per_project": 50,
                "upload_today": {"used_bytes": 0, "limit_bytes": 300_000_000u64},
                "client": {"backup_interval_s": 300, "on_air": "pause", "on_air_limit_kbps": 2000,
                           "multipart_threshold_bytes": THRESHOLD, "part_size_bytes": PART}}),
            )
        }
        ("POST", ["v1", "cloud", "projects"]) => {
            let b = json_body();
            let app = b["app_project_id"].as_str().unwrap_or("").to_string();
            let mut f = f.lock().unwrap();
            let created = !f.projects.contains_key(&app);
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
            if let Some(prev) = f.commits_by_key.get(&key) {
                return ok(200, prev.clone());
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
            let r = version(&mut f, base, &files, 0);
            f.commits_by_key.insert(key, r.clone());
            ok(201, r)
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

struct Env {
    _tmp: tempfile::TempDir,
    _rt: tokio::runtime::Runtime,
    st: AppState,
    fake: F,
    dir: PathBuf,
}

fn env() -> Env {
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
    secrets.set("kinetra-activation", "act-1").unwrap();
    st.cloud_service().set_test_env(&base, secrets, MACHINE);
    st.cloud_init();
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
    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn log(&self) -> Vec<String> {
        self.fake.lock().unwrap().log.clone()
    }

    fn clear_log(&self) {
        self.fake.lock().unwrap().log.clear();
    }

    fn puts(&self) -> usize {
        self.log().iter().filter(|l| l.starts_with("PUT")).count()
    }

    fn view(&self) -> CloudView {
        self.st.cloud_view()
    }

    /// Runs "Back up now" and waits for it to end.
    fn backup(&self) -> CloudView {
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

fn bytes(n: usize, seed: u8) -> Vec<u8> {
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
        let other = version(&mut f, None, &[], 0);
        let _ = other;
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
