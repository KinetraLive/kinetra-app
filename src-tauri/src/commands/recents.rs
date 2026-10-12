use tauri::State;

use crate::core::events::names;
use crate::core::model::RecentProject;
use crate::core::recents;
use crate::core::state::AppState;
use crate::error::AppResult;

/// Returns recents from memory immediately (`missing: false`), then checks existence in the
/// background and emits `recents-checked` with `missing` filled in.
#[tauri::command]
pub fn list_recents(state: State<'_, AppState>) -> AppResult<Vec<RecentProject>> {
    let list = state.recents();
    if !list.is_empty() {
        let events = state.events().clone();
        let to_check = list.clone();
        let spawned = std::thread::Builder::new()
            .name("recents-check".into())
            .spawn(move || {
                let checked = recents::check_missing(&to_check);
                match serde_json::to_value(&checked) {
                    Ok(payload) => events.emit(names::RECENTS_CHECKED, payload),
                    Err(e) => tracing::warn!("recents-checked payload: {e}"),
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("could not start recents check: {e}");
        }
    }
    Ok(list)
}

#[tauri::command]
pub fn remove_recent(state: State<'_, AppState>, path: String) -> AppResult<Vec<RecentProject>> {
    Ok(state.remove_recent(&path))
}

#[tauri::command]
pub fn clear_recents(state: State<'_, AppState>) -> AppResult<()> {
    state.clear_recents();
    Ok(())
}
