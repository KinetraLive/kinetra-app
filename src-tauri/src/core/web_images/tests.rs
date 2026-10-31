//! Web images: which links count, the downloader against a mock "internet", the sweep.

use std::sync::atomic::{AtomicU32, Ordering};

use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;

use super::*;

const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];

struct Net {
    rt: tokio::runtime::Runtime,
    base: String,
    hits: Arc<AtomicU32>,
}

/// A mock web server: `/logo.png` (a PNG served as text/plain, to prove the type is sniffed),
/// `/page` (HTML), `/big` (over the limit, no length header), `/gone` (404), `/hop` (a redirect
/// to the PNG), `/loop` (redirects forever). Every request counts in `hits`.
fn net() -> Net {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let hits = Arc::new(AtomicU32::new(0));
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let h = hits.clone();
    let app = axum::Router::new()
        .route(
            "/logo.png",
            get(|| async { ([(header::CONTENT_TYPE, "text/plain")], PNG).into_response() }),
        )
        .route(
            "/page",
            get(|| async {
                ([(header::CONTENT_TYPE, "text/html")], "<html><svg>").into_response()
            }),
        )
        .route(
            "/big",
            get(|| async {
                let chunks = (0..21).map(|_| Ok::<_, std::io::Error>(vec![0u8; 1 << 20]));
                axum::body::Body::from_stream(futures_util::stream::iter(chunks)).into_response()
            }),
        )
        .route("/gone", get(|| async { StatusCode::NOT_FOUND }))
        .route(
            "/hop",
            get(|| async { axum::response::Redirect::temporary("/logo.png") }),
        )
        .route(
            "/loop",
            get(|| async { axum::response::Redirect::temporary("/loop") }),
        )
        .layer(axum::middleware::map_request(
            move |r: axum::extract::Request| {
                let h = h.clone();
                async move {
                    h.fetch_add(1, Ordering::SeqCst);
                    r
                }
            },
        ));
    rt.spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Net { rt, base, hits }
}

