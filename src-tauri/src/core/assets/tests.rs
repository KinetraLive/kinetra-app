use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::*;
use crate::core::events::{names, EventSink};
use crate::core::model::{
    NewScreen, Rect, ScreenLayer, SourceColumn, SourceKind, TemplateSource, ASSET_MAX_BYTES,
};
use crate::core::protocol::Fields;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Value)>>);

impl EventSink for Recorder {
    fn emit(&self, event: &str, payload: Value) {
        self.0.lock().unwrap().push((event.to_string(), payload));
    }
}

fn env() -> (tempfile::TempDir, AppState, Arc<Recorder>, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let rec = Arc::new(Recorder::default());
    let st = AppState::load_with_debounce(
        tmp.path().join("config"),
        rec.clone(),
        Duration::from_secs(60),
    )
    .unwrap();
    let mut prefs = st.prefs();
    prefs.backup.enabled = false;
    st.set_prefs(prefs);
    st.create_project(tmp.path(), "Show").unwrap();
    let dir = st.project_dir().unwrap();
    (tmp, st, rec, dir)
}

fn file(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p.display().to_string()
}

#[test]
fn names_types_and_sniffing() {
    for ok in [
        "a.png",
        "Logo_2.jpg",
        "x-y.z.svg",
        "a.webp",
        "b.gif",
        "c.jpeg",
    ] {
        assert!(is_asset_name(ok), "{ok}");
    }
    for bad in [
        "", ".png", ".a.png", "a", "a.exe", "a b.png", "../a.png", "a/b.png", "a\\b.png", "a.PNG",
        "..", "C:a.png",
    ] {
        assert!(!is_asset_name(bad), "{bad}");
    }
    assert_eq!(
        sanitize("My Logo (final).PNG").as_deref(),
        Some("My-Logo-final.png")
    );
    assert_eq!(sanitize("ææ.jpg").as_deref(), Some("image.jpg"));
    assert_eq!(sanitize("x.txt"), None);
    assert!(sniff("png", PNG));
    assert!(!sniff("png", b"GIF89a"));
    assert!(sniff("jpg", &[0xFF, 0xD8, 0xFF, 0xE0]));
    assert!(sniff("gif", b"GIF89a..."));
    assert!(sniff("webp", b"RIFF\0\0\0\0WEBPVP8 "));
    assert!(sniff(
        "svg",
        b"\xEF\xBB\xBF  <?xml version=\"1.0\"?><svg xmlns=\"x\"/>"
    ));
    assert!(sniff("svg", b"<svg viewBox=\"0 0 1 1\"></svg>"));
    assert!(!sniff("svg", b"<html><script>alert(1)</script></html>"));
    assert_eq!(content_type("a.svg"), "image/svg+xml");
    assert_eq!(content_type("a.jpeg"), "image/jpeg");
    let mut m = BTreeSet::new();
    mentioned(
        "x /assets/a.png, /assets/b.svg\" /assets/../c.png /assets/a.png2",
        &mut m,
    );
    assert_eq!(m.into_iter().collect::<Vec<_>>(), ["a.png", "b.svg"]);
    assert_eq!(
        replace_refs(
            "/assets/a.png /assets/a.png2 /assets/a.png",
            "a.png",
            "b.png"
        ),
        "/assets/b.png /assets/a.png2 /assets/b.png"
    );
}

#[test]
fn import_checks_type_size_and_content_and_names_uniquely() {
    let (tmp, st, _, dir) = env();
    let src = tmp.path();
    let a = file(src, "Sponsor Logo.png", PNG);
    let got = st.import_assets(&[a.clone(), a.clone()]).unwrap();
    assert_eq!(got[0].name, "Sponsor-Logo.png");
    assert_eq!(got[0].url, "/assets/Sponsor-Logo.png");
    assert_eq!(got[1].name, "Sponsor-Logo-2.png", "unique names");
    assert_eq!(got[0].bytes, PNG.len() as u64);
    assert!(dir.join("assets/Sponsor-Logo.png").is_file());
    // Wrong content, wrong type, too large, missing.
    let fake = file(src, "fake.png", b"GIF89a....");
    assert!(matches!(
        st.import_assets(&[fake]),
        Err(AppError::Invalid(_))
    ));
    let txt = file(src, "notes.txt", b"hello");
    assert!(matches!(
        st.import_assets(&[txt]),
        Err(AppError::Invalid(_))
    ));
    let big = src.join("big.png");
    let f = fs::File::create(&big).unwrap();
    f.set_len(ASSET_MAX_BYTES + 1).unwrap();
    drop(f);
    assert!(matches!(
        st.import_assets(&[big.display().to_string()]),
        Err(AppError::Invalid(_))
    ));
    assert!(st
        .import_assets(&[src.join("nope.png").display().to_string()])
        .is_err());
    // No temp files left behind; the listing is sorted with usage.
    let names: Vec<String> = st
        .list_assets()
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["Sponsor-Logo-2.png", "Sponsor-Logo.png"]);
    let left: Vec<_> = fs::read_dir(dir.join("assets")).unwrap().collect();
    assert_eq!(left.len(), 2);
}

