//! Manifest paths (CLOUD_PROJECTS.md §8.2), the same rules as the server's `path_problem`
//! (kinetra-cloud `crates/core/src/cloud/manifest.rs`), so a file that can't be stored is
//! skipped and listed here instead of failing the whole commit.
//!
//! NFC: the server checks it with a Unicode table this app doesn't carry. A name made only of
//! characters below U+0300 is always NFC; anything else is sent as it is, and if the server
//! refuses it (`400 invalid_path`), that one file is dropped and listed (`upload`).

use std::collections::HashSet;

pub const MAX_PATH_BYTES: usize = 1024;
pub const MAX_SEGMENT_BYTES: usize = 255;

const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn is_reserved_name(segment: &str) -> bool {
    let stem = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return true;
    }
    ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix)
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
    })
}

/// `None` if `path` can be stored, else why not (operator words; shown in the report).
pub fn problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("empty path");
    }
    if path.len() > MAX_PATH_BYTES {
        return Some("The path is too long.");
    }
    if path.starts_with('/') {
        return Some("absolute path");
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Some("The path has an empty or relative part.");
        }
        if segment.len() > MAX_SEGMENT_BYTES {
            return Some("A name in the path is too long.");
        }
        if segment
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        {
            return Some("The name has a character Windows can't store.");
        }
        if segment.ends_with(' ') || segment.ends_with('.') {
            return Some("The name ends with a space or a dot.");
        }
        if is_reserved_name(segment) {
            return Some("The name is reserved on Windows.");
        }
    }
    None
}

/// Paths unique ignoring case (as the server compares them: Unicode lowercase). Feed paths in
/// order; a later one that matches an earlier one is a duplicate.
#[derive(Default)]
pub struct Unique(HashSet<String>);

impl Unique {
    /// `true` the first time this path (ignoring case) is seen.
    pub fn insert(&mut self, path: &str) -> bool {
        self.0.insert(path.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storable_paths() {
        for ok in [
            "project.json",
            "assets/intro.mov",
            "templates/abc/variants/default/template.html",
            "assets/Æøå logo.png",
            "CONSOLE.txt",
            "COM10.txt",
            "a.b.c",
        ] {
            assert_eq!(problem(ok), None, "{ok}");
        }
    }

    #[test]
    fn paths_the_server_refuses() {
        for bad in [
            "",
            "/abs",
            "a//b",
            "a/./b",
            "../x",
            "a\\b",
            "a:b",
            "what?.png",
            "star*.png",
            "tab\there",
            "trailing.",
            "trailing ",
            "dir./file",
            "CON",
            "con.txt",
            "Aux.json",
            "nul",
            "COM1.log",
            "lpt9",
        ] {
            assert!(problem(bad).is_some(), "{bad:?}");
        }
        assert!(problem(&"a".repeat(256)).is_some());
        assert!(problem(&format!("{}/x", "a/".repeat(600))).is_some());
    }

    #[test]
    fn case_only_duplicates() {
        let mut u = Unique::default();
        assert!(u.insert("assets/Logo.png"));
        assert!(!u.insert("assets/logo.png"));
        assert!(u.insert("assets/logo2.png"));
        assert!(u.insert("Ærø.txt"));
        assert!(!u.insert("ærø.txt"));
    }
}
