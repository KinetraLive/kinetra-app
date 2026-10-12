use std::path::PathBuf;

use tauri::{AppHandle, State, WebviewWindow};

use super::project::{blocking, blocking_from};
use crate::core::model::{ImportPreview, SettingsSection};
use crate::core::settings_io;
use crate::core::state::AppState;
use crate::error::AppResult;

/// Writes prefs (+ project settings and UI state when a project is open) to `path`.
/// Never includes secrets.
#[tauri::command]
pub async fn export_settings(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || settings_io::export(&state, &PathBuf::from(path))).await
}

#[tauri::command]
pub async fn preview_settings_import(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<ImportPreview> {
    let state = state.inner().clone();
    blocking(move || {
        let file = settings_io::read_file(&PathBuf::from(path))?;
        settings_io::preview(&state, &file)
    })
    .await
}

/// Takes a `pre-import` snapshot when a project is open, then applies only `sections`.
#[tauri::command]
pub async fn import_settings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
    sections: Vec<SettingsSection>,
) -> AppResult<()> {
    let state = state.inner().clone();
    let zoom = state.prefs().zoom;
    let st = state.clone();
    blocking_from(&window, move || {
        let file = settings_io::read_file(&PathBuf::from(path))?;
        settings_io::apply(&st, &file, &sections)
    })
    .await?;
    let now = state.prefs().zoom;
    if now != zoom {
        super::app::apply_zoom_all(&app, now);
    }
    Ok(())
}
