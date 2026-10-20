//! Restore end to end against the backup tests' fake of `/v1/cloud` and storage: a new folder (a
//! separate project), Download (the same cloud project on a computer without it), Replace (backed
//! up first, swapped, `restored_from` on the next commit), every file checked (a corrupt download
//! retried once, then reported damaged; Replace then changes nothing), local files reused instead
//! of downloaded, paths the server sends that may not be written, and the on-air pause.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::tests_backup::{bytes, env, Env};
use super::tests_service::wait_until;
use crate::core::model::{CloudRestoreMode, CloudRestoreRequest, CloudRestoreResult};

impl Env {
    fn server_id(&self) -> String {
        self.st.cloud_record().and_then(|r| r.server_id).unwrap()
    }

    fn head(&self) -> String {
        self.fake.lock().unwrap().head.clone().unwrap()
    }

    fn gets(&self) -> usize {
        self.log()
            .iter()
            .filter(|l| l.starts_with("GET blob"))
            .count()
    }

    /// Starts a restore and waits for its result.
    fn restore(
        &self,
        mode: CloudRestoreMode,
        version: Option<String>,
        folder: Option<&Path>,
    ) -> CloudRestoreResult {
        self.st
            .cloud_restore_start(CloudRestoreRequest {
                project_id: self.server_id(),
                version_id: version,
                mode,
                folder: folder.map(|f| f.to_string_lossy().into_owned()),
            })
            .unwrap();
        self.wait_restore()
    }

    fn wait_restore(&self) -> CloudRestoreResult {
        let t = Instant::now();
        while self.st.cloud_restore_running() {
            assert!(t.elapsed() < Duration::from_secs(20), "restore didn't end");
            std::thread::sleep(Duration::from_millis(10));
        }
        self.view().restore.unwrap().result.unwrap()
    }

    fn shows(&self) -> PathBuf {
        self.dir.parent().unwrap().to_path_buf()
    }

    /// Forgets this computer's hash cache, so nothing local is known (everything downloads).
    fn forget_hashes(&self) {
        let cid = self.st.project_cloud_id().unwrap().1.unwrap();
        let _ = std::fs::remove_file(super::scan::HashCache::path(self.st.config_dir(), &cid));
    }
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

fn cloud_id(dir: &Path) -> Option<String> {
    super::identity::cloud_id_at(dir)
}

fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".kinetra-restore-"))
        .collect()
}

#[test]
fn a_version_restores_into_a_new_folder_as_a_separate_project() {
    let e = env();
    e.write("assets/logo.png", &bytes(20_000, 1));
    e.write("assets/intro.mov", &bytes(150_000, 2));
    e.backup();
    let v1 = e.head();
    // The project moves on; the old version comes back next to it.
    e.write("assets/logo.png", &bytes(20_000, 9));
    e.backup();
    e.forget_hashes();
    e.clear_log();
    let to = e.shows().join("gala-old");
    let r = e.restore(CloudRestoreMode::NewFolder, Some(v1), Some(&to));
    assert!(r.ok, "{r:?}");
    assert!(r.damaged.is_empty());
    assert_eq!(read(&to.join("assets/logo.png")), bytes(20_000, 1));
    assert_eq!(read(&to.join("assets/intro.mov")), bytes(150_000, 2));
    assert!(to.join("project.json").is_file());
    // Each blob downloaded once and checked.
    assert_eq!(e.gets(), 4, "{:?}", e.log());
    // A separate project: its own cloud id, recorded for its folder.
    let new_id = cloud_id(&to).unwrap();
    assert_ne!(Some(new_id.clone()), cloud_id(&e.dir));
    assert_eq!(
        e.st.cloud_file().folders.get(&new_id).map(PathBuf::from),
        Some(to.clone())
    );
    // The open project is untouched; nothing is left behind.
    assert_eq!(read(&e.dir.join("assets/logo.png")), bytes(20_000, 9));
    assert!(leftovers(&e.shows()).is_empty());
}

