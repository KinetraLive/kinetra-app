//! The local Control API (see `core::control_api`): the switch, the token (read, regenerate)
//! and the connected clients. Thin: the core decides; the token never leaves the keychain except
//! through `control_api_token`, for the Copy button.

use tauri::State;

use crate::core::control_api::ControlApiView;
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// The switch and the connected clients.
#[tauri::command]
pub async fn control_api_view(state: State<'_, AppState>) -> AppResult<ControlApiView> {
    Ok(state.control_api_view())
}

/// Turns the Control API on (a token is made the first time) or off (clients disconnect).
#[tauri::command]
pub async fn control_api_set(state: State<'_, AppState>, on: bool) -> AppResult<ControlApiView> {
    let state = state.inner().clone();
    blocking(move || state.control_api_set_enabled(on)).await
}

/// The token (made when there is none), for Copy.
#[tauri::command]
pub async fn control_api_token(state: State<'_, AppState>) -> AppResult<String> {
    let state = state.inner().clone();
    blocking(move || state.control_api_token()).await
}

/// A new token; every connected client is disconnected.
#[tauri::command]
pub async fn control_api_regenerate_token(state: State<'_, AppState>) -> AppResult<String> {
    let state = state.inner().clone();
    blocking(move || state.control_api_regenerate_token()).await
}

/// Disconnects one client.
#[tauri::command]
pub async fn control_api_disconnect(state: State<'_, AppState>, id: u64) -> AppResult<()> {
    state.control_api_disconnect(id);
    Ok(())
}
