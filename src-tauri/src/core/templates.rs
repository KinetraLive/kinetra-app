//! Templates stored in the project (schema 2):
//!
//! ```text
//! templates/<id>/template.json                    shared meta: fields, variables, variants list
//! templates/<id>/variants/<variantId>/template.html
//! templates/<id>/variants/<variantId>/template.css
//! ```
//!
//! - A template has 1..=[`TEMPLATE_VARIANTS_MAX`] **variants**: looks that share one field schema
//!   and the variable definitions. `template.json` lists them (`variants: [{id, name, vars}]`,
//!   `vars` = that variant's variable defaults) with `defaultVariant`. Variant ids are slugs
//!   `[a-z][a-z0-9-]{0,31}` made from the name when a variant is created; names are trimmed,
//!   1..=40 characters, unique within the template (case-insensitive).
//! - Ids are 10-character nanoids over `[a-z0-9]` (safe folder names on every filesystem, no case
//!   collisions on Windows). Ids coming from the UI are validated (`[A-Za-z0-9_-]{1,64}`, so no
//!   separators, `..` or dot-files) and must name an existing folder.
//! - Everything is validated before anything is written (see [`validate_doc`]).
//! - Writes are atomic: files through `fsx::write_bytes_atomic`; a new template (or variant) is
//!   assembled in a hidden `.tmp-*` folder and renamed into place (`fsx::rename_own`).
//! - [`TemplateStore`] caches the summaries of the open project (read from `template.json` only)
//!   and serializes writes; every write invalidates the cache.
//!
//! # Migration (schema 1 → 2; [`migrate_v1`], on project open after a `pre-migrate` backup)
//! A schema-1 template's `template.html` / `template.css` move to `variants/default/` (variant
//! "Default", no `vars`), `template.json` is rewritten as schema 2 and the old files are removed.
//! Idempotent; every write is recorded as Kinetra's own, so the watcher ignores it. Until then a
//! schema-1 folder still reads (its root files are the `default` variant's look).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::fieldvals;
use crate::core::fsx;
use crate::core::live::{FieldDef, LiveError};
use crate::core::model::{
    PresetRef, TemplateDesign, TemplateDoc, TemplateMeta, TemplateSource, TemplateSummary,
    TemplateVariant, VariableDef, VariableKind, VariantDoc, DESIGN_SIZE_RANGE, TEMPLATE_CSS_MAX,
    TEMPLATE_FIELDS_MAX, TEMPLATE_HTML_MAX, TEMPLATE_VARIABLES_MAX, TEMPLATE_VARIANTS_MAX,
    VARIANT_NAME_MAX,
};
use crate::core::persist::{self, PersistError, Store};
use crate::core::presets;
use crate::core::protocol::{FieldType, Fields};
use crate::core::vars;
use crate::error::AppError;

pub const TEMPLATES_DIR: &str = "templates";
pub const HTML_FILE: &str = "template.html";
pub const CSS_FILE: &str = "template.css";
/// Folder of the variants' looks inside a template folder.
pub const VARIANTS_DIR: &str = "variants";
/// Id of the first variant of a new or migrated template.
pub const DEFAULT_VARIANT: &str = "default";
pub const DEFAULT_VARIANT_NAME: &str = "Default";

pub const NAME_MAX: usize = 100;
pub const DESCRIPTION_MAX: usize = 1000;
pub const LABEL_MAX: usize = 100;
pub const KEY_MAX: usize = 32;
pub const SELECT_OPTIONS_MAX: usize = 64;
pub const AUTO_OUT_RANGE: std::ops::RangeInclusive<u32> = 500..=600_000;
const ID_LEN: usize = 10;
const ID_ALPHABET: [char; 36] = [
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("Template name must be 1 to {NAME_MAX} characters")]
    Name,
    /// Markup or CSS a template may never contain (`core::template_safety`).
    #[error("Templates can't contain {0}.")]
    Unsafe(String),
    #[error("Description must be at most {DESCRIPTION_MAX} characters")]
    Description,
    #[error("Invalid template id: {0}")]
    BadId(String),
    #[error("Template not found: {0}")]
    NotFound(String),
    #[error("Unknown preset: {0}")]
    UnknownPreset(String),
    #[error("Preset {id} is not valid: {msg}")]
    BadPreset { id: String, msg: String },
    #[error(
        "Invalid key \"{0}\": start with a lowercase letter, then up to 31 lowercase letters, digits, - or _"
    )]
    Key(String),
    #[error("Duplicate key \"{0}\": field and variable keys must be unique")]
    DuplicateKey(String),
    #[error("{what} is too large ({} KB, at most {} KB)", size.div_ceil(1024), max / 1024)]
    TooLarge {
        what: &'static str,
        size: usize,
        max: usize,
    },
    #[error("Too many {what} ({count}, at most {max})")]
    TooMany {
        what: &'static str,
        count: usize,
        max: usize,
    },
    #[error("Field \"{key}\": {msg}")]
    Field { key: String, msg: String },
    #[error("Variable \"{key}\": {msg}")]
    Variable { key: String, msg: String },
    #[error("Unknown field: {0}")]
    UnknownField(String),
    #[error("Unknown variable: {0}")]
    UnknownVariable(String),
    #[error("Auto-out must be between 0.5 and 600 seconds")]
    AutoOut,
    #[error("Design size must be between 16 and 8192 pixels on each side")]
    DesignSize,
    #[error("Design position must be a number")]
    DesignPosition,
    #[error("Variant name must be 1 to {VARIANT_NAME_MAX} characters")]
    VariantName,
    #[error(
        "Invalid variant id \"{0}\": start with a lowercase letter, then up to 31 lowercase letters, digits or -"
    )]
    VariantId(String),
    #[error("There is already a variant named \u{201c}{0}\u{201d}")]
    DuplicateVariant(String),
    #[error("Duplicate variant id: {0}")]
    DuplicateVariantId(String),
    #[error("Variant not found: {0}")]
    VariantNotFound(String),
    #[error("This template wasn't made from a built-in look")]
    NotFromPreset,
    #[error("“{0}” already has the latest look")]
    UpToDate(String),
    #[error("The backup has no copy of this template")]
    NotInBackup,
    #[error("A template needs at least one variant")]
    LastVariant,
    #[error("The default variant \"{0}\" is not one of the template's variants")]
    DefaultVariant(String),
    #[error("Variant \u{201c}{variant}\u{201d}: {msg}")]
    VariantValue { variant: String, msg: String },
    #[error(transparent)]
    Persist(#[from] PersistError),
    #[error("{}: {source}", fsx::display_path(path))]
    Io { path: PathBuf, source: io::Error },
    #[error(transparent)]
    Live(#[from] LiveError),
}

impl From<TemplateError> for AppError {
    fn from(e: TemplateError) -> Self {
        match e {
            TemplateError::NotFound(_)
            | TemplateError::UnknownPreset(_)
            | TemplateError::VariantNotFound(_) => AppError::NotFound(e.to_string()),
            TemplateError::Persist(p) => p.into(),
            TemplateError::Io { path, source } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(&path)),
            )),
            TemplateError::Live(l) => l.into(),
            TemplateError::BadPreset { .. } => AppError::Internal(e.to_string()),
            _ => AppError::Invalid(e.to_string()),
        }
    }
}

pub type TemplateResult<T> = Result<T, TemplateError>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> TemplateError + '_ {
    move |source| TemplateError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// ---- template.json as a versioned store ----------------------------------------------------

// Here rather than a derive so `model.rs` (mirrored in TS) stays untouched.
#[allow(clippy::derivable_impls)]
impl Default for TemplateMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            preset: None,
            fields: Vec::new(),
            variables: Vec::new(),
            auto_out_ms: None,
            updated_at: String::new(),
            default_variant: String::new(),
            design: None,
        }
    }
}

/// `template.json` (minus `schemaVersion`): the shared meta plus the variants list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateFile {
    #[serde(flatten)]
    pub meta: TemplateMeta,
    #[serde(default)]
    pub variants: Vec<TemplateVariant>,
}

impl Store for TemplateFile {
    const FILE: &'static str = "template.json";
    const VERSION: u32 = 2;

    /// Schema 1 had one look: it becomes the `default` variant (the files move with
    /// [`migrate_v1`]; until then they are read from the template folder itself).
    fn migrate(mut v: Value, from: u32) -> Result<Value, PersistError> {
        if from < 2 {
            if let Value::Object(obj) = &mut v {
                obj.insert("defaultVariant".into(), Value::from(DEFAULT_VARIANT));
                obj.insert(
                    "variants".into(),
                    serde_json::json!([{ "id": DEFAULT_VARIANT, "name": DEFAULT_VARIANT_NAME, "vars": {} }]),
                );
            }
        }
        Ok(v)
    }
}

impl TemplateFile {
    fn variant(&self, id: &str) -> TemplateResult<&TemplateVariant> {
        self.variants
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| TemplateError::VariantNotFound(id.to_string()))
    }

    fn variant_mut(&mut self, id: &str) -> TemplateResult<&mut TemplateVariant> {
        self.variants
            .iter_mut()
            .find(|v| v.id == id)
            .ok_or_else(|| TemplateError::VariantNotFound(id.to_string()))
    }
}

// ---- Validation ----------------------------------------------------------------------------

/// `[a-z][a-z0-9_-]{0,31}`.
pub fn is_key(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= KEY_MAX
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}

/// A variant id: `[a-z][a-z0-9-]{0,31}` (also a safe folder name).
pub fn is_variant_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= KEY_MAX
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// A template id as it may appear in a path: `[A-Za-z0-9_-]{1,64}`. Never a separator, `.` or
/// `..`, so it always names a direct child of `templates/`.
pub fn is_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn validate_id(id: &str) -> TemplateResult<()> {
    if is_id(id) {
        Ok(())
    } else {
        Err(TemplateError::BadId(id.to_string()))
    }
}

/// Trimmed name, 1..=100 characters.
pub fn validate_name(name: &str) -> TemplateResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(TemplateError::Name);
    }
    Ok(name.to_string())
}

/// Trimmed variant name, 1..=40 characters.
pub fn validate_variant_name(name: &str) -> TemplateResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > VARIANT_NAME_MAX {
        return Err(TemplateError::VariantName);
    }
    Ok(name.to_string())
}

fn validate_label(label: &str) -> Result<(), String> {
    let n = label.trim().chars().count();
    if n == 0 || n > LABEL_MAX {
        Err(format!("label must be 1 to {LABEL_MAX} characters"))
    } else {
        Ok(())
    }
}

pub fn validate_html(html: &str) -> TemplateResult<()> {
    if html.len() > TEMPLATE_HTML_MAX {
        return Err(TemplateError::TooLarge {
            what: "HTML",
            size: html.len(),
            max: TEMPLATE_HTML_MAX,
        });
    }
    let problems = crate::core::template_safety::check_html(html);
    if !problems.is_empty() {
        return Err(TemplateError::Unsafe(problems.join(", ")));
    }
    Ok(())
}

pub fn validate_css(css: &str) -> TemplateResult<()> {
    if css.len() > TEMPLATE_CSS_MAX {
        return Err(TemplateError::TooLarge {
            what: "CSS",
            size: css.len(),
            max: TEMPLATE_CSS_MAX,
        });
    }
    let problems = crate::core::template_safety::check_css(css);
    if !problems.is_empty() {
        return Err(TemplateError::Unsafe(problems.join(", ")));
    }
    Ok(())
}

pub fn validate_fields(fields: &[FieldDef]) -> TemplateResult<()> {
    if fields.len() > TEMPLATE_FIELDS_MAX {
        return Err(TemplateError::TooMany {
            what: "fields",
            count: fields.len(),
            max: TEMPLATE_FIELDS_MAX,
        });
    }
    for f in fields {
        if !is_key(&f.key) {
            return Err(TemplateError::Key(f.key.clone()));
        }
        validate_label(&f.label).map_err(|msg| TemplateError::Field {
            key: f.key.clone(),
            msg,
        })?;
        fieldvals::validate_def(f).map_err(|msg| TemplateError::Field {
            key: f.key.clone(),
            msg,
        })?;
    }
    Ok(())
}

