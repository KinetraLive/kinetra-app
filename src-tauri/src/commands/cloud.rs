//! Cloud backup commands. The core decides access (no request unless the account has
//! `pro.cloud_sync` or this computer has seen its backups) and everything else; see
//! `core::cloud::projects`.

use tauri::State;

use super::project::blocking;
use crate::core::model::{CloudAnswer, CloudView};
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

/// Backup on or off for the open project.
#[tauri::command]
pub async fn cloud_set_backup(state: State<'_, AppState>, on: bool) -> AppResult<CloudView> {
    let state = state.inner().clone();
    blocking(move || {
        state.cloud_set_backup(on)?;
        Ok(state.cloud_view())
    })
    .await
}

/// The one-time "Backing up to the cloud" notice was shown for the open project.
#[tauri::command]
pub async fn cloud_notice_seen(state: State<'_, AppState>) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.cloud_notice_seen()).await
}

/// The user's answer to the open project's question (in the trash / another computer's).
#[tauri::command]
pub async fn cloud_answer(state: State<'_, AppState>, answer: CloudAnswer) -> AppResult<CloudView> {
    let state = state.inner().clone();
    state.cloud_answer(answer).await?;
    Ok(state.cloud_view())
}
