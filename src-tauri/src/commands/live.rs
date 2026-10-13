//! Output server status and live (on-air) control from the app. All in memory plus a broadcast,
//! so these stay sync and cheap.

use tauri::State;

use crate::core::live::{LiveLayerView, LiveScreenView, PreviewView};
use crate::core::model::{OscStatus, ServerStatus, StudioExit, StudioOp, StudioSettings};
use crate::core::protocol::Fields;
use crate::core::state::AppState;
use crate::error::AppResult;
use crate::server::{OscManager, ServerManager};

#[tauri::command]
pub fn server_status(server: State<'_, ServerManager>) -> AppResult<ServerStatus> {
    Ok(server.status())
}

/// The OSC listener (`osc-status` reports changes).
#[tauri::command]
pub fn osc_status(osc: State<'_, OscManager>) -> AppResult<OscStatus> {
    Ok(osc.status())
}

/// `noProject` when no project is open (live state only exists for an open project).
fn ensure_project(state: &AppState) -> AppResult<()> {
    state.project_dir().map(|_| ())
}

/// The user's screens in the live state, by name (case-insensitive), then id. Reserved screens
/// (the test screen and `__*` system screens such as the template preview) are never listed.
/// `noProject` when no project is open.
#[tauri::command]
pub fn list_screens(state: State<'_, AppState>) -> AppResult<Vec<LiveScreenView>> {
    screens(&state)
}

fn screens(state: &AppState) -> AppResult<Vec<LiveScreenView>> {
    ensure_project(state)?;
    Ok(state.live().user_screen_views())
}

/// `noProject` when no project is open; `notFound` for an unknown screen.
#[tauri::command]
pub fn live_screen(state: State<'_, AppState>, screen_id: String) -> AppResult<LiveScreenView> {
    ensure_project(&state)?;
    Ok(state.live().screen_view(&screen_id)?)
}

#[tauri::command]
pub fn live_show(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
) -> AppResult<LiveLayerView> {
    ensure_project(&state)?;
    Ok(state.live().show(&screen_id, &layer_id)?)
}

#[tauri::command]
pub fn live_hide(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
) -> AppResult<LiveLayerView> {
    ensure_project(&state)?;
    Ok(state.live().hide(&screen_id, &layer_id)?)
}

/// Merges `fields`; unknown keys are `invalid`. Goes on air immediately if the layer is on air.
#[tauri::command]
pub fn live_set_fields(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
    fields: Fields,
) -> AppResult<LiveLayerView> {
    ensure_project(&state)?;
    Ok(state.live().set_fields(&screen_id, &layer_id, fields)?)
}

// ---- Studio mode (Preview / Program) ----------------------------------------------------------

/// Stores the Studio settings (in ui-state) and turns Studio mode on or off. Turning it off while
/// a Preview has changes needs `exit` (`take` | `discard` | `keepStaged`): `invalid` "Preview has
/// changes" otherwise. In memory; ui-state.json is written coalesced.
#[tauri::command]
pub fn set_studio(
    state: State<'_, AppState>,
    settings: StudioSettings,
    exit: Option<StudioExit>,
) -> AppResult<()> {
    state.set_studio(settings, exit)
}

/// The one entry point for takes, cues, the T-bar and clearing a Preview (`AppState::
/// apply_studio_op`). Returns the screen's Preview. `invalid` when Studio mode is off.
#[tauri::command]
pub fn studio_op(
    state: State<'_, AppState>,
    screen_id: String,
    op: StudioOp,
) -> AppResult<PreviewView> {
    state.apply_studio_op(&screen_id, op)
}

/// A screen's Preview (Program + diff).
#[tauri::command]
pub fn preview_view(state: State<'_, AppState>, screen_id: String) -> AppResult<PreviewView> {
    state.preview_view(&screen_id)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::events::NullSink;
    use crate::core::live::{LiveState, PREVIEW_SCREEN_ID, TEST_LAYER_ID, TEST_SCREEN_ID};
    use crate::error::AppError;

    #[test]
    fn list_screens_needs_a_project_and_excludes_reserved_screens() {
        let tmp = tempfile::tempdir().unwrap();
        let st = AppState::load(tmp.path().join("config"), Arc::new(NullSink)).unwrap();
        assert!(matches!(screens(&st), Err(AppError::NoProject)));

        st.create_project(tmp.path(), "Show").unwrap();
        // The live state has `test` and `__preview`, but neither is a user screen.
        assert!(st.live().has_screen(TEST_SCREEN_ID));
        assert!(st.live().has_screen(PREVIEW_SCREEN_ID));
        assert!(screens(&st).unwrap().is_empty());

        // A user screen is listed and reflects live state.
        let mut live = LiveState::for_project();
        let mut user = live.screens[0].clone();
        user.info.id = "main".into();
        live.screens.push(user);
        st.live().reset(live);
        st.live().show("main", TEST_LAYER_ID).unwrap();
        let list = screens(&st).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].screen.id, "main");
        assert!(list[0].layers[0].on_air);

        st.close_project();
        assert!(matches!(screens(&st), Err(AppError::NoProject)));
    }
}

/// Transport on a media layer's Program playback: play, pause, stop, restart, seek (`ms`,
/// clamped to the layer's in..out). At once, in Studio mode too; never saved. `invalid` for a
/// layer that isn't media; `notFound` for an unknown screen or layer.
#[tauri::command]
pub fn media_control(
    state: State<'_, AppState>,
    screen_id: String,
    layer_id: String,
    action: crate::core::model::MediaAction,
    ms: Option<f64>,
) -> AppResult<()> {
    ensure_project(&state)?;
    let ms = ms.filter(|v| v.is_finite()).map(|v| v.max(0.0) as u64);
    state
        .live()
        .media_control(&screen_id, &layer_id, action, ms)?;
    Ok(())
}
