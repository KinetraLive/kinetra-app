//! The media library: images, video and audio in the project's `assets/` folder (created on
//! demand), served by the output server at `/assets/<name>`. Image fields, image source cells
//! and media layers hold that URL.
//!
//! - Names are path-safe: `[A-Za-z0-9._-]`, 1..=128 bytes, not starting with `.`, with an
//!   extension from `ASSET_EXTENSIONS` (images), `MEDIA_VIDEO_EXTENSIONS` or
//!   `MEDIA_AUDIO_EXTENSIONS` (lower case). Imports sanitize the original file name and add
//!   `-2`, `-3`, … when it is taken.
//! - Imports check the extension, the size (images ≤ `ASSET_IMAGE_MAX_BYTES`, video and audio
//!   ≤ `ASSET_MEDIA_MAX_BYTES`) and the first bytes (the file must really be the type its
//!   extension says), then copy to a hidden temp name and rename it into place (an own write,
//!   so the watcher ignores it). `import_assets` copies with `fs::copy` (a kernel copy);
//!   `import_media` runs as a job on the blocking pool, copying through one reused 1 MB buffer
//!   so it can report `asset-import` progress (≤ 10/s per file) and be cancelled
//!   (`cancel_import`); a cancelled or failed file's partial copy is removed.
//! - `AssetInfo.media` is the header probe (`core::media_probe`, cached by size + mtime).
//! - `usedBy` lists the screens (field values, media layers) and sources (cells) that mention
//!   `/assets/<name>`. A media file that appears, changes or goes away re-applies the screens
//!   whose media layers use it (duration, `assetMissing`).
//! - Renaming keeps the extension and rewrites every reference through the normal save paths.
//!
//! No lock is held across file I/O: reads of the screen and source stores are short copies.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::core::events::names;
use crate::core::fsx;
use crate::core::media_probe;
use crate::core::model::{
    AssetImportProgress, AssetInfo, FileChange, FileKind, FilesChanged, MediaKind, ScreenLayerKind,
    SourceKind, ASSET_EXTENSIONS, ASSET_IMAGE_MAX_BYTES, ASSET_MEDIA_MAX_BYTES,
    MEDIA_AUDIO_EXTENSIONS, MEDIA_VIDEO_EXTENSIONS,
};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

pub const ASSETS_DIR: &str = "assets";
/// What image values start with.
pub const URL_PREFIX: &str = "/assets/";
/// Longest asset file name, in bytes.
pub const NAME_MAX: usize = 128;

pub fn assets_root(project: &Path) -> PathBuf {
    project.join(ASSETS_DIR)
}

/// Every extension the media library takes (images, video, audio).
fn all_extensions() -> impl Iterator<Item = &'static str> {
    ASSET_EXTENSIONS
        .iter()
        .chain(MEDIA_VIDEO_EXTENSIONS)
        .chain(MEDIA_AUDIO_EXTENSIONS)
        .copied()
}

/// The lower-case extension of a name, if it is an asset type (image, video or audio).
pub fn asset_ext(name: &str) -> Option<&'static str> {
    let (_, ext) = name.rsplit_once('.')?;
    let lower = ext.to_ascii_lowercase();
    all_extensions().find(|e| *e == lower)
}

/// Whether an asset name is an image (by extension).
pub fn is_image_name(name: &str) -> bool {
    is_asset_name(name)
        && name
            .rsplit_once('.')
            .is_some_and(|(_, e)| ASSET_EXTENSIONS.contains(&e))
}

/// The size limit for a file of this (lower-case) extension.
pub fn max_bytes(ext: &str) -> u64 {
    if ASSET_EXTENSIONS.contains(&ext) {
        ASSET_IMAGE_MAX_BYTES
    } else {
        ASSET_MEDIA_MAX_BYTES
    }
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GB", bytes / (1024 * 1024 * 1024))
    } else {
        format!("{} MB", bytes / (1024 * 1024))
    }
}

