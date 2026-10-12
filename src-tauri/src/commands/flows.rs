//! Flows: `flows/<id>.json` and the flow engine. Every command answers `noProject` when no
//! project is open. Error codes: `invalid`, `notFound`, `io`, `noProject`. Progress of running
//! flows arrives as `flow-state`.

use tauri::State;

use crate::core::model::{FlowEdge, FlowInfo, FlowNode, NewFlow, SaveFlow};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

#[tauri::command]
pub async fn list_flows(state: State<'_, AppState>) -> AppResult<Vec<FlowInfo>> {
    let state = state.inner().clone();
    blocking(move || state.list_flows()).await
}

#[tauri::command]
pub async fn get_flow(state: State<'_, AppState>, id: String) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.get_flow(&id)).await
}

#[tauri::command]
pub async fn create_flow(state: State<'_, AppState>, flow: NewFlow) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.create_flow(flow)).await
}

#[tauri::command]
pub async fn save_flow(state: State<'_, AppState>, flow: SaveFlow) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.save_flow(flow)).await
}

#[tauri::command]
pub async fn save_flow_draft(
    state: State<'_, AppState>,
    id: String,
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.save_flow_draft(&id, nodes, edges)).await
}

#[tauri::command]
pub async fn deploy_flow(state: State<'_, AppState>, id: String) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.deploy_flow(&id)).await
}

#[tauri::command]
pub async fn discard_flow_draft(state: State<'_, AppState>, id: String) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.discard_flow_draft(&id)).await
}

#[tauri::command]
pub async fn rename_flow(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.rename_flow(&id, &name)).await
}

#[tauri::command]
pub async fn duplicate_flow(state: State<'_, AppState>, id: String) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.duplicate_flow(&id)).await
}

#[tauri::command]
pub async fn delete_flow(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.delete_flow(&id)).await
}

#[tauri::command]
pub async fn set_flow_armed(
    state: State<'_, AppState>,
    id: String,
    armed: bool,
) -> AppResult<FlowInfo> {
    let state = state.inner().clone();
    blocking(move || state.set_flow_armed(&id, armed)).await
}

/// Runs a flow from its Cue trigger (restart/ignore per its `retrigger`). The instance runs as a
/// task; progress arrives as `flow-state`.
#[tauri::command]
pub fn run_flow(state: State<'_, AppState>, id: String, value: Option<String>) -> AppResult<()> {
    state.run_flow(&id, value)
}

#[tauri::command]
pub fn stop_flow(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.stop_flow(&id)
}

/// Validates a graph without saving: errors (block deploy) and warnings per node. `id` (optional):
/// the flow being edited, so a Run flow of itself is reported.
#[tauri::command]
pub async fn check_flow(
    state: State<'_, AppState>,
    nodes: Vec<FlowNode>,
    edges: Vec<FlowEdge>,
    id: Option<String>,
) -> AppResult<crate::core::model::FlowCheck> {
    let state = state.inner().clone();
    blocking(move || state.check_flow_of(id.as_deref(), nodes, edges)).await
}

/// The node editor shows this flow: `flow-debug` batches start (counted subscriptions).
#[tauri::command]
pub fn subscribe_flow_debug(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.subscribe_flow_debug(&id)
}

#[tauri::command]
pub fn unsubscribe_flow_debug(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.unsubscribe_flow_debug(&id)
}

/// The built-in flow templates (gallery order).
#[tauri::command]
pub fn list_flow_templates() -> Vec<crate::core::model::FlowTemplateInfo> {
    crate::core::flow_templates::list()
}

/// Adds a flow template pointed at the given slots; returns the created flows.
#[tauri::command]
pub async fn add_flow_template(
    state: State<'_, AppState>,
    req: crate::core::model::AddFlowTemplate,
) -> AppResult<Vec<FlowInfo>> {
    let state = state.inner().clone();
    blocking(move || state.add_flow_template(req)).await
}
