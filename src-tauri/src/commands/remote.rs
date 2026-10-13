//! Remote control (see `core::remote`): the switch, the view, Lock remote take, "Move it here?",
//! a separate show, Test connection, Connection details and the diagnostics export. The core
//! decides everything (Pro, the project, the protocol); these are thin.

use std::path::PathBuf;

use serde_json::Value;
use tauri::State;

use crate::core::remote::{Activity, CuePosition, RemoteView, TestResult};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// The current view.
#[tauri::command]
pub async fn remote_view(state: State<'_, AppState>) -> AppResult<RemoteView> {
    Ok(state.remote_view())
}

/// Turns remote control on or off for the open project (this session only).
#[tauri::command]
pub async fn remote_set(state: State<'_, AppState>, on: bool) -> AppResult<RemoteView> {
    let state = state.inner().clone();
    blocking(move || state.remote_set(on)).await
}

/// "Start again" after the run ended or remote control stopped.
#[tauri::command]
pub async fn remote_start_again(state: State<'_, AppState>) -> AppResult<RemoteView> {
    let state = state.inner().clone();
    blocking(move || state.remote_start_again()).await
}

/// End remote control (the switch stays on: Start again).
#[tauri::command]
pub async fn remote_end(state: State<'_, AppState>) -> AppResult<RemoteView> {
    let state = state.inner().clone();
    blocking(move || state.remote_end()).await
}

/// Lock remote take.
#[tauri::command]
pub async fn remote_lock(state: State<'_, AppState>, on: bool) -> AppResult<RemoteView> {
    let state = state.inner().clone();
    blocking(move || state.remote_lock(on)).await
}

/// "Move it here?" → Move it here.
#[tauri::command]
pub async fn remote_takeover(state: State<'_, AppState>) -> AppResult<()> {
    state.remote_takeover()
}

/// "Make this a separate show": a new project ID.
#[tauri::command]
pub async fn remote_new_show(state: State<'_, AppState>) -> AppResult<RemoteView> {
    let state = state.inner().clone();
    blocking(move || state.remote_new_show()).await
}

/// Recent remote actions of this run, newest first.
#[tauri::command]
pub async fn remote_activity(state: State<'_, AppState>) -> AppResult<Vec<Activity>> {
    Ok(state.remote_activity())
}

/// The cue list's current and next cue (Control's "Next" mark).
#[tauri::command]
pub async fn cue_position(state: State<'_, AppState>) -> AppResult<CuePosition> {
    let state = state.inner().clone();
    blocking(move || Ok(state.remote_cue_position())).await
}

/// Test connection: a probe while off, a ping on the live connection while on.
#[tauri::command]
pub async fn remote_test(state: State<'_, AppState>) -> AppResult<TestResult> {
    let state = state.inner().clone();
    Ok(state.remote_test().await)
}

/// Opens (`true`) or closes a Connection details watch (counted); returns the view now.
#[tauri::command]
pub async fn remote_diag_watch(state: State<'_, AppState>, on: bool) -> AppResult<Value> {
    Ok(state.remote_diag_watch(on))
}

/// Writes the diagnostics export (no values, names, tokens or account IDs) to `path`.
#[tauri::command]
pub async fn remote_export_diagnostics(state: State<'_, AppState>, path: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.remote_export_diagnostics(&PathBuf::from(path))).await
}
