//! What goes into a backup (CLOUD_PROJECTS.md §2.3, §8): every regular file under the project
//! folder except the app's excludes; links are neither followed nor stored; names the server
//! can't store are skipped and listed, never dropped silently. Then a SHA-256 per file, cached
//! by (path, size, mtime) so unchanged files aren't read again.
//!
//! Excludes (owner decision 2026-10-15): `.backups/` (local snapshots; the cloud has history),
//! `.git/`, our own temp and quarantine files (`*.tmp-*`, `*.corrupt-*`), restore staging
//! (`.kinetra-restore-*/`), and the OS's `.DS_Store`, `Thumbs.db`, `desktop.ini`, Office's
//! `~$*` and LibreOffice's `.~lock.*#`.
//!
//! Hashing reads with one reused 1 MiB buffer at background priority (Windows: the thread's
//! I/O and memory priority too). Large files wait while anything is on air.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use super::paths::{self, Unique};
use crate::core::fsx;

/// At most this many files per version (§8.2).
pub const MAX_FILES: usize = 50_000;
/// Files at least this large wait while anything is on air (hashing large media).
pub const LARGE_FILE: u64 = 16 * 1024 * 1024;
const BUF: usize = 1024 * 1024;

/// A file to back up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Relative, `/`-separated (the manifest path).
    pub path: String,
    pub abs: PathBuf,
    pub size: u64,
    /// Nanoseconds since the Unix epoch.
    pub mtime_ns: u128,
}

/// A file that isn't backed up, and why (the backup report).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Listing {
    pub files: Vec<Found>,
    pub skipped: Vec<Skipped>,
}

/// Whether a name is one of the excludes (`dir`: it's a folder).
pub fn excluded(name: &str, dir: bool, top_level: bool) -> bool {
    let lower = name.to_lowercase();
    if dir {
        return (top_level && lower == ".backups")
            || lower == ".git"
            || lower.starts_with(".kinetra-restore-");
    }
    lower == ".ds_store"
        || lower == "thumbs.db"
        || lower == "desktop.ini"
        || name.starts_with("~$")
        || (name.starts_with(".~lock.") && name.ends_with('#'))
        || name.contains(".tmp-")
        || name.contains(".corrupt-")
}

fn mtime_ns(m: &std::fs::Metadata) -> u128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// A link, or on Windows any reparse point (junctions too): never followed or stored.
fn is_link(m: &std::fs::Metadata) -> bool {
    if m.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

/// Lists the project folder (sorted by path). `max_file_bytes`: larger files are skipped.
pub fn list(root: &Path, max_file_bytes: Option<u64>) -> io::Result<Listing> {
    let mut out = Listing::default();
    let mut unique = Unique::default();
    let mut walk = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .min_depth(1)
        .into_iter();
    while let Some(entry) = walk.next() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                let path = e
                    .path()
                    .and_then(|p| p.strip_prefix(root).ok())
                    .map(rel_lossy)
                    .unwrap_or_default();
                if e.depth() == 0 {
                    return Err(e
                        .into_io_error()
                        .unwrap_or_else(|| io::Error::other("walk")));
                }
                out.skipped.push(Skipped {
                    path,
                    reason: "Kinetra can't read it.".into(),
                });
                continue;
            }
        };
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
        let name = entry.file_name().to_string_lossy();
        let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
            out.skipped.push(Skipped {
                path: rel_lossy(rel),
                reason: "Kinetra can't read it.".into(),
            });
            continue;
        };
        let dir = meta.is_dir();
        if excluded(&name, dir, entry.depth() == 1) {
            if dir {
                walk.skip_current_dir();
            }
            continue;
        }
        if is_link(&meta) {
            if dir {
                walk.skip_current_dir();
            }
            out.skipped.push(Skipped {
                path: rel_lossy(rel),
                reason: "It's a link; the file it points to isn't backed up.".into(),
            });
            continue;
        }
        if dir {
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        let Some(path) = rel_utf8(rel) else {
            out.skipped.push(Skipped {
                path: rel_lossy(rel),
                reason: "The name isn't valid text.".into(),
            });
            continue;
        };
        if let Some(reason) = paths::problem(&path) {
            out.skipped.push(Skipped {
                path,
                reason: reason.into(),
            });
            continue;
        }
        if !unique.insert(&path) {
            out.skipped.push(Skipped {
                path,
                reason: "Another file has the same name in different capitals.".into(),
            });
            continue;
        }
        if max_file_bytes.is_some_and(|max| meta.len() > max) {
            out.skipped.push(Skipped {
                path,
                reason: "It's larger than cloud backup allows for one file.".into(),
            });
            continue;
        }
        out.files.push(Found {
            path,
            abs: entry.path().to_path_buf(),
            size: meta.len(),
            mtime_ns: mtime_ns(&meta),
        });
    }
    Ok(out)
}

/// The manifest path: relative, `/`-separated, in NFC (macOS gives decomposed names; the file
/// itself is never renamed: `Found::abs` keeps its real name).
fn rel_utf8(rel: &Path) -> Option<String> {
    use unicode_normalization::UnicodeNormalization;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/").nfc().collect())
}