/// Checks one variable definition: key, label, kind parameters and that its default is a valid
/// value.
pub fn validate_variable(v: &VariableDef) -> TemplateResult<()> {
    if !is_key(&v.key) {
        return Err(TemplateError::Key(v.key.clone()));
    }
    let bad = |msg: String| TemplateError::Variable {
        key: v.key.clone(),
        msg,
    };
    validate_label(&v.label).map_err(bad)?;
    match &v.kind {
        VariableKind::Color | VariableKind::Font => {}
        VariableKind::Range {
            min,
            max,
            step,
            unit,
        } => {
            if !(min.is_finite() && max.is_finite() && step.is_finite()) {
                return Err(bad("min, max and step must be numbers".into()));
            }
            if min >= max {
                return Err(bad("min must be less than max".into()));
            }
            if *step <= 0.0 {
                return Err(bad("step must be greater than 0".into()));
            }
            if !vars::is_unit(unit) {
                return Err(bad(format!(
                    "unit must be up to {} lowercase letters or %",
                    vars::UNIT_MAX
                )));
            }
        }
        VariableKind::Select { options } => {
            if options.is_empty() {
                return Err(bad("needs at least one option".into()));
            }
            if options.len() > SELECT_OPTIONS_MAX {
                return Err(bad(format!("at most {SELECT_OPTIONS_MAX} options")));
            }
            let mut seen = HashSet::new();
            for o in options {
                if !vars::is_option_value(&o.value) {
                    return Err(bad(format!(
                        "option value \"{}\" must be 1 to {} letters, digits, - or _",
                        o.value,
                        vars::OPTION_VALUE_MAX
                    )));
                }
                if !seen.insert(o.value.as_str()) {
                    return Err(bad(format!("duplicate option value \"{}\"", o.value)));
                }
                validate_label(&o.label)
                    .map_err(|m| bad(format!("option \"{}\": {m}", o.value)))?;
            }
        }
    }
    vars::css_value(v, &v.default).map_err(|m| bad(format!("default {m}")))?;
    Ok(())
}

pub fn validate_variables(variables: &[VariableDef]) -> TemplateResult<()> {
    if variables.len() > TEMPLATE_VARIABLES_MAX {
        return Err(TemplateError::TooMany {
            what: "variables",
            count: variables.len(),
            max: TEMPLATE_VARIABLES_MAX,
        });
    }
    variables.iter().try_for_each(validate_variable)
}

/// Keys must be unique across fields AND variables.
pub fn validate_unique_keys(fields: &[FieldDef], variables: &[VariableDef]) -> TemplateResult<()> {
    let mut seen = HashSet::new();
    for key in fields
        .iter()
        .map(|f| &f.key)
        .chain(variables.iter().map(|v| &v.key))
    {
        if !seen.insert(key.as_str()) {
            return Err(TemplateError::DuplicateKey(key.clone()));
        }
    }
    Ok(())
}

pub fn validate_auto_out(ms: Option<u32>) -> TemplateResult<()> {
    match ms {
        Some(ms) if !AUTO_OUT_RANGE.contains(&ms) => Err(TemplateError::AutoOut),
        _ => Ok(()),
    }
}

/// A design box: sides in [`DESIGN_SIZE_RANGE`], a finite position (any value: it is clamped
/// when a layer is placed).
pub fn validate_design(d: Option<&TemplateDesign>) -> TemplateResult<()> {
    let Some(d) = d else {
        return Ok(());
    };
    if !DESIGN_SIZE_RANGE.contains(&d.w) || !DESIGN_SIZE_RANGE.contains(&d.h) {
        return Err(TemplateError::DesignSize);
    }
    if [d.x, d.y].iter().flatten().any(|v| !v.is_finite()) {
        return Err(TemplateError::DesignPosition);
    }
    Ok(())
}

/// The parts every variant shares: description, fields, variables (with their defaults),
/// auto-out and the design box.
pub fn validate_shared(meta: &TemplateMeta) -> TemplateResult<()> {
    if meta.description.chars().count() > DESCRIPTION_MAX {
        return Err(TemplateError::Description);
    }
    validate_fields(&meta.fields)?;
    validate_variables(&meta.variables)?;
    validate_unique_keys(&meta.fields, &meta.variables)?;
    validate_auto_out(meta.auto_out_ms)?;
    validate_design(meta.design.as_ref())
}

/// Everything about a single-look template that is not its name (presets, the preview).
pub fn validate_content(meta: &TemplateMeta, html: &str, css: &str) -> TemplateResult<()> {
    validate_html(html)?;
    validate_css(css)?;
    validate_shared(meta)
}

/// A variant's variable defaults: declared keys only, each a valid value for its variable.
pub fn validate_variant_vars(
    variables: &[VariableDef],
    variant: &str,
    values: &Fields,
) -> TemplateResult<()> {
    for (k, v) in values {
        let def =
            variables
                .iter()
                .find(|d| &d.key == k)
                .ok_or_else(|| TemplateError::VariantValue {
                    variant: variant.to_string(),
                    msg: format!("unknown variable \"{k}\""),
                })?;
        vars::css_value(def, v).map_err(|msg| TemplateError::VariantValue {
            variant: variant.to_string(),
            msg: format!("variable \"{k}\" {msg}"),
        })?;
    }
    Ok(())
}

/// Drops variant variable values that no longer fit the variable definitions (unknown keys,
/// values a changed definition rejects): the definition's default applies to them again.
pub(crate) fn prune_variant_vars(variables: &[VariableDef], values: &mut Fields) {
    values.retain(|k, v| {
        variables
            .iter()
            .find(|d| &d.key == k)
            .is_some_and(|d| vars::css_value(d, v).is_ok())
    });
}

/// Rewrites retired font names ([`vars::FONT_ALIASES`]) in the font variables' defaults and in
/// the given variant values of those variables. True when anything changed.
pub(crate) fn normalize_fonts<'a>(
    variables: &mut [VariableDef],
    values: impl IntoIterator<Item = &'a mut Fields>,
) -> bool {
    let mut changed = false;
    let mut keys = Vec::new();
    for d in variables.iter_mut() {
        if matches!(d.kind, VariableKind::Font) {
            changed |= vars::normalize_font(&mut d.default);
            keys.push(d.key.clone());
        }
    }
    if keys.is_empty() {
        return changed;
    }
    for vals in values {
        for k in &keys {
            if let Some(v) = vals.get_mut(k) {
                changed |= vars::normalize_font(v);
            }
        }
    }
    changed
}

/// [`normalize_fonts`] over a whole `template.json`.
fn normalize_file_fonts(file: &mut TemplateFile) -> bool {
    normalize_fonts(
        &mut file.meta.variables,
        file.variants.iter_mut().map(|v| &mut v.vars),
    )
}

/// The variants list: 1..=[`TEMPLATE_VARIANTS_MAX`], ids valid and unique, names trimmed (in place), 1..=40 characters
/// and unique ignoring case, the default one of them, every `vars` valid.
pub fn validate_variants(
    meta: &TemplateMeta,
    variants: &mut [TemplateVariant],
) -> TemplateResult<()> {
    if variants.is_empty() {
        return Err(TemplateError::LastVariant);
    }
    if variants.len() > TEMPLATE_VARIANTS_MAX {
        return Err(TemplateError::TooMany {
            what: "variants",
            count: variants.len(),
            max: TEMPLATE_VARIANTS_MAX,
        });
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for v in variants.iter_mut() {
        if !is_variant_id(&v.id) {
            return Err(TemplateError::VariantId(v.id.clone()));
        }
        if !ids.insert(v.id.clone()) {
            return Err(TemplateError::DuplicateVariantId(v.id.clone()));
        }
        v.name = validate_variant_name(&v.name)?;
        if !names.insert(v.name.to_lowercase()) {
            return Err(TemplateError::DuplicateVariant(v.name.clone()));
        }
        validate_variant_vars(&meta.variables, &v.name, &v.vars)?;
    }
    if !ids.contains(&meta.default_variant) {
        return Err(TemplateError::DefaultVariant(meta.default_variant.clone()));
    }
    Ok(())
}

/// Validates a whole document; returns its trimmed name.
pub fn validate_doc(doc: &TemplateDoc) -> TemplateResult<String> {
    let name = validate_name(&doc.meta.name)?;
    validate_shared(&doc.meta)?;
    let mut list: Vec<TemplateVariant> = doc.variants.iter().map(variant_entry).collect();
    validate_variants(&doc.meta, &mut list)?;
    for v in &doc.variants {
        validate_html(&v.html)?;
        validate_css(&v.css)?;
    }
    Ok(name)
}

fn variant_entry(v: &VariantDoc) -> TemplateVariant {
    TemplateVariant {
        id: v.id.clone(),
        name: v.name.clone(),
        vars: v.vars.clone(),
    }
}

/// CSS values for `values` (by key, as the user sees them) under `defs`. Unknown keys and
/// invalid values are errors; a missing value uses the default.
pub fn resolve_vars(defs: &[VariableDef], values: &Fields) -> TemplateResult<Fields> {
    if let Some(k) = values.keys().find(|k| !defs.iter().any(|d| &d.key == *k)) {
        return Err(TemplateError::UnknownVariable(k.clone()));
    }
    defs.iter()
        .map(|d| {
            let v = values.get(&d.key).unwrap_or(&d.default);
            vars::css_value(d, v)
                .map(|css| (d.key.clone(), css))
                .map_err(|msg| TemplateError::Variable {
                    key: d.key.clone(),
                    msg,
                })
        })
        .collect()
}

/// Field types by key (what the renderer compiles against).
pub fn field_types(fields: &[FieldDef]) -> std::collections::BTreeMap<&str, FieldType> {
    fields.iter().map(|f| (f.key.as_str(), f.kind)).collect()
}

// ---- Blank template ------------------------------------------------------------------------

const BLANK_HTML: &str = "<div class=\"root\">\n  <div class=\"name\">{{name}}</div>\n</div>\n";
const BLANK_CSS: &str = "/* The renderer sets data-state=\"in\" | \"out\" on the root element. */\n\
.root {\n  position: absolute;\n  left: 154px;\n  bottom: 96px;\n  padding: 20px 32px;\n  \
background: rgba(10, 12, 16, 0.82);\n  color: #fff;\n  font: 700 52px/1.15 system-ui, sans-serif;\n}\n\n\
.root[data-state=\"in\"] {\n  animation: in 300ms cubic-bezier(0.2, 0, 0, 1) both;\n}\n\n\
.root[data-state=\"out\"] {\n  animation: out 240ms cubic-bezier(0.4, 0, 1, 1) forwards;\n}\n\n\
@keyframes in {\n  from {\n    opacity: 0;\n    transform: translateY(16px);\n  }\n}\n\n\
@keyframes out {\n  to {\n    opacity: 0;\n    transform: translateY(16px);\n  }\n}\n";

fn blank(name: String) -> (TemplateMeta, String, String) {
    let meta = TemplateMeta {
        name,
        fields: vec![FieldDef {
            key: "name".into(),
            label: "Name".into(),
            kind: FieldType::Text,
            default: String::new(),
            ..Default::default()
        }],
        updated_at: fsx::now_rfc3339(),
        ..Default::default()
    };
    (meta, BLANK_HTML.to_string(), BLANK_CSS.to_string())
}

/// One "Default" variant with `html` / `css` (a new template, a preset copy).
fn single_variant(meta: &mut TemplateMeta, html: String, css: String) -> Vec<VariantDoc> {
    meta.default_variant = DEFAULT_VARIANT.to_string();
    vec![VariantDoc {
        id: DEFAULT_VARIANT.into(),
        name: DEFAULT_VARIANT_NAME.into(),
        vars: Fields::new(),
        html,
        css,
    }]
}

// ---- Disk operations (uncached; `TemplateStore` wraps them) ---------------------------------

pub fn templates_root(project: &Path) -> PathBuf {
    project.join(TEMPLATES_DIR)
}

/// The folder of an existing template. `BadId` for an unsafe id, `NotFound` if it isn't there.
pub fn template_dir(project: &Path, id: &str) -> TemplateResult<PathBuf> {
    validate_id(id)?;
    let dir = templates_root(project).join(id);
    if dir.join(TemplateFile::FILE).is_file() {
        Ok(dir)
    } else {
        Err(TemplateError::NotFound(id.to_string()))
    }
}

/// The folder of a variant's look (`variants/<variantId>`).
pub fn variant_dir(template_dir: &Path, variant: &str) -> PathBuf {
    template_dir.join(VARIANTS_DIR).join(variant)
}

fn summary(id: String, file: &TemplateFile) -> TemplateSummary {
    let meta = &file.meta;
    TemplateSummary {
        id,
        name: meta.name.clone(),
        description: meta.description.clone(),
        preset: meta.preset.clone(),
        preset_update: crate::core::template_update::preset_update(meta.preset.as_ref()),
        field_count: meta.fields.len() as u32,
        variable_count: meta.variables.len() as u32,
        variant_count: file.variants.len() as u32,
        updated_at: meta.updated_at.clone(),
    }
}

/// Every template with a readable `template.json`, newest first (then by name). Reads only
/// `template.json`. Unreadable ones are logged and skipped.
pub fn list_uncached(project: &Path) -> TemplateResult<Vec<TemplateSummary>> {
    let root = templates_root(project);
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(&root)(e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_err(&root))?;
        let Ok(id) = entry.file_name().into_string() else {
            continue;
        };
        if !is_id(&id) || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        match persist::load_strict::<TemplateFile>(&entry.path()) {
            Ok(file) => out.push(summary(id, &file)),
            Err(PersistError::Missing(_)) => {}
            Err(e) => tracing::warn!("skipping template {id}: {e}"),
        }
    }
    out.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

fn read_text(path: &Path) -> TemplateResult<String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_err(path)(e)),
    }
}

