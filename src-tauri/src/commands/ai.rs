//! AI template commands. The core gates every start (`pro.ai`, full access) and checks the
//! input; the UI's limits come from `ai_limits` (one source of truth).

use serde::Serialize;
use tauri::State;

use crate::core::cloud::ai::{
    AiGenerateInput, AiJobView, AI_MAX_COLORS, AI_MAX_FONTS, AI_MAX_INPUT_CHARS, AI_TEMPLATE_TYPES,
};
use crate::core::cloud::api::AiQuota;
use crate::core::state::AppState;
use crate::error::AppResult;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiLimits {
    pub max_chars: usize,
    pub max_colors: usize,
    pub max_fonts: usize,
    pub types: &'static [&'static str],
}

#[tauri::command]
pub fn ai_limits() -> AiLimits {
    AiLimits {
        max_chars: AI_MAX_INPUT_CHARS,
        max_colors: AI_MAX_COLORS,
        max_fonts: AI_MAX_FONTS,
        types: AI_TEMPLATE_TYPES,
    }
}

#[tauri::command]
pub async fn ai_quota(state: State<'_, AppState>) -> AppResult<AiQuota> {
    state.inner().clone().ai_quota().await
}

#[tauri::command]
pub async fn ai_generate(
    state: State<'_, AppState>,
    input: AiGenerateInput,
) -> AppResult<AiJobView> {
    state.inner().clone().ai_generate(input).await
}

#[tauri::command]
pub async fn ai_tweak(
    state: State<'_, AppState>,
    template_id: String,
    instruction: String,
) -> AppResult<AiJobView> {
    state
        .inner()
        .clone()
        .ai_tweak(template_id, instruction)
        .await
}

#[tauri::command]
pub fn ai_cancel(state: State<'_, AppState>, job_id: String) {
    state.ai_cancel(&job_id);
}

#[tauri::command]
pub fn ai_jobs(state: State<'_, AppState>) -> Vec<AiJobView> {
    state.ai_jobs_list()
}
