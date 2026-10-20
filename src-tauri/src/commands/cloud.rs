//! Cloud backup commands. The core decides access (no request unless the account has
//! `pro.cloud_sync` or this computer has seen its backups) and everything else; see
//! `core::cloud::projects`.

use tauri::State;

use super::project::blocking;
use crate::core::model::{
    CloudAnswer, CloudProjectSummary, CloudRestoreMode, CloudRestorePlan, CloudRestoreRequest,
    CloudVersionPage, CloudVersionView, CloudView,
};
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

/// "Back up now": starts a backup of the open project (returns at once; `cloud-changed` follows).
#[tauri::command]
pub async fn cloud_backup_now(state: State<'_, AppState>) -> AppResult<CloudView> {
    let state = state.inner().clone();
    blocking(move || {
        state.cloud_backup_now()?;
        Ok(state.cloud_view())
    })
    .await
}

/// "Dismiss" on the conflict notice: both versions stay; the server marks it resolved.
#[tauri::command]
pub async fn cloud_conflict_dismiss(state: State<'_, AppState>) -> AppResult<CloudView> {
    let state = state.inner().clone();
    state.cloud_conflict_dismiss().await?;
    Ok(state.cloud_view())
}

/// Whether to ask "Back up before quitting?": changes not in the cloud and nothing on air.
#[tauri::command]
pub async fn cloud_exit_should_ask(state: State<'_, AppState>) -> AppResult<bool> {
    let state = state.inner().clone();
    blocking(move || Ok(state.cloud_exit_should_ask())).await
}

/// "Back up and quit": a backup with `reason: close` (follow it on `cloud-changed`).
#[tauri::command]
pub async fn cloud_backup_before_exit(state: State<'_, AppState>) -> AppResult<CloudView> {
    let state = state.inner().clone();
    blocking(move || {
        state.cloud_backup_before_exit()?;
        Ok(state.cloud_view())
    })
    .await
}

/// A page of the open project's history, newest first (`before`: the last page's `next`).
#[tauri::command]
pub async fn cloud_versions(
    state: State<'_, AppState>,
    before: Option<String>,
) -> AppResult<CloudVersionPage> {
    let state = state.inner().clone();
    state.cloud_versions(before).await
}

/// Keep a version (retention never deletes it) or let it go.
#[tauri::command]
pub async fn cloud_version_keep(
    state: State<'_, AppState>,
    id: String,
    kept: bool,
) -> AppResult<CloudVersionView> {
    let state = state.inner().clone();
    state.cloud_version_keep(&id, kept).await
}

/// A version's label (null or blank clears it).
#[tauri::command]
pub async fn cloud_version_label(
    state: State<'_, AppState>,
    id: String,
    label: Option<String>,
) -> AppResult<CloudVersionView> {
    let state = state.inner().clone();
    state.cloud_version_label(&id, label).await
}

/// Deletes a version (not the newest).
#[tauri::command]
pub async fn cloud_version_delete(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    state.cloud_version_delete(&id).await
}

/// The account's cloud projects (with the trash when `trashed`).
#[tauri::command]
pub async fn cloud_projects_list(
    state: State<'_, AppState>,
    trashed: bool,
) -> AppResult<Vec<CloudProjectSummary>> {
    let state = state.inner().clone();
    state.cloud_projects_list(trashed).await
}

#[tauri::command]
pub async fn cloud_project_rename(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<CloudProjectSummary> {
    let state = state.inner().clone();
    state.cloud_project_rename(&id, &name).await
}

/// Moves a cloud project to the trash (deleted for good after 7 days).
#[tauri::command]
pub async fn cloud_project_trash(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<CloudProjectSummary> {
    let state = state.inner().clone();
    state.cloud_project_trash(&id).await
}

/// Restores a cloud project from the trash.
#[tauri::command]
pub async fn cloud_project_untrash(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<CloudProjectSummary> {
    let state = state.inner().clone();
    state.cloud_project_untrash(&id).await
}

/// Where a restore would go, and (Replace) whether the project is backed up or copied first.
#[tauri::command]
pub async fn cloud_restore_plan(
    state: State<'_, AppState>,
    mode: CloudRestoreMode,
    name: String,
    at: Option<String>,
) -> AppResult<CloudRestorePlan> {
    let state = state.inner().clone();
    blocking(move || state.cloud_restore_plan(mode, &name, at.as_deref())).await
}

/// Starts restoring a version (returns at once; `cloud-changed` follows it).
#[tauri::command]
pub async fn cloud_restore_start(
    state: State<'_, AppState>,
    request: CloudRestoreRequest,
) -> AppResult<CloudView> {
    let state = state.inner().clone();
    blocking(move || {
        state.cloud_restore_start(request)?;
        Ok(state.cloud_view())
    })
    .await
}

/// Stops a running restore (the project stays as it was).
#[tauri::command]
pub async fn cloud_restore_cancel(state: State<'_, AppState>) -> AppResult<()> {
    state.inner().cloud_restore_cancel();
    Ok(())
}

/// Forgets a finished restore's result.
#[tauri::command]
pub async fn cloud_restore_dismiss(state: State<'_, AppState>) -> AppResult<CloudView> {
    let state = state.inner().clone();
    state.cloud_restore_dismiss();
    Ok(state.cloud_view())
}
