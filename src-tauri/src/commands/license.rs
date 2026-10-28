//! Sign-in and licensing commands. None of them takes license state from the UI: they start
//! actions (sign in, activate, deactivate, redeem) or return the read-only view. The core
//! decides (`core::cloud::service`).

use serde::Serialize;
use tauri::State;

use crate::core::cloud::api::{License, SlotReq};
use crate::core::cloud::config;
use crate::core::cloud::service::{LicenseView, RedeemResult, SigninView};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

#[tauri::command]
pub async fn license_view(state: State<'_, AppState>) -> AppResult<LicenseView> {
    let state = state.inner().clone();
    blocking(move || Ok(state.license_view())).await
}

/// Starts the device flow; returns the code to show. Progress comes as `license-changed`.
#[tauri::command]
pub async fn sign_in_start(state: State<'_, AppState>) -> AppResult<SigninView> {
    state.inner().clone().sign_in_start().await
}

#[tauri::command]
pub fn sign_in_cancel(state: State<'_, AppState>) {
    state.sign_in_cancel();
}

#[tauri::command]
pub async fn sign_out(state: State<'_, AppState>) -> AppResult<()> {
    state.inner().clone().sign_out().await
}

#[tauri::command]
pub async fn license_activate(
    state: State<'_, AppState>,
    slot: SlotReq,
    license_id: Option<String>,
) -> AppResult<()> {
    state
        .inner()
        .clone()
        .license_activate(slot, license_id)
        .await
}

#[tauri::command]
pub async fn license_deactivate(
    state: State<'_, AppState>,
    activation_id: String,
) -> AppResult<()> {
    state
        .inner()
        .clone()
        .license_deactivate(&activation_id)
        .await
}

/// "Pro is active on <computer>. Move Pro here?"
#[tauri::command]
pub async fn license_move_here(state: State<'_, AppState>, activation_id: String) -> AppResult<()> {
    state
        .inner()
        .clone()
        .license_move_here(&activation_id)
        .await
}

#[tauri::command]
pub async fn license_redeem(state: State<'_, AppState>, key: String) -> AppResult<RedeemResult> {
    state.inner().clone().license_redeem(&key).await
}

#[tauri::command]
pub async fn license_list(state: State<'_, AppState>) -> AppResult<Vec<License>> {
    state.inner().clone().license_list().await
}

#[tauri::command]
pub async fn license_check_now(state: State<'_, AppState>) -> AppResult<()> {
    state.inner().clone().license_check_now().await
}

/// The account site's pages (one config place in the core).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudLinks {
    pub account: &'static str,
    pub pricing: &'static str,
    pub download: &'static str,
}

#[tauri::command]
pub fn cloud_links() -> CloudLinks {
    CloudLinks {
        account: config::ACCOUNT_URL,
        pricing: config::PRICING_URL,
        download: config::DOWNLOAD_URL,
    }
}
