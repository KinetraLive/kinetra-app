//! Project folder watcher: one OS watcher (`notify`'s recommended backend, ReadDirectoryChangesW
//! on Windows; never polling) on the open project, recursive.
//!
//! Only `templates/<id>/template.json`, a variant's `templates/<id>/variants/<variantId>/
//! {template.html,template.css}` (schema 1: `templates/<id>/{template.html,template.css}`), the
//! `templates/<id>` and `templates/<id>/variants/<variantId>` folders themselves (moved in or
//! out), `screens/<id>.json`, `sources/<id>.json`,
//! `flows/<id>.json`, `controls.json` and images directly in `assets/` (path-safe names with an image extension)
//! matter. Temp files (`*.tmp-*`), set-aside files (`*.corrupt-*`), `.backups/` and everything else are ignored.
//!
//! Events are debounced on one std thread: it blocks on `recv` (zero CPU at rest), and after a
//! relevant event collects more until [`DEBOUNCE`] of quiet (at most [`DEBOUNCE_MAX`]), coalescing
//! per `(kind, id)`. Changes Kinetra made itself (`fsx::is_own_change`) are dropped; the rest go
//! to the handler as one batch.
//!
//! Dropping a [`ProjectWatcher`] stops the OS watcher and joins the thread. The handler must not
//! drop the watcher it runs on (a drop from the watcher's own thread does not join).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::core::assets::{is_asset_name, ASSETS_DIR};
use crate::core::controls::CONTROLS_FILE;
use crate::core::flows::FLOWS_DIR;
use crate::core::fsx;
use crate::core::model::FileKind;
use crate::core::screens::SCREENS_DIR;
use crate::core::sources::SOURCES_DIR;
use crate::core::templates::{
    is_id, is_variant_id, CSS_FILE, HTML_FILE, TEMPLATES_DIR, VARIANTS_DIR,
};

/// Quiet period before a batch is handled.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Longest a batch waits under a constant stream of events.
pub const DEBOUNCE_MAX: Duration = Duration::from_secs(2);

const TEMPLATE_JSON: &str = "template.json";

/// Receives each debounced batch of external changes: `(kind, id)`, each at most once.
pub type Handler = Box<dyn Fn(Vec<(FileKind, String)>) + Send + 'static>;

enum Msg {
    Event(notify::Result<notify::Event>),
    Stop,
}

pub struct ProjectWatcher {
    os: Option<RecommendedWatcher>,
    tx: Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    batches: Arc<AtomicU64>,
}

impl ProjectWatcher {
    /// Starts watching `dir` (recursively).
    pub fn start(dir: &Path, handler: Handler) -> notify::Result<Self> {
        Self::start_with(dir, DEBOUNCE, handler)
    }

    /// [`ProjectWatcher::start`] with another quiet period (tests use a long one).
    pub fn start_with(dir: &Path, debounce: Duration, handler: Handler) -> notify::Result<Self> {
        let (tx, rx) = mpsc::channel();
        let events = tx.clone();
        let mut os = notify::recommended_watcher(move |res| {
            let _ = events.send(Msg::Event(res));
        })?;
        os.watch(dir, RecursiveMode::Recursive)?;
        let root = dir.to_path_buf();
        let batches = Arc::new(AtomicU64::new(0));
        let counter = batches.clone();
        let thread = std::thread::Builder::new()
            .name("project-watcher".into())
            .spawn(move || run(root, rx, handler, debounce, &counter))
            .map_err(|e| notify::Error::generic(&e.to_string()))?;
        Ok(Self {
            os: Some(os),
            tx,
            thread: Some(thread),
            batches,
        })
    }

    /// Debounced batches of project-file events evaluated so far (own changes filtered out and
    /// the handler, if anything was left, returned). Lets tests wait for the watcher to settle
    /// instead of sleeping.
    pub fn batches(&self) -> u64 {
        self.batches.load(Ordering::SeqCst)
    }
}