/// Where a variant's look is read from: its `variants/<id>` folder; for the `default` variant of
/// a not yet migrated schema-1 folder, the template folder itself.
fn look_dir(dir: &Path, variant: &str) -> PathBuf {
    let vdir = variant_dir(dir, variant);
    if variant == DEFAULT_VARIANT && !vdir.is_dir() && dir.join(HTML_FILE).is_file() {
        dir.to_path_buf()
    } else {
        vdir
    }
}

/// A variant's HTML and CSS (missing files read as empty).
fn read_look(dir: &Path, variant: &str) -> TemplateResult<(String, String)> {
    let from = look_dir(dir, variant);
    Ok((
        read_text(&from.join(HTML_FILE))?,
        read_text(&from.join(CSS_FILE))?,
    ))
}

pub(crate) fn doc_of(id: &str, dir: &Path, mut file: TemplateFile) -> TemplateResult<TemplateDoc> {
    normalize_file_fonts(&mut file);
    let mut variants = Vec::with_capacity(file.variants.len());
    for v in file.variants {
        let (html, css) = read_look(dir, &v.id)?;
        variants.push(VariantDoc {
            id: v.id,
            name: v.name,
            vars: v.vars,
            html,
            css,
        });
    }
    Ok(TemplateDoc {
        id: id.to_string(),
        meta: file.meta,
        variants,
    })
}

pub fn get(project: &Path, id: &str) -> TemplateResult<TemplateDoc> {
    let dir = template_dir(project, id)?;
    let file = persist::load_strict::<TemplateFile>(&dir)?;
    doc_of(id, &dir, file)
}

/// A random 10-character id over `[a-z0-9]` (also used for screens and their layers).
pub(crate) fn random_id() -> String {
    nanoid::nanoid!(ID_LEN, &ID_ALPHABET)
}

/// A fresh id that is not taken under `root`.
fn new_id(root: &Path) -> String {
    loop {
        let id = random_id();
        if !root.join(&id).exists() {
            return id;
        }
    }
}

/// Writes one variant's files into `vdir` (created if needed).
fn write_look_files(vdir: &Path, html: &str, css: &str) -> TemplateResult<()> {
    fs::create_dir_all(vdir).map_err(io_err(vdir))?;
    let h = vdir.join(HTML_FILE);
    fsx::write_bytes_atomic(&h, html.as_bytes()).map_err(io_err(&h))?;
    let c = vdir.join(CSS_FILE);
    fsx::write_bytes_atomic(&c, css.as_bytes()).map_err(io_err(&c))
}

/// Writes a variant's look. A folder that doesn't exist yet is assembled in a hidden
/// `variants/.tmp-*` folder and renamed into place (recorded as Kinetra's own change, so the
/// watcher never sees a half-written variant). Only changed files are written otherwise.
pub(crate) fn write_look(dir: &Path, variant: &str, html: &str, css: &str) -> TemplateResult<()> {
    let vdir = variant_dir(dir, variant);
    if vdir.is_dir() {
        for (file, text) in [(HTML_FILE, html), (CSS_FILE, css)] {
            let path = vdir.join(file);
            if fs::read_to_string(&path).ok().as_deref() != Some(text) {
                fsx::write_bytes_atomic(&path, text.as_bytes()).map_err(io_err(&path))?;
            }
        }
        return Ok(());
    }
    let parent = dir.join(VARIANTS_DIR);
    fs::create_dir_all(&parent).map_err(io_err(&parent))?;
    let tmp = parent.join(format!(".tmp-{}", random_id()));
    let result = write_look_files(&tmp, html, css)
        .and_then(|()| fsx::rename_own(&tmp, &vdir).map_err(io_err(&vdir)));
    if result.is_err() {
        let _ = fs::remove_dir_all(&tmp);
    }
    result
}

/// Writes a new template folder atomically (assembled in `.tmp-*`, then renamed).
fn write_new(
    project: &Path,
    mut meta: TemplateMeta,
    variants: Vec<VariantDoc>,
) -> TemplateResult<TemplateDoc> {
    let mut list: Vec<TemplateVariant> = variants.iter().map(variant_entry).collect();
    validate_variants(&meta, &mut list)?;
    meta.updated_at = fsx::now_rfc3339();
    let root = templates_root(project);
    fs::create_dir_all(&root).map_err(io_err(&root))?;
    let id = new_id(&root);
    let tmp = root.join(format!(".tmp-{id}"));
    let file = TemplateFile {
        meta,
        variants: list,
    };
    let result = (|| {
        fs::create_dir_all(&tmp).map_err(io_err(&tmp))?;
        for v in &variants {
            write_look_files(&variant_dir(&tmp, &v.id), &v.html, &v.css)?;
        }
        persist::save(&tmp, &file)?;
        let dest = root.join(&id);
        fsx::rename_own(&tmp, &dest).map_err(io_err(&dest))?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let names: Vec<(String, String)> = file
        .variants
        .iter()
        .map(|v| (v.id.clone(), v.name.clone()))
        .collect();
    let mut variants = variants;
    for v in &mut variants {
        if let Some((_, n)) = names.iter().find(|(i, _)| *i == v.id) {
            v.name = n.clone();
        }
    }
    Ok(TemplateDoc {
        id,
        meta: file.meta,
        variants,
    })
}

/// Creates a template blank (one "Default" variant) or from a preset (every variant the preset
/// ships, its default variant as `defaultVariant`). `name` is trimmed, 1..=100 characters.
pub fn create(project: &Path, name: &str, source: &TemplateSource) -> TemplateResult<TemplateDoc> {
    let name = validate_name(name)?;
    match source {
        TemplateSource::Blank => {
            let (mut meta, html, css) = blank(name);
            validate_content(&meta, &html, &css)?;
            let variants = single_variant(&mut meta, html, css);
            write_new(project, meta, variants)
        }
        TemplateSource::Preset { preset_id } => {
            let p = presets::load(preset_id)?;
            let meta = TemplateMeta {
                name,
                preset: Some(PresetRef {
                    id: p.info.id.clone(),
                    version: p.info.version,
                }),
                updated_at: fsx::now_rfc3339(),
                ..p.meta
            };
            validate_shared(&meta)?;
            for v in &p.variants {
                validate_html(&v.html)?;
                validate_css(&v.css)?;
            }
            write_new(project, meta, p.variants)
        }
    }
}

// ---- Packages: a template as one JSON value -----------------------------------------------

/// Template `id` as a package: `{"manifest": <template.json incl. schemaVersion>,
/// "files": {"<variantId>": {"html", "css"}}}` (what `/v1/ai/templates/tweak` takes).
pub fn to_package(project: &Path, id: &str) -> TemplateResult<serde_json::Value> {
    let doc = get(project, id)?;
    let file = TemplateFile {
        meta: doc.meta.clone(),
        variants: doc.variants.iter().map(variant_entry).collect(),
    };
    let mut manifest = serde_json::to_value(&file)
        .map_err(|e| TemplateError::Unsafe(format!("an unreadable manifest ({e})")))?;
    if let serde_json::Value::Object(m) = &mut manifest {
        m.insert("schemaVersion".into(), serde_json::Value::from(2));
        m.remove("updatedAt");
    }
    let files: serde_json::Map<String, serde_json::Value> = doc
        .variants
        .iter()
        .map(|v| {
            (
                v.id.clone(),
                serde_json::json!({ "html": v.html, "css": v.css }),
            )
        })
        .collect();
    Ok(serde_json::json!({ "manifest": manifest, "files": files }))
}

/// Imports a package as a new template folder: the app sets the folder id and `updatedAt`.
/// Everything is validated like a save (limits, keys, variants, the hardening of
/// `core::template_safety`) before anything is written. `name`: the template's name
/// (default: the manifest's). `keep_preset`: a tweak keeps the input's `preset`; a generated
/// template has none.
pub fn import_package(
    project: &Path,
    package: &serde_json::Value,
    name: Option<&str>,
    keep_preset: bool,
) -> TemplateResult<TemplateDoc> {
    let bad = |what: &str| TemplateError::Unsafe(format!("a package with {what}"));
    let manifest = package.get("manifest").ok_or_else(|| bad("no manifest"))?;
    if manifest
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(2)
    {
        return Err(bad("an unsupported schemaVersion"));
    }
    let file: TemplateFile =
        serde_json::from_value(manifest.clone()).map_err(|_| bad("an unreadable manifest"))?;
    let files = package
        .get("files")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| bad("no files"))?;
    let mut meta = file.meta;
    meta.name = validate_name(name.unwrap_or(&meta.name))?;
    if !keep_preset {
        meta.preset = None;
    }
    let mut variants = Vec::new();
    for v in &file.variants {
        let f = files
            .get(&v.id)
            .ok_or_else(|| bad("a variant without files"))?;
        let html = f
            .get("html")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let css = f
            .get("css")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        validate_content(&meta, html, css)?;
        variants.push(VariantDoc {
            id: v.id.clone(),
            name: v.name.clone(),
            vars: v.vars.clone(),
            html: html.to_string(),
            css: css.to_string(),
        });
    }
    if variants.is_empty() {
        return Err(bad("no variants"));
    }
    validate_shared(&meta)?;
    write_new(project, meta, variants)
}

/// What [`add_style_variant`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum StyleAdded {
    /// The template already has the variant: nothing written.
    Present,
    /// The variant was copied in (and missing definitions merged); the template as saved.
    Added(Box<TemplateDoc>),
}

/// Copies the look `look` (a preset's variant: id, name, html, css, vars) into template `id`
/// unless it already has a variant with that id. Field and variable definitions of the preset
/// (`fields`, `variables`) whose keys the template lacks are appended; existing definitions are
/// never changed. The variant's values for variables the template defines differently are
/// dropped (the template's default applies). A name taken by another variant gets " 2", " 3", ...
/// Everything is validated before anything is written; a template that would exceed a limit
/// (variants, fields, variables) is an error and stays untouched.
pub fn add_style_variant(
    project: &Path,
    id: &str,
    look: &VariantDoc,
    fields: &[FieldDef],
    variables: &[VariableDef],
) -> TemplateResult<StyleAdded> {
    let (dir, mut file) = load_for_write(project, id)?;
    if file.variants.iter().any(|v| v.id == look.id) {
        return Ok(StyleAdded::Present);
    }
    if file.variants.len() >= TEMPLATE_VARIANTS_MAX {
        return Err(TemplateError::TooMany {
            what: "variants",
            count: file.variants.len() + 1,
            max: TEMPLATE_VARIANTS_MAX,
        });
    }
    validate_html(&look.html)?;
    validate_css(&look.css)?;
    let taken = |k: &str, m: &TemplateMeta| {
        m.fields.iter().any(|f| f.key == k) || m.variables.iter().any(|v| v.key == k)
    };
    for f in fields {
        if !taken(&f.key, &file.meta) {
            file.meta.fields.push(f.clone());
        }
    }
    for v in variables {
        if !taken(&v.key, &file.meta) {
            file.meta.variables.push(v.clone());
        }
    }
    validate_shared(&file.meta)?;
    let mut vars = look.vars.clone();
    prune_variant_vars(&file.meta.variables, &mut vars);
    let base = validate_variant_name(&look.name)?;
    let name_taken = |n: &str| {
        file.variants
            .iter()
            .any(|v| v.name.to_lowercase() == n.to_lowercase())
    };
    let mut name = base.clone();
    let mut n = 2;
    while name_taken(&name) {
        name = format!("{base} {n}");
        n += 1;
    }
    let name = validate_variant_name(&name)?;
    file.variants.push(TemplateVariant {
        id: look.id.clone(),
        name,
        vars,
    });
    validate_variants(&file.meta, &mut file.variants)?;
    write_look(&dir, &look.id, &look.html, &look.css)?;
    save_file(&dir, &mut file)?;
    Ok(StyleAdded::Added(Box::new(doc_of(id, &dir, file)?)))
}

/// Loads `template.json` for a write: `NotFound`, `NewerVersion` (never overwritten) and corrupt
/// files are errors.
pub(crate) fn load_for_write(project: &Path, id: &str) -> TemplateResult<(PathBuf, TemplateFile)> {
    let dir = template_dir(project, id)?;
    let mut file = persist::load_strict::<TemplateFile>(&dir)?;
    normalize_file_fonts(&mut file);
    Ok((dir, file))
}

/// Writes `template.json` with a fresh `updatedAt`.
pub(crate) fn save_file(dir: &Path, file: &mut TemplateFile) -> TemplateResult<()> {
    normalize_file_fonts(file);
    file.meta.updated_at = fsx::now_rfc3339();
    persist::save(dir, file)?;
    Ok(())
}

