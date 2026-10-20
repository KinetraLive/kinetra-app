//! Small filesystem helpers: atomic JSON writes, timestamps and user-facing path display, plus the
//! registry of Kinetra's own recent writes that the project watcher ignores.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Writes `value` as pretty JSON (2-space indent, trailing newline) to `path` atomically: the
/// bytes go to `<file>.tmp-<id>` in the same directory, are synced, and the temp file is renamed
/// over the target. On any error the temp file is removed and the target is left untouched.
pub fn write_json_atomic<T: Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    write_bytes_atomic(path, &bytes)
}

/// Pauses between rename attempts; the total wait stays under ~0.3 s.
const RENAME_BACKOFF_MS: &[u64] = &[10, 20, 40, 80, 160];

/// `fs::rename`, retried briefly on `PermissionDenied`. On Windows a file or folder that was just
/// written can be held open for a moment by antivirus or the search indexer, and renaming it (or
/// replacing a target that is held) fails with "access denied" until the handle closes.
pub fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    let mut waits = RENAME_BACKOFF_MS.iter();
    loop {
        match fs::rename(from, to) {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => match waits.next() {
                Some(ms) => std::thread::sleep(Duration::from_millis(*ms)),
                None => return Err(e),
            },
            other => return other,
        }
    }
}

/// Atomic write of raw bytes, same guarantees as [`write_json_atomic`].
pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(".tmp-{}", nanoid::nanoid!(10)));
    let tmp = path.with_file_name(tmp_name);

    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        rename_own(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

// ---- Own writes ------------------------------------------------------------------------------

/// How long a recorded own write or removal is remembered. It must outlast the watcher's longest
/// batch ([`crate::core::watch::DEBOUNCE_MAX`] plus the quiet period) with a wide margin for a
/// watcher thread that is scheduled late on a loaded machine: an own write seen after its record
/// expired is reported as an external edit. A file write only matches while the file keeps the
/// exact mtime it was written with, so a long memory never hides a later external edit of it.
pub const OWN_WRITE_TTL: Duration = Duration::from_secs(10);

/// What Kinetra did to a path.
#[derive(Clone, Copy)]
enum Own {
    /// Wrote this file; it had this mtime right after the write.
    Write(SystemTime),
    /// Put this folder in place. A folder's mtime changes whenever an entry inside it is
    /// created or replaced (including Kinetra's own atomic writes into it), so it is not
    /// compared: the files inside report themselves.
    Folder,
    /// Removed it (a file, or a folder with everything in it).
    Removal,
}

struct OwnChange {
    path: PathBuf,
    what: Own,
    at: Instant,
}

/// Process-wide: every atomic write records itself here, so the project watcher can tell
/// Kinetra's own changes from external edits. Entries expire after [`OWN_WRITE_TTL`]; the list is
/// pruned on every access, so it stays tiny.
static OWN_CHANGES: Mutex<Vec<OwnChange>> = Mutex::new(Vec::new());

fn own_changes() -> MutexGuard<'static, Vec<OwnChange>> {
    let mut g = OWN_CHANGES.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    g.retain(|c| now.duration_since(c.at) < OWN_WRITE_TTL);
    g
}

/// How `path` looks now: `Some(Folder)` for a folder, `Some(Write(mtime))` for a file, `None`
/// when it is gone (or unreadable).
fn current(path: &Path) -> Option<Own> {
    let meta = fs::metadata(path).ok()?;
    if meta.is_dir() {
        Some(Own::Folder)
    } else {
        meta.modified().ok().map(Own::Write)
    }
}

fn push(path: &Path, what: Own) {
    own_changes().push(OwnChange {
        path: path.to_path_buf(),
        what,
        at: Instant::now(),
    });
    call_change_hooks(path);
}

/// Told about every change Kinetra makes to a file or folder (after it is recorded, before or
/// after it hits the disk). Must be cheap and must not block: called on every write.
pub type ChangeHook = dyn Fn(&Path) + Send + Sync;

/// Process-wide (several `AppState`s in tests): held weakly, so a dropped owner unregisters.
static CHANGE_HOOKS: Mutex<Vec<std::sync::Weak<ChangeHook>>> = Mutex::new(Vec::new());

/// Registers `hook` for every own change (see [`ChangeHook`]) for as long as the `Arc` lives.
pub fn add_change_hook(hook: &std::sync::Arc<ChangeHook>) {
    let mut g = CHANGE_HOOKS.lock().unwrap_or_else(PoisonError::into_inner);
    g.retain(|w| w.strong_count() > 0);
    g.push(std::sync::Arc::downgrade(hook));
}

fn call_change_hooks(path: &Path) {
    let hooks: Vec<_> = {
        let g = CHANGE_HOOKS.lock().unwrap_or_else(PoisonError::into_inner);
        if g.is_empty() {
            return;
        }
        g.iter().filter_map(std::sync::Weak::upgrade).collect()
    };
    for h in hooks {
        h(path);
    }
}

