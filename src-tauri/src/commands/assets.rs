//! Images in the project's `assets/` folder, served by the output server at `/assets/<name>`.
//! Image fields and image source cells hold `/assets/<name>`.
//!
//! Every command answers `noProject` without a project. Error codes: `invalid` (type, size,
//! name), `notFound`, `io`, `noProject`.

use tauri::State;

use crate::core::model::AssetInfo;
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// Every image in `assets/`, sorted by name, with `usedBy`.
#[tauri::command]
pub async fn list_assets(state: State<'_, AppState>) -> AppResult<Vec<AssetInfo>> {
    let state = state.inner().clone();
    blocking(move || state.list_assets()).await
}

/// Copies the given files (absolute paths from the file dialog) into `assets/` with path-safe,
/// unique names: images (≤ 20 MB), video and audio (≤ 8 GiB; kernel copy, no progress: the UI
/// uses `import_media`). Returns the imported assets
/// (a file that fails is reported in the error; earlier ones stay imported).
#[tauri::command]
pub async fn import_assets(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> AppResult<Vec<AssetInfo>> {
    let state = state.inner().clone();
    blocking(move || state.import_assets(&paths)).await
}

/// Deletes an image. Fields and cells that use it keep the URL (it shows nothing).
#[tauri::command]
pub async fn delete_asset(state: State<'_, AppState>, name: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.delete_asset(&name)).await
}

/// Renames an image (keeping its extension) and updates every screen and source that uses it.
#[tauri::command]
pub async fn rename_asset(
    state: State<'_, AppState>,
    name: String,
    new_name: String,
) -> AppResult<AssetInfo> {
    let state = state.inner().clone();
    blocking(move || state.rename_asset(&name, &new_name)).await
}

/// Starts importing files (images, video, audio) into `assets/` as a background job; returns its
/// id. Progress per file arrives as `asset-import` events (≤ 10/s, then one with `done`); a file
/// that fails reports its `error` and the job goes on with the next.
#[tauri::command]
pub async fn import_media(state: State<'_, AppState>, paths: Vec<String>) -> AppResult<String> {
    state.import_media(paths)
}

/// Cancels an import job: the file being copied is removed, the rest are skipped. A job that
/// already finished is a no-op.
#[tauri::command]
pub fn cancel_import(job: String) -> AppResult<()> {
    crate::core::assets::cancel_job(&job);
    Ok(())
}
