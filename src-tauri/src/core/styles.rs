//! The style library on the open project: applying a style (project-wide or to one layer) with
//! one undo step, and the brand kit.
//!
//! # Applying a style (`apply_style`)
//! 1. **Variants.** Every project template in scope (project: all templates; layer: the layer's)
//!    whose `preset.id` is a library preset that ships the variant `style`, and that doesn't have
//!    that variant yet, gets it: the look and variable values are copied from the preset and
//!    missing field and variable definitions are merged by key (`templates::add_style_variant`;
//!    additive, existing definitions never change). A template that would exceed a limit stays
//!    as it is (its layers count as unchanged).
//! 2. **Layers.** Every layer in scope whose template has the variant switches to it, by the live
//!    rules (the path `set_layer_variant` takes: off air at once; on air under `onTake` staged
//!    for the next take; under `immediate` swapped now with `transition`). Field data is kept.
//!    One apply per screen; the screen is written by the content flusher (coalesced, one write).
//! 3. **Project scope** also sets `settings.style`.
//!
//! The result counts switched layers, unchanged layers (their template has no such variant and
//! no library preset to copy it from) and their distinct templates, and the variants copied.
//!
//! # Undo
//! The previous variant of every switched layer (and, for the project scope, the previous
//! `settings.style`) is kept in memory under a token. Only the latest token is valid (a newer
//! apply replaces it; a project switch forgets it); undoing restores by the same live rules and
//! consumes it. Copied variants are not removed.
//!
//! # Brand kit (`save_brand_kit`)
//! Validated like variable values (colours `#rgb`/`#rrggbb`/`#rrggbbaa`/`transparent`, fonts by curated name),
//! the logo an `/assets/<name>` URL of an existing image. Saved in `project.json`; every screen
//! re-applies by the live rules (one apply per screen). Precedence: see `screens::plan`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use crate::core::assets;
use crate::core::live::{is_reserved_screen, LiveError, VariantSwitch};
use crate::core::model::{
    ApplyStyleResult, ApplyStyleScope, BrandKit, ScreenLayerKind, StyleInfo, TemplateDoc,
    VariantTransition,
};
use crate::core::presets::{self, Preset};
use crate::core::screen_live::apply_doc_with;
use crate::core::screens::LoadedTemplate;
use crate::core::state::AppState;
use crate::core::templates::{self, StyleAdded};
use crate::core::vars;
use crate::error::{AppError, AppResult};

/// The latest `apply_style`, for one undo step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleUndo {
    pub token: String,
    /// `Some(previous settings.style)` for the project scope.
    pub style_before: Option<Option<String>>,
    pub transition: VariantTransition,
    /// Screen id → (layer id, the layer's `variant` before), in screen order.
    pub layers: BTreeMap<String, Vec<(String, Option<String>)>>,
}

/// Validates and normalizes a brand kit: values trimmed, empty ones dropped, colours lowercase.
/// Fonts must be curated font names; the logo an `/assets/<name>` URL (existence is checked by
/// `save_brand_kit`).
pub fn normalize_brand(brand: BrandKit) -> AppResult<BrandKit> {
    let clean = |v: Option<String>| v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let colour = |what: &str, v: Option<String>| -> AppResult<Option<String>> {
        match clean(v) {
            Some(c) if vars::is_color(&c) => Ok(Some(c.to_ascii_lowercase())),
            Some(c) => Err(AppError::Invalid(format!(
                "Brand {what}: \u{201c}{c}\u{201d} is not a color; use #rgb, #rrggbb, #rrggbbaa or transparent"
            ))),
            None => Ok(None),
        }
    };
    let font = |what: &str, v: Option<String>| -> AppResult<Option<String>> {
        match clean(v) {
            // A retired name (`vars::FONT_ALIASES`) is saved as its replacement.
            Some(f) if vars::canonical_font(&f).is_some() => {
                Ok(vars::canonical_font(&f).map(str::to_string))
            }
            Some(f) => Err(AppError::Invalid(format!(
                "Brand {what}: \u{201c}{f}\u{201d} is not one of the available fonts"
            ))),
            None => Ok(None),
        }
    };
    let logo = match clean(brand.logo) {
        Some(url) => match url.strip_prefix(assets::URL_PREFIX) {
            Some(name) if assets::is_image_name(name) => Some(url),
            _ => {
                return Err(AppError::Invalid(format!(
                    "Brand logo: \u{201c}{url}\u{201d} is not one of the project's images"
                )))
            }
        },
        None => None,
    };
    Ok(BrandKit {
        accent: colour("accent", brand.accent)?,
        secondary: colour("secondary color", brand.secondary)?,
        text: colour("text color", brand.text)?,
        plate: colour("plate color", brand.plate)?,
        font_heading: font("heading font", brand.font_heading)?,
        font_body: font("body font", brand.font_body)?,
        logo,
    })
}