impl Drop for ProjectWatcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        // Stops the OS watcher and drops its sender.
        drop(self.os.take());
        if let Some(t) = self.thread.take() {
            if t.thread().id() != std::thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

/// Which project file a path is, if it is one the watcher cares about.
pub fn classify(root: &Path, path: &Path) -> Option<(FileKind, String)> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [file] if *file == CONTROLS_FILE => Some((FileKind::Controls, CONTROLS_FILE.to_string())),
        [dir, file] if *dir == SCREENS_DIR => {
            let id = file.strip_suffix(".json")?;
            is_id(id).then(|| (FileKind::Screen, id.to_string()))
        }
        [dir, file] if *dir == SOURCES_DIR => {
            let id = file.strip_suffix(".json")?;
            is_id(id).then(|| (FileKind::Source, id.to_string()))
        }
        [dir, file] if *dir == FLOWS_DIR => {
            let id = file.strip_suffix(".json")?;
            is_id(id).then(|| (FileKind::Flow, id.to_string()))
        }
        [dir, file] if *dir == ASSETS_DIR && is_asset_name(file) => {
            Some((FileKind::Asset, file.to_string()))
        }
        [dir, id] if *dir == TEMPLATES_DIR && is_id(id) => {
            Some((FileKind::Template, id.to_string()))
        }
        [dir, id, file]
            if *dir == TEMPLATES_DIR
                && is_id(id)
                && [TEMPLATE_JSON, HTML_FILE, CSS_FILE].contains(file) =>
        {
            Some((FileKind::Template, id.to_string()))
        }
        [dir, id, variants, variant]
            if *dir == TEMPLATES_DIR
                && is_id(id)
                && *variants == VARIANTS_DIR
                && is_variant_id(variant) =>
        {
            Some((FileKind::Template, id.to_string()))
        }
        [dir, id, variants, variant, file]
            if *dir == TEMPLATES_DIR
                && is_id(id)
                && *variants == VARIANTS_DIR
                && is_variant_id(variant)
                && [HTML_FILE, CSS_FILE].contains(file) =>
        {
            Some((FileKind::Template, id.to_string()))
        }
        _ => None,
    }
}

fn kind_order(k: FileKind) -> u8 {
    match k {
        // Templates and sources first, so screens changed in the same batch see them.
        FileKind::Template => 0,
        FileKind::Source => 1,
        FileKind::Screen => 2,
        FileKind::Asset => 3,
        FileKind::Flow => 4,
        FileKind::Controls => 5,
    }
}

type Pending = BTreeMap<(u8, String), (FileKind, Vec<PathBuf>)>;

fn collect(root: &Path, pending: &mut Pending, ev: notify::Result<notify::Event>) {
    let ev = match ev {
        Ok(ev) => ev,
        Err(e) => {
            tracing::warn!("project watcher: {e}");
            return;
        }
    };
    if matches!(ev.kind, EventKind::Access(_)) {
        return;
    }
    // A template folder's own timestamp changes whenever a file inside is replaced; only the
    // folder appearing, disappearing or being renamed matters (its files report themselves).
    let folder_event = matches!(
        ev.kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    );
    for path in ev.paths {
        // `templates/<id>` or `templates/<id>/variants/<variantId>`.
        let is_folder = path
            .strip_prefix(root)
            .is_ok_and(|rel| matches!(rel.components().count(), 2 | 4))
            && path.starts_with(root.join(TEMPLATES_DIR));
        if is_folder && !folder_event {
            continue;
        }
        if let Some((kind, id)) = classify(root, &path) {
            let entry = pending
                .entry((kind_order(kind), id))
                .or_insert_with(|| (kind, Vec::new()));
            if !entry.1.contains(&path) {
                entry.1.push(path);
            }
        }
    }
}