fn rel_lossy(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether `f` still has the size and mtime it was listed with.
pub fn unchanged(f: &Found) -> bool {
    std::fs::metadata(&f.abs).is_ok_and(|m| m.len() == f.size && mtime_ns(&m) == f.mtime_ns)
}

/// Re-reads a file's size and mtime (after it changed).
pub fn restat(f: &Found) -> io::Result<Found> {
    let m = std::fs::metadata(&f.abs)?;
    Ok(Found {
        size: m.len(),
        mtime_ns: mtime_ns(&m),
        ..f.clone()
    })
}

/// `modified_at` of the manifest: RFC 3339 UTC.
pub fn rfc3339(mtime_ns: u128) -> String {
    let secs = (mtime_ns / 1_000_000_000) as i64;
    let nanos = (mtime_ns % 1_000_000_000) as u32;
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()
        .and_then(|t| t.replace_nanosecond(nanos).ok())
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

// ---- Hashing ---------------------------------------------------------------------------------

/// Lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

/// Why hashing stopped.
#[derive(Debug)]
pub enum HashError {
    Canceled,
    Io(io::Error),
}

/// SHA-256 of a file. `wait` is asked between chunks of a large file (`LARGE_FILE`): it blocks
/// while on air and says `false` to cancel.
pub fn sha256_file(
    path: &Path,
    size: u64,
    buf: &mut [u8],
    wait: &dyn Fn() -> bool,
    cancel: &AtomicBool,
) -> Result<String, HashError> {
    let mut f = File::open(path).map_err(HashError::Io)?;
    let mut h = Sha256::new();
    let large = size >= LARGE_FILE;
    loop {
        if cancel.load(Ordering::Relaxed) || (large && !wait()) {
            return Err(HashError::Canceled);
        }
        let n = f.read(buf).map_err(HashError::Io)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

/// The SHA-256 cache of one project: path → (size, mtime, hash). In the app config dir
/// (`cloud-hashes/<cloudId>.json`), never in the project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HashCache {
    pub schema_version: u32,
    pub files: BTreeMap<String, CachedHash>,
    /// Blobs of the last version committed from here: storage has them (a full account still
    /// commits these when it refuses new bytes).
    #[serde(skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub committed: std::collections::BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedHash {
    pub size: u64,
    /// Nanoseconds since the Unix epoch, as text (JSON numbers lose precision).
    pub mtime: String,
    pub sha256: String,
}

impl HashCache {
    pub fn path(config_dir: &Path, cloud_id: &str) -> PathBuf {
        config_dir
            .join("cloud-hashes")
            .join(format!("{cloud_id}.json"))
    }

    pub fn load(config_dir: &Path, cloud_id: &str) -> Self {
        std::fs::read(Self::path(config_dir, cloud_id))
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|c| c.schema_version == 1)
            .unwrap_or_default()
    }

    pub fn save(&self, config_dir: &Path, cloud_id: &str) -> io::Result<()> {
        let path = Self::path(config_dir, cloud_id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut c = self.clone();
        c.schema_version = 1;
        fsx::write_json_atomic(&path, &c)
    }

    fn get(&self, f: &Found) -> Option<String> {
        self.files
            .get(&f.path)
            .filter(|c| c.size == f.size && c.mtime == f.mtime_ns.to_string())
            .map(|c| c.sha256.clone())
    }

    fn put(&mut self, f: &Found, sha256: &str) {
        self.files.insert(
            f.path.clone(),
            CachedHash {
                size: f.size,
                mtime: f.mtime_ns.to_string(),
                sha256: sha256.to_string(),
            },
        );
    }
}

/// A listed file with its hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hashed {
    pub file: Found,
    pub sha256: String,
}

/// Hashes every listed file (cached ones aren't read). Files that can't be read are skipped
/// and listed. `progress(bytes, read)` is called after each file (`read`: it wasn't cached).
/// The cache keeps only listed files.
pub fn hash_all(
    files: Vec<Found>,
    cache: &mut HashCache,
    wait: &dyn Fn() -> bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, bool),
) -> Result<(Vec<Hashed>, Vec<Skipped>), HashError> {
    let _priority = BackgroundPriority::enter();
    let mut buf = vec![0u8; BUF];
    let mut out = Vec::with_capacity(files.len());
    let mut skipped = Vec::new();
    let mut kept = BTreeMap::new();
    for f in files {
        let sha = match cache.get(&f) {
            Some(s) => {
                progress(f.size, false);
                s
            }
            None => match sha256_file(&f.abs, f.size, &mut buf, wait, cancel) {
                Ok(s) => {
                    progress(f.size, true);
                    s
                }
                Err(HashError::Canceled) => return Err(HashError::Canceled),
                Err(HashError::Io(_)) => {
                    skipped.push(Skipped {
                        path: f.path.clone(),
                        reason: "Kinetra can't read it (it may be open in another program).".into(),
                    });
                    continue;
                }
            },
        };
        cache.put(&f, &sha);
        if let Some(c) = cache.files.get(&f.path) {
            kept.insert(f.path.clone(), c.clone());
        }
        out.push(Hashed {
            file: f,
            sha256: sha,
        });
    }
    cache.files = kept;
    Ok((out, skipped))
}

/// The calling thread at background priority (lower CPU, I/O and memory priority on Windows)
/// until dropped.
pub struct BackgroundPriority {
    #[cfg(windows)]
    on: bool,
}

impl BackgroundPriority {
    pub fn enter() -> Self {
        #[cfg(windows)]
        {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
            };
            // SAFETY: the pseudo-handle of the calling thread is always valid.
            let on = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) }
                .is_ok();
            Self { on }
        }
        #[cfg(not(windows))]
        {
            Self {}
        }
    }
}

impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.on {
            use windows::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_END,
            };
            // SAFETY: as above; ends the mode this guard began on this thread.
            let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END) };
        }
    }
}

#[cfg(test)]
mod tests;