/// Rewrites retired font names ([`vars::FONT_ALIASES`]) in a brand kit as loaded. True when
/// anything changed.
pub fn normalize_brand_fonts(brand: &mut BrandKit) -> bool {
    let mut changed = false;
    for f in [&mut brand.font_heading, &mut brand.font_body]
        .into_iter()
        .flatten()
    {
        changed |= vars::normalize_font(f);
    }
    changed
}

fn not_found_screen(id: &str) -> AppError {
    AppError::NotFound(format!("Screen not found: {id}"))
}

impl AppState {
    /// The design styles of the library, in order.
    pub fn list_styles(&self) -> Vec<StyleInfo> {
        presets::styles()
    }

    /// Applies `style` (see the module docs).
    pub fn apply_style(
        &self,
        style: &str,
        scope: &ApplyStyleScope,
        transition: VariantTransition,
    ) -> AppResult<ApplyStyleResult> {
        let dir = self.project_dir()?;
        if !templates::is_variant_id(style) || !presets::is_style(style) {
            return Err(AppError::Invalid(format!("Unknown style: {style}")));
        }
        // The layer scope's layer must exist before anything is written.
        let only = match scope {
            ApplyStyleScope::Project => None,
            ApplyStyleScope::Layer {
                screen_id,
                layer_id,
            } => {
                if is_reserved_screen(screen_id) {
                    return Err(not_found_screen(screen_id));
                }
                let doc = self
                    .screens()
                    .doc(screen_id)
                    .ok_or_else(|| not_found_screen(screen_id))?;
                let layer = doc
                    .meta
                    .layers
                    .iter()
                    .find(|l| &l.id == layer_id)
                    .ok_or_else(|| LiveError::UnknownLayer(layer_id.clone()))?;
                let ScreenLayerKind::Template { template_id, .. } = &layer.kind else {
                    return Err(AppError::Invalid("A media layer has no looks".to_string()));
                };
                Some((screen_id.clone(), layer_id.clone(), template_id.clone()))
            }
        };

        // 1. Variants, copied without the screen lock (template writes take their own lock).
        let candidates: Vec<(String, String)> = self
            .templates()
            .list(&dir)?
            .into_iter()
            .filter(|t| only.as_ref().is_none_or(|(_, _, tid)| *tid == t.id))
            .filter_map(|t| t.preset.map(|p| (t.id, p.id)))
            .collect();
        let mut by_preset: HashMap<String, Option<Preset>> = HashMap::new();
        let mut changed: Vec<TemplateDoc> = Vec::new();
        for (tid, preset_id) in candidates {
            let preset = by_preset
                .entry(preset_id.clone())
                .or_insert_with(|| presets::style_variant(&preset_id, style));
            let Some(p) = preset.as_ref() else {
                continue;
            };
            let Some(look) = p.variant(style) else {
                continue;
            };
            match self.templates().add_style_variant(
                &dir,
                &tid,
                look,
                &p.meta.fields,
                &p.meta.variables,
            ) {
                Ok(StyleAdded::Added(doc)) => changed.push(*doc),
                Ok(StyleAdded::Present) => {}
                Err(e) => tracing::warn!("style {style} not added to template {tid}: {e}"),
            }
        }
        let added = changed.len() as u32;

        // 2. Layers, one apply per screen.
        let mut result = ApplyStyleResult {
            added,
            ..Default::default()
        };
        let mut undo_layers: BTreeMap<String, Vec<(String, Option<String>)>> = BTreeMap::new();
        {
            let (dir, _w) = self.screen_guard()?;
            let mut touched: BTreeSet<String> = BTreeSet::new();
            for doc in &changed {
                let t = Arc::new(LoadedTemplate::from_doc(doc));
                for user in self.screens().set_template(&doc.id, Some(t)) {
                    touched.insert(user.id);
                }
            }
            // In scope, plus the other users of a template that changed (their live state
            // follows the merged definitions).
            let screen_ids: Vec<String> = match &only {
                Some((sid, _, _)) => {
                    let mut ids = vec![sid.clone()];
                    ids.extend(touched.iter().filter(|t| *t != sid).cloned());
                    ids
                }
                None => self
                    .screens()
                    .doc_ids()
                    .into_iter()
                    .filter(|id| !is_reserved_screen(id))
                    .collect(),
            };
            let mut unchanged_templates: BTreeSet<String> = BTreeSet::new();
            let mode = self.live_edit();
            let mut dirty = false;
            for sid in screen_ids {
                let Some(mut doc) = self.screens().doc(&sid) else {
                    continue;
                };
                let tmap = self.screens().templates_for(&dir, &doc);
                let mut switches = Vec::new();
                for layer in &mut doc.meta.layers {
                    if only
                        .as_ref()
                        .is_some_and(|(s, lid, _)| *s != sid || *lid != layer.id)
                    {
                        continue;
                    }
                    let ScreenLayerKind::Template {
                        template_id,
                        variant,
                        ..
                    } = &mut layer.kind
                    else {
                        continue;
                    };
                    let Some(t) = tmap
                        .get(template_id.as_str())
                        .filter(|t| t.has_variant(style))
                    else {
                        result.unchanged += 1;
                        unchanged_templates.insert(template_id.clone());
                        continue;
                    };
                    let shown = t.variant(variant.as_deref()).map(|(v, _)| v.id.as_str());
                    if shown == Some(style) {
                        continue;
                    }
                    let is_default = t.default_variant().is_some_and(|d| d.id == style);
                    undo_layers
                        .entry(sid.clone())
                        .or_default()
                        .push((layer.id.clone(), variant.clone()));
                    *variant = (!is_default).then(|| style.to_string());
                    switches.push(VariantSwitch {
                        layer_id: layer.id.clone(),
                        transition,
                        direct: false,
                    });
                }
                if switches.is_empty() && !touched.contains(&sid) {
                    continue;
                }
                result.switched += switches.len() as u32;
                self.apply_style_doc(&dir, &doc, mode, &switches);
                if !switches.is_empty() {
                    self.screens().mark_dirty(&sid);
                    dirty = true;
                }
            }
            result.unchanged_templates = unchanged_templates.len() as u32;
            if dirty {
                self.schedule_content_flush();
            }
        }

        // 3. Project style.
        let style_before = match scope {
            ApplyStyleScope::Project => {
                let before = self.current_project().and_then(|p| p.settings.style);
                if before.as_deref() != Some(style) {
                    self.update_project_settings(|s| s.style = Some(style.to_string()))?;
                }
                Some(before)
            }
            ApplyStyleScope::Layer { .. } => None,
        };
        let style_changed = style_before
            .as_ref()
            .is_some_and(|b| b.as_deref() != Some(style));
        let undo = (!undo_layers.is_empty() || style_changed).then(|| StyleUndo {
            token: templates::random_id(),
            style_before,
            transition,
            layers: undo_layers,
        });
        result.undo = undo.as_ref().map(|u| u.token.clone());
        self.screens().set_style_undo(undo);
        Ok(result)
    }

