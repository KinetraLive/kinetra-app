use std::path::PathBuf;

use tauri::{State, WebviewWindow};

use crate::core::events::with_origin;
use crate::core::model::{FolderKind, Project, ProjectSettings};
use crate::core::project;
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// Runs disk-touching work on the blocking pool, off the UI event loop.
pub(crate) async fn blocking<T, F>(f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
}

/// [`blocking`] with `window` as the origin of the changes: cross-window events
/// (`prefs-changed`, `ui-state-changed`, `project-changed`) skip the window that asked.
pub(crate) async fn blocking_from<T, F>(window: &WebviewWindow, f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    let label = window.label().to_string();
    blocking(move || with_origin(&label, f)).await
}

/// Classifies a folder without writing anything.
#[tauri::command]
pub async fn inspect_folder(state: State<'_, AppState>, path: String) -> AppResult<FolderKind> {
    let _ = state;
    blocking(move || Ok(project::classify(&PathBuf::from(path))?)).await
}

/// Creates `<parent>/<slug(name)>`, initializes it, and opens it.
#[tauri::command]
pub async fn create_project(
    window: WebviewWindow,
    state: State<'_, AppState>,
    parent: String,
    name: String,
) -> AppResult<Project> {
    let state = state.inner().clone();
    blocking_from(&window, move || {
        state.create_project(&PathBuf::from(parent), &name)
    })
    .await
}

/// Initializes an existing folder (empty, or foreign after explicit opt-in) and opens it.
/// `name` defaults to the folder name.
#[tauri::command]
pub async fn init_project(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
    name: Option<String>,
) -> AppResult<Project> {
    let state = state.inner().clone();
    blocking_from(&window, move || {
        state.init_project(&PathBuf::from(path), name.as_deref())
    })
    .await
}

/// Opens an existing project, closing the current one first.
#[tauri::command]
pub async fn open_project(
    window: WebviewWindow,
    state: State<'_, AppState>,
    path: String,
) -> AppResult<Project> {
    let state = state.inner().clone();
    blocking_from(&window, move || state.open_project(&PathBuf::from(path))).await
}

#[tauri::command]
pub async fn close_project(window: WebviewWindow, state: State<'_, AppState>) -> AppResult<()> {
    let state = state.inner().clone();
    blocking_from(&window, move || {
        state.ensure_not_moving()?;
        state.close_project();
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn current_project(state: State<'_, AppState>) -> AppResult<Option<Project>> {
    Ok(state.current_project())
}

#[tauri::command]
pub async fn update_project_settings(
    window: WebviewWindow,
    state: State<'_, AppState>,
    settings: ProjectSettings,
) -> AppResult<Project> {
    // In-memory update plus a coalesced write; no disk access on this path.
    with_origin(window.label(), || state.set_project_settings(settings))
}
