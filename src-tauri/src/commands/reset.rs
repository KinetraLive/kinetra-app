use tauri::{AppHandle, State, WebviewWindow};

use super::project::blocking_from;
use crate::core::model::ResetScope;
use crate::core::reset;
use crate::core::state::AppState;
use crate::error::AppResult;

/// Takes a `pre-reset` snapshot when a project is open, then resets `scope`.
#[tauri::command]
pub async fn reset_settings(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    scope: ResetScope,
) -> AppResult<()> {
    let state = state.inner().clone();
    let zoom = state.prefs().zoom;
    let st = state.clone();
    blocking_from(&window, move || reset::reset(&st, scope)).await?;
    let now = state.prefs().zoom;
    if now != zoom {
        super::app::apply_zoom_all(&app, now);
    }
    Ok(())
}
