//! Screens (project `screens/<id>.json`): CRUD plus the layout editor's live drag.
//!
//! Disk work runs on the blocking pool (`super::project::blocking`); `set_layer_rect` is in memory
//! plus a broadcast and stays sync. Every command answers `noProject` when no project is open.
//!
//! Saving applies the live rules: off-air layers update the output now; on-air layers follow
//! `liveEdit` (`onTake` stages the change until the next `live_show`, `immediate` applies it).
//!
//! Error codes: `invalid` (validation, bad id), `notFound` (unknown screen or layer), `io`,
//! `newerVersion` (screen file from a newer Kinetra), `noProject`.

use tauri::State;

use crate::core::live::LiveLayerView;
use crate::core::model::{
    NewScreen, Rect, ScreenDoc, ScreenState, ScreenSummary, VariantTransition,
};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// Newest first by `updatedAt` (cached; invalidated on write and by the watcher).
#[tauri::command]
pub async fn list_screen_docs(state: State<'_, AppState>) -> AppResult<Vec<ScreenSummary>> {
    let state = state.inner().clone();
    blocking(move || state.list_screen_docs()).await
}

#[tauri::command]
pub async fn get_screen(state: State<'_, AppState>, id: String) -> AppResult<ScreenState> {
    let state = state.inner().clone();
    blocking(move || state.get_screen(&id)).await
}

/// Creates `screens/<new id>.json` with no layers and adds it to the live state.
#[tauri::command]
pub async fn create_screen(
    state: State<'_, AppState>,
    screen: NewScreen,
) -> AppResult<ScreenState> {
    let state = state.inner().clone();
    blocking(move || state.create_screen(&screen)).await
}

/// Validates, writes atomically, sets `updatedAt`, diffs against the live state and applies the
/// live rules (layers added/removed/reordered, template switched, fields, vars, rect).
#[tauri::command]
pub async fn save_screen(state: State<'_, AppState>, doc: ScreenDoc) -> AppResult<ScreenState> {
    let state = state.inner().clone();
    blocking(move || state.save_screen(doc)).await
}

#[tauri::command]
pub async fn rename_screen(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<ScreenSummary> {
    let state = state.inner().clone();
    blocking(move || state.rename_screen(&id, &name)).await
}

/// Copies the screen with new screen and layer ids; every layer of the copy starts off air.
#[tauri::command]
pub async fn duplicate_screen(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<ScreenState> {
    let state = state.inner().clone();
    blocking(move || state.duplicate_screen(&id, &name)).await
}

/// Deletes the file and drops the screen from the live state (its renderers get `unknownScreen`).
#[tauri::command]
pub async fn delete_screen(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.delete_screen(&id)).await
}

/// While dragging: moves a layer's box in memory and broadcasts `rect` (no disk write; the editor
/// saves after the drop). Follows the live rules: an on-air layer under `onTake` is staged.
#[tauri::command]
pub fn set_layer_rect(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
    rect: Rect,
) -> AppResult<LiveLayerView> {
    state.set_layer_rect(&screen_id, &layer_id, rect)
}

/// Shows another variant of a layer's template. Saved to the screen (coalesced, like Control
/// field edits); on air it follows `liveEdit` (`onTake`: staged for the next take; `immediate`:
/// swapped now with `transition`). `notFound` for an unknown screen, layer or variant; `invalid`
/// when the layer's template is missing. Returns the screen (document, live view, warnings).
#[tauri::command]
pub async fn set_layer_variant(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
    variant: String,
    transition: VariantTransition,
) -> AppResult<ScreenState> {
    let state = state.inner().clone();
    blocking(move || state.set_layer_variant(&screen_id, &layer_id, &variant, transition)).await
}