/// Saves the shared parts (name, description, fields, variables, auto-out) and the looks and
/// variable defaults of the variants in `doc` that exist on disk. The variants list itself
/// (which variants exist, their names, the default) only changes through the variant
/// operations: variants missing from `doc` are kept, unknown ones ignored, names and
/// `defaultVariant` taken from disk. Variant variable values that the (new) definitions reject
/// are dropped (the definition's default applies again). Validates everything, then writes only
/// the files that changed (`template.json` last) and sets `updatedAt`; an unchanged document
/// writes nothing. The preset origin is kept from disk. Returns the template as saved.
pub fn save(project: &Path, doc: TemplateDoc) -> TemplateResult<TemplateDoc> {
    let name = validate_name(&doc.meta.name)?;
    validate_shared(&doc.meta)?;
    for v in &doc.variants {
        validate_html(&v.html)?;
        validate_css(&v.css)?;
    }
    let (dir, old) = load_for_write(project, &doc.id)?;
    let TemplateDoc {
        id,
        mut meta,
        variants: incoming,
    } = doc;
    meta.name = name;
    meta.preset = old.meta.preset.clone();
    meta.default_variant = old.meta.default_variant.clone();
    meta.updated_at = old.meta.updated_at.clone();
    let mut file = TemplateFile {
        meta,
        variants: old.variants.clone(),
    };
    let mut looks = Vec::new();
    for v in &mut file.variants {
        if let Some(new) = incoming.iter().find(|n| n.id == v.id) {
            v.vars = new.vars.clone();
            let (html, css) = read_look(&dir, &v.id)?;
            if html != new.html || css != new.css || look_dir(&dir, &v.id) == dir {
                looks.push((v.id.clone(), new.html.clone(), new.css.clone()));
            }
        }
        prune_variant_vars(&file.meta.variables, &mut v.vars);
    }
    normalize_file_fonts(&mut file);
    validate_variants(&file.meta, &mut file.variants)?;
    if looks.is_empty() && file == old {
        return doc_of(&id, &dir, file);
    }
    for (variant, html, css) in &looks {
        write_look(&dir, variant, html, css)?;
    }
    save_file(&dir, &mut file)?;
    doc_of(&id, &dir, file)
}

/// Saves one variant's look and variable defaults (validated strictly: unknown variables or
/// invalid values are `invalid`). Writes only what changed; nothing when nothing did.
pub fn save_variant(
    project: &Path,
    id: &str,
    variant: &str,
    html: &str,
    css: &str,
    values: Fields,
) -> TemplateResult<TemplateDoc> {
    validate_html(html)?;
    validate_css(css)?;
    let (dir, mut file) = load_for_write(project, id)?;
    let name = file.variant(variant)?.name.clone();
    validate_variant_vars(&file.meta.variables, &name, &values)?;
    let (old_html, old_css) = read_look(&dir, variant)?;
    let look_changed = old_html != html || old_css != css || look_dir(&dir, variant) == dir;
    let entry = file.variant_mut(variant)?;
    let vars_changed = entry.vars != values;
    entry.vars = values;
    if !look_changed && !vars_changed {
        return doc_of(id, &dir, file);
    }
    if look_changed {
        write_look(&dir, variant, html, css)?;
    }
    save_file(&dir, &mut file)?;
    doc_of(id, &dir, file)
}

/// A variant id made from `name`: lowercase ASCII letters and digits, other runs become `-`,
/// starting with a letter, at most 32 characters, unique among `taken` (a `-2`, `-3`, ...
/// suffix when needed).
pub fn variant_slug(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let mut base = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
        } else if !base.ends_with('-') && !base.is_empty() {
            base.push('-');
        }
    }
    let mut base = base.trim_matches('-').to_string();
    if !base.starts_with(|c: char| c.is_ascii_lowercase()) {
        base = format!("v-{base}").trim_end_matches('-').to_string();
    }
    if base == "v" {
        base = "variant".into();
    }
    base.truncate(KEY_MAX);
    let base = base.trim_end_matches('-').to_string();
    if !taken(&base) {
        return base;
    }
    (2..1000)
        .map(|n| {
            let suffix = format!("-{n}");
            let mut b = base.clone();
            b.truncate(KEY_MAX - suffix.len());
            format!("{}{suffix}", b.trim_end_matches('-'))
        })
        .find(|s| !taken(s))
        .unwrap_or_else(|| "variant".into())
}

/// Duplicates variant `from` (look and variable defaults) as a new variant named `name`, added
/// after the others. Its id is a slug of the name.
pub fn create_variant(
    project: &Path,
    id: &str,
    from: &str,
    name: &str,
) -> TemplateResult<TemplateDoc> {
    let name = validate_variant_name(name)?;
    let (dir, mut file) = load_for_write(project, id)?;
    let src = file.variant(from)?.clone();
    if file.variants.len() >= TEMPLATE_VARIANTS_MAX {
        return Err(TemplateError::TooMany {
            what: "variants",
            count: file.variants.len() + 1,
            max: TEMPLATE_VARIANTS_MAX,
        });
    }
    if file
        .variants
        .iter()
        .any(|v| v.name.to_lowercase() == name.to_lowercase())
    {
        return Err(TemplateError::DuplicateVariant(name));
    }
    let taken = |s: &str| file.variants.iter().any(|v| v.id == s) || variant_dir(&dir, s).exists();
    let new_id = variant_slug(&name, &taken);
    let (html, css) = read_look(&dir, &src.id)?;
    write_look(&dir, &new_id, &html, &css)?;
    file.variants.push(TemplateVariant {
        id: new_id,
        name,
        vars: src.vars,
    });
    validate_variants(&file.meta, &mut file.variants)?;
    save_file(&dir, &mut file)?;
    doc_of(id, &dir, file)
}

pub fn rename_variant(
    project: &Path,
    id: &str,
    variant: &str,
    name: &str,
) -> TemplateResult<TemplateDoc> {
    let name = validate_variant_name(name)?;
    let (dir, mut file) = load_for_write(project, id)?;
    file.variant(variant)?;
    if file
        .variants
        .iter()
        .any(|v| v.id != variant && v.name.to_lowercase() == name.to_lowercase())
    {
        return Err(TemplateError::DuplicateVariant(name));
    }
    let entry = file.variant_mut(variant)?;
    if entry.name != name {
        entry.name = name;
        save_file(&dir, &mut file)?;
    }
    doc_of(id, &dir, file)
}

/// Removes a variant (not the last one). If it was the default, the first remaining variant
/// becomes the default. Layers showing it fall back to the default (`variantMissing`).
pub fn delete_variant(project: &Path, id: &str, variant: &str) -> TemplateResult<TemplateDoc> {
    let (dir, mut file) = load_for_write(project, id)?;
    file.variant(variant)?;
    if file.variants.len() == 1 {
        return Err(TemplateError::LastVariant);
    }
    file.variants.retain(|v| v.id != variant);
    if file.meta.default_variant == variant {
        file.meta.default_variant = file.variants[0].id.clone();
    }
    save_file(&dir, &mut file)?;
    let vdir = variant_dir(&dir, variant);
    if vdir.exists() {
        fsx::remove_dir_all_own(&vdir).map_err(io_err(&vdir))?;
    }
    doc_of(id, &dir, file)
}

pub fn set_default_variant(project: &Path, id: &str, variant: &str) -> TemplateResult<TemplateDoc> {
    let (dir, mut file) = load_for_write(project, id)?;
    file.variant(variant)?;
    if file.meta.default_variant != variant {
        file.meta.default_variant = variant.to_string();
        save_file(&dir, &mut file)?;
    }
    doc_of(id, &dir, file)
}

pub fn rename(project: &Path, id: &str, name: &str) -> TemplateResult<TemplateSummary> {
    let name = validate_name(name)?;
    let (dir, mut file) = load_for_write(project, id)?;
    if file.meta.name != name {
        file.meta.name = name;
        save_file(&dir, &mut file)?;
    }
    Ok(summary(id.to_string(), &file))
}

/// Copies a template with every variant.
pub fn duplicate(project: &Path, id: &str, name: &str) -> TemplateResult<TemplateDoc> {
    let name = validate_name(name)?;
    let src = get(project, id)?;
    let meta = TemplateMeta { name, ..src.meta };
    write_new(project, meta, src.variants)
}

/// Removes the template's folder (backups already hold copies).
pub fn delete(project: &Path, id: &str) -> TemplateResult<()> {
    let dir = template_dir(project, id)?;
    fsx::remove_dir_all_own(&dir).map_err(io_err(&dir))
}

// ---- Migration (schema 1 → 2) ----------------------------------------------------------------

/// The schema of a `template.json` (no key = 1), or `None` when it is unreadable.
fn schema_of(dir: &Path) -> Option<u64> {
    let bytes = fs::read(dir.join(TemplateFile::FILE)).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let obj = v.as_object()?;
    match obj.get("schemaVersion") {
        None => Some(1),
        Some(n) => n.as_u64(),
    }
}

/// Ids of the template folders still on schema 1 (they need [`migrate_v1`]).
pub fn needing_migration(project: &Path) -> Vec<String> {
    let root = templates_root(project);
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|id| is_id(id) && schema_of(&root.join(id)) == Some(1))
        .collect();
    ids.sort();
    ids
}

/// Moves a schema-1 template's look to `variants/default/` and rewrites `template.json` as
/// schema 2 (see the module docs). Does nothing for a folder already on schema 2; a newer or
/// unreadable `template.json` is an error and nothing is changed.
pub fn migrate_v1(project: &Path, id: &str) -> TemplateResult<()> {
    let dir = template_dir(project, id)?;
    if schema_of(&dir) != Some(1) {
        return Ok(());
    }
    let file = persist::load_strict::<TemplateFile>(&dir)?;
    let html = read_text(&dir.join(HTML_FILE))?;
    let css = read_text(&dir.join(CSS_FILE))?;
    validate_html(&html)?;
    validate_css(&css)?;
    // The look first: an interrupted run is redone on the next open (template.json is still v1
    // and its root files are still there).
    let vdir = variant_dir(&dir, DEFAULT_VARIANT);
    if vdir.is_dir() {
        write_look_files(&vdir, &html, &css)?;
    } else {
        write_look(&dir, DEFAULT_VARIANT, &html, &css)?;
    }
    persist::save(&dir, &file)?;
    for f in [HTML_FILE, CSS_FILE] {
        let path = dir.join(f);
        if path.exists() {
            fsx::remove_file_own(&path).map_err(io_err(&path))?;
        }
    }
    Ok(())
}

/// Migrates every schema-1 template of the project. Returns the failures (each template is
/// independent; a failed one keeps reading as before).
pub fn migrate_all(project: &Path) -> Vec<(String, TemplateError)> {
    needing_migration(project)
        .into_iter()
        .filter_map(|id| migrate_v1(project, &id).err().map(|e| (id, e)))
        .collect()
}

/// Rewrites retired font names ([`vars::FONT_ALIASES`]) in every schema-2 `template.json` of the
/// project that holds one (on open; `updatedAt` is kept). Files on another schema are left alone
/// (schema 1 is migrated first; newer ones are never overwritten). Returns the failures.
pub fn rewrite_retired_fonts(project: &Path) -> Vec<(String, TemplateError)> {
    let root = templates_root(project);
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|id| is_id(id))
        .collect();
    ids.sort();
    let mut failed = Vec::new();
    for id in ids {
        let dir = root.join(&id);
        let path = dir.join(TemplateFile::FILE);
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        // Cheap test first: most files name no retired font at all.
        let text = String::from_utf8_lossy(&bytes);
        if !vars::FONT_ALIASES
            .iter()
            .any(|(old, _)| text.contains(&format!("\"{old}\"")))
        {
            continue;
        }
        if schema_of(&dir) != Some(u64::from(TemplateFile::VERSION)) {
            continue;
        }
        let result = persist::decode::<TemplateFile>(&path, &bytes)
            .map_err(TemplateError::from)
            .and_then(|mut file| {
                if normalize_file_fonts(&mut file) {
                    persist::save(&dir, &file)?;
                }
                Ok(())
            });
        if let Err(e) = result {
            failed.push((id, e));
        }
    }
    failed
}

// ---- Cached store --------------------------------------------------------------------------

#[derive(Default)]
struct Cache {
    /// Project the cached list belongs to.
    project: Option<PathBuf>,
    list: Option<Arc<Vec<TemplateSummary>>>,
    /// Bumped on every invalidation, so a list read concurrently with a write is never cached.
    generation: u64,
}