/// Records that Kinetra removed or is about to remove `path` (a file, or a folder with everything
/// in it). Call it BEFORE the removal: the watcher may look at the path the moment it is gone, and
/// a record of a path that still exists matches nothing.
pub fn record_own_removal(path: &Path) {
    push(path, Own::Removal);
}

/// `fs::rename` (retried like [`rename_retrying`]) as Kinetra's own change. The record is made
/// BEFORE the rename, from `from`'s metadata (a rename keeps the mtime): recorded afterwards, a
/// watcher thread that looks at `to` between the rename and the record would take the write for
/// an external edit and reload over newer in-memory state.
pub fn rename_own(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(what) = current(from) {
        push(to, what);
    }
    rename_retrying(from, to)
}

/// `fs::remove_file` as Kinetra's own change (recorded first, see [`record_own_removal`]).
pub fn remove_file_own(path: &Path) -> io::Result<()> {
    record_own_removal(path);
    fs::remove_file(path)
}

/// `fs::remove_dir_all` as Kinetra's own change (recorded first, see [`record_own_removal`]).
pub fn remove_dir_all_own(path: &Path) -> io::Result<()> {
    record_own_removal(path);
    fs::remove_dir_all(path)
}

/// Whether the current state of `path` is explained by a recent own change: a file with the same
/// mtime as right after the own write, a folder Kinetra put in place, or a removal (of it, or of a
/// folder containing it) while it is still gone. Anything else is an external change.
pub fn is_own_change(path: &Path) -> bool {
    let now = current(path);
    own_changes().iter().any(|c| match (c.what, now) {
        (Own::Write(t), Some(Own::Write(m))) => c.path == path && t == m,
        (Own::Folder, Some(Own::Folder)) => c.path == path,
        (Own::Removal, None) => path.starts_with(&c.path),
        _ => false,
    })
}

fn now_utc_seconds() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(0).unwrap_or(now)
}

/// Milliseconds since the Unix epoch (0 if the clock is before 1970).
pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Current UTC time as RFC 3339 with second precision, e.g. `2026-10-03T17:12:00Z`.
pub fn now_rfc3339() -> String {
    let now = now_utc_seconds();
    now.format(&Rfc3339).unwrap_or_else(|_| compact(now))
}

/// Current UTC time in a filename-safe compact form, e.g. `20261003T171200Z`.
pub fn now_compact() -> String {
    compact(now_utc_seconds())
}