#[test]
fn files_keep_the_time_they_were_backed_up_with() {
    let e = env();
    e.write("a.txt", b"hello");
    let before = std::fs::metadata(e.dir.join("a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    e.backup();
    e.forget_hashes();
    let to = e.shows().join("copy");
    assert!(e.restore(CloudRestoreMode::NewFolder, None, Some(&to)).ok);
    let after = std::fs::metadata(to.join("a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let diff = after
        .duration_since(before)
        .unwrap_or_else(|e| e.duration());
    assert!(diff < Duration::from_micros(1), "{diff:?}");
}

#[test]
fn local_files_with_the_right_hash_are_copied_not_downloaded() {
    let e = env();
    e.write("assets/intro.mov", &bytes(150_000, 2));
    e.write("a.txt", b"hello");
    e.backup();
    e.clear_log();
    let to = e.shows().join("copy");
    let r = e.restore(CloudRestoreMode::NewFolder, None, Some(&to));
    assert!(r.ok, "{r:?}");
    assert_eq!(e.gets(), 0, "{:?}", e.log());
    assert!(r.reused >= 2);
    assert_eq!(read(&to.join("assets/intro.mov")), bytes(150_000, 2));
}

#[test]
fn a_local_file_that_changed_since_it_was_hashed_is_downloaded() {
    let e = env();
    e.write("a.txt", b"hello");
    e.backup();
    // Same size, new bytes, and the cache's (size, mtime) still matches: only the re-hash after
    // copying notices.
    let p = e.dir.join("a.txt");
    let t = std::fs::metadata(&p).unwrap().modified().unwrap();
    std::fs::write(&p, b"HELLO").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(t)
        .unwrap();
    e.clear_log();
    let to = e.shows().join("copy");
    assert!(e.restore(CloudRestoreMode::NewFolder, None, Some(&to)).ok);
    assert_eq!(read(&to.join("a.txt")), b"hello");
    assert_eq!(e.gets(), 1, "{:?}", e.log());
}

#[test]
fn a_corrupt_download_is_retried_once_then_reported_damaged() {
    let e = env();
    e.write("a.txt", b"first");
    e.write("b.txt", b"second");
    e.write("c.txt", b"third");
    e.backup();
    e.forget_hashes();
    let sha = |b: &[u8]| super::scan::hex(&<sha2::Sha256 as sha2::Digest>::digest(b));
    {
        let mut f = e.fake.lock().unwrap();
        f.corrupt_gets.insert(sha(b"first"), 1); // fine on the retry
        f.corrupt_gets.insert(sha(b"second"), 2); // damaged
    }
    let to = e.shows().join("copy");
    let r = e.restore(CloudRestoreMode::NewFolder, None, Some(&to));
    // A new folder keeps the rest and names the damaged file.
    assert!(r.ok, "{r:?}");
    assert_eq!(r.damaged, vec!["b.txt".to_string()]);
    assert_eq!(r.damaged_count, 1);
    assert_eq!(read(&to.join("a.txt")), b"first");
    assert_eq!(read(&to.join("c.txt")), b"third");
    // A damaged file is never written, not even its wrong bytes.
    assert!(!to.join("b.txt").exists());
    assert!(leftovers(&e.shows()).is_empty());
}

#[test]
fn download_continues_the_same_cloud_project_on_a_computer_without_it() {
    let e = env();
    e.write("a.txt", b"hello");
    e.backup();
    let (pid, head) = (e.server_id(), e.head());
    let cid = cloud_id(&e.dir).unwrap();
    // This computer no longer has the project (another computer, in effect).
    e.st.close_project();
    std::fs::remove_dir_all(&e.dir).unwrap();
    let to = e.shows().join("gala-download");
    e.st.cloud_restore_start(CloudRestoreRequest {
        project_id: pid.clone(),
        version_id: None,
        mode: CloudRestoreMode::Download,
        folder: Some(to.to_string_lossy().into_owned()),
    })
    .unwrap();
    let r = e.wait_restore();
    assert!(r.ok, "{r:?}");
    assert_eq!(read(&to.join("a.txt")), b"hello");
    // The same cloud backup: same cloud id, the server id and the head it continues from.
    assert_eq!(cloud_id(&to), Some(cid.clone()));
    let rec = e.st.cloud_record_of(&cid).unwrap();
    assert_eq!(rec.server_id, Some(pid));
    assert_eq!(rec.head, Some(head));
    assert_eq!(
        e.st.cloud_file().folders.get(&cid).map(PathBuf::from),
        Some(to)
    );
}

#[test]
fn download_of_a_project_this_computer_has_is_a_separate_copy() {
    let e = env();
    e.write("a.txt", b"hello");
    e.backup();
    let to = e.shows().join("second");
    let r = e.restore(CloudRestoreMode::Download, None, Some(&to));
    assert!(r.ok, "{r:?}");
    assert_ne!(cloud_id(&to), cloud_id(&e.dir));
}

#[test]
fn replace_backs_up_first_swaps_the_files_and_the_next_backup_says_restored_from() {
    let e = env();
    e.write("assets/logo.png", &bytes(20_000, 1));
    e.write("keep.txt", b"same");
    e.backup();
    let v1 = e.head();
    e.write("assets/logo.png", &bytes(20_000, 9));
    e.write("new.txt", b"added later");
    let versions_before = e.fake.lock().unwrap().versions.len();
    let r = e.restore(CloudRestoreMode::Replace, Some(v1.clone()), None);
    assert!(r.ok, "{r:?}");
    assert_eq!(r.copy_folder, None);
    // The project as it was went to the cloud first.
    let versions = e.fake.lock().unwrap().versions.clone();
    assert_eq!(versions.len(), versions_before + 1);
    let safety = versions.last().unwrap()["manifest"]
        .as_array()
        .unwrap()
        .clone();
    assert!(safety.iter().any(|f| f["path"] == "new.txt"));
    // The version's files are back; a file it doesn't have is gone.
    assert_eq!(read(&e.dir.join("assets/logo.png")), bytes(20_000, 1));
    assert_eq!(read(&e.dir.join("keep.txt")), b"same");
    assert!(!e.dir.join("new.txt").exists());
    assert!(leftovers(&e.dir).is_empty());
    // Still the same project, and it says where it came from on the next backup.
    assert_eq!(e.st.current_project().unwrap().name, "Gala");
    let cid = cloud_id(&e.dir).unwrap();
    assert_eq!(
        e.st.cloud_record_of(&cid).unwrap().restored_from,
        Some(v1.clone())
    );
    e.backup();
    assert_eq!(e.fake.lock().unwrap().restored.last(), Some(&json!(v1)));
    assert_eq!(e.st.cloud_record_of(&cid).unwrap().restored_from, None);
}

#[test]
fn replace_copies_the_project_first_when_it_cant_back_up() {
    let e = env();
    e.write("a.txt", b"old");
    e.backup();
    let v1 = e.head();
    e.write("a.txt", b"new");
    // The plan lapsed: read-only, so no backup; the project is copied next to it instead.
    e.fake.lock().unwrap().quota_state = "lapsed".into();
    e.rt_block(e.st.cloud_refresh(false)).unwrap();
    let r = e.restore(CloudRestoreMode::Replace, Some(v1), None);
    assert!(r.ok, "{r:?}");
    let copy = PathBuf::from(r.copy_folder.unwrap());
    assert_eq!(read(&copy.join("a.txt")), b"new");
    assert_eq!(read(&e.dir.join("a.txt")), b"old");
}

#[test]
fn replace_with_a_damaged_file_changes_nothing() {
    let e = env();
    e.write("a.txt", b"old");
    e.write("b.txt", b"kept");
    e.backup();
    let v1 = e.head();
    e.write("a.txt", b"newer");
    let sha = super::scan::hex(&<sha2::Sha256 as sha2::Digest>::digest(b"old"));
    e.fake.lock().unwrap().corrupt_gets.insert(sha, 2);
    let r = e.restore(CloudRestoreMode::Replace, Some(v1), None);
    assert!(!r.ok);
    assert_eq!(r.damaged, vec!["a.txt".to_string()]);
    assert!(r.error.unwrap().starts_with("Nothing was replaced"));
    assert_eq!(read(&e.dir.join("a.txt")), b"newer");
    assert_eq!(read(&e.dir.join("b.txt")), b"kept");
    assert!(leftovers(&e.dir).is_empty());
}

#[test]
fn replace_is_refused_on_air() {
    let e = env();
    e.write("a.txt", b"x");
    e.backup();
    let _obs = e.st.live().renderer_attach("main", true);
    let r = e.st.cloud_restore_start(CloudRestoreRequest {
        project_id: e.server_id(),
        version_id: None,
        mode: CloudRestoreMode::Replace,
        folder: None,
    });
    assert!(r.is_err());
    assert!(!e.st.cloud_restore_running());
}

#[test]
fn downloads_wait_while_on_air() {
    let e = env();
    e.write("a.txt", b"hello");
    e.backup();
    e.forget_hashes();
    e.clear_log();
    let obs = e.st.live().renderer_attach("main", true);
    let to = e.shows().join("copy");
    e.st.cloud_restore_start(CloudRestoreRequest {
        project_id: e.server_id(),
        version_id: None,
        mode: CloudRestoreMode::NewFolder,
        folder: Some(to.to_string_lossy().into_owned()),
    })
    .unwrap();
    wait_until("paused", || e.view().restore.is_some_and(|r| r.paused));
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(e.gets(), 0);
    drop(obs);
    let r = e.wait_restore();
    assert!(r.ok, "{r:?}");
    assert!(e.gets() >= 1);
}

#[test]
fn paths_the_server_sends_that_may_not_be_written_are_refused() {
    let e = env();
    e.write("a.txt", b"hello");
    e.backup();
    e.forget_hashes();
    {
        // A version listing a path outside the folder and a name a backup never holds.
        let mut f = e.fake.lock().unwrap();
        let v = f.versions.last_mut().unwrap();
        let mut files: Vec<Value> = v["manifest"].as_array().unwrap().clone();
        let a = files[0].clone();
        for bad in ["../evil.txt", ".git/config"] {
            let mut x = a.clone();
            x["path"] = json!(bad);
            files.push(x);
        }
        v["manifest"] = json!(files);
    }
    let to = e.shows().join("copy");
    let r = e.restore(CloudRestoreMode::NewFolder, None, Some(&to));
    assert_eq!(
        r.damaged,
        vec!["../evil.txt".to_string(), ".git/config".to_string()]
    );
    assert!(!e.shows().join("evil.txt").exists());
    assert!(!to.join(".git").exists());
    assert_eq!(read(&to.join("a.txt")), b"hello");
}

#[test]
fn a_folder_that_isnt_empty_is_refused_before_anything_starts() {
    let e = env();
    e.write("a.txt", b"hello");
    e.backup();
    let to = e.shows().join("busy");
    std::fs::create_dir_all(&to).unwrap();
    std::fs::write(to.join("x"), b"x").unwrap();
    let r = e.st.cloud_restore_start(CloudRestoreRequest {
        project_id: e.server_id(),
        version_id: None,
        mode: CloudRestoreMode::NewFolder,
        folder: Some(to.to_string_lossy().into_owned()),
    });
    assert!(r.is_err());
    assert!(e.view().restore.is_none());
}

#[test]
fn the_plan_suggests_a_free_folder_next_to_the_project() {
    let e = env();
    let p =
        e.st.cloud_restore_plan(
            CloudRestoreMode::NewFolder,
            "Gala",
            Some("2026-10-15T14:02:00Z"),
        )
        .unwrap();
    let f = PathBuf::from(p.folder);
    assert_eq!(f.parent(), Some(e.shows().as_path()));
    assert!(f
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("gala-2026-10-1"));
    assert!(!f.exists());
    // Replace: backed up first (Pro, backup on, writable).
    e.write("a.txt", b"x");
    e.backup();
    let p =
        e.st.cloud_restore_plan(CloudRestoreMode::Replace, "Gala", None)
            .unwrap();
    assert!(p.backup_first);
    assert_eq!(p.copy_folder, None);
}