/// The open project's templates: a summary cache (filled from `template.json` files, dropped on
/// every write and on project open/close) plus a lock that serializes template writes. The cache
/// lock is never held across file I/O.
#[derive(Default)]
pub struct TemplateStore {
    cache: Mutex<Cache>,
    writes: Mutex<()>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TemplateStore {
    /// Drops the cache (project opened, closed, moved or reloaded).
    pub fn forget(&self) {
        let mut c = lock(&self.cache);
        c.project = None;
        c.list = None;
        c.generation += 1;
    }

    /// Drops the cached list (a template changed on disk outside Kinetra).
    pub fn invalidate(&self) {
        let mut c = lock(&self.cache);
        c.list = None;
        c.generation += 1;
    }

    /// Summaries, newest first. Served from memory after the first call for a project.
    pub fn list(&self, project: &Path) -> TemplateResult<Vec<TemplateSummary>> {
        let generation = {
            let c = lock(&self.cache);
            if let (Some(p), Some(list)) = (&c.project, &c.list) {
                if p == project {
                    return Ok(list.as_ref().clone());
                }
            }
            c.generation
        };
        let list = list_uncached(project)?;
        let mut c = lock(&self.cache);
        if c.generation == generation {
            c.project = Some(project.to_path_buf());
            c.list = Some(Arc::new(list.clone()));
        }
        Ok(list)
    }

    /// Whether a list for `project` is cached (for tests).
    pub fn is_cached(&self, project: &Path) -> bool {
        let c = lock(&self.cache);
        c.list.is_some() && c.project.as_deref() == Some(project)
    }

    fn write<T>(&self, f: impl FnOnce() -> TemplateResult<T>) -> TemplateResult<T> {
        let _w = lock(&self.writes);
        let result = f();
        // Invalidate even on failure: a partial write may have changed something.
        self.invalidate();
        result
    }

    pub fn get(&self, project: &Path, id: &str) -> TemplateResult<TemplateDoc> {
        get(project, id)
    }

    pub fn create(
        &self,
        project: &Path,
        name: &str,
        source: &TemplateSource,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| create(project, name, source))
    }

    pub fn save(&self, project: &Path, doc: TemplateDoc) -> TemplateResult<TemplateDoc> {
        self.write(|| save(project, doc))
    }