#[test]
fn delete_rename_and_usage() {
    let (tmp, st, _, dir) = env();
    let a = file(tmp.path(), "logo.png", PNG);
    st.import_assets(&[a]).unwrap();
    // A screen field and a source cell use it.
    let t = st
        .templates()
        .create(
            &dir,
            "T",
            &TemplateSource::Preset {
                preset_id: "classic-bar".into(),
            },
        )
        .unwrap();
    let mut s = st
        .create_screen(&NewScreen {
            name: "S".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap()
        .doc;
    let mut fields = Fields::new();
    fields.insert("name".into(), "/assets/logo.png".into());
    s.meta.layers.push(ScreenLayer {
        id: "l1".into(),
        name: "L".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 100,
            h: 100,
        },
        kind: ScreenLayerKind::Template {
            template_id: t.id.clone(),
            fields,
            vars: Fields::new(),
            bindings: Default::default(),
            variant: None,
            sizing: None,
            opacity: None,
        },
        anchor: None,
    });
    let s = st.save_screen(s).unwrap().doc;
    let mut src = st.create_source("Sponsors").unwrap();
    src.meta.kind = SourceKind::Text {
        columns: vec![SourceColumn {
            key: "logo".into(),
            label: "Logo".into(),
            kind: crate::core::model::ColumnKind::Image,
        }],
        rows: vec![[("logo".to_string(), "/assets/logo.png".to_string())]
            .into_iter()
            .collect()],
        index: 0,
        auto: None,
    };
    let src = st.save_source(src).unwrap();
    // Image cells must be asset or http(s) URLs.
    let mut bad = src.clone();
    if let SourceKind::Text { rows, .. } = &mut bad.meta.kind {
        rows[0].insert("logo".into(), "javascript:alert(1)".into());
    }
    assert!(matches!(st.save_source(bad), Err(AppError::Invalid(_))));
    let list = st.list_assets().unwrap();
    let mut used = list[0].used_by.clone();
    used.sort();
    let mut want = vec![s.id.clone(), src.id.clone()];
    want.sort();
    assert_eq!(used, want);
    // Rename keeps the extension and rewrites the references.
    let r = st.rename_asset("logo.png", "Main Logo.jpg").unwrap();
    assert_eq!(r.name, "Main-Logo.png");
    assert_eq!(r.used_by.len(), 2);
    assert!(dir.join("assets/Main-Logo.png").is_file());
    assert!(!dir.join("assets/logo.png").exists());
    let doc = st.get_screen(&s.id).unwrap().doc;
    let ScreenLayerKind::Template { fields, .. } = &doc.meta.layers[0].kind else {
        unreachable!()
    };
    assert_eq!(fields["name"], "/assets/Main-Logo.png");
    let SourceKind::Text { rows, .. } = st.get_source(&src.id).unwrap().meta.kind else {
        panic!()
    };
    assert_eq!(rows[0]["logo"], "/assets/Main-Logo.png");
    // Taken name, bad names, missing.
    let b = file(tmp.path(), "b.png", PNG);
    st.import_assets(&[b]).unwrap();
    assert!(matches!(
        st.rename_asset("b.png", "Main Logo"),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        st.rename_asset("../b.png", "x"),
        Err(AppError::Invalid(_))
    ));
    assert!(matches!(
        st.rename_asset("zz.png", "x"),
        Err(AppError::NotFound(_))
    ));
    // Delete.
    st.delete_asset("b.png").unwrap();
    assert!(matches!(
        st.delete_asset("b.png"),
        Err(AppError::NotFound(_))
    ));
    assert!(matches!(
        st.delete_asset("../x.png"),
        Err(AppError::Invalid(_))
    ));
    assert_eq!(st.list_assets().unwrap().len(), 1);
}

#[test]
fn the_watcher_reports_images_added_outside() {
    let (_tmp, st, rec, dir) = env();
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(dir.join("assets/outside.png"), PNG).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let hit = rec.0.lock().unwrap().iter().any(|(n, v)| {
            n == names::PROJECT_FILES_CHANGED
                && v["kind"] == "asset"
                && v["id"] == "outside.png"
                && v["change"] == "changed"
        });
        if hit {
            break;
        }
        assert!(Instant::now() < deadline, "no asset event");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Own imports are not reported: wait until the watcher has evaluated the import's batch.
    let wait_batches = |n: u64| {
        let deadline = Instant::now() + Duration::from_secs(8);
        while st.watcher_batches() < n {
            assert!(Instant::now() < deadline, "no watcher batch");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    wait_batches(1);
    let n = st.watcher_batches();
    rec.0.lock().unwrap().clear();
    let src = dir.join("in.png");
    fs::write(&src, PNG).unwrap();
    st.import_assets(&[src.display().to_string()]).unwrap();
    wait_batches(n + 1);
    let own = rec
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|(n, v)| n == names::PROJECT_FILES_CHANGED && v["id"] == "in.png");
    assert!(!own);
}

fn e2e_clip(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../e2e/media")
        .join(name)
}

#[test]
fn media_names_limits_and_sniffing() {
    for ok in [
        "a.mp4", "b.webm", "c.mov", "d.m4v", "e.mp3", "f.wav", "g.m4a", "h.aac",
    ] {
        assert!(is_asset_name(ok), "{ok}");
        assert!(!is_image_name(ok), "{ok}");
    }
    assert!(is_image_name("a.png"));
    assert_eq!(max_bytes("png"), ASSET_IMAGE_MAX_BYTES);
    assert_eq!(max_bytes("mp4"), ASSET_MEDIA_MAX_BYTES);
    assert_eq!(content_type("a.webm"), "video/webm");
    assert_eq!(content_type("a.mp3"), "audio/mpeg");
    assert!(sniff("mp4", b"\0\0\0\x18ftypisom"));
    assert!(!sniff("mp4", b"RIFF\0\0\0\0WAVE"));
    assert!(sniff("webm", &[0x1A, 0x45, 0xDF, 0xA3, 0]));
    assert!(sniff("wav", b"RIFF\0\0\0\0WAVEfmt "));
    assert!(sniff("mp3", b"ID3\x03"));
    assert!(sniff("aac", &[0xFF, 0xF1, 0x50]));
    assert_eq!(
        sanitize("My Clip (final).MP4").as_deref(),
        Some("My-Clip-final.mp4")
    );
    assert_eq!(sanitize("???.webm").as_deref(), Some("video.webm"));
}

fn wait_jobs() {
    let deadline = Instant::now() + Duration::from_secs(10);
    while running_jobs() > 0 {
        assert!(Instant::now() < deadline, "import job still running");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn import_media_reports_progress_and_brings_media_layers_in() {
    use crate::core::model::{MediaFit, MediaLayerConfig, ScreenLayerKind, ScreenWarningCode};
    let src = e2e_clip("clip-vp9.webm");
    if !src.exists() {
        eprintln!("e2e clip missing; skipped");
        return;
    }
    let (tmp, st, rec, dir) = env();
    // A screen with a media layer whose file isn't in the project yet: assetMissing.
    let screen = st
        .create_screen(&NewScreen {
            name: "Main".into(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
        })
        .unwrap();
    let mut doc = screen.doc.clone();
    doc.meta.layers.push(ScreenLayer {
        id: "clip".into(),
        name: "Clip".into(),
        rect: Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        },
        kind: ScreenLayerKind::Media {
            media: MediaLayerConfig {
                asset: "clip-vp9.webm".into(),
                fit: MediaFit::Fill,
                in_ms: 0,
                out_ms: Some(999_999),
                volume: 80,
                muted: false,
                end_action: crate::core::model::MediaEndAction::Hold,
                autoplay: true,
            },
            opacity: None,
        },
        anchor: None,
    });
    let saved = st.save_screen(doc).unwrap();
    let ScreenLayerKind::Media { media, .. } = &saved.doc.meta.layers[0].kind else {
        unreachable!()
    };
    assert_eq!(media.asset, "/assets/clip-vp9.webm", "stored as a URL");
    assert!(saved
        .warnings
        .iter()
        .any(|w| w.code == ScreenWarningCode::AssetMissing));
    assert!(st.live().layer_view(&saved.doc.id, "clip").is_err());

    let job = st
        .import_media(vec![
            src.display().to_string(),
            file(tmp.path(), "x.mp4", b"nope"),
        ])
        .unwrap();
    wait_jobs();
    let events: Vec<Value> = rec
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|(e, _)| e == names::ASSET_IMPORT)
        .map(|(_, p)| p.clone())
        .collect();
    assert!(events.iter().all(|e| e["job"] == job.as_str()));
    let done: Vec<&Value> = events.iter().filter(|e| e["done"] == true).collect();
    assert_eq!(done.len(), 2, "{events:?}");
    assert_eq!(done[0]["name"], "clip-vp9.webm");
    assert_eq!(done[0]["error"], Value::Null);
    assert_eq!(done[0]["copied"], done[0]["total"]);
    assert!(
        done[1]["error"].as_str().unwrap().contains("doesn't match"),
        "{:?}",
        done[1]
    );
    assert!(dir.join("assets/clip-vp9.webm").is_file());
    assert!(!dir.join("assets/x.mp4").exists());
    // No temp files left behind.
    let leftovers: Vec<_> = fs::read_dir(dir.join("assets"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty());
    // The layer is live now, with the probed duration and the out point clamped to it.
    let v = st.live().layer_view(&saved.doc.id, "clip").unwrap();
    let d = v.media.unwrap().duration_ms.unwrap();
    assert!(d > 500, "{d}");
    let out = st
        .live()
        .with_layer(&saved.doc.id, "clip", |l| {
            l.media.clone().unwrap().config.out_ms
        })
        .unwrap();
    assert_eq!(out, Some(d));
    // The library lists it with its probe and usage.
    let list = st.list_assets().unwrap();
    let a = list.iter().find(|a| a.name == "clip-vp9.webm").unwrap();
    assert_eq!(a.used_by, vec![saved.doc.id.clone()]);
    assert_eq!(
        a.media.as_ref().unwrap().video_codec.as_deref(),
        Some("vp9")
    );
}

#[test]
fn cancelled_copy_leaves_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut big = b"\0\0\0\x18ftypisom".to_vec();
    big.resize(8 * 1024 * 1024, 7);
    let src = file(tmp.path(), "big.mp4", &big);
    let project = tmp.path().join("p");
    fs::create_dir_all(&project).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(true);
    let seen = Mutex::new(Vec::new());
    let names = run_import(&project, "j", &[src], &cancel, &|p| {
        seen.lock().unwrap().push(p)
    });
    assert!(names.is_empty());
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.last().unwrap().error.as_deref(), Some("Canceled"));
    let left = fs::read_dir(project.join("assets"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0);
    assert!(!cancel_job("no-such-job"));
}

#[test]
fn import_events_carry_the_picked_position_when_the_name_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut mp4 = b"\0\0\0\x18ftypisom".to_vec();
    mp4.resize(64 * 1024, 7);
    let a = file(tmp.path(), "Video(25).mp4", &mp4);
    let b = file(tmp.path(), "plain.mp4", &mp4);
    let project = tmp.path().join("p");
    fs::create_dir_all(&project).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let seen = Mutex::new(Vec::new());
    let names = run_import(&project, "j", &[a, b], &cancel, &|p| {
        seen.lock().unwrap().push(p)
    });
    assert_eq!(names.len(), 2);
    let seen = seen.into_inner().unwrap();
    let done: Vec<(u32, &str)> = seen
        .iter()
        .filter(|p| p.done)
        .map(|p| (p.index, p.name.as_str()))
        .collect();
    // The first picked file is placed under a sanitized name; its events keep index 0.
    assert_eq!(done.len(), 2);
    assert_eq!(done[0].0, 0);
    assert_ne!(done[0].1, "Video(25).mp4");
    assert_eq!(done[1], (1, "plain.mp4"));
    assert!(seen.iter().all(|p| p.index <= 1));
}
