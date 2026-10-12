//! Control mode: persisted field edits, Clear, Clear all and PANIC. In memory plus a broadcast;
//! `set_layer_fields` also marks the screen dirty for a coalesced write (≤ 1 per ~0.8 s per
//! screen, flushed on close and exit). Every command answers `noProject` without a project.
//!
//! Every change also reaches the UI as `live-changed` (one `LiveScreenView` per screen whose view
//! changed), so callers may rely on either the return value or the event.

use tauri::State;

use crate::core::live::{LiveLayerView, LiveScreenView};
use crate::core::model::ControlsDoc;
use crate::core::protocol::Fields;
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// Sets a layer's field values in its screen (saved) and applies them by the live rules: off
/// air now; on air follows `liveEdit` (`onTake` stages until the next `live_show`). Unknown keys
/// and bound fields are `invalid`; an unknown screen or layer is `notFound`.
///
/// Async on the blocking pool: it serializes with screen saves and may load the layer's template
/// from disk. Await it before a `live_show` that must include the edit.
#[tauri::command]
pub async fn set_layer_fields(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
    fields: Fields,
) -> AppResult<LiveLayerView> {
    let state = state.inner().clone();
    blocking(move || state.set_layer_fields(&screen_id, &layer_id, fields)).await
}

/// Every on-air layer of the screen plays out (animated).
#[tauri::command]
pub fn live_clear(state: State<'_, AppState>, screen_id: String) -> AppResult<LiveScreenView> {
    state.live_clear(&screen_id)
}

/// Every on-air layer of every user screen plays out (animated).
#[tauri::command]
pub fn live_clear_all(state: State<'_, AppState>) -> AppResult<Vec<LiveScreenView>> {
    state.live_clear_all()
}

/// PANIC: every layer of every screen off air at once (renderers get `cut`: no out-animation),
/// auto-outs cancelled. Staged changes are kept. Returns the user screens' views.
#[tauri::command]
pub fn live_panic(state: State<'_, AppState>) -> AppResult<Vec<LiveScreenView>> {
    state.live_panic()
}

// ---- MIDI mappings (controls.json) -----------------------------------------------------------

/// The open project's MIDI mappings. `noProject` without a project.
#[tauri::command]
pub fn get_controls(state: State<'_, AppState>) -> AppResult<ControlsDoc> {
    state.get_controls()
}

/// Validates and stores the mappings (written coalesced); `invalid` for a bad mapping. Conflicts
/// are allowed (shown, not rejected). Emits `controls-changed` when something changed.
#[tauri::command]
pub fn save_controls(state: State<'_, AppState>, doc: ControlsDoc) -> AppResult<ControlsDoc> {
    state.save_controls(doc)
}