/// A path-safe asset file name (see the module docs).
pub fn is_asset_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && name
            .rsplit_once('.')
            .is_some_and(|(stem, ext)| !stem.is_empty() && all_extensions().any(|e| e == ext))
}

/// The value image fields hold for an asset.
pub fn url_of(name: &str) -> String {
    format!("{URL_PREFIX}{name}")
}

/// MIME type by (asset) extension.
pub fn content_type(name: &str) -> &'static str {
    match asset_ext(name) {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("aac") => "audio/aac",
        _ => "application/octet-stream",
    }
}

/// Whether the first bytes of a file match its extension.
pub fn sniff(ext: &str, head: &[u8]) -> bool {
    match ext {
        "png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        "jpg" | "jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "gif" => head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a"),
        "webp" => head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP",
        "svg" => {
            let text = String::from_utf8_lossy(head);
            let t = text.trim_start_matches('\u{feff}').trim_start();
            let lower = t.to_ascii_lowercase();
            (lower.starts_with("<?xml")
                || lower.starts_with("<svg")
                || lower.starts_with("<!--")
                || lower.starts_with("<!doctype svg"))
                && lower.contains("<svg")
        }
        "mp4" | "m4v" | "m4a" | "mov" => {
            head.len() >= 8
                && matches!(
                    &head[4..8],
                    b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot"
                )
        }
        "webm" => head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]),
        "wav" => head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WAVE",
        "mp3" => {
            head.starts_with(b"ID3")
                || (head.len() >= 2 && head[0] == 0xFF && head[1] & 0xE0 == 0xE0)
        }
        "aac" => {
            head.starts_with(b"ID3")
                || (head.len() >= 2 && head[0] == 0xFF && head[1] & 0xF6 == 0xF0)
        }
        _ => false,
    }
}

/// A safe name from a file name: other characters become `-`, runs collapse, the extension is
/// lower-cased. `None` when the extension is not an asset type.
pub fn sanitize(file_name: &str) -> Option<String> {
    let ext = asset_ext(file_name)?;
    let stem = &file_name[..file_name.rfind('.')?];
    let mut out = String::with_capacity(stem.len());
    for c in stem.chars() {
        let ok = c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.';
        let c = if ok { c } else { '-' };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let mut stem = out.trim_matches(|c| c == '-' || c == '.').to_string();
    if stem.is_empty() {
        stem = match media_probe::kind_of_ext(ext) {
            Some(MediaKind::Video) => "video",
            Some(MediaKind::Audio) => "audio",
            _ => "image",
        }
        .into();
    }
    let max = NAME_MAX - ext.len() - 1 - 8;
    if stem.len() > max {
        stem.truncate(max);
    }
    Some(format!("{stem}.{ext}"))
}

/// `name`, or `name-2`, `name-3`, … whichever is free in `assets/`.
fn unique(root: &Path, name: &str) -> String {
    if !root.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|i| format!("{stem}-{i}.{ext}"))
        .find(|n| !root.join(n).exists())
        .unwrap_or_else(|| name.to_string())
}

