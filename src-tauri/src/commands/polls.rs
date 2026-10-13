//! Live poll commands. The core decides access (`pro.polls` for creating, editing and opening;
//! no poll request at all unless the account has it or this machine knows the poll) and checks
//! every input; see `core::polls`.

use tauri::State;

use super::project::blocking;
use crate::core::model::{PollData, PollInput, PollQuota, SourceDoc};
use crate::core::state::AppState;
use crate::error::AppResult;

/// Limits and usage of the account's plan; null when no request may be made (Free).
#[tauri::command]
pub async fn polls_quota(state: State<'_, AppState>) -> AppResult<Option<PollQuota>> {
    let state = state.inner().clone();
    state.polls_quota().await
}

/// The account's polls. `onDemand` is "Show my existing polls": asks the server even without Pro.
#[tauri::command]
pub async fn list_polls(state: State<'_, AppState>, on_demand: bool) -> AppResult<Vec<PollData>> {
    let state = state.inner().clone();
    state.list_polls(on_demand).await
}

#[tauri::command]
pub async fn create_poll(state: State<'_, AppState>, input: PollInput) -> AppResult<PollData> {
    let state = state.inner().clone();
    state.create_poll(input).await
}

#[tauri::command]
pub async fn update_poll(
    state: State<'_, AppState>,
    id: String,
    input: PollInput,
) -> AppResult<PollData> {
    let state = state.inner().clone();
    state.update_poll(&id, input).await
}

#[tauri::command]
pub async fn get_poll(state: State<'_, AppState>, id: String) -> AppResult<PollData> {
    let state = state.inner().clone();
    state.get_poll(&id).await
}

#[tauri::command]
pub async fn open_poll(state: State<'_, AppState>, id: String) -> AppResult<PollData> {
    let state = state.inner().clone();
    state.open_poll(&id).await
}

#[tauri::command]
pub async fn close_poll(state: State<'_, AppState>, id: String) -> AppResult<PollData> {
    let state = state.inner().clone();
    state.close_poll(&id).await
}

#[tauri::command]
pub async fn reset_poll(state: State<'_, AppState>, id: String) -> AppResult<PollData> {
    let state = state.inner().clone();
    state.reset_poll(&id).await
}

#[tauri::command]
pub async fn delete_poll(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    state.delete_poll(&id).await
}

/// The Polls page shows `id` live (counted: pair every `on` with an `off`).
#[tauri::command]
pub async fn watch_poll(state: State<'_, AppState>, id: String, on: bool) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.watch_poll(&id, on)).await
}

/// A new poll source for a server poll.
#[tauri::command]
pub async fn create_poll_source(
    state: State<'_, AppState>,
    name: String,
    poll: PollData,
) -> AppResult<SourceDoc> {
    let state = state.inner().clone();
    blocking(move || state.create_poll_source(&name, poll)).await
}

/// The source already showing `pollId`, if any.
#[tauri::command]
pub fn poll_source_for(state: State<'_, AppState>, poll_id: String) -> Option<String> {
    state.poll_source_for(&poll_id)
}

/// The QR code for a voting link (`data:image/svg+xml,…`), made locally.
#[tauri::command]
pub fn poll_qr(text: String) -> String {
    crate::core::qr::data_url(&text)
}

/// "Add to screen": the Poll graphic on a screen, bound to the poll source.
#[tauri::command]
pub async fn add_poll_to_screen(
    state: State<'_, AppState>,
    source_id: String,
    screen_id: String,
) -> AppResult<crate::core::model::ScreenState> {
    let state = state.inner().clone();
    blocking(move || state.add_poll_to_screen(&source_id, &screen_id)).await
}
