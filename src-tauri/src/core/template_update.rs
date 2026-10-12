//! Updating a project template made from a built-in preset to the preset's latest look.
//!
//! A template copied from a preset keeps `preset: {id, version}`. When the app ships a newer
//! version of that preset ([`presets::version`] > the template's), the template is outdated
//! ([`preset_update`], on every `TemplateSummary`). Presets bump `preset.version` whenever their
//! HTML, CSS, variables or design change.
//!
//! [`update`] (after the caller's `pre-update` backup):
//! - every variant the preset also ships (same id) gets the preset's HTML, CSS and variable
//!   defaults (`vars`); a `default` variant (a legacy one-look preset, or a copy made before the
//!   preset had variants) gets the preset's `default` look, else its default variant's look;
//! - variants the preset doesn't ship (the user's own) are kept as they are;
//! - fields and variables are merged by key: the preset's definitions in its order (a new key is
//!   added; an existing one takes the preset's label, type and options but keeps its default
//!   unless that is a default the preset used to ship (`history.json`), no longer fits, or the
//!   type changed), then the template's own keys the preset never had; keys an older version had
//!   and the current one dropped (`history.json` `removed`) go;
//! - `design` is copied, `preset.version` set; name, description, auto-out, variant names and the
//!   default variant are kept.
//!
//! Everything is validated before anything is written; looks first, `template.json` last. Screen
//! layers (field values, variable overrides) are never touched here; the caller re-applies the
//! screens that use the template.
//!
//! [`restore`] puts a template back as a backup snapshot holds it (Undo).

use std::fs;
use std::path::Path;

use crate::core::backup;
use crate::core::fieldvals;
use crate::core::fsx;
use crate::core::live::FieldDef;
use crate::core::model::{
    BackupReason, PresetRef, PresetUpdate, TemplateDoc, TemplateUpdated, TemplateVariant,
    VariableDef,
};
use crate::core::persist;
use crate::core::presets::{self, History, Preset};
use crate::core::state::AppState;
use crate::core::templates::{
    self, TemplateError, TemplateFile, TemplateResult, DEFAULT_VARIANT, TEMPLATES_DIR, VARIANTS_DIR,
};
use crate::core::vars;
use crate::error::AppResult;

/// The update a template made from preset `preset` can take, if the app ships a newer version.
pub fn preset_update(preset: Option<&PresetRef>) -> Option<PresetUpdate> {
    let p = preset?;
    let to = presets::version(&p.id)?;
    (to > p.version).then_some(PresetUpdate {
        from: p.version,
        to,
    })
}

/// The update template `id` can take: `NotFromPreset` / `UpToDate` when there is none.
pub fn check(project: &Path, id: &str) -> TemplateResult<PresetUpdate> {
    let (_, file) = templates::load_for_write(project, id)?;
    let Some(r) = file.meta.preset.as_ref() else {
        return Err(TemplateError::NotFromPreset);
    };
    if presets::version(&r.id).is_none() {
        return Err(TemplateError::NotFromPreset);
    }
    preset_update(Some(r)).ok_or_else(|| TemplateError::UpToDate(file.meta.name.clone()))
}

/// Whether a template's default `old` for a key is the user's own: not what the preset ships
/// now or shipped before.
fn own_default(history: &History, key: &str, old: &str, new: &str) -> bool {
    old != new
        && !history
            .previous_defaults
            .get(key)
            .is_some_and(|d| d.iter().any(|v| v == old))
}

/// Fields merged as the module docs say.
fn merge_fields(own: &[FieldDef], preset: &[FieldDef], history: &History) -> Vec<FieldDef> {
    let mut out: Vec<FieldDef> = preset
        .iter()
        .map(|p| {
            let mut def = p.clone();
            if let Some(o) = own.iter().find(|o| o.key == p.key) {
                if o.kind == p.kind && own_default(history, &p.key, &o.default, &p.default) {
                    let mut kept = def.clone();
                    kept.default = o.default.clone();
                    if fieldvals::validate_def(&kept).is_ok() {
                        def = kept;
                    }
                }
            }
            def
        })
        .collect();
    out.extend(
        own.iter()
            .filter(|o| !preset.iter().any(|p| p.key == o.key))
            .filter(|o| !history.removed.contains(&o.key))
            .cloned(),
    );
    out
}

/// Variables merged as the module docs say.
fn merge_variables(
    own: &[VariableDef],
    preset: &[VariableDef],
    history: &History,
) -> Vec<VariableDef> {
    let mut out: Vec<VariableDef> = preset
        .iter()
        .map(|p| {
            let mut def = p.clone();
            if let Some(o) = own.iter().find(|o| o.key == p.key) {
                let same_kind = std::mem::discriminant(&o.kind) == std::mem::discriminant(&p.kind);
                if same_kind
                    && own_default(history, &p.key, &o.default, &p.default)
                    && vars::css_value(p, &o.default).is_ok()
                {
                    def.default = o.default.clone();
                }
            }
            def
        })
        .collect();
    out.extend(
        own.iter()
            .filter(|o| !preset.iter().any(|p| p.key == o.key))
            .filter(|o| !history.removed.contains(&o.key))
            .cloned(),
    );
    out
}

/// The preset look a template variant takes: the same id, else (for `default`) the preset's
/// `default` or default look.
fn look_for<'a>(preset: &'a Preset, variant: &str) -> Option<&'a crate::core::model::VariantDoc> {
    preset.variant(variant).or_else(|| {
        (variant == DEFAULT_VARIANT && preset.variant(DEFAULT_VARIANT).is_none())
            .then(|| preset.default_look())
    })
}

