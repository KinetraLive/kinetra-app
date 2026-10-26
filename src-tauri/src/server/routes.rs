//! Routes and the WebSocket session (`/assets/*` is in `super::assets`). Tauri-free: [`serve`] needs only a listener, the `LiveHub`
//! and a shutdown future, so the `fixture_server` example runs it without the app.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rust_embed::RustEmbed;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::time::{interval_at, Instant, MissedTickBehavior};

use super::assets::AssetDir;
use crate::core::live_hub::LiveHub;
use crate::core::protocol::{ClientMsg, ServerMsg, WireErrorCode, PROTOCOL_VERSION};

/// WebSocket keepalive: the only periodic work per connection.
pub const PING_INTERVAL: Duration = Duration::from_secs(20);

const NOT_BUILT: &str = "Renderer not built. Run: pnpm --dir renderer build";

/// The renderer bundle. Embedded in release builds; read from disk in debug builds.
#[derive(RustEmbed)]
#[folder = "../renderer/dist/"]
struct RendererDist;

#[derive(Clone)]
struct Ctx {
    hub: Arc<LiveHub>,
    /// The open project, for `/assets/*`.
    assets: AssetDir,
    /// Becomes `true` when the server shuts down; every WebSocket session closes on it.
    shutdown: watch::Receiver<bool>,
}

/// Serves the output routes on `listener` until `shutdown` resolves, then shuts down gracefully:
/// stops accepting, sends a Close frame on every open WebSocket and waits for in-flight HTTP
/// requests. Callers bound the wait (the server manager aborts after 1 s).
pub async fn serve(
    listener: TcpListener,
    hub: Arc<LiveHub>,
    assets: AssetDir,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    serve_with(listener, hub, assets, None, shutdown).await
}

/// Extra routes merged into the output server, made per server start. The receiver turns `true`
/// when that server shuts down, so long-lived connections (WebSockets) can close.
pub type ExtraRoutes = Arc<dyn Fn(watch::Receiver<bool>) -> Router + Send + Sync>;

/// [`serve`] with extra routes merged in: the Control API (`super::control`) in the app, the
/// e2e fixture server's scriptable mock API under `/__mock/*`. Handlers can read the peer's
/// address (`ConnectInfo<SocketAddr>`).
pub async fn serve_with(
    listener: TcpListener,
    hub: Arc<LiveHub>,
    assets: AssetDir,
    extra: Option<ExtraRoutes>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let (tx, rx) = watch::channel(false);
    let mut app = router(Ctx {
        hub,
        assets,
        shutdown: rx.clone(),
    });
    if let Some(extra) = extra {
        app = app.merge(extra(rx));
    }
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown.await;
        let _ = tx.send(true);
    })
    .await
}

fn router(ctx: Ctx) -> Router {
    Router::new()
        .route("/screen/{id}", get(screen_page))
        // Studio mode's Preview output: the same page; the renderer connects with bus=preview.
        .route("/screen/{id}/preview", get(screen_page))
        .route("/screen/{id}/preview/", get(screen_page))
        // Test patterns for display outputs (a static page reading its query).
        .route("/pattern", get(pattern_page))
        .route("/renderer/{*path}", get(renderer_asset))
        .route("/fonts/{*path}", get(font_asset))
        .route("/assets/{name}", get(project_asset))
        .route("/ws", get(ws_upgrade))
        .route("/health", get(health))
        .fallback(not_found)
        .layer(middleware::map_response(nosniff))
        .with_state(ctx)
}

async fn nosniff(mut res: Response) -> Response {
    res.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found").into_response()
}

/// Project images (see `super::assets`).
async fn project_asset(
    Path(name): Path<String>,
    headers: HeaderMap,
    State(ctx): State<Ctx>,
) -> Response {
    super::assets::asset_response(&ctx.assets, &name, &headers).await
}

async fn health() -> Response {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })).into_response()
}

/// The same `index.html` for every screen id; the renderer reads the id from the URL.
async fn screen_page(Path(_id): Path<String>) -> Response {
    html_page("index.html", SCREEN_CSP)
}

/// Every template renders under this policy (BACKEND_PLAN part 3, "sandbox"): only the
/// renderer's own scripts run (no inline scripts or `on*` handlers from template markup), nothing
/// loads from the network (images, media, fonts and connections only from this server, plus
/// `data:` / `blob:` images and media), no forms, frames, plugins or base-URL tricks.
/// Inline styles stay allowed: template CSS goes into shadow-root `<style>` elements.
pub const SCREEN_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self'; connect-src 'self'; object-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; form-action 'none'; base-uri 'none'";

/// The test pattern page: static, one inline script reading its query, no network.
pub const PATTERN_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; form-action 'none'; base-uri 'none'";

/// The test pattern page (`renderer/pattern.html`): bars, grid, safe areas or the output name,
/// chosen by its query.
async fn pattern_page() -> Response {
    html_page("pattern.html", PATTERN_CSP)
}

