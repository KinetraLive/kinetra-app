//! Automation: timers (Control) and source auto-advance (macros are now Cue flows). All of it runs server-side
//! (tokio tasks on Tauri's runtime), so it keeps going whichever page is open, and stops with the
//! project. Automation changes go on air directly (they bypass `liveEdit = onTake`).
//!
//! Every command answers `noProject` when no project is open. Error codes: `invalid`,
//! `notFound`, `io`, `noProject`.

use tauri::State;

use crate::core::live::LiveLayerView;
use crate::core::model::{AutoAdvance, SourceSummary, TimerAction};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// One state change of a `timer` field (`key`) of a layer: saved to the screen and applied on
/// air at once (one `fields` message; renderers compute the display, no ticks). A countdown with
/// `end: hide` takes the layer off air at zero. `invalid` if `key` is not a timer field.
#[tauri::command]
pub async fn set_timer(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
    key: String,
    action: TimerAction,
) -> AppResult<LiveLayerView> {
    let state = state.inner().clone();
    blocking(move || state.set_timer(&screen_id, &layer_id, &key, action)).await
}

/// Turns auto-advance of a text source on (settings) or off (null). Saved in the source file;
/// armed only while a bound layer is on air.
#[tauri::command]
pub async fn set_auto_advance(
    state: State<'_, AppState>,
    source_id: String,
    auto: Option<AutoAdvance>,
) -> AppResult<SourceSummary> {
    let state = state.inner().clone();
    blocking(move || state.set_auto_advance(&source_id, auto)).await
}