    pub fn import_package(
        &self,
        project: &Path,
        package: &serde_json::Value,
        name: Option<&str>,
        keep_preset: bool,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| import_package(project, package, name, keep_preset))
    }

    pub fn save_variant(
        &self,
        project: &Path,
        id: &str,
        variant: &str,
        html: &str,
        css: &str,
        values: Fields,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| save_variant(project, id, variant, html, css, values))
    }

    pub fn create_variant(
        &self,
        project: &Path,
        id: &str,
        from: &str,
        name: &str,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| create_variant(project, id, from, name))
    }

    pub fn rename_variant(
        &self,
        project: &Path,
        id: &str,
        variant: &str,
        name: &str,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| rename_variant(project, id, variant, name))
    }

    pub fn delete_variant(
        &self,
        project: &Path,
        id: &str,
        variant: &str,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| delete_variant(project, id, variant))
    }

    pub fn set_default_variant(
        &self,
        project: &Path,
        id: &str,
        variant: &str,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| set_default_variant(project, id, variant))
    }

    pub fn rename(&self, project: &Path, id: &str, name: &str) -> TemplateResult<TemplateSummary> {
        self.write(|| rename(project, id, name))
    }

    pub fn duplicate(&self, project: &Path, id: &str, name: &str) -> TemplateResult<TemplateDoc> {
        self.write(|| duplicate(project, id, name))
    }

    pub fn delete(&self, project: &Path, id: &str) -> TemplateResult<()> {
        self.write(|| delete(project, id))
    }

    /// [`add_style_variant`] under the write lock.
    pub fn add_style_variant(
        &self,
        project: &Path,
        id: &str,
        look: &VariantDoc,
        fields: &[FieldDef],
        variables: &[VariableDef],
    ) -> TemplateResult<StyleAdded> {
        self.write(|| add_style_variant(project, id, look, fields, variables))
    }

    /// [`crate::core::template_update::update`] under the write lock.
    pub fn update_from_preset(&self, project: &Path, id: &str) -> TemplateResult<TemplateDoc> {
        self.write(|| crate::core::template_update::update(project, id))
    }

    /// [`crate::core::template_update::restore`] under the write lock.
    pub fn restore_from(
        &self,
        project: &Path,
        id: &str,
        snapshot: &Path,
    ) -> TemplateResult<TemplateDoc> {
        self.write(|| crate::core::template_update::restore(project, id, snapshot))
    }

    /// [`rewrite_retired_fonts`] under the write lock.
    pub fn rewrite_retired_fonts(&self, project: &Path) -> Vec<(String, TemplateError)> {
        self.write(|| Ok(rewrite_retired_fonts(project)))
            .unwrap_or_default()
    }

    /// [`migrate_all`] under the write lock.
    pub fn migrate_all(&self, project: &Path) -> Vec<(String, TemplateError)> {
        self.write(|| Ok(migrate_all(project))).unwrap_or_default()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::core::model::SelectOption;
    use serde_json::Value;

    fn project() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join(TEMPLATES_DIR)).unwrap();
        t
    }

    fn color(key: &str, default: &str) -> VariableDef {
        VariableDef {
            key: key.into(),
            label: "Colour".into(),
            kind: VariableKind::Color,
            default: default.into(),
        }
    }

    fn range(min: f64, max: f64, step: f64, default: &str) -> VariableDef {
        VariableDef {
            key: "size".into(),
            label: "Size".into(),
            kind: VariableKind::Range {
                min,
                max,
                step,
                unit: "px".into(),
            },
            default: default.into(),
        }
    }

    fn select(options: &[&str], default: &str) -> VariableDef {
        VariableDef {
            key: "align".into(),
            label: "Align".into(),
            kind: VariableKind::Select {
                options: options
                    .iter()
                    .map(|v| SelectOption {
                        value: v.to_string(),
                        label: v.to_string(),
                    })
                    .collect(),
            },
            default: default.into(),
        }
    }

    fn field(key: &str) -> FieldDef {
        FieldDef {
            key: key.into(),
            label: "Label".into(),
            kind: FieldType::Text,
            default: String::new(),
            ..Default::default()
        }
    }

    fn rich_doc(p: &Path) -> TemplateDoc {
        let mut doc = create(p, "  Lower third  ", &TemplateSource::Blank).unwrap();
        doc.meta.description = "Two lines".into();
        doc.meta.fields.push(FieldDef {
            key: "title".into(),
            label: "Title".into(),
            kind: FieldType::Multiline,
            default: "x".into(),
            ..Default::default()
        });
        doc.meta.variables = vec![
            color("accent", "#22b8cf"),
            VariableDef {
                key: "font".into(),
                label: "Font".into(),
                kind: VariableKind::Font,
                default: "IBM Plex Sans".into(),
            },
            range(0.0, 24.0, 1.0, "4"),
            select(&["left", "right"], "left"),
        ];
        doc.meta.auto_out_ms = Some(5000);
        doc.variants[0].html = "<div>{{name}} {{title}}</div>".into();
        doc
    }

    #[test]
    fn round_trip_create_get_save_list() {
        let t = project();
        let p = t.path();
        let doc = create(p, "  Lower third  ", &TemplateSource::Blank).unwrap();
        assert_eq!(doc.meta.name, "Lower third", "trimmed");
        assert_eq!(doc.id.len(), 10);
        assert!(doc
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        assert_eq!(doc.meta.fields.len(), 1);
        assert_eq!(doc.meta.fields[0].key, "name");
        assert!(doc.variants[0].html.trim_start().starts_with('<'));
        assert!(doc.meta.preset.is_none());

        let dir = p.join(TEMPLATES_DIR).join(&doc.id);
        let json = fs::read_to_string(dir.join("template.json")).unwrap();
        assert!(json.starts_with("{\n  \"schemaVersion\": 2,"), "{json}");
        assert_eq!(get(p, &doc.id).unwrap(), doc);

        let mut edited = rich_doc(p);
        edited.id = doc.id.clone();
        let saved = save(p, edited.clone()).unwrap();
        assert_eq!(saved.meta.variables, edited.meta.variables);
        assert_eq!(get(p, &doc.id).unwrap(), saved);

        let list = list_uncached(p).unwrap();
        assert_eq!(list.len(), 2, "rich_doc created one more");
        let s = list.iter().find(|s| s.id == doc.id).unwrap();
        assert_eq!(s.field_count, 2);
        assert_eq!(s.variable_count, 4);
        assert_eq!(s.description, "Two lines");
        // No leftovers from atomic writes.
        let names: Vec<String> = fs::read_dir(p.join(TEMPLATES_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.starts_with('.')), "{names:?}");
        let files: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files.len(), 2, "{files:?}");
    }

    #[test]
    fn save_writes_only_changed_files() {
        let t = project();
        let p = t.path();
        let doc = create(p, "A", &TemplateSource::Blank).unwrap();
        let dir = p.join(TEMPLATES_DIR).join(&doc.id);
        // Mark the files so a rewrite is visible.
        let stamp = |f: &str| {
            let path = if f == "template.json" {
                dir.join(f)
            } else {
                dir.join("variants/default").join(f)
            };
            fs::metadata(path).unwrap().modified().unwrap()
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        let before = (stamp(HTML_FILE), stamp(CSS_FILE), stamp("template.json"));

        // Unchanged: nothing is written, updatedAt kept.
        let same = save(p, doc.clone()).unwrap();
        assert_eq!(same.meta.updated_at, doc.meta.updated_at);
        assert_eq!(
            before,
            (stamp(HTML_FILE), stamp(CSS_FILE), stamp("template.json"))
        );

        let mut changed = doc.clone();
        changed.variants[0].css.push_str("\n.x{}");
        save(p, changed).unwrap();
        assert_eq!(stamp(HTML_FILE), before.0, "html untouched");
        assert_ne!(stamp(CSS_FILE), before.1);
        assert_ne!(stamp("template.json"), before.2);
    }

    #[test]
    fn save_keeps_preset_origin_and_requires_existing_template() {
        let t = project();
        let p = t.path();
        let mut doc = create(p, "A", &TemplateSource::Blank).unwrap();
        doc.meta.preset = Some(PresetRef {
            id: "forged".into(),
            version: 9,
        });
        doc.meta.name = "B".into();
        let saved = save(p, doc.clone()).unwrap();
        assert!(saved.meta.preset.is_none());
        doc.id = "nothere123".into();
        assert!(matches!(save(p, doc), Err(TemplateError::NotFound(_))));
    }

    fn assert_invalid(
        p: &Path,
        base: &TemplateDoc,
        edit: impl FnOnce(&mut TemplateDoc),
        what: &str,
    ) {
        let mut d = base.clone();
        edit(&mut d);
        let err = save(p, d).expect_err(what);
        assert!(
            matches!(AppError::from(err), AppError::Invalid(_)),
            "{what} should be invalid"
        );
        assert_eq!(&get(p, &base.id).unwrap(), base, "{what}: nothing written");
    }

    #[test]
    fn every_validation_rule() {
        let t = project();
        let p = t.path();
        let base = save(p, rich_doc(p)).unwrap();
        // Names.
        assert_invalid(p, &base, |d| d.meta.name = "   ".into(), "blank name");
        assert_invalid(p, &base, |d| d.meta.name = "x".repeat(101), "long name");
        assert_invalid(
            p,
            &base,
            |d| d.meta.description = "x".repeat(DESCRIPTION_MAX + 1),
            "long description",
        );
        // Keys.
        for bad in ["", "Name", "1a", "_a", "a b", "a.b", "a/b", &"a".repeat(33)] {
            assert_invalid(p, &base, |d| d.meta.fields[0].key = bad.into(), bad);
            assert_invalid(p, &base, |d| d.meta.variables[0].key = bad.into(), bad);
        }
        assert!(is_key("a") && is_key("a-b_9") && is_key(&"a".repeat(32)));
        assert_invalid(
            p,
            &base,
            |d| d.meta.fields[1].key = "name".into(),
            "dup field",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[0].key = "name".into(),
            "field/variable dup",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[1].key = "accent".into(),
            "dup variable",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.fields[0].label = " ".into(),
            "empty label",
        );
        // Limits.
        assert_invalid(
            p,
            &base,
            |d| d.variants[0].html = "x".repeat(TEMPLATE_HTML_MAX + 1),
            "html",
        );
        assert_invalid(
            p,
            &base,
            |d| d.variants[0].css = "x".repeat(TEMPLATE_CSS_MAX + 1),
            "css",
        );
        assert_invalid(
            p,
            &base,
            |d| {
                d.meta.fields = (0..=TEMPLATE_FIELDS_MAX)
                    .map(|i| field(&format!("f{i}")))
                    .collect()
            },
            "fields",
        );
        assert_invalid(
            p,
            &base,
            |d| {
                d.meta.variables = (0..=TEMPLATE_VARIABLES_MAX)
                    .map(|i| color(&format!("v{i}"), "#fff"))
                    .collect()
            },
            "variables",
        );
        // Variable defaults.
        let inj = "red; } body { display:none";
        for i in 0..4 {
            assert_invalid(
                p,
                &base,
                |d| d.meta.variables[i].default = inj.into(),
                "injection",
            );
        }
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[0].default = "teal".into(),
            "colour",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[1].default = "Arial".into(),
            "font",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[2].default = "25".into(),
            "range max",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[3].default = "up".into(),
            "select",
        );
        // Range parameters.
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[2] = range(5.0, 5.0, 1.0, "5"),
            "min=max",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[2] = range(6.0, 5.0, 1.0, "5"),
            "min>max",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[2] = range(0.0, 5.0, 0.0, "1"),
            "step 0",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[2] = range(0.0, 5.0, -1.0, "1"),
            "step<0",
        );
        assert_invalid(
            p,
            &base,
            |d| {
                d.meta.variables[2] = range(0.0, 5.0, 1.0, "1");
                if let VariableKind::Range { unit, .. } = &mut d.meta.variables[2].kind {
                    *unit = "px;".into();
                }
            },
            "unit",
        );
        // Select options.
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[3] = select(&[], ""),
            "no options",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[3] = select(&["a", "a"], "a"),
            "duplicate option",
        );
        assert_invalid(
            p,
            &base,
            |d| d.meta.variables[3] = select(&["a b"], "a b"),
            "unsafe option",
        );
        // Auto-out.
        assert_invalid(p, &base, |d| d.meta.auto_out_ms = Some(499), "auto-out low");
        assert_invalid(
            p,
            &base,
            |d| d.meta.auto_out_ms = Some(600_001),
            "auto-out high",
        );
        let mut ok = base.clone();
        ok.meta.auto_out_ms = Some(500);
        save(p, ok.clone()).unwrap();
        ok.meta.auto_out_ms = Some(600_000);
        save(p, ok.clone()).unwrap();
        ok.meta.auto_out_ms = None;
        save(p, ok).unwrap();
    }

    #[test]
    fn unsafe_ids_are_rejected() {
        let t = project();
        let p = t.path();
        // A real folder next to `templates/` that traversal would reach.
        fs::create_dir_all(p.join("screens")).unwrap();
        fs::write(p.join("template.json"), "{\"name\":\"x\"}").unwrap();
        for bad in [
            "..",
            "../screens",
            "a/b",
            "a\\b",
            ".",
            "",
            "C:",
            "x y",
            &"a".repeat(65),
        ] {
            assert!(matches!(get(p, bad), Err(TemplateError::BadId(_))), "{bad}");
            assert!(matches!(delete(p, bad), Err(TemplateError::BadId(_))));
            assert!(rename(p, bad, "n").is_err());
            assert!(duplicate(p, bad, "n").is_err());
        }
        assert!(matches!(
            AppError::from(get(p, "..").unwrap_err()),
            AppError::Invalid(_)
        ));
        assert!(matches!(
            AppError::from(get(p, "abc").unwrap_err()),
            AppError::NotFound(_)
        ));
        assert!(p.join("screens").is_dir());
    }

    #[test]
    fn duplicate_rename_delete() {
        let t = project();
        let p = t.path();
        let mut src = rich_doc(p);
        src = save(p, src).unwrap();
        let copy = duplicate(p, &src.id, "Copy").unwrap();
        assert_ne!(copy.id, src.id);
        assert_eq!(copy.meta.name, "Copy");
        assert_eq!(copy.variants, src.variants);
        assert_eq!(copy.meta.variables, src.meta.variables);
        assert!(duplicate(p, &src.id, "").is_err());

        let s = rename(p, &copy.id, " Renamed ").unwrap();
        assert_eq!(s.name, "Renamed");
        assert_eq!(get(p, &copy.id).unwrap().meta.name, "Renamed");
        assert!(matches!(rename(p, &copy.id, ""), Err(TemplateError::Name)));

        delete(p, &copy.id).unwrap();
        assert!(!p.join(TEMPLATES_DIR).join(&copy.id).exists());
        assert!(matches!(get(p, &copy.id), Err(TemplateError::NotFound(_))));
        assert!(matches!(
            delete(p, &copy.id),
            Err(TemplateError::NotFound(_))
        ));
        assert_eq!(list_uncached(p).unwrap().len(), 1);
    }

    #[test]
    fn list_skips_junk_and_sorts_newest_first() {
        let t = project();
        let p = t.path();
        let a = create(p, "A", &TemplateSource::Blank).unwrap();
        let mut b = create(p, "B", &TemplateSource::Blank).unwrap();
        b.meta.updated_at = "2000-01-01T00:00:00Z".into();
        let bdir = p.join(TEMPLATES_DIR).join(&b.id);
        let mut file = persist::load_strict::<TemplateFile>(&bdir).unwrap();
        file.meta.updated_at = "2000-01-01T00:00:00Z".into();
        persist::save(&bdir, &file).unwrap();
        let root = p.join(TEMPLATES_DIR);
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken/template.json"), "{ nope").unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::create_dir_all(root.join(".tmp-x")).unwrap();
        fs::write(root.join("stray.txt"), "x").unwrap();
        let ids: Vec<String> = list_uncached(p)
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, [a.id, b.id]);
        // Missing project templates dir: empty list.
        let empty = tempfile::tempdir().unwrap();
        assert!(list_uncached(empty.path()).unwrap().is_empty());
    }

    #[test]
    fn newer_schema_is_not_overwritten() {
        let t = project();
        let p = t.path();
        let doc = create(p, "A", &TemplateSource::Blank).unwrap();
        let file = p.join(TEMPLATES_DIR).join(&doc.id).join("template.json");
        let text = "{\"schemaVersion\": 9, \"name\": \"future\"}";
        fs::write(&file, text).unwrap();
        assert!(matches!(
            AppError::from(get(p, &doc.id).unwrap_err()),
            AppError::NewerVersion { found: 9, .. }
        ));
        assert!(rename(p, &doc.id, "x").is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), text);
    }

    #[test]
    fn cache_serves_from_memory_and_invalidates_on_writes() {
        let t = project();
        let p = t.path();
        let store = TemplateStore::default();
        assert!(store.list(p).unwrap().is_empty());
        assert!(store.is_cached(p));

        let doc = store.create(p, "A", &TemplateSource::Blank).unwrap();
        assert!(!store.is_cached(p), "create invalidates");
        assert_eq!(store.list(p).unwrap().len(), 1);

        // A file written behind the store's back is not seen while cached...
        create(p, "Outside", &TemplateSource::Blank).unwrap();
        assert_eq!(store.list(p).unwrap().len(), 1, "served from memory");

        // ...but any write through the store refreshes the list.
        let mut edited = doc.clone();
        edited.meta.name = "Saved".into();
        store.save(p, edited).unwrap();
        assert!(!store.is_cached(p));
        let list = store.list(p).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().any(|s| s.name == "Saved"));

        store.rename(p, &doc.id, "Renamed").unwrap();
        assert!(store.list(p).unwrap().iter().any(|s| s.name == "Renamed"));

        let dup = store.duplicate(p, &doc.id, "Dup").unwrap();
        assert_eq!(store.list(p).unwrap().len(), 3);

        store.delete(p, &dup.id).unwrap();
        let list = store.list(p).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|s| s.id != dup.id));

        // A failed write still invalidates; a different project never sees this cache.
        assert!(store.rename(p, "missing", "x").is_err());
        assert!(!store.is_cached(p));
        let other = project();
        assert!(store.list(other.path()).unwrap().is_empty());
        store.forget();
        assert!(!store.is_cached(other.path()));
    }

    #[test]
    fn resolve_vars_uses_defaults_and_rejects_unknown() {
        let defs = vec![color("accent", "#000"), range(0.0, 10.0, 1.0, "2")];
        let css = resolve_vars(&defs, &Fields::new()).unwrap();
        assert_eq!(css["accent"], "#000");
        assert_eq!(css["size"], "2px");
        let v: Fields = [("size".to_string(), "7.5".to_string())].into();
        assert_eq!(resolve_vars(&defs, &v).unwrap()["size"], "7.5px");
        let v: Fields = [("nope".to_string(), "1".to_string())].into();
        assert!(matches!(
            resolve_vars(&defs, &v),
            Err(TemplateError::UnknownVariable(_))
        ));
        let v: Fields = [(
            "accent".to_string(),
            "red; } body { display:none".to_string(),
        )]
        .into();
        assert!(matches!(
            resolve_vars(&defs, &v),
            Err(TemplateError::Variable { .. })
        ));
    }

    #[test]
    fn create_from_a_multi_variant_preset_copies_every_variant() {
        let _g = presets::testing::install(presets::testing::library());
        let t = project();
        let p = t.path();
        let doc = create(
            p,
            "Guest",
            &TemplateSource::Preset {
                preset_id: "lt".into(),
            },
        )
        .unwrap();
        let ids: Vec<&str> = doc.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["classic", "frame"]);
        assert_eq!(doc.meta.default_variant, "classic");
        assert_eq!(
            doc.variants[1].html,
            "<div class=\"lt-frame\">{{name}}</div>"
        );
        assert_eq!(doc.variants[1].vars["accent"], "#22b8cf");
        let dir = p.join(TEMPLATES_DIR).join(&doc.id);
        assert!(variant_dir(&dir, "frame").join(CSS_FILE).is_file());
        assert_eq!(get(p, &doc.id).unwrap(), doc);
        assert_eq!(doc.meta.preset.as_ref().map(|r| r.id.as_str()), Some("lt"));
        // A schema-1 preset still gives one Default variant.
        let legacy = create(
            p,
            "Bar",
            &TemplateSource::Preset {
                preset_id: "legacy-bar".into(),
            },
        )
        .unwrap();
        assert_eq!(legacy.variants.len(), 1);
        assert_eq!(legacy.variants[0].id, DEFAULT_VARIANT);
        assert_eq!(legacy.meta.default_variant, DEFAULT_VARIANT);
    }

    #[test]
    fn add_style_variant_copies_the_look_and_merges_definitions() {
        let _g = presets::testing::install(presets::testing::library());
        let t = project();
        let p = t.path();
        // A project template made from the old one-look preset: "default" only, fewer fields,
        // its own (different) definition of "accent".
        let (mut meta, html, css) = blank("Old".into());
        meta.preset = Some(PresetRef {
            id: "lt".into(),
            version: 1,
        });
        meta.variables = vec![range(0.0, 10.0, 1.0, "2")];
        meta.variables[0].key = "accent".into();
        let variants = single_variant(&mut meta, html, css);
        let old = write_new(p, meta, variants).unwrap();
        let preset = presets::load("lt").unwrap();
        let look = preset.variant("frame").unwrap().clone();
        let added = add_style_variant(
            p,
            &old.id,
            &look,
            &preset.meta.fields,
            &preset.meta.variables,
        )
        .unwrap();
        let StyleAdded::Added(doc) = added else {
            panic!("not added")
        };
        let ids: Vec<&str> = doc.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids, ["default", "frame"]);
        assert_eq!(doc.meta.default_variant, "default", "default unchanged");
        let keys: Vec<&str> = doc.meta.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            ["name", "title", "label", "logo"],
            "missing fields appended"
        );
        // The existing "accent" definition is kept (a range); the preset's others are added.
        assert!(matches!(
            doc.meta.variables[0].kind,
            VariableKind::Range { .. }
        ));
        assert_eq!(doc.meta.variables.len(), 8);
        // The frame's accent value doesn't fit the template's range: dropped; the font stays.
        let frame = &doc.variants[1];
        assert!(!frame.vars.contains_key("accent"));
        assert_eq!(frame.vars["font-heading"], "Space Grotesk");
        assert_eq!(frame.html, look.html);
        assert_eq!(get(p, &old.id).unwrap(), *doc);
        // Again: nothing to do.
        assert_eq!(
            add_style_variant(
                p,
                &old.id,
                &look,
                &preset.meta.fields,
                &preset.meta.variables
            )
            .unwrap(),
            StyleAdded::Present
        );
    }

    #[test]
    fn add_style_variant_renames_a_taken_name_and_respects_limits() {
        let _g = presets::testing::install(presets::testing::library());
        let t = project();
        let p = t.path();
        let doc = create(p, "Mine", &TemplateSource::Blank).unwrap();
        let doc = create_variant(p, &doc.id, DEFAULT_VARIANT, "Frame").unwrap();
        assert_eq!(doc.variants[1].id, "frame");
        let mut look = presets::load("lt")
            .unwrap()
            .variant("frame")
            .unwrap()
            .clone();
        // Same id: present.
        assert_eq!(
            add_style_variant(p, &doc.id, &look, &[], &[]).unwrap(),
            StyleAdded::Present
        );
        look.id = "frame-x".into();
        let StyleAdded::Added(d) = add_style_variant(p, &doc.id, &look, &[], &[]).unwrap() else {
            panic!()
        };
        assert_eq!(d.variants[2].name, "Frame 2");
        // Full: an error, nothing written.
        for i in 0..TEMPLATE_VARIANTS_MAX - 3 {
            create_variant(p, &doc.id, DEFAULT_VARIANT, &format!("V{i}")).unwrap();
        }
        let before = get(p, &doc.id).unwrap();
        assert_eq!(before.variants.len(), TEMPLATE_VARIANTS_MAX);
        look.id = "another".into();
        assert!(matches!(
            add_style_variant(p, &doc.id, &look, &[], &[]),
            Err(TemplateError::TooMany { .. })
        ));
        assert_eq!(get(p, &doc.id).unwrap(), before);
    }

    #[test]
    fn create_from_every_embedded_preset() {
        let t = project();
        let p = t.path();
        for id in presets::ids() {
            let preset = presets::load(&id).unwrap_or_else(|e| panic!("preset {id}: {e}"));
            let doc = create(
                p,
                "From preset",
                &TemplateSource::Preset {
                    preset_id: id.clone(),
                },
            )
            .unwrap_or_else(|e| panic!("create from preset {id}: {e}"));
            assert_eq!(doc.meta.name, "From preset");
            assert_eq!(
                doc.meta.preset,
                Some(PresetRef {
                    id: id.clone(),
                    version: preset.info.version
                })
            );
            assert_eq!(doc.variants.len(), preset.variants.len(), "{id}");
            for (got, want) in doc.variants.iter().zip(&preset.variants) {
                assert_eq!(got.id, want.id, "{id}");
                assert_eq!(got.html, want.html, "{id}");
                assert_eq!(got.css, want.css, "{id}");
                assert_eq!(got.vars, want.vars, "{id}");
            }
            assert_eq!(
                doc.meta.default_variant, preset.meta.default_variant,
                "{id}"
            );
            assert_eq!(doc.meta.fields, preset.meta.fields, "{id}");
            assert_eq!(doc.meta.variables, preset.meta.variables, "{id}");
            assert_eq!(get(p, &doc.id).unwrap(), doc, "{id}");
            resolve_vars(&doc.meta.variables, &Fields::new())
                .unwrap_or_else(|e| panic!("preset {id} defaults: {e}"));
        }
        assert!(matches!(
            create(
                p,
                "x",
                &TemplateSource::Preset {
                    preset_id: "nope".into()
                }
            ),
            Err(TemplateError::UnknownPreset(_))
        ));
        assert!(matches!(
            create(
                p,
                "x",
                &TemplateSource::Preset {
                    preset_id: "../x".into()
                }
            ),
            Err(TemplateError::UnknownPreset(_))
        ));
    }

    // ---- Variants -----------------------------------------------------------------------------

    fn vars(pairs: &[(&str, &str)]) -> Fields {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn ids(doc: &TemplateDoc) -> Vec<&str> {
        doc.variants.iter().map(|v| v.id.as_str()).collect()
    }

    #[test]
    fn new_templates_have_one_default_variant_on_disk() {
        let t = project();
        let p = t.path();
        let doc = create(p, "A", &TemplateSource::Blank).unwrap();
        assert_eq!(doc.meta.default_variant, DEFAULT_VARIANT);
        assert_eq!(ids(&doc), [DEFAULT_VARIANT]);
        assert_eq!(doc.variants[0].name, DEFAULT_VARIANT_NAME);
        let dir = p.join(TEMPLATES_DIR).join(&doc.id);
        assert!(dir.join("variants/default/template.html").is_file());
        assert!(dir.join("variants/default/template.css").is_file());
        assert!(!dir.join(HTML_FILE).exists() && !dir.join(CSS_FILE).exists());
        let json: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("template.json")).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 2);
        assert_eq!(json["defaultVariant"], "default");
        assert_eq!(
            json["variants"],
            serde_json::json!([{"id": "default", "name": "Default", "vars": {}}])
        );
        assert_eq!(list_uncached(p).unwrap()[0].variant_count, 1);
        // The editor's JSON shape (mirrors types.ts): flat meta, variants with their looks.
        let wire = serde_json::to_value(&doc).unwrap();
        assert_eq!(wire["defaultVariant"], "default");
        assert!(wire["variants"][0]["html"].is_string());
        assert!(wire.get("html").is_none());
    }

    #[test]
    fn variant_create_rename_default_delete() {
        let t = project();
        let p = t.path();
        let base = save(p, rich_doc(p)).unwrap();
        let id = base.id.clone();
        let dir = p.join(TEMPLATES_DIR).join(&id);
        let base = save_variant(
            p,
            &id,
            DEFAULT_VARIANT,
            "<div>{{name}}</div>",
            ".a{}",
            vars(&[("accent", "#ff0000")]),
        )
        .unwrap();

        // Duplicate: a slug of the name, look and variable defaults copied, added last.
        let doc = create_variant(p, &id, DEFAULT_VARIANT, "  Compact Look ").unwrap();
        assert_eq!(ids(&doc), ["default", "compact-look"]);
        let c = &doc.variants[1];
        assert_eq!(c.name, "Compact Look");
        assert_eq!(c.html, base.variants[0].html);
        assert_eq!(c.css, ".a{}");
        assert_eq!(c.vars, vars(&[("accent", "#ff0000")]));
        assert!(dir.join("variants/compact-look/template.html").is_file());
        assert_eq!(list_uncached(p).unwrap()[0].variant_count, 2);
        assert_eq!(get(p, &id).unwrap(), doc);

        // Names are unique ignoring case; limits.
        for bad in ["compact look", "DEFAULT", "", "   ", &"x".repeat(41)] {
            let err = create_variant(p, &id, DEFAULT_VARIANT, bad).unwrap_err();
            assert!(matches!(AppError::from(err), AppError::Invalid(_)), "{bad}");
        }
        assert!(matches!(
            AppError::from(create_variant(p, &id, "nope", "X").unwrap_err()),
            AppError::NotFound(_)
        ));
        // Same slug again: suffixed.
        let doc = create_variant(p, &id, "compact-look", "Compact: look!").unwrap();
        assert_eq!(doc.variants[2].id, "compact-look-2");
        for i in 3..TEMPLATE_VARIANTS_MAX {
            create_variant(p, &id, DEFAULT_VARIANT, &format!("V{i}")).unwrap();
        }
        let err = create_variant(p, &id, DEFAULT_VARIANT, "One too many").unwrap_err();
        assert!(matches!(err, TemplateError::TooMany { .. }), "{err}");
        assert_eq!(get(p, &id).unwrap().variants.len(), TEMPLATE_VARIANTS_MAX);

        // Rename: the id stays; a taken name is refused.
        let doc = rename_variant(p, &id, "compact-look", " Small ").unwrap();
        assert_eq!(doc.variants[1].id, "compact-look");
        assert_eq!(doc.variants[1].name, "Small");
        assert!(rename_variant(p, &id, "compact-look", "default").is_err());
        assert!(
            rename_variant(p, &id, "compact-look", "SMALL").is_ok(),
            "itself"
        );
        assert!(matches!(
            AppError::from(rename_variant(p, &id, "nope", "X").unwrap_err()),
            AppError::NotFound(_)
        ));

        // Default, then delete it: the first remaining one becomes the default.
        let doc = set_default_variant(p, &id, "compact-look").unwrap();
        assert_eq!(doc.meta.default_variant, "compact-look");
        let doc = delete_variant(p, &id, "compact-look").unwrap();
        assert_eq!(doc.meta.default_variant, "default");
        assert!(!dir.join("variants/compact-look").exists());
        assert!(!ids(&doc).contains(&"compact-look"));
        set_default_variant(p, &id, "compact-look-2").unwrap();
        let doc = delete_variant(p, &id, "compact-look-2").unwrap();
        assert_eq!(doc.meta.default_variant, "default", "first remaining");
        assert_eq!(doc.variants.len(), TEMPLATE_VARIANTS_MAX - 2);
        assert!(set_default_variant(p, &id, "nope").is_err());

        // Never the last one.
        let rest: Vec<String> = doc.variants.iter().skip(1).map(|v| v.id.clone()).collect();
        for v in rest {
            delete_variant(p, &id, &v).unwrap();
        }
        assert!(matches!(
            delete_variant(p, &id, DEFAULT_VARIANT),
            Err(TemplateError::LastVariant)
        ));
        let names: Vec<String> = fs::read_dir(dir.join(VARIANTS_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["default"], "no leftovers");
    }

    #[test]
    fn variant_slugs() {
        let none = |_: &str| false;
        assert_eq!(variant_slug("Compact", &none), "compact");
        assert_eq!(variant_slug(" Big  & Bold! ", &none), "big-bold");
        assert_eq!(variant_slug("2 lines", &none), "v-2-lines");
        assert_eq!(variant_slug("Ærø", &none), "r");
        assert_eq!(variant_slug("***", &none), "variant");
        assert_eq!(variant_slug("ÆØÅ", &none), "variant");
        let long = variant_slug(&"abc ".repeat(20), &none);
        assert!(long.len() <= KEY_MAX && is_variant_id(&long), "{long}");
        let taken = |s: &str| s == "compact" || s == "compact-2";
        assert_eq!(variant_slug("Compact", &taken), "compact-3");
        let a32 = "a".repeat(KEY_MAX);
        let long_taken = |s: &str| s == a32;
        let s = variant_slug(&"a".repeat(40), &long_taken);
        assert!(
            s.ends_with("-2") && s.len() <= KEY_MAX && is_variant_id(&s),
            "{s}"
        );
        for id in ["default", "a", "a-1", "compact-look"] {
            assert!(is_variant_id(id), "{id}");
        }
        for id in [
            "",
            "A",
            "1a",
            "-a",
            "a_b",
            "a.b",
            "a/b",
            ".tmp-x",
            &"a".repeat(33),
        ] {
            assert!(!is_variant_id(id), "{id}");
        }
    }

    #[test]
    fn save_variant_is_strict_and_writes_only_changes() {
        let t = project();
        let p = t.path();
        let base = save(p, rich_doc(p)).unwrap();
        let id = base.id.clone();
        let html = base.variants[0].html.clone();
        let css = base.variants[0].css.clone();
        for bad in [
            vars(&[("nope", "1")]),
            vars(&[("accent", "teal")]),
            vars(&[("size", "99")]),
            vars(&[("accent", "red; } body { display:none")]),
        ] {
            let err = save_variant(p, &id, DEFAULT_VARIANT, &html, &css, bad).unwrap_err();
            assert!(matches!(AppError::from(err), AppError::Invalid(_)));
        }
        let big = "x".repeat(TEMPLATE_HTML_MAX + 1);
        assert!(save_variant(p, &id, DEFAULT_VARIANT, &big, &css, Fields::new()).is_err());
        assert!(matches!(
            AppError::from(save_variant(p, &id, "nope", &html, &css, Fields::new()).unwrap_err()),
            AppError::NotFound(_)
        ));
        assert_eq!(get(p, &id).unwrap(), base, "nothing written");

        // Unchanged: nothing written (updatedAt kept).
        let same = save_variant(p, &id, DEFAULT_VARIANT, &html, &css, Fields::new()).unwrap();
        assert_eq!(same.meta.updated_at, base.meta.updated_at);
        let doc = save_variant(
            p,
            &id,
            DEFAULT_VARIANT,
            "<p>{{name}}</p>",
            &css,
            vars(&[("size", "7")]),
        )
        .unwrap();
        assert_eq!(doc.variants[0].html, "<p>{{name}}</p>");
        assert_eq!(doc.variants[0].vars, vars(&[("size", "7")]));
        assert_eq!(get(p, &id).unwrap(), doc);
    }

    #[test]
    fn save_keeps_the_variant_list_and_prunes_stale_values() {
        let t = project();
        let p = t.path();
        let base = save(p, rich_doc(p)).unwrap();
        let id = base.id.clone();
        create_variant(p, &id, DEFAULT_VARIANT, "Compact").unwrap();
        save_variant(
            p,
            &id,
            "compact",
            "<b>{{name}}</b>",
            "",
            vars(&[("accent", "#00ff00"), ("align", "right")]),
        )
        .unwrap();

        // A doc that only knows the default variant (an older editor state) and renames the
        // template, with a forged default and a bogus variant: names, default and the list come
        // from disk, the unknown variant is ignored, the missing one kept.
        let mut doc = base.clone();
        doc.meta.name = "Renamed".into();
        doc.meta.default_variant = "compact".into();
        doc.variants[0].name = "Forged".into();
        doc.variants.push(VariantDoc {
            id: "ghost".into(),
            name: "Ghost".into(),
            vars: Fields::new(),
            html: String::new(),
            css: String::new(),
        });
        let saved = save(p, doc).unwrap();
        assert_eq!(saved.meta.name, "Renamed");
        assert_eq!(saved.meta.default_variant, "default");
        assert_eq!(ids(&saved), ["default", "compact"]);
        assert_eq!(saved.variants[0].name, "Default");
        assert_eq!(saved.variants[1].html, "<b>{{name}}</b>");

        // Removing a variable and narrowing a select drops the values that no longer fit.
        let mut doc = saved.clone();
        doc.meta.variables.retain(|v| v.key != "accent");
        doc.meta.variables[2] = select(&["left", "center"], "left");
        let saved = save(p, doc).unwrap();
        assert!(
            saved.variants[1].vars.is_empty(),
            "{:?}",
            saved.variants[1].vars
        );

        // A doc's own variant values for a variable it removes in the same save are pruned too.
        let mut doc = saved.clone();
        doc.variants[0].vars = vars(&[("size", "3")]);
        doc.meta.variables.retain(|v| v.key != "size");
        let saved = save(p, doc).unwrap();
        assert!(saved.variants[0].vars.is_empty());
        assert_eq!(get(p, &id).unwrap(), saved);
    }

    #[test]
    fn validation_of_the_variant_list() {
        let t = project();
        let p = t.path();
        let base = save(p, rich_doc(p)).unwrap();
        let entry = |id: &str, name: &str| VariantDoc {
            id: id.into(),
            name: name.into(),
            vars: Fields::new(),
            html: String::new(),
            css: String::new(),
        };
        let check = |edit: &dyn Fn(&mut TemplateDoc)| {
            let mut d = base.clone();
            edit(&mut d);
            validate_doc(&d)
        };
        assert!(check(&|_| {}).is_ok());
        assert!(check(&|d| d.variants.clear()).is_err());
        assert!(check(&|d| d.variants.push(entry("default", "Other"))).is_err());
        assert!(check(&|d| d.variants.push(entry("Bad", "Other"))).is_err());
        assert!(check(&|d| d.variants.push(entry("b", "DEFAULT"))).is_err());
        assert!(check(&|d| d.variants.push(entry("b", &"x".repeat(41)))).is_err());
        assert!(check(&|d| d.meta.default_variant = "nope".into()).is_err());
        assert!(check(&|d| d.variants[0].vars = vars(&[("nope", "1")])).is_err());
        assert!(check(&|d| d.variants[0].vars = vars(&[("accent", "teal")])).is_err());
        assert!(check(&|d| d.variants[0].css = "x".repeat(TEMPLATE_CSS_MAX + 1)).is_err());
        assert!(check(&|d| {
            d.variants = (0..=TEMPLATE_VARIANTS_MAX)
                .map(|i| entry(&format!("v{i}"), &format!("V{i}")))
                .collect();
            d.meta.default_variant = "v0".into();
        })
        .is_err());
        assert!(check(&|d| {
            d.variants = (0..TEMPLATE_VARIANTS_MAX)
                .map(|i| entry(&format!("v{i}"), &format!("V{i}")))
                .collect();
            d.meta.default_variant = "v7".into();
        })
        .is_ok());
    }

    #[test]
    fn duplicate_copies_every_variant() {
        let t = project();
        let p = t.path();
        let src = save(p, rich_doc(p)).unwrap();
        create_variant(p, &src.id, DEFAULT_VARIANT, "Compact").unwrap();
        save_variant(
            p,
            &src.id,
            "compact",
            "<i></i>",
            "i{}",
            vars(&[("size", "9")]),
        )
        .unwrap();
        let src = set_default_variant(p, &src.id, "compact").unwrap();
        let copy = duplicate(p, &src.id, "Copy").unwrap();
        assert_eq!(copy.variants, src.variants);
        assert_eq!(copy.meta.default_variant, "compact");
        assert_eq!(get(p, &copy.id).unwrap(), copy);
    }

    /// Writes a schema-1 template folder as an older Kinetra did.
    pub(crate) fn write_v1(project: &Path, id: &str, html: &str, css: &str) {
        let dir = project.join(TEMPLATES_DIR).join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("template.json"),
            serde_json::json!({
                "schemaVersion": 1,
                "name": "Old",
                "description": "",
                "preset": null,
                "fields": [{"key": "name", "label": "Name", "type": "text", "default": ""}],
                "variables": [{"key": "accent", "label": "Accent", "type": "color", "default": "#123456"}],
                "autoOutMs": null,
                "updatedAt": "2026-01-01T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        fs::write(dir.join(HTML_FILE), html).unwrap();
        fs::write(dir.join(CSS_FILE), css).unwrap();
    }

    #[test]
    fn migration_moves_the_look_to_the_default_variant_once() {
        let t = project();
        let p = t.path();
        write_v1(p, "old0000001", "<div>{{name}}</div>", ".a{}");
        let dir = p.join(TEMPLATES_DIR).join("old0000001");
        // Before migrating, a schema-1 folder still reads (its root files are the default look).
        let before = get(p, "old0000001").unwrap();
        assert_eq!(ids(&before), ["default"]);
        assert_eq!(before.variants[0].html, "<div>{{name}}</div>");
        assert_eq!(list_uncached(p).unwrap()[0].variant_count, 1);
        let modern = create(p, "New", &TemplateSource::Blank).unwrap();
        assert_eq!(needing_migration(p), ["old0000001"]);

        assert!(migrate_all(p).is_empty());
        assert!(needing_migration(p).is_empty());
        assert!(!dir.join(HTML_FILE).exists() && !dir.join(CSS_FILE).exists());
        assert_eq!(
            fs::read_to_string(dir.join("variants/default/template.html")).unwrap(),
            "<div>{{name}}</div>"
        );
        assert_eq!(
            fs::read_to_string(dir.join("variants/default/template.css")).unwrap(),
            ".a{}"
        );
        let json: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("template.json")).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 2);
        assert_eq!(json["variants"][0]["name"], "Default");
        assert_eq!(json["updatedAt"], "2026-01-01T00:00:00Z", "kept");
        let after = get(p, "old0000001").unwrap();
        assert_eq!(after, before, "the same template");

        // Idempotent: a second run changes nothing.
        let stamp = fs::metadata(dir.join("template.json"))
            .unwrap()
            .modified()
            .unwrap();
        assert!(migrate_all(p).is_empty());
        migrate_v1(p, "old0000001").unwrap();
        assert_eq!(
            fs::metadata(dir.join("template.json"))
                .unwrap()
                .modified()
                .unwrap(),
            stamp
        );
        assert_eq!(get(p, &modern.id).unwrap(), modern, "untouched");

        // An interrupted run (look copied, template.json still v1) is redone from the old files.
        write_v1(p, "old0000002", "<p>new</p>", "p{}");
        let d2 = p.join(TEMPLATES_DIR).join("old0000002");
        fs::create_dir_all(d2.join("variants/default")).unwrap();
        fs::write(d2.join("variants/default/template.html"), "<p>stale</p>").unwrap();
        migrate_v1(p, "old0000002").unwrap();
        assert_eq!(get(p, "old0000002").unwrap().variants[0].html, "<p>new</p>");
        assert!(!d2.join(HTML_FILE).exists());
    }

    #[test]
    fn retired_font_names_read_as_the_replacement_and_are_rewritten_once() {
        let (old, new) = vars::FONT_ALIASES[0];
        let t = project();
        let p = t.path();
        let doc = create(p, "Fonts", &TemplateSource::Blank).unwrap();
        let dir = p.join(TEMPLATES_DIR).join(&doc.id);
        let path = dir.join("template.json");
        let mut json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        json["variables"] = serde_json::json!([
            { "key": "font", "label": "Font", "type": "font", "default": old },
            { "key": "pick", "label": "Pick", "type": "select", "default": old,
              "options": [{ "value": old, "label": "Old" }] }
        ]);
        json["variants"][0]["vars"] = serde_json::json!({ "font": old, "pick": old });
        json["updatedAt"] = "2026-01-01T00:00:00Z".into();
        fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        // Read: the font variable's default and the variant value are the replacement; a select
        // option that happens to share the name is not a font and stays.
        let got = get(p, &doc.id).unwrap();
        assert_eq!(got.meta.variables[0].default, new);
        assert_eq!(got.meta.variables[1].default, old);
        assert_eq!(got.variants[0].vars["font"], new);
        assert_eq!(got.variants[0].vars["pick"], old);
        assert!(validate_doc(&got).is_ok());

        // On open: rewritten in place, updatedAt kept.
        assert!(rewrite_retired_fonts(p).is_empty());
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["variables"][0]["default"], new);
        assert_eq!(json["variables"][1]["default"], old);
        assert_eq!(json["variants"][0]["vars"]["font"], new);
        assert_eq!(json["variants"][0]["vars"]["pick"], old);
        assert_eq!(json["updatedAt"], "2026-01-01T00:00:00Z");
        // Nothing left to do: the file is not written again.
        let stamp = fs::metadata(&path).unwrap().modified().unwrap();
        assert!(rewrite_retired_fonts(p).is_empty());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp);

        // Saving a document that still names the retired font writes the replacement.
        let mut doc = get(p, &doc.id).unwrap();
        doc.meta.variables[0].default = old.into();
        doc.variants[0].vars.insert("font".into(), old.into());
        let saved = save(p, doc).unwrap();
        assert_eq!(saved.meta.variables[0].default, new);
        let json: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["variables"][0]["default"], new);
        assert_eq!(json["variants"][0]["vars"]["font"], new);
    }

    #[test]
    fn migration_leaves_newer_and_broken_files_alone() {
        let t = project();
        let p = t.path();
        write_v1(p, "future0001", "<p></p>", "");
        let dir = p.join(TEMPLATES_DIR).join("future0001");
        let text = "{\"schemaVersion\": 3, \"name\": \"future\"}";
        fs::write(dir.join("template.json"), text).unwrap();
        let root = p.join(TEMPLATES_DIR);
        fs::create_dir_all(root.join("broken")).unwrap();
        fs::write(root.join("broken/template.json"), "{ nope").unwrap();
        assert!(needing_migration(p).is_empty());
        assert!(migrate_all(p).is_empty());
        migrate_v1(p, "future0001").unwrap();
        assert_eq!(fs::read_to_string(dir.join("template.json")).unwrap(), text);
        assert!(dir.join(HTML_FILE).is_file());
        assert!(!dir.join(VARIANTS_DIR).exists());
        // Newer: read-only.
        assert!(matches!(
            AppError::from(get(p, "future0001").unwrap_err()),
            AppError::NewerVersion { found: 3, .. }
        ));
        assert!(create_variant(p, "future0001", "default", "X").is_err());
        assert!(save_variant(p, "future0001", "default", "", "", Fields::new()).is_err());
        assert_eq!(fs::read_to_string(dir.join("template.json")).unwrap(), text);
    }

    #[test]
    fn saving_an_unmigrated_template_writes_the_new_layout() {
        let t = project();
        let p = t.path();
        write_v1(p, "old0000003", "<div></div>", "");
        let doc = get(p, "old0000003").unwrap();
        let saved = save(p, doc.clone()).unwrap();
        let dir = p.join(TEMPLATES_DIR).join("old0000003");
        assert!(dir.join("variants/default/template.html").is_file());
        assert_eq!(saved.variants, doc.variants);
        let json: Value =
            serde_json::from_str(&fs::read_to_string(dir.join("template.json")).unwrap()).unwrap();
        assert_eq!(json["schemaVersion"], 2);
    }

    #[test]
    fn design_is_optional_validated_and_round_trips() {
        use crate::core::model::{ContentSizing, TemplateDesign};
        let d = |w, h, x: Option<f64>| TemplateDesign {
            w,
            h,
            x,
            y: None,
            sizing: ContentSizing::Fit,
        };
        assert!(validate_design(None).is_ok());
        assert!(validate_design(Some(&d(16, 8192, Some(-50.0)))).is_ok());
        for bad in [
            d(15, 100, None),
            d(100, 8193, None),
            d(100, 100, Some(f64::NAN)),
        ] {
            assert!(validate_design(Some(&bad)).is_err(), "{bad:?}");
        }
        // An old template.json without it loads unchanged; written only when set.
        let old: TemplateFile = serde_json::from_str(r#"{"name":"A","variants":[]}"#).unwrap();
        assert_eq!(old.meta.design, None);
        assert!(!serde_json::to_string(&old).unwrap().contains("design"));
        let mut f = old.clone();
        f.meta.design = Some(TemplateDesign {
            w: 640,
            h: 360,
            x: None,
            y: Some(700.0),
            sizing: ContentSizing::Reflow,
        });
        let j = serde_json::to_string(&f).unwrap();
        assert!(
            j.contains(r#""design":{"w":640,"h":360,"y":700.0,"sizing":"reflow"}"#),
            "{j}"
        );
        let back: TemplateFile = serde_json::from_str(&j).unwrap();
        assert_eq!(back, f);
        // `sizing` defaults to reflow when missing.
        let m: TemplateDesign = serde_json::from_str(r#"{"w":100,"h":50}"#).unwrap();
        assert_eq!(m.sizing, ContentSizing::Reflow);
    }
}
