//! App-level commands: build info, the Preferences window, WebView zoom, full screen, exit.

use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewUrl, WebviewWindow};

use crate::core::events::{names, with_origin};
use crate::core::model::{AppInfo, BuildChannel, Prefs};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// The main window (menu bar, project UI).
pub const MAIN_WINDOW: &str = "main";
/// The Preferences window: one instance, created on open, destroyed on close.
pub const PREFS_WINDOW: &str = "prefs";

/// Logical size of the Preferences window and its minimum.
const PREFS_SIZE: (f64, f64) = (980.0, 720.0);
const PREFS_MIN_SIZE: (f64, f64) = (760.0, 520.0);
/// Like the main window: shown by the page after first paint, or by this fallback.
const SHOW_FALLBACK: Duration = Duration::from_secs(3);

/// The build channel and commit baked in by build.rs.
pub fn build_info() -> (BuildChannel, &'static str) {
    let channel = BuildChannel::parse(env!("KINETRA_CHANNEL")).unwrap_or(BuildChannel::Stable);
    (channel, env!("KINETRA_COMMIT"))
}

#[tauri::command]
pub fn app_info(app: AppHandle) -> AppResult<AppInfo> {
    let path = app.path();
    let (channel, commit) = build_info();
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
        tauri_version: tauri::VERSION.to_string(),
        webview_version: tauri::webview_version().unwrap_or_else(|_| "unknown".into()),
        config_dir: path.app_config_dir()?.display().to_string(),
        log_dir: path.app_log_dir()?.display().to_string(),
        channel,
        commit: commit.to_string(),
    })
}

/// A section id or old `/settings` anchor as the Preferences window takes it: a leading `#` is
/// dropped; 1–32 of `a-z 0-9 -`. Anything else opens the default section.
pub fn section_token(section: &str) -> Option<&str> {
    let s = section.strip_prefix('#').unwrap_or(section);
    let ok = !s.is_empty()
        && s.len() <= 32
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    ok.then_some(s)
}

/// Opens the Preferences window at `section`, or focuses it (asking the page for the section
/// with `prefs-section`). Async: creating a window from a sync command deadlocks on Windows.
#[tauri::command]
pub async fn open_preferences(
    app: AppHandle,
    state: State<'_, AppState>,
    section: Option<String>,
) -> AppResult<()> {
    let section = section
        .as_deref()
        .and_then(section_token)
        .map(str::to_string);
    if let Some(w) = app.get_webview_window(PREFS_WINDOW) {
        if let Some(s) = &section {
            app.emit_to(PREFS_WINDOW, names::PREFS_SECTION, json!({ "section": s }))?;
        }
        let _ = w.unminimize();
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    let url = match &section {
        Some(s) => format!("preferences?section={s}"),
        None => "preferences".to_string(),
    };
    let mut builder =
        tauri::WebviewWindowBuilder::new(&app, PREFS_WINDOW, WebviewUrl::App(url.into()))
            .title("Preferences")
            .inner_size(PREFS_SIZE.0, PREFS_SIZE.1)
            .min_inner_size(PREFS_MIN_SIZE.0, PREFS_MIN_SIZE.1)
            .visible(false);
    builder = match centered_on_main(&app) {
        Some((x, y)) => builder.position(x, y),
        None => builder.center(),
    };
    let w = builder.build()?;
    apply_zoom(&w, state.prefs().zoom);
    show_if_never_shown(app, PREFS_WINDOW);
    Ok(())
}

/// Logical top-left that centers the Preferences window on the main window.
fn centered_on_main<R: Runtime>(app: &AppHandle<R>) -> Option<(f64, f64)> {
    let main = app.get_webview_window(MAIN_WINDOW)?;
    let scale = main.scale_factor().ok()?;
    let pos = main.outer_position().ok()?.to_logical::<f64>(scale);
    let size = main.outer_size().ok()?.to_logical::<f64>(scale);
    Some((
        pos.x + (size.width - PREFS_SIZE.0) / 2.0,
        pos.y + (size.height - PREFS_SIZE.1) / 2.0,
    ))
}

/// Shows window `label` if its page hasn't after [`SHOW_FALLBACK`] (a crash before mount), so
/// it is never invisible.
pub fn show_if_never_shown<R: Runtime>(app: AppHandle<R>, label: &'static str) {
    std::thread::spawn(move || {
        std::thread::sleep(SHOW_FALLBACK);
        if let Some(w) = app.get_webview_window(label) {
            if !w.is_visible().unwrap_or(true) {
                tracing::warn!("{label}: the page never showed its window; showing it");
                let _ = w.show();
            }
        }
    });
}

/// Applies `prefs.zoom` (`None` = 1) to one window.
pub fn apply_zoom<R: Runtime>(w: &WebviewWindow<R>, zoom: Option<f64>) {
    if let Err(e) = w.set_zoom(zoom.unwrap_or(1.0)) {
        tracing::warn!("zoom on {} failed: {e}", w.label());
    }
}

/// Applies `prefs.zoom` to every app window (main, Preferences).
pub fn apply_zoom_all<R: Runtime>(app: &AppHandle<R>, zoom: Option<f64>) {
    for label in [MAIN_WINDOW, PREFS_WINDOW] {
        if let Some(w) = app.get_webview_window(label) {
            apply_zoom(&w, zoom);
        }
    }
}

/// Stores `prefs.zoom` (clamped 0.5–2, steps of 0.1) and applies it to every app window.
/// Returns the stored zoom (1 when reset).
#[tauri::command]
pub fn set_zoom(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    zoom: f64,
) -> AppResult<f64> {
    let stored = with_origin(window.label(), || {
        state.set_prefs(Prefs {
            zoom: Some(zoom),
            ..state.prefs()
        })
    });
    apply_zoom_all(&app, stored.zoom);
    Ok(stored.zoom.unwrap_or(1.0))
}

/// Toggles full screen on the main window; returns the new state.
#[tauri::command]
pub fn toggle_fullscreen(app: AppHandle) -> AppResult<bool> {
    let w = app
        .get_webview_window(MAIN_WINDOW)
        .ok_or_else(|| AppError::Internal("The main window is gone".into()))?;
    let on = !w.is_fullscreen()?;
    w.set_fullscreen(on)?;
    Ok(on)
}

/// Flushes every store and exits (the Exit handler in lib.rs then stops the server, OSC and
/// MIDI). The UI confirms first when anything is on air.
#[tauri::command]
pub fn exit_app(app: AppHandle, state: State<'_, AppState>) {
    state.flush_all();
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_info_has_a_channel_and_a_commit() {
        let (channel, commit) = build_info();
        assert!(matches!(
            channel,
            BuildChannel::Stable | BuildChannel::Canary
        ));
        assert!(
            commit == "unknown"
                || (!commit.is_empty() && commit.bytes().all(|b| b.is_ascii_hexdigit())),
            "{commit}"
        );
        let v = serde_json::to_value(channel).unwrap();
        assert!(v == "stable" || v == "canary");
        assert_eq!(BuildChannel::parse("canary"), Some(BuildChannel::Canary));
        assert_eq!(BuildChannel::parse("beta"), None);
    }

    #[test]
    fn section_tokens() {
        assert_eq!(section_token("about"), Some("about"));
        assert_eq!(section_token("#settings-file"), Some("settings-file"));
        assert_eq!(section_token(""), None);
        assert_eq!(section_token("a&b=c"), None);
        assert_eq!(section_token("About"), None);
        assert_eq!(section_token(&"a".repeat(33)), None);
    }
}