/// A page of the renderer build, never cached (the hashed assets it loads are).
fn html_page(name: &str, csp: &'static str) -> Response {
    match RendererDist::get(name) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
                (header::CONTENT_SECURITY_POLICY, csp),
            ],
            Body::from(file.data),
        )
            .into_response(),
        None => (
            StatusCode::SERVICE_UNAVAILABLE,
            [
                (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            NOT_BUILT,
        )
            .into_response(),
    }
}

/// Hashed renderer assets: cached forever.
async fn renderer_asset(Path(path): Path<String>) -> Response {
    match RendererDist::get(&path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, content_type(&path)),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            Body::from(file.data),
        )
            .into_response(),
        None => not_found().await,
    }
}

/// Template fonts, from the renderer bundle's `fonts/` folder (`<slug>.css` with `unicode-range`
/// faces, `<slug>/*.woff2`). Only css and woff2; plain relative paths only. Cached forever.
async fn font_asset(Path(path): Path<String>) -> Response {
    font_response(&path)
}

/// `true` for `seg(/seg)*` where every segment is `[A-Za-z0-9._-]+` and not `.`/`..`, ending in
/// `.css` or `.woff2`.
fn is_font_path(path: &str) -> bool {
    let ext_ok = path.ends_with(".css") || path.ends_with(".woff2");
    ext_ok
        && path.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

fn font_response(path: &str) -> Response {
    if !is_font_path(path) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    match RendererDist::get(&format!("fonts/{path}")) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, content_type(path)),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            Body::from(file.data),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "json" => "application/json",
        "png" => "image/png",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[derive(Deserialize)]
struct WsQuery {
    screen: String,
    /// `preview` = the screen's Preview bus (Studio mode); anything else, or none, = Program.
    #[serde(default)]
    bus: Option<String>,
    /// `1` = an app preview (muted iframe): its media reports don't count toward readiness.
    #[serde(default)]
    mute: Option<String>,
    /// `1` = an NDI output's capture page: counted for readiness, not as an OBS-style Program
    /// renderer (`LiveHub::program_renderers`).
    #[serde(default)]
    cap: Option<String>,
}

async fn ws_upgrade(
    ws: WebSocketUpgrade,
    Query(q): Query<WsQuery>,
    State(ctx): State<Ctx>,
) -> Response {
    let preview = q.bus.as_deref() == Some("preview");
    // Media readiness counts Program renderers only: not the Preview bus, not app previews.
    let counted = !preview && q.mute.as_deref() != Some("1");
    let capture = q.cap.as_deref() == Some("1");
    ws.on_upgrade(move |socket| session(socket, q.screen, preview, counted, capture, ctx))
}

/// Largest renderer → server message accepted (a `ready` is ~100 bytes).
const CLIENT_MSG_MAX: usize = 4096;

/// Text message sharing the hub's serialized JSON (no copy per client).
pub(super) fn shared_text(json: &Arc<str>) -> Message {
    struct Shared(Arc<str>);
    impl AsRef<[u8]> for Shared {
        fn as_ref(&self) -> &[u8] {
            self.0.as_bytes()
        }
    }
    match Utf8Bytes::try_from(Bytes::from_owner(Shared(json.clone()))) {
        Ok(text) => Message::Text(text),
        Err(_) => Message::Text(Utf8Bytes::from(&**json)),
    }
}

fn msg_text(msg: &ServerMsg) -> Option<Message> {
    serde_json::to_string(msg)
        .ok()
        .map(|s| Message::Text(s.into()))
}

/// One renderer connection: `hello`, `snapshot` (or `error` for an unknown screen), then the
/// screen's diffs from the hub until the peer leaves or the server shuts down. With `preview`
/// it is the screen's Preview bus: the hub derives that screen's Preview while the connection
/// lasts, and only preview-bus messages are forwarded.
async fn session(
    mut socket: WebSocket,
    screen: String,
    preview: bool,
    counted: bool,
    capture: bool,
    ctx: Ctx,
) {
    let Ctx {
        hub, mut shutdown, ..
    } = ctx;
    // Media readiness: Program renderers that aren't app previews register (dropped on exit).
    let reg = counted.then(|| hub.renderer_attach_as(&screen, true, capture));
    // Subscribe before reading the snapshot so no change can fall in between.
    let mut rx = hub.subscribe();

    let hello = ServerMsg::Hello {
        protocol: PROTOCOL_VERSION,
        now: crate::core::fsx::now_epoch_ms(),
    };
    // Held for the whole session: dropping it ends the derived Preview when nobody watches.
    let (_preview_sub, snapshot) = if preview {
        let (sub, snap) = hub.preview_attach(&screen);
        (Some(sub), snap)
    } else {
        (None, hub.snapshot_json(&screen))
    };
    let first = match snapshot {
        Some(snapshot) => Some(Message::Text(snapshot.into())),
        // Unknown screen: say so and stay idle. If the screen appears later (project switch),
        // its snapshot arrives through the hub like any other message.
        None => msg_text(&ServerMsg::Error {
            code: WireErrorCode::UnknownScreen,
        }),
    };
    for msg in [msg_text(&hello), first].into_iter().flatten() {
        if socket.send(msg).await.is_err() {
            return;
        }
    }

    let mut ping = interval_at(Instant::now() + PING_INTERVAL, PING_INTERVAL);
    ping.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            // Shutting down, or the server task is gone (sender dropped): close either way.
            () = async { let _ = shutdown.wait_for(|&down| down).await; } => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            msg = rx.recv() => {
                let out = match msg {
                    Ok(m) if *m.screen_id == *screen && m.preview == preview => {
                        shared_text(&m.json)
                    }
                    Ok(_) => continue,
                    Err(RecvError::Lagged(n)) => {
                        tracing::debug!("renderer for {screen} lagged {n} messages; resyncing");
                        rx = rx.resubscribe();
                        let snapshot = if preview {
                            hub.preview_snapshot_json(&screen)
                        } else {
                            hub.snapshot_json(&screen)
                        };
                        match snapshot {
                            Some(s) => Message::Text(s.into()),
                            None => continue,
                        }
                    }
                    Err(RecvError::Closed) => break,
                };
                if socket.send(out).await.is_err() {
                    break;
                }
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                // Status only (`ready`, `gone`); ignored from uncounted connections. Anything
                // else (pongs, unknown or oversized text) is ignored.
                Some(Ok(Message::Text(t))) if t.len() <= CLIENT_MSG_MAX => {
                    match (&reg, serde_json::from_str::<ClientMsg>(t.as_str())) {
                        (Some(reg), Ok(ClientMsg::Ready { layer_id, ok, error })) => {
                            hub.renderer_ready(reg, &layer_id, ok, error);
                        }
                        (Some(reg), Ok(ClientMsg::Gone { layer_id })) => {
                            hub.renderer_gone(reg, &layer_id);
                        }
                        _ => {}
                    }
                }
                Some(Ok(_)) => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_types_by_extension() {
        assert_eq!(
            content_type("assets/a-1.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("x.CSS"), "text/css; charset=utf-8");
        assert_eq!(content_type("f.woff2"), "font/woff2");
        assert_eq!(content_type("noext"), "application/octet-stream");
        assert_eq!(content_type("a.exe"), "application/octet-stream");
    }

    #[test]
    fn font_paths_are_plain_and_typed() {
        for ok in [
            "barlow.css",
            "barlow/barlow-latin-400-normal.woff2",
            "a_b-c.css",
        ] {
            assert!(is_font_path(ok), "{ok}");
        }
        for bad in [
            "",
            "../index.html",
            "../fonts/barlow.css",
            "barlow/../../x.css",
            "./barlow.css",
            "/barlow.css",
            "barlow//x.css",
            "barlow\\x.css",
            "C:x.css",
            "barlow.js",
            "barlow.woff",
            "index.html",
        ] {
            assert!(!is_font_path(bad), "{bad}");
            assert_eq!(font_response(bad).status(), StatusCode::NOT_FOUND, "{bad}");
        }
        // Tolerates a renderer dist without fonts: a missing file is a 404.
        assert_eq!(
            font_response("definitely-missing.css").status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn built_fonts_are_served_with_types_and_immutable_cache() {
        // Only meaningful once the renderer build has put fonts into its dist; otherwise there is
        // nothing to check beyond the 404 above.
        let fonts: Vec<String> = RendererDist::iter()
            .map(|p| p.replace('\\', "/"))
            .filter_map(|p| p.strip_prefix("fonts/").map(str::to_string))
            .filter(|p| is_font_path(p))
            .collect();
        for (ext, ty) in [
            (".css", "text/css; charset=utf-8"),
            (".woff2", "font/woff2"),
        ] {
            if let Some(path) = fonts.iter().find(|p| p.ends_with(ext)) {
                let res = font_response(path);
                assert_eq!(res.status(), StatusCode::OK, "{path}");
                let h = res.headers();
                assert_eq!(h[header::CONTENT_TYPE], ty, "{path}");
                assert_eq!(
                    h[header::CACHE_CONTROL],
                    "public, max-age=31536000, immutable"
                );
            }
        }
    }

    #[test]
    fn shared_text_reuses_the_payload() {
        let json: Arc<str> = Arc::from(r#"{"t":"clear"}"#);
        let Message::Text(t) = shared_text(&json) else {
            panic!("not text")
        };
        assert_eq!(t.as_str(), &*json);
        assert_eq!(t.as_str().as_ptr(), json.as_ptr(), "no copy");
    }
}