    /// Applies a screen with variant switches; a failure is logged (the screen keeps its last
    /// good live state). Caller holds the screen write lock.
    fn apply_style_doc(
        &self,
        dir: &Path,
        doc: &crate::core::model::ScreenDoc,
        mode: crate::core::model::LiveEdit,
        switches: &[VariantSwitch],
    ) {
        if let Err(e) = apply_doc_with(
            self.screens(),
            self.sources(),
            self.live(),
            dir,
            doc,
            mode,
            &[],
            switches,
        ) {
            tracing::warn!("applying screen {} for a style: {e}", doc.id);
        }
    }

    /// Restores what the `apply_style` with this token changed (see the module docs). An unknown
    /// or stale token is `invalid` ("Nothing to undo").
    pub fn undo_apply_style(&self, token: &str) -> AppResult<()> {
        let undo = {
            let (dir, _w) = self.screen_guard()?;
            let undo = self
                .screens()
                .take_style_undo(token)
                .ok_or_else(|| AppError::Invalid("Nothing to undo".into()))?;
            let mode = self.live_edit();
            let mut dirty = false;
            for (sid, layers) in &undo.layers {
                let Some(mut doc) = self.screens().doc(sid) else {
                    continue;
                };
                let mut switches = Vec::new();
                for (lid, before) in layers {
                    let Some(layer) = doc.meta.layers.iter_mut().find(|l| &l.id == lid) else {
                        continue;
                    };
                    let ScreenLayerKind::Template { variant, .. } = &mut layer.kind else {
                        continue;
                    };
                    if variant != before {
                        *variant = before.clone();
                        switches.push(VariantSwitch {
                            layer_id: lid.clone(),
                            transition: undo.transition,
                            direct: false,
                        });
                    }
                }
                if switches.is_empty() {
                    continue;
                }
                self.apply_style_doc(&dir, &doc, mode, &switches);
                self.screens().mark_dirty(sid);
                dirty = true;
            }
            if dirty {
                self.schedule_content_flush();
            }
            undo
        };
        if let Some(before) = undo.style_before {
            self.update_project_settings(|s| s.style = before)?;
        }
        Ok(())
    }

