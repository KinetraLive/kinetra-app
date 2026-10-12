//! Templates (project `templates/<id>/`), built-in presets, and the editor's preview screen.
//!
//! Disk work runs on the blocking pool (`super::project::blocking`); preview commands other than
//! `preview_template` are in-memory + broadcast and stay sync. Every command except
//! `list_presets` answers `noProject` when no project is open.
//!
//! Error codes: `invalid` (validation, bad id), `notFound` (unknown template, variant or preset),
//! `io`, `newerVersion` (template.json from a newer Kinetra), `noProject`.
//!
//! Every write that changes how a template looks re-applies the screen layers using it by the
//! live rules (`AppState::template_saved`).

use tauri::State;

use crate::core::live::{LiveLayerView, PREVIEW_LAYER_ID, PREVIEW_SCREEN_ID};
use crate::core::model::{
    PresetInfo, TemplateDoc, TemplateDraft, TemplateSource, TemplateSummary, TemplateUpdated,
};
use crate::core::protocol::Fields;
use crate::core::state::AppState;
use crate::core::templates::TemplateError;
use crate::core::{presets, preview};
use crate::error::AppResult;

use super::project::blocking;

/// Newest first by `updatedAt`. Reads `template.json` only (cached; invalidated on write).
#[tauri::command]
pub async fn list_templates(state: State<'_, AppState>) -> AppResult<Vec<TemplateSummary>> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        Ok(state.templates().list(&dir)?)
    })
    .await
}

#[tauri::command]
pub async fn get_template(state: State<'_, AppState>, id: String) -> AppResult<TemplateDoc> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        Ok(state.templates().get(&dir, &id)?)
    })
    .await
}

/// Creates `templates/<new id>/` from a preset or blank. `name` is trimmed, 1..=100 chars.
#[tauri::command]
pub async fn create_template(
    state: State<'_, AppState>,
    name: String,
    source: TemplateSource,
) -> AppResult<TemplateDoc> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        Ok(state.templates().create(&dir, &name, &source)?)
    })
    .await
}

/// Saves the shared parts (name, description, fields, variables, auto-out) and the looks and
/// variable defaults of the variants in `doc`. The variants list (which exist, names, default)
/// only changes through the variant commands: variants missing from `doc` are kept, unknown ids
/// ignored. Variant variable values the new definitions reject are dropped. Validates everything
/// before writing; writes only the files that changed, atomically; sets `updatedAt`. Returns the
/// saved document (every variant).
#[tauri::command]
pub async fn save_template(state: State<'_, AppState>, doc: TemplateDoc) -> AppResult<TemplateDoc> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        let saved = state.templates().save(&dir, doc)?;
        // Screen layers using it follow (by the live-edit rules).
        state.template_saved(&saved);
        Ok(saved)
    })
    .await
}

/// Updates a template made from a built-in preset to the preset's latest look (its summary's
/// `presetUpdate`): a `pre-update` backup first, then the preset's looks replace the variants it
/// also ships; added variants, field values and overrides on screens are kept
/// (`core::template_update`). Screens using it follow by the live rules. `invalid` when it has
/// no update.
#[tauri::command]
pub async fn update_template_from_preset(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<TemplateUpdated> {
    let state = state.inner().clone();
    blocking(move || state.update_template_from_preset(&id)).await
}

/// Undo of `update_template_from_preset`: template `id` as backup `backup` holds it. Screens
/// using it follow. `notFound` for an unknown backup, `invalid` when it has no copy of the
/// template.
#[tauri::command]
pub async fn undo_template_update(
    state: State<'_, AppState>,
    id: String,
    backup: String,
) -> AppResult<TemplateDoc> {
    let state = state.inner().clone();
    blocking(move || state.undo_template_update(&id, &backup)).await
}

/// Runs a variant operation and lets the screen layers using the template follow.
async fn variant_op(
    state: State<'_, AppState>,
    op: impl FnOnce(&AppState, &std::path::Path) -> AppResult<TemplateDoc> + Send + 'static,
) -> AppResult<TemplateDoc> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        let saved = op(&state, &dir)?;
        state.template_saved(&saved);
        Ok(saved)
    })
    .await
}