fn run(
    root: PathBuf,
    rx: Receiver<Msg>,
    handler: Handler,
    debounce: Duration,
    batches: &AtomicU64,
) {
    loop {
        let mut pending = Pending::new();
        // Blocks with no timeout at rest.
        match rx.recv() {
            Ok(Msg::Event(ev)) => collect(&root, &mut pending, ev),
            Ok(Msg::Stop) | Err(_) => return,
        }
        if pending.is_empty() {
            continue;
        }
        let start = Instant::now();
        loop {
            let left = DEBOUNCE_MAX.saturating_sub(start.elapsed());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(debounce.min(left)) {
                Ok(Msg::Event(ev)) => collect(&root, &mut pending, ev),
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => break,
            }
        }
        // Checked now, after the quiet period, against the state the files settled in. Own changes
        // are recorded before they hit the disk (`fsx::rename_own`, `fsx::remove_file_own`), so
        // there is no window in which one looks external.
        let batch: Vec<(FileKind, String)> = pending
            .into_iter()
            .filter(|(_, (_, paths))| !paths.iter().all(|p| fsx::is_own_change(p)))
            .map(|((_, id), (kind, _))| (kind, id))
            .collect();
        if !batch.is_empty() {
            handler(batch);
        }
        batches.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn classifies_only_project_files() {
        let root = Path::new("C:/p");
        let c = |rel: &str| classify(root, &root.join(rel));
        assert_eq!(
            c("screens/abc123.json"),
            Some((FileKind::Screen, "abc123".into()))
        );
        assert_eq!(
            c("templates/t1/template.json"),
            Some((FileKind::Template, "t1".into()))
        );
        assert_eq!(
            c("templates/t1/template.html"),
            Some((FileKind::Template, "t1".into()))
        );
        assert_eq!(
            c("templates/t1/template.css"),
            Some((FileKind::Template, "t1".into()))
        );
        assert_eq!(c("templates/t1"), Some((FileKind::Template, "t1".into())));
        for variant in [
            "templates/t1/variants/default",
            "templates/t1/variants/compact-2/template.html",
            "templates/t1/variants/default/template.css",
        ] {
            assert_eq!(
                c(variant),
                Some((FileKind::Template, "t1".into())),
                "{variant}"
            );
        }
        assert_eq!(
            c("sources/src0000001.json"),
            Some((FileKind::Source, "src0000001".into()))
        );
        for ignored in [
            "sources/abc.json.tmp-123",
            "sources/abc.json.corrupt-20260101T000000Z",
            "sources/a.b.json",
            "sources/sub/x.json",
            "sources",
            "screens/abc.json.tmp-123",
            "screens/abc.json.corrupt-20260101T000000Z",
            "screens/a.b.json",
            "screens",
            "screens/sub/x.json",
            "templates/.tmp-abc/template.json",
            "templates/t1/template.json.tmp-xyz",
            "templates/t1/notes.txt",
            "templates/t1/sub/template.css",
            "templates/t1/variants",
            "templates/t1/variants/.tmp-abc",
            "templates/t1/variants/.tmp-abc/template.html",
            "templates/t1/variants/Bad/template.html",
            "templates/t1/variants/default/template.json",
            "templates/t1/variants/default/template.html.tmp-xyz",
            "templates/t1/variants/default/sub/template.css",
            ".backups/x/screens/abc.json",
            "project.json",
            "ui-state.json",
            "assets/.a.png.tmp-123",
            "assets/a.exe",
            "assets/sub/a.png",
            "assets/a b.png",
            "assets/Logo.SVG",
        ] {
            assert_eq!(c(ignored), None, "{ignored}");
        }
        assert_eq!(classify(root, Path::new("D:/other/screens/a.json")), None);
        assert_eq!(c("assets/a.png"), Some((FileKind::Asset, "a.png".into())));
    }

    #[test]
    fn debounces_filters_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        let root = fsx::absolute(dir.path());
        std::fs::create_dir_all(root.join("screens")).unwrap();
        std::fs::create_dir_all(root.join("templates/t1")).unwrap();
        type Batches = Arc<Mutex<Vec<Vec<(FileKind, String)>>>>;
        let seen: Batches = Arc::default();
        let sink = seen.clone();
        // A quiet period far longer than any stall of this thread between two writes, so they
        // surely land in one batch.
        let w = ProjectWatcher::start_with(
            &root,
            Duration::from_millis(800),
            Box::new(move |b| sink.lock().unwrap().push(b)),
        )
        .unwrap();
        let wait_batches = |n: u64| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while w.batches() < n {
                assert!(
                    Instant::now() < deadline,
                    "watcher never evaluated batch {n}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        // Several writes to one file plus another: one batch, coalesced, templates first.
        for i in 0..3 {
            std::fs::write(root.join("screens/s1.json"), format!("{i}")).unwrap();
        }
        std::fs::write(root.join("templates/t1/template.css"), "a{}").unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        wait_batches(1);
        assert_eq!(
            *seen.lock().unwrap(),
            [vec![
                (FileKind::Template, "t1".to_string()),
                (FileKind::Screen, "s1".to_string())
            ]]
        );
        // Own writes are ignored: the watcher evaluates their batch and reports nothing.
        seen.lock().unwrap().clear();
        let n = w.batches();
        fsx::write_json_atomic(&root.join("screens/s2.json"), &1).unwrap();
        wait_batches(n + 1);
        assert!(
            seen.lock().unwrap().is_empty(),
            "{:?}",
            seen.lock().unwrap()
        );
        // After drop the thread is joined and nothing more arrives.
        drop(w);
        std::fs::write(root.join("screens/s3.json"), "x").unwrap();
        std::thread::sleep(DEBOUNCE * 3);
        assert!(seen.lock().unwrap().is_empty());
    }
}
