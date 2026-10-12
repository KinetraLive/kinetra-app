//! Recipes (pre-made, declarative automation: slideshow, schedule, countdown) and the schedule
//! engine's operator actions. Recipes are installed from the built-in gallery
//! (`presets/recipes/<id>/`) into `<project>/recipes/<id>.json`; their flows run server-side.
//!
//! Every command answers `noProject` without a project. Error codes: `invalid`, `notFound`,
//! `io`, `newerVersion`, `noProject`.

use tauri::State;

use crate::core::model::{InstallRecipe, RecipeInfo, RecipeInstance, ScheduleAction, ScheduleView};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// The built-in recipes, in gallery order.
#[tauri::command]
pub fn list_recipes() -> AppResult<Vec<RecipeInfo>> {
    Ok(crate::core::recipes::list())
}

/// The project's installed recipes, in creation order.
#[tauri::command]
pub async fn list_recipe_instances(state: State<'_, AppState>) -> AppResult<Vec<RecipeInstance>> {
    state.list_recipe_instances()
}

/// Installs a recipe: creates its sources (with sample rows), templates (from presets), layers
/// (on an existing or a new screen) and its flows (deployed, disarmed), then returns the instance.
#[tauri::command]
pub async fn install_recipe(
    state: State<'_, AppState>,
    request: InstallRecipe,
) -> AppResult<RecipeInstance> {
    let state = state.inner().clone();
    blocking(move || state.install_recipe(&request)).await
}

/// Removes a recipe instance. With `delete_created`, also deletes the source, templates,
/// layers and flows it created (layers on air play out first); otherwise they stay as normal items.
#[tauri::command]
pub async fn remove_recipe(
    state: State<'_, AppState>,
    id: String,
    delete_created: bool,
) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.remove_recipe(&id, delete_created)).await
}

/// Points a recipe whose source is gone (or any recipe) at another source of the same kind:
/// updates the instance's `sourceId`, every reference to the old source in its flows (deployed
/// and draft; redeployed, armed unchanged) and the bindings of its own layers. Returns the
/// updated instance. `notFound` for an unknown recipe or source; `invalid` for a source of the
/// wrong kind (slideshow / sponsor wall: text; schedule / countdown: schedule).
#[tauri::command]
pub async fn relink_recipe_source(
    state: State<'_, AppState>,
    id: String,
    source_id: String,
) -> AppResult<RecipeInstance> {
    let state = state.inner().clone();
    blocking(move || state.relink_recipe_source(&id, &source_id)).await
}

/// The schedule source's items with computed statuses (NOW / UP NEXT / …).
#[tauri::command]
pub async fn schedule_view(
    state: State<'_, AppState>,
    source_id: String,
) -> AppResult<ScheduleView> {
    state.schedule_view(&source_id)
}

/// An operator action (Start now, End, Set next, Delay, …): saved, re-planned and applied to
/// every bound screen at once (automation: on air directly). Returns the new view.
#[tauri::command]
pub async fn schedule_action(
    state: State<'_, AppState>,
    source_id: String,
    action: ScheduleAction,
) -> AppResult<ScheduleView> {
    let state = state.inner().clone();
    blocking(move || state.schedule_action(&source_id, action)).await
}
