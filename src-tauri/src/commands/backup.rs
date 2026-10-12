use tauri::{State, WebviewWindow};

use super::project::{blocking, blocking_from};
use crate::core::model::{BackupInfo, BackupReason, Project};
use crate::core::state::AppState;
use crate::error::AppResult;

/// Newest first. Reads manifests only.
#[tauri::command]
pub async fn list_backups(state: State<'_, AppState>) -> AppResult<Vec<BackupInfo>> {
    let state = state.inner().clone();
    blocking(move || state.list_backups()).await
}

/// Manual snapshot; always written, even if unchanged.
#[tauri::command]
pub async fn create_backup(state: State<'_, AppState>) -> AppResult<BackupInfo> {
    let state = state.inner().clone();
    blocking(move || state.create_snapshot(BackupReason::Manual)).await
}

/// Takes a `pre-restore` snapshot, restores `id`, reloads the project.
#[tauri::command]
pub async fn restore_backup(
    window: WebviewWindow,
    state: State<'_, AppState>,
    id: String,
) -> AppResult<Project> {
    let state = state.inner().clone();
    blocking_from(&window, move || state.restore_backup(&id)).await
}

#[tauri::command]
pub async fn delete_backup(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.delete_backup(&id)).await
}
