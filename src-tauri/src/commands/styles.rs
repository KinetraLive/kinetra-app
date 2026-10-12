//! The style library: list the styles, apply one to the project or a layer (one undo step), and
//! the project's brand kit. See `core::styles`.
//!
//! Error codes: `invalid` (unknown style, bad brand value, "Nothing to undo"), `notFound`
//! (unknown screen or layer), `io`, `noProject`.

use tauri::{State, WebviewWindow};

use crate::core::model::{
    ApplyStyleResult, ApplyStyleScope, BrandKit, StyleInfo, VariantTransition,
};
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking_from;

/// The design styles of the library, in order (`presets/library.json`).
#[tauri::command]
pub fn list_styles(state: State<'_, AppState>) -> Vec<StyleInfo> {
    state.list_styles()
}

/// Copies the style's variant into project templates made from library presets that lack it,
/// switches the layers in scope by the live rules and (project scope) sets `settings.style`.
#[tauri::command]
pub async fn apply_style(
    window: WebviewWindow,
    state: State<'_, AppState>,
    style: String,
    scope: ApplyStyleScope,
    transition: VariantTransition,
) -> AppResult<ApplyStyleResult> {
    let state = state.inner().clone();
    blocking_from(&window, move || {
        state.apply_style(&style, &scope, transition)
    })
    .await
}

/// Restores the layers (and project style) the `apply_style` with this token changed.
#[tauri::command]
pub async fn undo_apply_style(
    window: WebviewWindow,
    state: State<'_, AppState>,
    token: String,
) -> AppResult<()> {
    let state = state.inner().clone();
    blocking_from(&window, move || state.undo_apply_style(&token)).await
}

/// Validates and saves the brand kit; every screen re-applies by the live rules.
#[tauri::command]
pub async fn save_brand_kit(
    window: WebviewWindow,
    state: State<'_, AppState>,
    brand: BrandKit,
) -> AppResult<BrandKit> {
    let state = state.inner().clone();
    blocking_from(&window, move || state.save_brand_kit(brand)).await
}