fn compact(t: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// Makes `path` absolute without touching the filesystem and without the Windows verbatim
/// (`\\?\`) prefix that `canonicalize` adds.
pub fn absolute(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    PathBuf::from(display_path(&abs))
}

/// A path as it should be shown to the user: the Windows verbatim prefix (`\\?\`, `\\?\UNC\`) is
/// stripped.
pub fn display_path(path: &Path) -> String {
    strip_verbatim(&path.to_string_lossy()).to_string()
}

/// Strips a leading `\\?\` (or turns `\\?\UNC\server\share` into `\\server\share`).
pub fn strip_verbatim(s: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        std::borrow::Cow::Owned(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        std::borrow::Cow::Borrowed(rest)
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// App-config folder name used before the rename to Kinetra (it is the old bundle identifier).
pub const LEGACY_CONFIG_DIR_NAME: &str = "com.rorvikmedieteknikk.broadcast-app";
/// Files carried over from the legacy config folder.
pub const LEGACY_CONFIG_FILES: &[&str] = &["prefs.json", "recents.json"];

/// One-time carry-over after the rename: copies `prefs.json` and `recents.json` from the legacy
/// config folder (a sibling of `config_dir`) into `config_dir`, each only if it doesn't exist there
/// yet. The legacy folder is never modified. Failures are logged, never fatal.
pub fn migrate_legacy_config(config_dir: &Path) {
    let Some(parent) = config_dir.parent() else {
        return;
    };
    match copy_missing(
        &parent.join(LEGACY_CONFIG_DIR_NAME),
        config_dir,
        LEGACY_CONFIG_FILES,
    ) {
        Ok(copied) if !copied.is_empty() => {
            tracing::info!("carried over {copied:?} from the previous app folder");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("could not carry over settings from the previous app folder: {e}"),
    }
}

/// Copies each of `files` from `from` to `to` when it exists in `from` and not in `to`. Returns the
/// names copied. Does nothing when `from` doesn't exist.
pub fn copy_missing(
    from: &Path,
    to: &Path,
    files: &[&'static str],
) -> io::Result<Vec<&'static str>> {
    if !from.is_dir() {
        return Ok(Vec::new());
    }
    let mut copied = Vec::new();
    for &name in files {
        let src = from.join(name);
        let dst = to.join(name);
        if src.is_file() && !dst.exists() {
            fs::create_dir_all(to)?;
            write_bytes_atomic(&dst, &fs::read(&src)?)?;
            copied.push(name);
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_config_is_copied_once_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        let old = root.path().join(LEGACY_CONFIG_DIR_NAME);
        let new = root.path().join("no.rorvikmedieteknikk.kinetra");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join("prefs.json"),
            r#"{"schemaVersion":1,"theme":"dark"}"#,
        )
        .unwrap();
        fs::write(
            old.join("recents.json"),
            r#"{"schemaVersion":1,"projects":[]}"#,
        )
        .unwrap();
        fs::write(old.join("broadcast.log"), "log").unwrap();

        migrate_legacy_config(&new);
        assert!(new.join("prefs.json").is_file());
        assert!(new.join("recents.json").is_file());
        assert!(
            !new.join("broadcast.log").exists(),
            "only prefs and recents move"
        );
        assert!(
            old.join("prefs.json").is_file(),
            "the legacy folder is left untouched"
        );

        // A later start must not overwrite what the new app has since written.
        fs::write(
            new.join("prefs.json"),
            r#"{"schemaVersion":1,"theme":"light"}"#,
        )
        .unwrap();
        fs::write(
            old.join("prefs.json"),
            r#"{"schemaVersion":1,"theme":"system"}"#,
        )
        .unwrap();
        migrate_legacy_config(&new);
        assert!(fs::read_to_string(new.join("prefs.json"))
            .unwrap()
            .contains("light"));
    }

    #[test]
    fn no_legacy_folder_is_a_no_op() {
        let root = tempfile::tempdir().unwrap();
        let new = root.path().join("no.rorvikmedieteknikk.kinetra");
        assert!(
            copy_missing(&root.path().join("missing"), &new, LEGACY_CONFIG_FILES)
                .unwrap()
                .is_empty()
        );
        assert!(!new.exists());
    }

    fn tmp_files(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp-"))
            .collect()
    }

    #[test]
    fn writes_pretty_json_with_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        write_json_atomic(&path, &serde_json::json!({ "a": { "b": 1 } })).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, "{\n  \"a\": {\n    \"b\": 1\n  }\n}\n");
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[test]
    fn overwrites_existing_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        fs::write(&path, "old contents").unwrap();
        write_json_atomic(&path, &serde_json::json!({ "v": 2 })).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["v"], 2);
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[test]
    fn failed_write_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        // Target is a directory, so the rename fails.
        let target = dir.path().join("taken");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("x"), "x").unwrap();
        assert!(write_json_atomic(&target, &1).is_err());
        assert!(tmp_files(dir.path()).is_empty());
    }

    #[test]
    fn timestamps_have_expected_shape() {
        let r = now_rfc3339();
        assert!(r.ends_with('Z') && r.len() == 20, "{r}");
        let c = now_compact();
        assert_eq!(c.len(), 16, "{c}");
    }

    #[test]
    fn own_writes_and_removals_are_recognized() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.json");
        write_json_atomic(&path, &1).unwrap();
        assert!(is_own_change(&path));
        // Rewritten from outside (different mtime): external.
        fs::write(&path, "2").unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(5))
            .unwrap();
        assert!(!is_own_change(&path));
        // A removed folder covers the files that were in it, while they stay gone.
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        let inner = sub.join("x.css");
        fs::write(&inner, "a{}").unwrap();
        assert!(!is_own_change(&inner), "never recorded");
        fs::remove_dir_all(&sub).unwrap();
        record_own_removal(&sub);
        assert!(is_own_change(&inner));
        assert!(is_own_change(&sub));
        fs::create_dir(&sub).unwrap();
        fs::write(&inner, "b{}").unwrap();
        assert!(!is_own_change(&inner), "recreated from outside");
    }

    #[test]
    fn a_folder_put_in_place_stays_own_while_files_are_written_into_it() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join(".tmp-t1");
        fs::create_dir(&tmp).unwrap();
        write_json_atomic(&tmp.join("template.json"), &1).unwrap();
        let dest = dir.path().join("t1");
        rename_own(&tmp, &dest).unwrap();
        assert!(is_own_change(&dest));
        // An own write into it changes the folder's mtime; the folder is still ours.
        write_json_atomic(&dest.join("template.json"), &2).unwrap();
        assert!(is_own_change(&dest));
        assert!(is_own_change(&dest.join("template.json")));
        // Removed by us (recorded before the removal): own while it stays gone.
        remove_dir_all_own(&dest).unwrap();
        assert!(is_own_change(&dest));
        // A file renamed into place keeps its mtime, so the record made before the rename matches.
        let from = dir.path().join("a.png.tmp");
        fs::write(&from, "x").unwrap();
        let to = dir.path().join("a.png");
        rename_own(&from, &to).unwrap();
        assert!(is_own_change(&to));
    }

    #[test]
    fn strips_verbatim_prefix() {
        assert_eq!(strip_verbatim(r"\\?\C:\a\b"), r"C:\a\b");
        assert_eq!(strip_verbatim(r"\\?\UNC\srv\share"), r"\\srv\share");
        assert_eq!(strip_verbatim(r"C:\a"), r"C:\a");
    }
}
