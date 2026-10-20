//! Live cloud backup check against the dev API (run by hand, only with the owner's OK):
//! a throwaway project "Claude live test" backs up through the real API and storage.
//!
//! ```text
//! KINETRA_TRUSTED_KEYS=src-tauri/license/dev-trusted-keys.json KINETRA_DEV_PROFILE=dev \
//!   KINETRA_API_BASE=https://vps-b17d613f.tail33aa62.ts.net:8445 cargo run --example cloud_live
//! ```
//! Build with `KINETRA_CHANNEL=canary` when the dev app is a canary build (the keychain service
//! follows the channel).
//!
//! Signs in as the dev profile does: its session is read from that profile's keychain (`Kinetra
//! [Canary ]dev-<profile>`), one license sync activates and stores the license token there (as
//! the app would), and its `license.json` is copied into a temporary config folder. Checks: a first backup (a 20 MB file in parts, a decomposed name in
//! NFC), a second with no changes sends nothing, a changed file sends only itself, a multipart
//! upload stopped halfway resumes, nothing goes up while a Program renderer is connected, and no
//! URL, signature or hash reaches the log. Then the test project goes to the trash (it counts
//! toward storage until it's purged after 7 days). Prints only ids, sizes and states.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kinetra_lib::core::cloud::api::Api;
use kinetra_lib::core::cloud::config;
use kinetra_lib::core::events::NullSink;
use kinetra_lib::core::secrets::{KeyringSecrets, SecretStore};
use kinetra_lib::core::state::AppState;

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Log {
    type Writer = Log;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn check(ok: bool, what: &str) {
    println!("{} {what}", if ok { "PASS" } else { "FAIL" });
    if !ok {
        std::process::exit(1);
    }
}

fn wait_done(st: &AppState, limit: Duration) {
    let t = Instant::now();
    while st.cloud_backup_running() {
        if t.elapsed() > limit {
            check(false, "the backup finished in time");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn pseudo(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

fn main() {
    let profile = std::env::var("KINETRA_DEV_PROFILE").unwrap_or_default();
    let base = config::api_base();
    if profile.is_empty() || base == config::API_BASE {
        eprintln!("Set KINETRA_DEV_PROFILE and KINETRA_API_BASE (the dev API).");
        std::process::exit(2);
    }
    let log = Log::default();
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(log.clone())
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .finish(),
    )
    .unwrap();

    let tmp = std::env::temp_dir().join(format!("kinetra-cloud-live-{}", std::process::id()));
    let config_dir = tmp.join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    let appdata = std::env::var("APPDATA").unwrap();
    let theirs = Path::new(&appdata)
        .join("no.rorvikmedieteknikk.kinetra")
        .join(format!("dev-{profile}"))
        .join("license.json");
    std::fs::copy(&theirs, config_dir.join("license.json"))
        .expect("the dev profile's license.json");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let st = AppState::load(config_dir.clone(), Arc::new(NullSink)).expect("state");
    st.live().set_runtime(rt.handle().clone());
    st.cloud_init();
    // Activate / refresh with the dev key trusted (stores the license token in the profile's
    // keychain, as the app does).
    let reached = rt.block_on(st.cloud_sync());
    println!("license sync reached the server: {reached}");
    let lv = st.license_view();
    println!(
        "license: status={} plan={:?} cloud={}",
        lv.status, lv.plan, lv.features.cloud
    );
    check(lv.features.cloud, "the dev license has cloud backup");
    let view = rt.block_on(st.cloud_refresh(false)).expect("quota");
    let q = view.quota.clone().unwrap();
    println!(
        "quota: state={:?} used={} of {} today={} of {}",
        view.state, q.used_bytes, q.limit_bytes, q.upload_today_used, q.upload_today_limit
    );

    // Content unique to this run: storage keeps blobs per account (a trashed project's too), so
    // the same bytes as an earlier run would never be sent again.
    let run = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u32)
        | 1;
    // The test project.
    let parent = tmp.join("shows");
    std::fs::create_dir_all(&parent).unwrap();
    let st2 = st.clone();
    let p2 = parent.clone();
    let dir = rt.block_on(async move {
        tokio::task::spawn_blocking(move || st2.create_project(&p2, "Claude live test").unwrap())
            .await
            .unwrap()
    });
    let dir = std::path::PathBuf::from(dir.path);
    write(&dir, "assets/small.bin", &pseudo(50_000, run ^ 1));
    write(&dir, "assets/la\u{030A}vt.txt", b"decomposed name");
    write(&dir, "assets/big.bin", &pseudo(20_000_000, run ^ 2));

    // A multipart upload stopped after its first part, then resumed.
    st.cloud_backup_now().expect("backup starts");
    let t = Instant::now();
    let cloud_json = config_dir.join("cloud.json");
    loop {
        let text = std::fs::read_to_string(&cloud_json).unwrap_or_default();
        if text.contains("\"etag\"") {
            break;
        }
        if t.elapsed() > Duration::from_secs(300) || !st.cloud_backup_running() {
            check(false, "a part was stored and recorded");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let st3 = st.clone();
    rt.block_on(async move {
        tokio::task::spawn_blocking(move || st3.close_project())
            .await
            .unwrap()
    });
    check(
        !st.cloud_backup_running(),
        "closing the project stopped the backup",
    );
    let st4 = st.clone();
    let d4 = dir.clone();
    rt.block_on(async move {
        tokio::task::spawn_blocking(move || st4.open_project(&d4).unwrap())
            .await
            .unwrap()
    });
    let t = Instant::now();
    st.cloud_backup_now().expect("backup starts again");
    wait_done(&st, Duration::from_secs(600));
    let v = st.cloud_view().project.unwrap();
    println!(
        "backup 1: {:.1} s, error={:?}, skipped={}",
        t.elapsed().as_secs_f64(),
        v.error,
        v.skipped
    );
    check(
        v.error.is_none() && v.last_backup_at.is_some(),
        "the first backup finished",
    );
    let text = std::fs::read_to_string(&cloud_json).unwrap();
    check(
        !text.contains("\"etag\""),
        "the resumed upload completed (no parts left)",
    );

    // The server's view of it.
    let secrets = KeyringSecrets;
    let session = secrets.get("kinetra-session").unwrap().expect("session");
    let api = Api::new(base.clone()).unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    let pid = file["accounts"]
        .as_object()
        .unwrap()
        .values()
        .find_map(|a| {
            a["projects"]
                .as_object()
                .unwrap()
                .values()
                .find_map(|p| p["serverId"].as_str().map(str::to_string))
        })
        .expect("server id");
    let versions = |api: &Api| {
        rt.block_on(api.cloud_versions(&session, &pid, None, 50))
            .unwrap()
            .versions
    };
    let vs = versions(&api);
    let manifest = rt
        .block_on(api.cloud_manifest(&session, &pid, &vs[0].id))
        .unwrap()
        .manifest;
    let names: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    println!(
        "version 1: {} files, new_bytes={}",
        vs[0].file_count, vs[0].new_bytes
    );
    check(
        names.contains(&"assets/l\u{e5}vt.txt"),
        "a decomposed name went up in NFC",
    );
    check(
        names.contains(&"assets/big.bin"),
        "the 20 MB file is in the version",
    );

    // No changes: nothing new.
    st.cloud_backup_now().unwrap();
    wait_done(&st, Duration::from_secs(120));
    let vs = versions(&api);
    println!("version 2: new_bytes={}", vs[0].new_bytes);
    check(
        vs[0].new_bytes == 0,
        "a backup with no changes uploads nothing",
    );

    // One changed file.
    std::thread::sleep(Duration::from_millis(20));
    write(&dir, "assets/small.bin", &pseudo(50_000, run ^ 3));
    st.cloud_backup_now().unwrap();
    wait_done(&st, Duration::from_secs(120));
    let vs = versions(&api);
    println!("version 3: new_bytes={}", vs[0].new_bytes);
    check(
        vs[0].new_bytes == 50_000,
        "only the changed file was uploaded",
    );

    // On air: nothing goes up until it's off.
    write(&dir, "assets/onair.bin", &pseudo(30_000, run ^ 4));
    let obs = st.live().renderer_attach("main", true);
    st.cloud_backup_now().unwrap();
    let t = Instant::now();
    while !st
        .cloud_view()
        .project
        .and_then(|p| p.run)
        .is_some_and(|r| r.paused)
    {
        if t.elapsed() > Duration::from_secs(60) {
            check(false, "the backup paused while on air");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_secs(2));
    let before = versions(&api).len();
    check(st.cloud_backup_running(), "still waiting while on air");
    drop(obs);
    wait_done(&st, Duration::from_secs(120));
    check(
        versions(&api).len() == before + 1,
        "it backed up once off air",
    );

    // Logs.
    let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let leaks = [
        "X-Amz",
        "x-amz-signature",
        "kntr_s_",
        &manifest.files[0].sha256,
    ];
    check(
        text.contains("storage PUT") && leaks.iter().all(|l| !text.contains(l)),
        "no URL, signature, session or hash in the log",
    );

    // The history, through the app.
    let page = rt.block_on(st.cloud_versions(None)).unwrap();
    println!("history: {} versions", page.versions.len());
    check(page.versions.len() >= 4 && page.versions[0].is_head, "the history lists every version, newest first");
    let middle = page.versions[1].id.clone();
    let oldest = page.versions.last().unwrap().id.clone();
    let v = rt.block_on(st.cloud_version_keep(&middle, true)).unwrap();
    check(v.kept, "a version can be kept");
    let v = rt
        .block_on(st.cloud_version_label(&middle, Some("Claude live test".into())))
        .unwrap();
    check(v.label.as_deref() == Some("Claude live test"), "a version can be labeled");
    let head = page.versions[0].id.clone();
    check(
        rt.block_on(st.cloud_version_delete(&head)).is_err(),
        "the newest version can't be deleted",
    );
    rt.block_on(st.cloud_version_delete(&oldest)).unwrap();
    let after = rt.block_on(st.cloud_versions(None)).unwrap();
    check(
        after.versions.len() == page.versions.len() - 1,
        "an older version can be deleted",
    );
    let list = rt.block_on(st.cloud_projects_list(false)).unwrap();
    check(
        list.iter().any(|p| p.id == pid && p.open && p.here),
        "the project list shows this project",
    );
    let renamed = rt
        .block_on(st.cloud_project_rename(&pid, "Claude live test (renamed)"))
        .unwrap();
    check(renamed.name == "Claude live test (renamed)", "a cloud project can be renamed");

    // Clean up: the test project goes to the trash (through the app: the open project asks).
    let trashed = rt.block_on(st.cloud_project_trash(&pid)).unwrap();
    println!("trashed: purge_at={:?}", trashed.purge_at);
    check(
        st.cloud_view()
            .project
            .is_some_and(|p| p.question.is_some()),
        "the open project asks what to do after its backup went to the trash",
    );
    let view = rt.block_on(st.cloud_refresh(false)).unwrap();
    let q = view.quota.unwrap();
    println!(
        "quota after: used={} today={}",
        q.used_bytes, q.upload_today_used
    );
    st.cloud_projects_stop();
    let _ = std::fs::remove_dir_all(&tmp);
    println!("done");
}
