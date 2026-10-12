use tauri::{AppHandle, State, WebviewWindow};

use crate::core::events::with_origin;
use crate::core::model::{Prefs, UiState};
use crate::core::state::AppState;
use crate::error::AppResult;

#[tauri::command]
pub fn get_prefs(state: State<'_, AppState>) -> AppResult<Prefs> {
    Ok(state.prefs())
}

/// Replaces prefs in memory and schedules a coalesced write. Returns the stored value
/// (clamped where out of range).
#[tauri::command]
pub fn set_prefs(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    prefs: Prefs,
) -> AppResult<Prefs> {
    let before = state.prefs().zoom;
    let stored = with_origin(window.label(), || state.set_prefs(prefs));
    if stored.zoom != before {
        super::app::apply_zoom_all(&app, stored.zoom);
    }
    Ok(stored)
}

/// Errors with `noProject` when no project is open.
#[tauri::command]
pub fn get_ui_state(state: State<'_, AppState>) -> AppResult<UiState> {
    state.ui_state()
}

#[tauri::command]
pub fn set_ui_state(
    window: WebviewWindow,
    state: State<'_, AppState>,
    ui_state: UiState,
) -> AppResult<UiState> {
    with_origin(window.label(), || state.set_ui_state(ui_state))
}