#[test]
fn which_links_count() {
    for ok in [
        "https://example.com/a.png",
        "http://192.168.1.4:8080/img?id=3",
        "HTTPS://Example.com/x",
    ] {
        assert!(is_web_url(ok), "{ok}");
    }
    for bad in [
        "",
        "/assets/logo.png",
        "//example.com/a.png",
        "file:///C:/x.png",
        "javascript:alert(1)",
        "data:image/png;base64,AAAA",
        "ftp://example.com/a.png",
        "https://user:pw@example.com/a.png",
        "https://exa mple.com/a.png",
        "https://example.com/a\n.png",
        "https://",
    ] {
        assert!(!is_web_url(bad), "{bad}");
    }
    assert!(!is_web_url(&format!(
        "https://e.com/{}",
        "a".repeat(URL_MAX)
    )));
    let k = key("https://example.com/a.png");
    assert_eq!(k.len(), 32);
    assert!(k.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(k, key("https://example.com/b.png"));
}

#[test]
fn links_in_values_and_lists() {
    let url = "https://example.com/a.png";
    let list = serde_json::json!([{ "name": "Ada", "photo": url }]).to_string();
    assert!(mentions(url, url));
    assert!(mentions(&list, url));
    assert!(!mentions("https://example.com/a.png?x", url));
    assert!(!mentions("see https://example.com/a.png", url));
    let mut out = HashSet::new();
    urls_in(&list, &mut out);
    urls_in("plain text", &mut out);
    urls_in("https://example.com/b.png", &mut out);
    let doc =
        serde_json::json!({ "rows": [{ "logo": "http://h/c.svg", "n": 3 }], "x": "/assets/a.png" });
    urls_in_json(&doc, &mut out);
    let mut got: Vec<_> = out.into_iter().collect();
    got.sort();
    assert_eq!(
        got,
        [
            "http://h/c.svg",
            "https://example.com/a.png",
            "https://example.com/b.png"
        ]
    );
}

#[test]
fn downloads_once_keeps_the_copy_and_refuses_what_isnt_an_image() {
    let n = net();
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    let web = WebImages::default();
    let logo = format!("{}/logo.png", n.base);

    // Two requests at once: one download; the copy is a .png whatever the server's type said.
    let (a, b) =
        n.rt.block_on(async { tokio::join!(web.fetch(project, &logo), web.fetch(project, &logo)) });
    let name = a.unwrap();
    assert_eq!(b.unwrap(), name);
    assert_eq!(name, format!("{}.png", key(&logo)));
    assert_eq!(n.hits.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read(project.join(DIR).join(&name)).unwrap(), PNG);
    assert_eq!(copy_name(&project.join(DIR), &key(&logo)), Some(name));
    // The project's .gitignore now ignores the copies.
    let ignore = std::fs::read_to_string(project.join(".gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l == ".web-images/"), "{ignore}");

    // A redirect to an image is followed.
    let hop = format!("{}/hop", n.base);
    assert!(n.rt.block_on(web.fetch(project, &hop)).is_ok());

    for (path, words) in [
        ("/page", "isn't a PNG"),
        ("/big", "larger than 20 MB"),
        ("/gone", "HTTP 404"),
        ("/loop", "redirects too many"),
    ] {
        let url = format!("{}{path}", n.base);
        let e = n.rt.block_on(web.fetch(project, &url)).unwrap_err();
        assert!(e.contains(words), "{path}: {e}");
        assert_eq!(copy_name(&project.join(DIR), &key(&url)), None, "{path}");
    }
    // A failed link isn't tried again right away.
    let before = n.hits.load(Ordering::SeqCst);
    let gone = format!("{}/gone", n.base);
    let e = n.rt.block_on(web.fetch(project, &gone)).unwrap_err();
    assert!(e.contains("a moment ago"), "{e}");
    assert_eq!(n.hits.load(Ordering::SeqCst), before);
}

#[test]
fn an_unreachable_server_fails_cleanly() {
    let n = net();
    let tmp = tempfile::tempdir().unwrap();
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/x.png", free.local_addr().unwrap());
    drop(free);
    let e =
        n.rt.block_on(WebImages::default().fetch(tmp.path(), &url))
            .unwrap_err();
    assert!(e.contains("Couldn't connect"), "{e}");
    assert!(!tmp.path().join(DIR).exists());
}

#[test]
fn the_sweep_keeps_only_copies_in_use() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(DIR);
    std::fs::create_dir_all(&dir).unwrap();
    let (kept, old) = ("https://a/kept.png", "https://a/old.png");
    std::fs::write(dir.join(format!("{}.png", key(kept))), PNG).unwrap();
    std::fs::write(dir.join(format!("{}.svg", key(old))), "<svg/>").unwrap();
    std::fs::write(dir.join("stray.tmp-1"), "x").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    let used = HashSet::from([kept.to_string()]);
    assert_eq!(sweep(tmp.path(), &used), 2);
    let mut left: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, [format!("{}.png", key(kept)), "sub".to_string()]);
    // No folder: nothing to do.
    assert_eq!(sweep(&tmp.path().join("nope"), &used), 0);
}

#[test]
fn links_on_disk_come_from_screens_and_template_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path();
    std::fs::create_dir_all(p.join("screens")).unwrap();
    std::fs::create_dir_all(p.join("templates/t1")).unwrap();
    std::fs::write(
        p.join("screens/s1.json"),
        r#"{"layers":[{"fields":{"pic":"https://a/one.png","name":"Ada"}}]}"#,
    )
    .unwrap();
    std::fs::write(
        p.join("templates/t1/template.json"),
        r#"{"fields":[{"key":"pic","default":"https://a/two.png"}]}"#,
    )
    .unwrap();
    std::fs::write(p.join("screens/broken.json"), "{oops").unwrap();
    let mut out = HashSet::new();
    urls_on_disk(p, &mut out);
    assert_eq!(
        out,
        HashSet::from([
            "https://a/one.png".to_string(),
            "https://a/two.png".to_string()
        ])
    );
}
