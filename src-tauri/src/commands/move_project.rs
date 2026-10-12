use std::path::PathBuf;

use tauri::{State, WebviewWindow};

use super::project::blocking_from;
use crate::core::copy::{self, MoveOptions};
use crate::core::model::Project;
use crate::core::state::AppState;
use crate::error::AppResult;

/// Copies the open project to `dest`, verifies, switches to it. Emits `project-move-progress`.
/// The original is deleted only if `delete_original` and verification passed.
#[tauri::command]
pub async fn move_project(
    window: WebviewWindow,
    state: State<'_, AppState>,
    dest: String,
    delete_original: bool,
) -> AppResult<Project> {
    let state = state.inner().clone();
    blocking_from(&window, move || {
        copy::move_project(
            &state,
            &PathBuf::from(dest),
            delete_original,
            &MoveOptions::default(),
        )
    })
    .await
}

/// Asks the running move to stop; no-op when none is running.
#[tauri::command]
pub fn cancel_move(state: State<'_, AppState>) -> AppResult<()> {
    state.cancel_move();
    Ok(())
}
