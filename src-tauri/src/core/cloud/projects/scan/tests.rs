//! What goes into a backup: excludes, links, names the server refuses, case-only duplicates,
//! the size limit, and hashes (with the cache, cancel and the on-air wait).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn lists_project_files_and_leaves_out_the_excludes() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    for f in [
        "project.json",
        "screens/a.json",
        "templates/t/variants/default/template.html",
        "assets/logo.png",
        "assets/empty.txt",
        ".gitignore",
    ] {
        write(r, f, b"x");
    }
    for f in [
        ".backups/2026-10-15-auto/project.json",
        ".git/HEAD",
        "assets/.git/x",
        "screens/a.json.tmp-abc123",
        "screens/b.json.corrupt-20261015",
        ".kinetra-restore-xyz/project.json",
        "Thumbs.db",
        "assets/desktop.ini",
        "assets/.DS_Store",
        "~$notes.docx",
        ".~lock.sheet.ods#",
    ] {
        write(r, f, b"x");
    }
    // A nested ".backups" is a user folder, not our snapshots.
    write(r, "assets/.backups/keep.txt", b"x");
    let l = list(r, None).unwrap();
    let paths: Vec<&str> = l.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            ".gitignore",
            "assets/.backups/keep.txt",
            "assets/empty.txt",
            "assets/logo.png",
            "project.json",
            "screens/a.json",
            "templates/t/variants/default/template.html",
        ]
    );
    assert!(l.skipped.is_empty(), "{:?}", l.skipped);
}

#[test]
fn names_the_server_refuses_and_large_files_are_skipped_and_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(r, "project.json", b"{}");
    write(r, "assets/big.mov", &[0u8; 2048]);
    write(r, "assets/ok.png", &[0u8; 10]);
    let l = list(r, Some(1024)).unwrap();
    let paths: Vec<&str> = l.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["assets/ok.png", "project.json"]);
    let skipped: Vec<(&str, &str)> = l
        .skipped
        .iter()
        .map(|s| (s.path.as_str(), s.reason.as_str()))
        .collect();
    assert!(skipped.contains(&(
        "assets/big.mov",
        "It's larger than cloud backup allows for one file."
    )));
}

#[test]
#[cfg(windows)]
fn links_are_not_followed_or_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path().join("p");
    let outside = tmp.path().join("outside");
    write(&r, "project.json", b"{}");
    write(&outside, "secret.txt", b"x");
    // A junction needs no privilege (a symlink would).
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(r.join("assets"))
        .arg(&outside)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");
    let l = list(&r, None).unwrap();
    assert_eq!(l.files.len(), 1);
    assert_eq!(l.skipped.len(), 1);
    assert_eq!(l.skipped[0].path, "assets");
    assert!(l.skipped[0].reason.contains("link"));
}

#[test]
fn hashes_with_a_cache_by_path_size_and_mtime() {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    write(r, "a.txt", b"abc");
    write(r, "empty.txt", b"");
    let cancel = AtomicBool::new(false);
    let mut cache = HashCache::default();
    let mut read = 0u64;
    let l = list(r, None).unwrap();
    let (h, skipped) = hash_all(l.files, &mut cache, &|| true, &cancel, &mut |n, r| {
        if r {
            read += n + 1
        }
    })
    .unwrap();
    assert!(skipped.is_empty());
    assert_eq!(h[0].sha256, ABC);
    assert_eq!(h[1].sha256, EMPTY);
    assert_eq!(read, 3 + 1 + 1, "both files were read");
    // Unchanged: nothing is read again.
    let config = tempfile::tempdir().unwrap();
    cache.save(config.path(), "cid").unwrap();
    let mut cache = HashCache::load(config.path(), "cid");
    let mut read = 0u64;
    let l = list(r, None).unwrap();
    let (h2, _) = hash_all(l.files, &mut cache, &|| true, &cancel, &mut |n, r| {
        if r {
            read += n + 1
        }
    })
    .unwrap();
    assert_eq!(read, 0);
    assert_eq!(h2, h);
    // Changed (size): read again; a deleted file leaves the cache.
    write(r, "a.txt", b"abcd");
    fs::remove_file(r.join("empty.txt")).unwrap();
    let l = list(r, None).unwrap();
    let (h3, _) = hash_all(l.files, &mut cache, &|| true, &cancel, &mut |_, _| {}).unwrap();
    assert_eq!(h3.len(), 1);
    assert_ne!(h3[0].sha256, ABC);
    assert_eq!(cache.files.len(), 1);
}

#[test]
fn large_files_ask_the_gate_and_cancel_stops() {
    let tmp = tempfile::tempdir().unwrap();
    let big = tmp.path().join("big.bin");
    fs::write(&big, vec![7u8; (LARGE_FILE + 10) as usize]).unwrap();
    let cancel = AtomicBool::new(false);
    let asked = AtomicUsize::new(0);
    let mut buf = vec![0u8; BUF];
    let wait = || {
        asked.fetch_add(1, Ordering::Relaxed);
        true
    };
    sha256_file(&big, LARGE_FILE + 10, &mut buf, &wait, &cancel).unwrap();
    assert!(asked.load(Ordering::Relaxed) >= 16, "asked between chunks");
    // The gate says stop (canceled while on air).
    let r = sha256_file(&big, LARGE_FILE + 10, &mut buf, &|| false, &cancel);
    assert!(matches!(r, Err(HashError::Canceled)));
    // Small files never wait.
    let small = tmp.path().join("small.bin");
    fs::write(&small, b"abc").unwrap();
    assert_eq!(
        sha256_file(&small, 3, &mut buf, &|| false, &cancel).unwrap(),
        ABC
    );
}

#[test]
fn modified_at_is_rfc3339_utc() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(1_791_590_400_500_000_000), "2026-10-10T00:00:00.5Z");
    assert_eq!(hex(&[0, 15, 255]), "000fff");
}