/// Saves one variant's look and variable defaults (strict: unknown variables or invalid values
/// are `invalid`). Writes only what changed. Returns the whole template.
#[tauri::command]
pub async fn save_template_variant(
    state: State<'_, AppState>,
    template_id: String,
    variant_id: String,
    html: String,
    css: String,
    vars: Fields,
) -> AppResult<TemplateDoc> {
    variant_op(state, move |st, dir| {
        Ok(st
            .templates()
            .save_variant(dir, &template_id, &variant_id, &html, &css, vars)?)
    })
    .await
}

/// Duplicates `fromVariant` (look and variable defaults) as a new variant named `name` (trimmed,
/// 1..=40 characters, unique ignoring case; at most `TEMPLATE_VARIANTS_MAX` variants). Its id is a slug of the name.
#[tauri::command]
pub async fn create_template_variant(
    state: State<'_, AppState>,
    template_id: String,
    from_variant: String,
    name: String,
) -> AppResult<TemplateDoc> {
    variant_op(state, move |st, dir| {
        Ok(st
            .templates()
            .create_variant(dir, &template_id, &from_variant, &name)?)
    })
    .await
}

#[tauri::command]
pub async fn rename_template_variant(
    state: State<'_, AppState>,
    template_id: String,
    variant_id: String,
    name: String,
) -> AppResult<TemplateDoc> {
    variant_op(state, move |st, dir| {
        Ok(st
            .templates()
            .rename_variant(dir, &template_id, &variant_id, &name)?)
    })
    .await
}

/// Not the last variant (`invalid`). If it was the default, the first remaining one becomes the
/// default; layers showing it fall back to the default (`variantMissing` warning).
#[tauri::command]
pub async fn delete_template_variant(
    state: State<'_, AppState>,
    template_id: String,
    variant_id: String,
) -> AppResult<TemplateDoc> {
    variant_op(state, move |st, dir| {
        Ok(st
            .templates()
            .delete_variant(dir, &template_id, &variant_id)?)
    })
    .await
}

#[tauri::command]
pub async fn set_default_template_variant(
    state: State<'_, AppState>,
    template_id: String,
    variant_id: String,
) -> AppResult<TemplateDoc> {
    variant_op(state, move |st, dir| {
        Ok(st
            .templates()
            .set_default_variant(dir, &template_id, &variant_id)?)
    })
    .await
}

#[tauri::command]
pub async fn rename_template(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<TemplateSummary> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        Ok(state.templates().rename(&dir, &id, &name)?)
    })
    .await
}

#[tauri::command]
pub async fn duplicate_template(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> AppResult<TemplateDoc> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        Ok(state.templates().duplicate(&dir, &id, &name)?)
    })
    .await
}