/// Updates template `id` to its preset's latest look (see the module docs). Returns the template
/// as saved. `NotFromPreset` / `UpToDate` write nothing.
pub fn update(project: &Path, id: &str) -> TemplateResult<TemplateDoc> {
    check(project, id)?;
    let (dir, mut file) = templates::load_for_write(project, id)?;
    let Some(r) = file.meta.preset.clone() else {
        return Err(TemplateError::NotFromPreset);
    };
    let preset = presets::load(&r.id)?;
    let history = presets::history(&r.id);

    file.meta.fields = merge_fields(&file.meta.fields, &preset.meta.fields, &history);
    file.meta.variables = merge_variables(&file.meta.variables, &preset.meta.variables, &history);
    // The user's key may clash with a new preset key of the other kind: the preset wins.
    let field_keys: Vec<String> = file.meta.fields.iter().map(|f| f.key.clone()).collect();
    let preset_vars: Vec<&str> = preset
        .meta
        .variables
        .iter()
        .map(|v| v.key.as_str())
        .collect();
    file.meta.fields.retain(|f| {
        !preset_vars.contains(&f.key.as_str()) || preset.meta.fields.iter().any(|p| p.key == f.key)
    });
    file.meta.variables.retain(|v| {
        !field_keys.contains(&v.key) || preset.meta.variables.iter().any(|p| p.key == v.key)
    });
    file.meta.design = preset.meta.design;
    file.meta.preset = Some(PresetRef {
        id: r.id.clone(),
        version: preset.info.version,
    });
    templates::validate_shared(&file.meta)?;

    let mut looks = Vec::new();
    for v in &mut file.variants {
        if let Some(look) = look_for(&preset, &v.id) {
            v.vars = look.vars.clone();
            looks.push((v.id.clone(), look.html.clone(), look.css.clone()));
        }
        templates::prune_variant_vars(&file.meta.variables, &mut v.vars);
    }
    templates::validate_variants(&file.meta, &mut file.variants)?;
    for (_, html, css) in &looks {
        templates::validate_html(html)?;
        templates::validate_css(css)?;
    }

    for (variant, html, css) in &looks {
        templates::write_look(&dir, variant, html, css)?;
    }
    templates::save_file(&dir, &mut file)?;
    templates::doc_of(id, &dir, file)
}

/// Puts template `id` back as the snapshot folder `snapshot` (`.backups/<id>/`) holds it: its
/// `template.json` and every variant's look; variant folders the snapshot doesn't have are
/// removed. `NotInBackup` when the snapshot has no copy of it.
pub fn restore(project: &Path, id: &str, snapshot: &Path) -> TemplateResult<TemplateDoc> {
    let (dir, _) = templates::load_for_write(project, id)?;
    let src = snapshot.join(TEMPLATES_DIR).join(id);
    let mut file = match persist::load_strict::<TemplateFile>(&src) {
        Ok(f) => f,
        Err(persist::PersistError::Missing(_)) => return Err(TemplateError::NotInBackup),
        Err(e) => return Err(e.into()),
    };
    let old = templates::doc_of(id, &src, file.clone())?;
    templates::validate_doc(&old)?;
    let mut list: Vec<TemplateVariant> = file.variants.clone();
    templates::validate_variants(&file.meta, &mut list)?;
    for v in &old.variants {
        templates::write_look(&dir, &v.id, &v.html, &v.css)?;
    }
    let vdir = dir.join(VARIANTS_DIR);
    if let Ok(entries) = fs::read_dir(&vdir) {
        for e in entries.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            let gone =
                templates::is_variant_id(&name) && !old.variants.iter().any(|v| v.id == name);
            if gone && e.file_type().is_ok_and(|t| t.is_dir()) {
                fsx::remove_dir_all_own(&e.path()).map_err(|source| TemplateError::Io {
                    path: e.path(),
                    source,
                })?;
            }
        }
    }
    templates::save_file(&dir, &mut file)?;
    templates::doc_of(id, &dir, file)
}

// ---- AppState ------------------------------------------------------------------------------

impl AppState {
    /// Takes a `pre-update` backup, updates template `id` to its preset's latest look
    /// ([`update`]) and re-applies every screen using it by the live rules (on-air layers follow
    /// Live edit / Studio mode, like a template save). Nothing happens (no backup either) when
    /// the template has no update.
    pub fn update_template_from_preset(&self, id: &str) -> AppResult<TemplateUpdated> {
        let dir = self.project_dir()?;
        check(&dir, id)?;
        let backup = self.create_snapshot(BackupReason::PreUpdate)?;
        let doc = self.templates().update_from_preset(&dir, id)?;
        self.template_saved(&doc);
        Ok(TemplateUpdated { doc, backup })
    }

    /// Undo of [`AppState::update_template_from_preset`]: template `id` as backup `backup` holds
    /// it ([`restore`]); the screens using it follow.
    pub fn undo_template_update(&self, id: &str, backup: &str) -> AppResult<TemplateDoc> {
        let dir = self.project_dir()?;
        let snapshot = backup::backup_path(&dir, backup)?;
        let doc = self.templates().restore_from(&dir, id, &snapshot)?;
        self.template_saved(&doc);
        Ok(doc)
    }
}

#[cfg(test)]
mod tests;
