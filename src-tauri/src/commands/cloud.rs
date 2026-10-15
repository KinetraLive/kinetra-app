//! Cloud backup commands. The core decides access (no request unless the account has
//! `pro.cloud_sync` or this computer has seen its backups) and everything else; see
//! `core::cloud::projects`.

use tauri::State;

use super::project::blocking;
use crate::core::model::CloudView;
use crate::core::state::AppState;
use crate::error::AppResult;

/// Cloud backup as the UI shows it (mirrored by `cloud-changed`).
#[tauri::command]
pub async fn cloud_view(state: State<'_, AppState>) -> AppResult<CloudView> {
    let state = state.inner().clone();
    blocking(move || Ok(state.cloud_view())).await
}

/// Asks the server for the quota now ("Check now"). `onDemand` ("Check for backups") asks even
/// without Pro.
#[tauri::command]
pub async fn cloud_refresh(state: State<'_, AppState>, on_demand: bool) -> AppResult<CloudView> {
    let state = state.inner().clone();
    state.cloud_refresh(on_demand).await
}