#[tauri::command]
pub async fn delete_template(state: State<'_, AppState>, id: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || {
        let dir = state.project_dir()?;
        state.templates().delete(&dir, &id)?;
        // Its screen layers become "template missing" (on-air ones animate out first).
        state.template_deleted(&id);
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn list_presets() -> AppResult<Vec<PresetInfo>> {
    Ok(presets::list())
}

/// Loads the saved template (its default variant) into the preview screen (`__preview`), off
/// air, field values = the field labels (preview text, never output content). The editor shows
/// another variant with `preview_draft` (`variant` + its html/css/variable values).
#[tauri::command]
pub async fn preview_template(state: State<'_, AppState>, id: String) -> AppResult<LiveLayerView> {
    let state = state.inner().clone();
    blocking(move || load_preview(&state, &id)).await
}

fn load_preview(state: &AppState, id: &str) -> AppResult<LiveLayerView> {
    let dir = state.project_dir()?;
    let doc = state.templates().get(&dir, id)?;
    let variant = doc
        .variants
        .iter()
        .find(|v| v.id == doc.meta.default_variant)
        .or_else(|| doc.variants.first())
        .ok_or(TemplateError::LastVariant)?;
    Ok(state
        .live()
        .update_layer(PREVIEW_SCREEN_ID, PREVIEW_LAYER_ID, |l| {
            preview::load_template(l, &doc.meta, variant)
        })?)
}

/// Applies unsaved editor changes to the preview layer. `invalid` if a variable value fails
/// validation (nothing is applied then).
#[tauri::command]
pub fn preview_draft(state: State<'_, AppState>, draft: TemplateDraft) -> AppResult<LiveLayerView> {
    apply_draft(&state, draft)
}

fn apply_draft(state: &AppState, draft: TemplateDraft) -> AppResult<LiveLayerView> {
    state.project_dir()?;
    Ok(state
        .live()
        .update_layer::<TemplateError>(PREVIEW_SCREEN_ID, PREVIEW_LAYER_ID, |l| {
            preview::apply_draft(l, draft)
        })?)
}

#[tauri::command]
pub fn preview_show(state: State<'_, AppState>) -> AppResult<LiveLayerView> {
    set_preview_on_air(&state, true)
}

#[tauri::command]
pub fn preview_hide(state: State<'_, AppState>) -> AppResult<LiveLayerView> {
    set_preview_on_air(&state, false)
}

fn set_preview_on_air(state: &AppState, on: bool) -> AppResult<LiveLayerView> {
    state.project_dir()?;
    let live = state.live();
    Ok(if on {
        live.show(PREVIEW_SCREEN_ID, PREVIEW_LAYER_ID)?
    } else {
        live.hide(PREVIEW_SCREEN_ID, PREVIEW_LAYER_ID)?
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::Value;

    use super::*;
    use crate::core::events::NullSink;
    use crate::core::protocol::Fields;
    use crate::error::AppError;

    fn json(m: &crate::core::live_hub::HubMsg) -> Value {
        serde_json::from_str(&m.json).unwrap()
    }

    #[test]
    fn preview_needs_a_project_and_follows_the_template() {
        let tmp = tempfile::tempdir().unwrap();
        let st = AppState::load(tmp.path().join("config"), Arc::new(NullSink)).unwrap();
        assert!(matches!(load_preview(&st, "x"), Err(AppError::NoProject)));
        assert!(matches!(
            apply_draft(&st, TemplateDraft::default()),
            Err(AppError::NoProject)
        ));
        assert!(matches!(
            set_preview_on_air(&st, true),
            Err(AppError::NoProject)
        ));

        st.create_project(tmp.path(), "Show").unwrap();
        let dir = st.project_dir().unwrap();
        let doc = st
            .templates()
            .create(&dir, "Lower", &TemplateSource::Blank)
            .unwrap();
        assert!(matches!(
            load_preview(&st, "nope"),
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            load_preview(&st, "../x"),
            Err(AppError::Invalid(_))
        ));

        let mut rx = st.live().subscribe();
        let view = load_preview(&st, &doc.id).unwrap();
        assert!(!view.on_air);
        assert_eq!(view.fields["name"], "Name", "preview text is the label");
        let m = json(&rx.try_recv().unwrap());
        assert_eq!(m["t"], "layer");
        assert_eq!(m["layer"]["templateRev"], 2);

        let bad = TemplateDraft {
            variable_values: Some(Fields::from([("x".to_string(), "1".to_string())])),
            ..Default::default()
        };
        assert!(matches!(apply_draft(&st, bad), Err(AppError::Invalid(_))));
        assert!(rx.try_recv().is_err(), "invalid draft sends nothing");

        let draft = TemplateDraft {
            field_values: Some(Fields::from([("name".to_string(), "Ada".to_string())])),
            ..Default::default()
        };
        assert_eq!(apply_draft(&st, draft).unwrap().fields["name"], "Ada");
        assert_eq!(json(&rx.try_recv().unwrap())["t"], "fields");

        assert!(set_preview_on_air(&st, true).unwrap().on_air);
        assert!(!set_preview_on_air(&st, false).unwrap().on_air);

        // Closing the project drops the preview screen; reopening seeds a fresh one.
        st.close_project();
        assert!(!st.live().has_screen(PREVIEW_SCREEN_ID));
        st.open_project(&dir).unwrap();
        let view = st
            .live()
            .layer_view(PREVIEW_SCREEN_ID, PREVIEW_LAYER_ID)
            .unwrap();
        assert!(view.fields.is_empty() && !view.on_air);
    }

    #[test]
    fn template_cache_is_dropped_on_project_switch() {
        let tmp = tempfile::tempdir().unwrap();
        let st = AppState::load(tmp.path().join("config"), Arc::new(NullSink)).unwrap();
        st.create_project(tmp.path(), "One").unwrap();
        let one = st.project_dir().unwrap();
        st.templates()
            .create(&one, "A", &TemplateSource::Blank)
            .unwrap();
        assert_eq!(st.templates().list(&one).unwrap().len(), 1);
        assert!(st.templates().is_cached(&one));
        st.create_project(tmp.path(), "Two").unwrap();
        assert!(!st.templates().is_cached(&one));
        let two = st.project_dir().unwrap();
        assert!(st.templates().list(&two).unwrap().is_empty());
        st.close_project();
        assert!(!st.templates().is_cached(&two));
    }
}