fn info(root: &Path, name: &str, used_by: Vec<String>) -> AppResult<AssetInfo> {
    let meta = fs::metadata(root.join(name))?;
    let media = media_probe::info_cached(root, name);
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| {
            time::OffsetDateTime::from(t)
                .format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .unwrap_or_default();
    Ok(AssetInfo {
        name: name.to_string(),
        url: url_of(name),
        bytes: meta.len(),
        modified_at,
        used_by,
        media,
    })
}

/// Asset names mentioned in a text (`/assets/<name>` followed by a non-name character).
pub fn mentioned(text: &str, out: &mut BTreeSet<String>) {
    let mut rest = text;
    while let Some(i) = rest.find(URL_PREFIX) {
        let after = &rest[i + URL_PREFIX.len()..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
            .unwrap_or(after.len());
        let name = &after[..end];
        if is_asset_name(name) {
            out.insert(name.to_string());
        }
        rest = &after[end..];
    }
}

/// `text` with every `/assets/<from>` reference (exact name) replaced by `/assets/<to>`.
pub fn replace_refs(text: &str, from: &str, to: &str) -> String {
    rename_refs(text, &[(from.to_string(), to.to_string())])
}

/// `text` with every `/assets/<from>` reference (exact name) of `renames` replaced by
/// `/assets/<to>`, in one pass (a new name is never renamed again).
pub fn rename_refs(text: &str, renames: &[(String, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find(URL_PREFIX) {
        let after = &rest[i + URL_PREFIX.len()..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
            .unwrap_or(after.len());
        out.push_str(&rest[..i + URL_PREFIX.len()]);
        let name = &after[..end];
        out.push_str(
            renames
                .iter()
                .find(|(from, _)| from == name)
                .map_or(name, |(_, to)| to.as_str()),
        );
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

fn invalid(m: impl Into<String>) -> AppError {
    AppError::Invalid(m.into())
}

/// Checks a file before it is imported: a library type, within its size limit, content
/// matching its extension. Returns the sanitized name and the size.
pub fn check_import(src: &Path) -> AppResult<(String, u64)> {
    let file_name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid(format!("{}: not a file name", src.display())))?;
    let shown = file_name.to_string();
    let name = sanitize(file_name).ok_or_else(|| {
        invalid(format!(
            "{shown}: only images (PNG, JPG, SVG, GIF, WebP), video (MP4, WebM, MOV) and audio (MP3, WAV, AAC, M4A) can be imported"
        ))
    })?;
    let ext = asset_ext(&name).unwrap_or("");
    let meta = fs::metadata(src)
        .map_err(|e| AppError::Io(io::Error::new(e.kind(), format!("{}: {e}", src.display()))))?;
    if !meta.is_file() {
        return Err(invalid(format!("{shown}: not a file")));
    }
    let max = max_bytes(ext);
    if meta.len() > max {
        return Err(invalid(format!(
            "{shown} is too large (at most {})",
            size_text(max)
        )));
    }
    let mut head = [0u8; 512];
    let n = fs::File::open(src)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|e| AppError::Io(io::Error::new(e.kind(), format!("{}: {e}", src.display()))))?;
    if !sniff(ext, &head[..n]) {
        return Err(invalid(format!(
            "{shown} is not a {} file (its content doesn't match its extension)",
            ext.to_uppercase()
        )));
    }
    Ok((name, meta.len()))
}

/// Copies one file into `assets/` (see the module docs). Returns the new name.
pub fn import_one(project: &Path, src: &Path) -> AppResult<String> {
    let (name, _) = check_import(src)?;
    place(project, &name, |tmp| fs::copy(src, tmp).map(|_| ()))
}

// ---- Import jobs (progress + cancel) -----------------------------------------------------------

/// Copy buffer of an import job (one per job, reused for every chunk).
const COPY_BUF: usize = 1024 * 1024;
/// Shortest time between two progress events of a file.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

fn jobs() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static J: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    J.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Asks a running import job to stop (its current file's partial copy is removed). `false`
/// when no such job runs.
pub fn cancel_job(job: &str) -> bool {
    match jobs()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(job)
    {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

/// Number of running import jobs (tests and diagnostics).
pub fn running_jobs() -> usize {
    jobs().lock().unwrap_or_else(PoisonError::into_inner).len()
}

/// Copies `src` to `dst` through `buf`, calling `progress(copied)` at most every
/// [`PROGRESS_INTERVAL`]; stops with `Interrupted` when `cancel` is set.
fn copy_with_progress(
    src: &Path,
    dst: &Path,
    buf: &mut [u8],
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> io::Result<u64> {
    let mut from = fs::File::open(src)?;
    let mut to = fs::File::create(dst)?;
    let mut copied = 0u64;
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Canceled"));
        }
        let n = match from.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        to.write_all(&buf[..n])?;
        copied += n as u64;
        if last.elapsed() >= PROGRESS_INTERVAL {
            last = Instant::now();
            progress(copied);
        }
    }
    to.sync_all()?;
    Ok(copied)
}

/// Runs an import job: each file checked, copied with progress and placed atomically. A file
/// that fails is reported (`error`) and the job goes on with the next; a cancel stops the job.
/// Returns the names imported. `emit` gets every progress event.
pub fn run_import(
    project: &Path,
    job: &str,
    paths: &[String],
    cancel: &AtomicBool,
    emit: &dyn Fn(AssetImportProgress),
) -> Vec<String> {
    let mut done = Vec::new();
    let mut buf = vec![0u8; COPY_BUF];
    for (index, p) in paths.iter().enumerate() {
        let index = u32::try_from(index).unwrap_or(u32::MAX);
        let src = Path::new(p);
        let shown = src
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(p)
            .to_string();
        let ev = |name: &str, copied: u64, total: u64, done: bool, error: Option<String>| {
            emit(AssetImportProgress {
                job: job.to_string(),
                index,
                name: name.to_string(),
                copied,
                total,
                done,
                error,
            })
        };
        if cancel.load(Ordering::SeqCst) {
            ev(&shown, 0, 0, true, Some("Canceled".into()));
            continue;
        }
        let (name, total) = match check_import(src) {
            Ok(v) => v,
            Err(e) => {
                ev(&shown, 0, 0, true, Some(e.to_string()));
                continue;
            }
        };
        ev(&name, 0, total, false, None);
        let placed = place(project, &name, |tmp| {
            copy_with_progress(src, tmp, &mut buf, cancel, |c| {
                ev(&name, c, total, false, None)
            })
            .map(|_| ())
        });
        match placed {
            Ok(final_name) => {
                ev(&final_name, total, total, true, None);
                done.push(final_name);
            }
            Err(AppError::Io(e)) if e.kind() == io::ErrorKind::Interrupted => {
                ev(&name, 0, total, true, Some("Canceled".into()));
            }
            Err(e) => ev(&name, 0, total, true, Some(e.to_string())),
        }
    }
    done
}

/// Writes `bytes` into `assets/` as `name`, or a free variant of it (`-2`, `-3`, …; an existing
/// file is never replaced), with the import's checks (name, size, first bytes) and its atomic
/// write (hidden temp file renamed into place as an own write). Returns the name used. For files
/// Kinetra ships itself (a recipe's sample logos).
pub fn add_bytes(project: &Path, name: &str, bytes: &[u8]) -> AppResult<String> {
    let safe = sanitize(name)
        .filter(|n| n == name)
        .ok_or_else(|| invalid(format!("Invalid image name: {name}")))?;
    let ext = asset_ext(&safe).unwrap_or("");
    if bytes.len() as u64 > ASSET_IMAGE_MAX_BYTES {
        return Err(invalid(format!(
            "{name} is too large (at most {} MB)",
            ASSET_IMAGE_MAX_BYTES / (1024 * 1024)
        )));
    }
    if !sniff(ext, &bytes[..bytes.len().min(512)]) {
        return Err(invalid(format!(
            "{name} is not a {} image",
            ext.to_uppercase()
        )));
    }
    place(project, &safe, |tmp| {
        let mut f = fs::File::create(tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    })
}

/// Puts a new file into `assets/` under a free variant of `name`: `write` fills a hidden temp
/// file, which is then renamed into place (recorded as an own write, so the watcher ignores it).
fn place(
    project: &Path,
    name: &str,
    write: impl FnOnce(&Path) -> io::Result<()>,
) -> AppResult<String> {
    let root = assets_root(project);
    fs::create_dir_all(&root)?;
    let name = unique(&root, name);
    let tmp = root.join(format!(".{name}.tmp-{}", nanoid::nanoid!(6)));
    let target = root.join(&name);
    let done = write(&tmp).and_then(|()| fsx::rename_own(&tmp, &target));
    if let Err(e) = done {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(name)
}

fn check_name(name: &str) -> AppResult<()> {
    if is_asset_name(name) {
        Ok(())
    } else {
        Err(invalid(format!("Invalid image name: {name}")))
    }
}

impl AppState {
    /// Asset name → ids of the screens and sources that mention it (from memory).
    fn asset_usage(&self) -> BTreeMap<String, Vec<String>> {
        let mut usage: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in self.screens().doc_ids() {
            let Some(doc) = self.screens().doc(&id) else {
                continue;
            };
            let mut names = BTreeSet::new();
            for l in &doc.meta.layers {
                match &l.kind {
                    ScreenLayerKind::Template { fields, .. } => {
                        for v in fields.values() {
                            mentioned(v, &mut names);
                        }
                    }
                    ScreenLayerKind::Media { media, .. } => mentioned(&media.asset, &mut names),
                }
            }
            for n in names {
                usage.entry(n).or_default().push(doc.id.clone());
            }
        }
        for doc in self.sources().docs() {
            let mut names = BTreeSet::new();
            if let SourceKind::Text { rows, .. } = &doc.meta.kind {
                for v in rows.iter().flat_map(|r| r.values()) {
                    mentioned(v, &mut names);
                }
            }
            for n in names {
                usage.entry(n).or_default().push(doc.id.clone());
            }
        }
        for v in usage.values_mut() {
            v.sort();
            v.dedup();
        }
        usage
    }

    /// Every image in `assets/`, by name, with `usedBy`. Touches the disk.
    pub fn list_assets(&self) -> AppResult<Vec<AssetInfo>> {
        let dir = self.project_dir()?;
        let root = assets_root(&dir);
        let entries = match fs::read_dir(&root) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| is_asset_name(n))
            .collect();
        names.sort_by_key(|n| n.to_lowercase());
        let mut usage = self.asset_usage();
        Ok(names
            .iter()
            .filter_map(|n| info(&root, n, usage.remove(n).unwrap_or_default()).ok())
            .collect())
    }

    /// Imports files (see the module docs). A file that fails stops the import with its error;
    /// the files before it stay imported.
    pub fn import_assets(&self, paths: &[String]) -> AppResult<Vec<AssetInfo>> {
        let dir = self.project_dir()?;
        let root = assets_root(&dir);
        let mut out = Vec::with_capacity(paths.len());
        for p in paths {
            let name = import_one(&dir, Path::new(p))?;
            self.media_file_changed(&dir, &name);
            out.push(info(&root, &name, Vec::new())?);
        }
        Ok(out)
    }

    /// Starts an import job on the blocking pool (or a thread without a runtime) and returns
    /// its id; progress goes out as `asset-import` events. See the module docs.
    pub fn import_media(&self, paths: Vec<String>) -> AppResult<String> {
        let dir = self.project_dir()?;
        let job = nanoid::nanoid!(10);
        let cancel = Arc::new(AtomicBool::new(false));
        jobs()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(job.clone(), cancel.clone());
        let state = self.clone();
        let id = job.clone();
        let work = move || {
            let events = state.events().clone();
            let emit = |p: AssetImportProgress| {
                // A file in place: the screens using it follow before its `done` goes out.
                if p.done && p.error.is_none() {
                    state.media_file_changed(&dir, &p.name);
                }
                match serde_json::to_value(&p) {
                    Ok(v) => events.emit(names::ASSET_IMPORT, v),
                    Err(e) => tracing::warn!("asset-import payload: {e}"),
                }
            };
            run_import(&dir, &id, &paths, &cancel, &emit);
            jobs()
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        };
        match self.live().runtime_handle() {
            Some(h) => {
                h.spawn_blocking(work);
            }
            None => {
                std::thread::spawn(work);
            }
        }
        Ok(job)
    }

    /// A media library file appeared, changed or went away: the screens whose media layers use
    /// it are applied again (probe, duration, `assetMissing`).
    pub fn media_file_changed(&self, dir: &Path, name: &str) {
        let Ok((cur, _w)) = self.screen_guard() else {
            return;
        };
        if cur != dir {
            return;
        }
        for id in self.screens().doc_ids() {
            let Some(doc) = self.screens().doc(&id) else {
                continue;
            };
            if !crate::core::screens::media_names(&doc).any(|n| n == name) {
                continue;
            }
            if let Err(e) = self.apply_screen_doc(dir, &doc) {
                tracing::warn!("applying screen {id} after {name} changed: {e}");
            }
        }
    }

    /// Deletes an image (references keep the URL; it shows nothing).
    pub fn delete_asset(&self, name: &str) -> AppResult<()> {
        let dir = self.project_dir()?;
        check_name(name)?;
        let path = assets_root(&dir).join(name);
        match fsx::remove_file_own(&path) {
            Ok(()) => {
                self.media_file_changed(&dir, name);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(AppError::NotFound(format!("Image not found: {name}")))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Renames an image, keeping its extension, and rewrites every screen and source that uses
    /// it (saved through the normal paths, so on-air layers follow the live rules).
    pub fn rename_asset(&self, name: &str, new_name: &str) -> AppResult<AssetInfo> {
        let dir = self.project_dir()?;
        check_name(name)?;
        let root = assets_root(&dir);
        let ext = asset_ext(name).unwrap_or("png");
        let stem = new_name.trim();
        let stem = match stem.rsplit_once('.') {
            Some((s, e)) if asset_ext(&format!("x.{e}")).is_some() => s,
            _ => stem,
        };
        let target = sanitize(&format!("{stem}.{ext}"))
            .filter(|n| is_asset_name(n))
            .ok_or_else(|| invalid(format!("Invalid image name: {new_name}")))?;
        if target == name {
            return info(
                &root,
                name,
                self.asset_usage().remove(name).unwrap_or_default(),
            );
        }
        if !root.join(name).is_file() {
            return Err(AppError::NotFound(format!("Image not found: {name}")));
        }
        if root.join(&target).exists() && !target.eq_ignore_ascii_case(name) {
            return Err(invalid(format!("An image named {target} already exists")));
        }
        fsx::record_own_removal(&root.join(name));
        fsx::rename_own(&root.join(name), &root.join(&target))?;
        let usage = self.asset_usage().remove(name).unwrap_or_default();
        for id in &usage {
            if let Some(mut doc) = self.screens().doc(id) {
                let mut changed = false;
                for l in &mut doc.meta.layers {
                    let values: Vec<&mut String> = match &mut l.kind {
                        ScreenLayerKind::Template { fields, .. } => fields.values_mut().collect(),
                        ScreenLayerKind::Media { media, .. } => vec![&mut media.asset],
                    };
                    for v in values {
                        let r = replace_refs(v, name, &target);
                        if r != *v {
                            *v = r;
                            changed = true;
                        }
                    }
                }
                if changed {
                    self.save_screen(doc)?;
                }
            } else if let Some(mut doc) = self.sources().doc(id) {
                let mut changed = false;
                if let SourceKind::Text { rows, .. } = &mut doc.meta.kind {
                    for v in rows.iter_mut().flat_map(|r| r.values_mut()) {
                        let r = replace_refs(v, name, &target);
                        if r != *v {
                            *v = r;
                            changed = true;
                        }
                    }
                }
                if changed {
                    self.save_source(doc)?;
                }
            }
        }
        // The brand kit's logo follows the rename.
        let old_url = url_of(name);
        if let Some(p) = self.current_project() {
            if p.settings.brand.logo.as_deref() == Some(old_url.as_str()) {
                let mut brand = p.settings.brand;
                brand.logo = Some(url_of(&target));
                self.update_project_settings(|s| s.brand = brand.clone())?;
                self.apply_brand(&brand);
            }
        }
        info(
            &root,
            &target,
            self.asset_usage().remove(&target).unwrap_or_default(),
        )
    }

    /// The watcher saw `assets/<name>` change outside Kinetra.
    pub(crate) fn asset_file_changed(&self, dir: &Path, name: &str) -> FilesChanged {
        let exists = assets_root(dir).join(name).is_file();
        self.media_file_changed(dir, name);
        FilesChanged {
            kind: FileKind::Asset,
            id: name.to_string(),
            change: if exists {
                FileChange::Changed
            } else {
                FileChange::Removed
            },
            message: None,
        }
    }
}

#[cfg(test)]
mod tests;
