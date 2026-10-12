//! Sources (project `sources/<id>.json`): text sources. A source change re-applies every
//! screen bound to it through the live rules (off air now; on air under `onTake` staged).
//!
//! Disk work runs on the blocking pool (`super::project::blocking`). Every command answers
//! `noProject` when no project is open.
//!
//! Error codes: `invalid` (validation, bad id), `notFound`, `io`, `newerVersion`, `noProject`.

use tauri::State;

use crate::core::model::{ApiTestResult, SourceDoc, SourceStep, SourceSummary};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// Sorted by name (case-insensitive), then id. Includes `usedBy` (screen ids).
#[tauri::command]
pub async fn list_sources(state: State<'_, AppState>) -> AppResult<Vec<SourceSummary>> {
    let state = state.inner().clone();
    blocking(move || state.list_sources()).await
}

#[tauri::command]
pub async fn get_source(state: State<'_, AppState>, id: String) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.get_source(&id)).await
}

/// A new text source with one column (`text`, "Text") and no rows.
#[tauri::command]
pub async fn create_source(state: State<'_, AppState>, name: String) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.create_source(&name)).await
}

/// Validates, writes atomically, sets `updatedAt`, and re-applies the screens bound to it. The
/// document's `index` is saved too (clamped to the rows).
#[tauri::command]
pub async fn save_source(state: State<'_, AppState>, doc: SourceDoc) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.save_source(doc)).await
}

#[tauri::command]
pub async fn rename_source(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<SourceSummary> {
    let state = state.inner().clone();
    blocking(move || state.rename_source(&id, &name)).await
}

#[tauri::command]
pub async fn duplicate_source(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.duplicate_source(&id, &name)).await
}

/// Deletes the file; bound layers fall back to their own values (`sourceMissing` warning).
#[tauri::command]
pub async fn delete_source(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.delete_source(&id)).await
}

/// Next / Prev / go to a row (clamped). In memory + re-apply of bound screens; the index is
/// written coalesced (≤ 1 write per ~0.8 s). Returns the updated summary.
#[tauri::command]
pub async fn step_source(
    state: State<'_, AppState>,
    id: String,
    step: SourceStep,
) -> AppResult<SourceSummary> {
    let state = state.inner().clone();
    blocking(move || state.step_source(&id, step)).await
}

/// A new schedule source (a run of show) with no items, default labels, ripple off.
#[tauri::command]
pub async fn create_schedule(
    state: State<'_, AppState>,
    name: String,
) -> AppResult<crate::core::model::SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.create_schedule(&name)).await
}

// ---- API sources ----

/// A new API source (GET, no URL yet, JSON, no columns, every 10 s while used).
#[tauri::command]
pub async fn create_api_source(state: State<'_, AppState>, name: String) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.create_api_source(&name)).await
}

/// One fetch with the draft (unsaved) settings; secrets by id from the keychain. Nothing is
/// saved. Fetch and mapping errors are in the result's `error`.
#[tauri::command]
pub async fn test_api_source(
    state: State<'_, AppState>,
    doc: SourceDoc,
) -> AppResult<ApiTestResult> {
    let state = state.inner().clone();
    state.test_api_source(doc).await
}

/// Stores a header's value in the OS keychain (write-only; never read back). Returns the doc
/// with the header's `secret` id set and `stored: true`.
#[tauri::command]
pub async fn set_source_secret(
    state: State<'_, AppState>,
    source_id: String,
    header: String,
    value: String,
) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.set_source_secret(&source_id, &header, &value)).await
}

/// Deletes a header's keychain value (the header stays, without a value).
#[tauri::command]
pub async fn clear_source_secret(
    state: State<'_, AppState>,
    source_id: String,
    header: String,
) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.clear_source_secret(&source_id, &header)).await
}

/// Fetch now (clears a 401/403 hold and the backoff).
#[tauri::command]
pub async fn retry_api_source(state: State<'_, AppState>, id: String) -> AppResult<SourceSummary> {
    let state = state.inner().clone();
    blocking(move || state.retry_api_source(&id)).await
}

/// Pauses or resumes polling (saved in the source file).
#[tauri::command]
pub async fn set_api_paused(
    state: State<'_, AppState>,
    id: String,
    paused: bool,
) -> AppResult<SourceSummary> {
    let state = state.inner().clone();
    blocking(move || state.set_api_paused(&id, paused)).await
}

/// The editor shows the source (`on`) or stopped showing it; counted. A watched source polls.
#[tauri::command]
pub async fn watch_api_source(state: State<'_, AppState>, id: String, on: bool) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.watch_api_source(&id, on)).await
}