    /// Validates and saves the project's brand kit, then re-applies every screen. Returns the
    /// kit as stored (normalized).
    pub fn save_brand_kit(&self, brand: BrandKit) -> AppResult<BrandKit> {
        let dir = self.project_dir()?;
        let brand = normalize_brand(brand)?;
        if let Some(name) = brand
            .logo
            .as_deref()
            .and_then(|l| l.strip_prefix(assets::URL_PREFIX))
        {
            if !assets::assets_root(&dir).join(name).is_file() {
                return Err(AppError::Invalid(format!(
                    "Brand logo: the image {name} isn't in the project's assets"
                )));
            }
        }
        self.update_project_settings(|s| s.brand = brand.clone())?;
        self.apply_brand(&brand);
        Ok(brand)
    }

    /// Makes `brand` the kit plans resolve against and re-applies every screen by the live rules
    /// (one apply per screen); nothing when it is the kit already in use.
    pub(crate) fn apply_brand(&self, brand: &BrandKit) {
        let Ok((dir, _w)) = self.screen_guard() else {
            return;
        };
        if *self.screens().brand() == *brand {
            return;
        }
        self.screens().set_brand(brand.clone());
        for id in self.screens().doc_ids() {
            if is_reserved_screen(&id) {
                continue;
            }
            if let Some(doc) = self.screens().doc(&id) {
                if let Err(e) = self.apply_screen_doc(&dir, &doc) {
                    tracing::warn!("applying screen {id} for the brand kit: {e}");
                }
            }
        }
    }

    /// Layers new to `doc` (not in the remembered document) that show their template's default
    /// variant start on the project style when their template has it. Caller holds the screen
    /// write lock.
    pub(crate) fn start_new_layers_on_style(
        &self,
        dir: &Path,
        doc: &mut crate::core::model::ScreenDoc,
    ) {
        let Some(style) = self.current_project().and_then(|p| p.settings.style) else {
            return;
        };
        let before: BTreeSet<String> = self
            .screens()
            .doc(&doc.id)
            .map(|d| d.meta.layers.into_iter().map(|l| l.id).collect())
            .unwrap_or_default();
        let fresh = |id: &str| !before.contains(id);
        if !doc.meta.layers.iter().any(|l| {
            fresh(&l.id) && matches!(&l.kind, ScreenLayerKind::Template { variant: None, .. })
        }) {
            return;
        }
        let tmap = self.screens().templates_for(dir, doc);
        for layer in &mut doc.meta.layers {
            if !fresh(&layer.id) {
                continue;
            }
            let ScreenLayerKind::Template {
                template_id,
                variant,
                ..
            } = &mut layer.kind
            else {
                continue;
            };
            if variant.is_some() {
                continue;
            }
            if let Some(t) = tmap.get(template_id.as_str()) {
                let is_default = t.default_variant().is_some_and(|d| d.id == style);
                if t.has_variant(&style) && !is_default {
                    *variant = Some(style.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
