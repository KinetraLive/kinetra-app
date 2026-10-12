//! Pure operations on the recent-projects list. Persistence and locking live in `state`.

use std::path::Path;

use crate::core::fsx;
use crate::core::model::{RecentProject, RECENTS_CAP};

/// Comparison key for a path: verbatim prefix stripped, trailing separators stripped and, on
/// Windows, `/` turned into `\` and compared case-insensitively.
pub fn normalize(path: &str) -> String {
    let s = fsx::strip_verbatim(path);
    if cfg!(windows) {
        let s = s.replace('/', "\\").to_lowercase();
        s.trim_end_matches('\\').to_string()
    } else {
        let t = s.trim_end_matches('/');
        if t.is_empty() && !s.is_empty() {
            "/".into()
        } else {
            t.to_string()
        }
    }
}

/// True when `a` and `b` name the same folder (by [`normalize`]).
pub fn same_path(a: &str, b: &str) -> bool {
    normalize(a) == normalize(b)
}

/// Moves `path` to the front (inserting it if absent), updating its name and time. Duplicates by
/// normalized path are removed and the list is capped at `RECENTS_CAP`.
pub fn promote(list: &mut Vec<RecentProject>, path: &str, name: &str, now: &str) {
    let key = normalize(path);
    list.retain(|r| normalize(&r.path) != key);
    list.insert(
        0,
        RecentProject {
            path: path.to_string(),
            name: name.to_string(),
            last_opened_at: now.to_string(),
            missing: false,
        },
    );
    list.truncate(RECENTS_CAP);
}

/// Removes every entry for `path`. Returns whether anything was removed.
pub fn remove(list: &mut Vec<RecentProject>, path: &str) -> bool {
    let key = normalize(path);
    let before = list.len();
    list.retain(|r| normalize(&r.path) != key);
    list.len() != before
}

/// Copy of `list` with `missing` filled in: an entry is present when its folder exists and holds
/// a `project.json`. Touches the disk once or twice per entry; run it off the UI thread.
pub fn check_missing(list: &[RecentProject]) -> Vec<RecentProject> {
    list.iter()
        .map(|r| {
            let dir = Path::new(&r.path);
            let present = dir.is_dir() && dir.join("project.json").is_file();
            RecentProject {
                missing: !present,
                ..r.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(list: &[RecentProject]) -> Vec<&str> {
        list.iter().map(|r| r.path.as_str()).collect()
    }

    #[test]
    fn promote_inserts_and_moves_to_front() {
        let mut l = Vec::new();
        promote(&mut l, "/a", "A", "t1");
        promote(&mut l, "/b", "B", "t2");
        assert_eq!(paths(&l), ["/b", "/a"]);
        promote(&mut l, "/a", "A2", "t3");
        assert_eq!(paths(&l), ["/a", "/b"]);
        assert_eq!(l[0].name, "A2");
        assert_eq!(l[0].last_opened_at, "t3");
    }

    #[test]
    fn promote_dedupes_trailing_separator() {
        let mut l = Vec::new();
        promote(&mut l, "/a/", "A", "t1");
        promote(&mut l, "/a", "A", "t2");
        assert_eq!(paths(&l), ["/a"]);
    }

    #[cfg(windows)]
    #[test]
    fn promote_dedupes_windows_variants() {
        let mut l = Vec::new();
        promote(&mut l, r"C:\Shows\Main", "A", "t1");
        promote(&mut l, "c:/shows/main/", "A", "t2");
        promote(&mut l, r"\\?\C:\SHOWS\MAIN", "A", "t3");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].last_opened_at, "t3");
        assert!(same_path(r"C:\x\", "c:/X"));
    }

    #[test]
    fn promote_caps_list() {
        let mut l = Vec::new();
        for i in 0..15 {
            promote(&mut l, &format!("/p{i}"), "P", "t");
        }
        assert_eq!(l.len(), RECENTS_CAP);
        assert_eq!(l[0].path, "/p14");
        assert_eq!(l[RECENTS_CAP - 1].path, "/p5");
    }

    #[test]
    fn remove_entry() {
        let mut l = Vec::new();
        promote(&mut l, "/a", "A", "t");
        promote(&mut l, "/b", "B", "t");
        assert!(remove(&mut l, "/a/"));
        assert_eq!(paths(&l), ["/b"]);
        assert!(!remove(&mut l, "/zzz"));
    }

    #[test]
    fn check_missing_stats_paths() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("p");
        crate::core::project::init_in(&proj, "P").unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        let gone = dir.path().join("gone");

        let mut l = Vec::new();
        for p in [&proj, &plain, &gone] {
            promote(&mut l, &p.to_string_lossy(), "x", "t");
        }
        let checked = check_missing(&l);
        let missing: Vec<bool> = checked.iter().map(|r| r.missing).collect();
        assert_eq!(missing, [true, true, false]);
        assert!(l.iter().all(|r| !r.missing));
    }
}
