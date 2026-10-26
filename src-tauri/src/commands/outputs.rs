//! Display outputs (see `core::outputs`): displays, statuses, test patterns, Identify. Output
//! configs are saved through the prefs store (`set_prefs` with `Prefs.outputs`, clamped); every
//! change reconciles the windows.
//!
//! Enumerating displays reads the registry, and Identify creates windows (which must never
//! happen on the main thread from a command), so these run on the blocking pool.

use tauri::State;

use crate::core::model::{DisplayInfo, OutputStatus, TestPattern};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// The connected displays (enumerated now).
#[tauri::command]
pub async fn list_displays(state: State<'_, AppState>) -> AppResult<Vec<DisplayInfo>> {
    let state = state.inner().clone();
    blocking(move || Ok(state.list_displays())).await
}

/// Opens (`true`) or closes (`false`) a display watch (counted; Preferences › Outputs): while
/// one is open, display changes are pushed as `displays-changed`. Returns the displays now.
#[tauri::command]
pub async fn watch_displays(state: State<'_, AppState>, on: bool) -> AppResult<Vec<DisplayInfo>> {
    let state = state.inner().clone();
    blocking(move || Ok(state.watch_displays(on))).await
}

/// Every configured output's status.
#[tauri::command]
pub async fn list_outputs(state: State<'_, AppState>) -> AppResult<Vec<OutputStatus>> {
    let state = state.inner().clone();
    blocking(move || Ok(state.list_outputs())).await
}

/// Shows a test pattern on an output and its key pair, or the screen again (`null`). An off
/// output shows while its pattern does.
#[tauri::command]
pub async fn output_test_pattern(
    state: State<'_, AppState>,
    id: String,
    pattern: Option<TestPattern>,
) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.output_test_pattern(&id, pattern)).await
}

/// Shows each display's name, resolution and refresh rate full-screen for 3 s.
#[tauri::command]
pub async fn identify_displays(state: State<'_, AppState>) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.identify_displays()).await
}

/// Whether NDI outputs can run here, whether the NDI Runtime is installed (a file check; nothing
/// is loaded), its download link and a new output's frame rate for the system's region. Found
/// after an output failed without it: that output starts.
#[tauri::command]
pub async fn ndi_info(state: State<'_, AppState>) -> AppResult<crate::ndi::NdiInfo> {
    let state = state.inner().clone();
    blocking(move || {
        let info = crate::ndi::info();
        if info.installed {
            state.outputs_ndi_runtime_found();
        }
        Ok(info)
    })
    .await
}

/// Turns every output off (Output › All outputs off).
#[tauri::command]
pub async fn all_outputs_off(state: State<'_, AppState>) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || {
        state.all_outputs_off();
        Ok(())
    })
    .await
}
