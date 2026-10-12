//! Screens stored in the project: `screens/<id>.json` = `{ "schemaVersion": 1, ...ScreenMeta }`.
//!
//! - Ids are 10-character nanoids over `[a-z0-9]`, like templates; ids from the UI are validated
//!   with `templates::is_id` (no separators, `.` or `..`).
//! - Everything is validated before anything is written ([`validate`]); writes are atomic
//!   (`fsx`), and a file from a newer schema is never overwritten.
//! - Soft problems never stop a screen from loading: a layer whose template is missing stays in
//!   the file but is left out of the live state, and unknown or invalid field/variable values are
//!   dropped in the live state (template defaults apply). Both are reported as
//!   [`ScreenWarning`]s by [`plan`].
//! - A template layer's `bindings` take bound fields from a source's current row (`plan` gets
//!   the rows from `SourceStore`); a missing source, column or row is a soft `SourceMissing`.
//! - `guides` and `safeAreas` are editor-only (validated and saved here, omitted when empty);
//!   [`plan`] never reads them, so they never reach the live state or a renderer.
//! - [`ScreenStore`] holds the open project's summary cache, the lock serializing screen writes,
//!   and what the live state was built from (the last good documents and their templates), so
//!   template changes and drags can be re-applied without reading the disk.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use serde::Serialize;

use crate::core::anchors;
use crate::core::assets;
use crate::core::fieldvals;
use crate::core::fsx;
use crate::core::live::{media_template, LayerContent, LayerPlan, ScreenPlan, TemplateDef};
use crate::core::media::MediaContent;
use crate::core::media_probe;
use crate::core::model::{
    BrandKit, LayerAnchor, MediaInfo, MediaKind, MediaLayerConfig, NewScreen, Rect, RowSelector,
    SafeArea, ScreenDoc, ScreenGuides, ScreenLayer, ScreenLayerKind, ScreenMeta, ScreenSummary,
    ScreenWarning, ScreenWarningCode, SourceBinding, SourceKindName, TemplateDesign, TemplateDoc,
    TemplateMeta, VariableKind, MEDIA_VOLUME_MAX, SAFE_AREA_NAME_MAX, SCREEN_GUIDES_MAX,
    SCREEN_LAYERS_MAX, SCREEN_NAME_MAX, SCREEN_SAFE_AREAS_MAX, SCREEN_SIZE_RANGE,
};
use crate::core::persist::{self, PersistError, Recovered, Store};
use crate::core::protocol::{
    ContentSizing, DesignSize, FieldType, Fields, ScreenInfo, LIST_ROWS_MAX,
};
use crate::core::schedule;
use crate::core::sources::SourceMap;
use crate::core::styles::StyleUndo;
use crate::core::templates::{self, is_id, is_key};
use crate::core::vars;
use crate::error::AppError;

pub const SCREENS_DIR: &str = "screens";
/// Longest layer name.
pub const LAYER_NAME_MAX: usize = 100;
/// Longest field or variable value in a screen file, in bytes.
pub const VALUE_MAX: usize = 64 * 1024;
/// Largest layer box side: four times the largest screen.
pub const RECT_SIDE_MAX: u32 = 4 * 8192;

#[derive(Debug, thiserror::Error)]
pub enum ScreenError {
    #[error("{0}")]
    Invalid(String),
    #[error("Screen not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Persist(#[from] PersistError),
    #[error("{}: {source}", fsx::display_path(path))]
    Io { path: PathBuf, source: io::Error },
}

impl From<ScreenError> for AppError {
    fn from(e: ScreenError) -> Self {
        match e {
            ScreenError::Invalid(m) => AppError::Invalid(m),
            ScreenError::NotFound(_) => AppError::NotFound(e.to_string()),
            ScreenError::Persist(p) => p.into(),
            ScreenError::Io { path, source } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(&path)),
            )),
        }
    }
}

pub type ScreenResult<T> = Result<T, ScreenError>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> ScreenError + '_ {
    move |source| ScreenError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid(msg: impl Into<String>) -> ScreenError {
    ScreenError::Invalid(msg.into())
}

// ---- The screen file as a versioned store ---------------------------------------------------

// Here rather than a derive so `model.rs` (mirrored in TS) stays untouched.
impl Default for ScreenMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            width: 1920,
            height: 1080,
            background: "transparent".into(),
            layers: Vec::new(),
            guides: ScreenGuides::default(),
            safe_areas: Vec::new(),
            updated_at: String::new(),
        }
    }
}

impl Store for ScreenMeta {
    /// Unused: screen files are named after their id (see [`screen_path`]).
    const FILE: &'static str = "screen.json";
    const VERSION: u32 = 1;
}

#[derive(Serialize)]
struct Versioned<'a> {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(flatten)]
    meta: &'a ScreenMeta,
}

// ---- Validation ----------------------------------------------------------------------------

pub fn validate_id(id: &str) -> ScreenResult<()> {
    if is_id(id) {
        Ok(())
    } else {
        Err(invalid(format!("Invalid screen id: {id}")))
    }
}

/// Trimmed, 1..=`SCREEN_NAME_MAX` characters.
pub fn validate_name(name: &str) -> ScreenResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > SCREEN_NAME_MAX {
        return Err(invalid(format!(
            "Screen name must be 1 to {SCREEN_NAME_MAX} characters"
        )));
    }
    Ok(name.to_string())
}

pub fn validate_size(width: u32, height: u32) -> ScreenResult<()> {
    if SCREEN_SIZE_RANGE.contains(&width) && SCREEN_SIZE_RANGE.contains(&height) {
        Ok(())
    } else {
        Err(invalid(format!(
            "Resolution must be between {} and {} pixels on each side",
            SCREEN_SIZE_RANGE.start(),
            SCREEN_SIZE_RANGE.end()
        )))
    }
}

/// `transparent`, `#rrggbb` or `#rrggbbaa`.
pub fn validate_background(bg: &str) -> ScreenResult<()> {
    let ok = bg == "transparent"
        || bg.strip_prefix('#').is_some_and(|hex| {
            matches!(hex.len(), 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
        });
    if ok {
        Ok(())
    } else {
        Err(invalid(
            "Background must be transparent or a color like #000000",
        ))
    }
}

/// Width and height 1..=`RECT_SIDE_MAX`, and the box must overlap the screen.
pub fn validate_rect(rect: &Rect, width: u32, height: u32) -> ScreenResult<()> {
    if !(1..=RECT_SIDE_MAX).contains(&rect.w) || !(1..=RECT_SIDE_MAX).contains(&rect.h) {
        return Err(invalid(format!(
            "Layer size must be between 1 and {RECT_SIDE_MAX} pixels on each side"
        )));
    }
    let (x, y) = (i64::from(rect.x), i64::from(rect.y));
    let inside = x < i64::from(width)
        && y < i64::from(height)
        && x + i64::from(rect.w) > 0
        && y + i64::from(rect.h) > 0;
    if !inside {
        return Err(invalid("The layer must overlap the screen"));
    }
    Ok(())
}

/// The screen a template's `design.x`/`design.y` refer to.
pub const DESIGN_REFERENCE: (u32, u32) = (1920, 1080);

/// Box and sizing of a new layer showing a template on a `width` × `height` screen (the same
/// rule as the layout editor's `designRect`, src/lib/components/screens/editor/model.ts):
/// - with a `design`: the design size, shrunk uniformly (`min(W / w, H / h)`, each side at
///   least one) only when it is larger than the screen; at `round(design.x × W / 1920)` /
///   `round(design.y × H / 1080)`, or centered (`floor((W − w) / 2)`) when absent; clamped
///   inside the screen; sizing `fit` when `design.sizing` is fit, else none (reflow until the
///   user resizes the layer in the editor, which scales it; only a Reflow the user picks stays);
/// - without: the full screen and no sizing (`reflow`), as before sizing existed.
pub fn place_layer(
    design: Option<&TemplateDesign>,
    width: u32,
    height: u32,
) -> (Rect, Option<ContentSizing>) {
    let full = Rect {
        x: 0,
        y: 0,
        w: width,
        h: height,
    };
    let Some(d) = design else {
        return (full, None);
    };
    let (sw, sh) = (f64::from(width), f64::from(height));
    let (mut w, mut h) = (d.w, d.h);
    if w > width || h > height {
        let k = (sw / f64::from(w)).min(sh / f64::from(h));
        w = ((f64::from(w) * k).round() as u32).max(1);
        h = ((f64::from(h) * k).round() as u32).max(1);
    }
    let pos = |p: Option<f64>, screen: f64, reference: u32, size: u32| {
        let free = screen - f64::from(size);
        let v = match p.filter(|v| v.is_finite()) {
            Some(v) => (v * screen / f64::from(reference)).round(),
            None => (free / 2.0).floor(),
        };
        v.min(free).max(0.0) as i32
    };
    let rect = Rect {
        x: pos(d.x, sw, DESIGN_REFERENCE.0, w),
        y: pos(d.y, sh, DESIGN_REFERENCE.1, h),
        w,
        h,
    };
    (
        rect,
        (d.sizing == ContentSizing::Fit).then_some(ContentSizing::Fit),
    )
}

fn validate_values(layer: &str, what: &str, values: &Fields) -> ScreenResult<()> {
    for (k, v) in values {
        if !is_key(k) {
            return Err(invalid(format!(
                "Layer \"{layer}\": invalid {what} key \"{k}\""
            )));
        }
        if v.len() > VALUE_MAX {
            return Err(invalid(format!(
                "Layer \"{layer}\": {what} \"{k}\" is too long (at most {} KB)",
                VALUE_MAX / 1024
            )));
        }
    }
    Ok(())
}

/// Binding keys are field keys, source ids are path-safe ids and columns are keys. Whether the
/// template has the field or the source has the column is a soft check ([`plan`]).
fn validate_bindings(layer: &str, bindings: &BTreeMap<String, SourceBinding>) -> ScreenResult<()> {
    for (k, b) in bindings {
        if !is_key(k) {
            return Err(invalid(format!(
                "Layer \"{layer}\": invalid binding key \"{k}\""
            )));
        }
        if !is_id(&b.source_id) {
            return Err(invalid(format!(
                "Layer \"{layer}\": field \"{k}\" is bound to an invalid source id: {}",
                b.source_id
            )));
        }
        // `""` = the whole source (list fields).
        if !b.column.is_empty() && !is_key(&b.column) {
            return Err(invalid(format!(
                "Layer \"{layer}\": field \"{k}\" is bound to an invalid column: {}",
                b.column
            )));
        }
        if let Some(s) = &b.sort {
            if !is_key(&s.column) {
                return Err(invalid(format!(
                    "Layer \"{layer}\": field \"{k}\" is sorted by an invalid column: {}",
                    s.column
                )));
            }
        }
        if let Some(n) = b.limit {
            if !(1..=LIST_ROWS_MAX).contains(&(n as usize)) {
                return Err(invalid(format!(
                    "Layer \"{layer}\": field \"{k}\": the row limit must be 1 to {LIST_ROWS_MAX}"
                )));
            }
        }
    }
    Ok(())
}

/// The asset name of a media layer's `asset` (`/assets/<name>` or a bare name).
pub fn media_asset_name(asset: &str) -> &str {
    asset.strip_prefix(assets::URL_PREFIX).unwrap_or(asset)
}

/// A media layer's config: a media library file (stored as `/assets/<name>`), volume 0–100,
/// an out point after the in point. Clamping to the file's duration happens in [`plan`].
fn validate_media(layer: &str, m: &mut MediaLayerConfig) -> ScreenResult<()> {
    let name = media_asset_name(m.asset.trim()).to_string();
    if !assets::is_asset_name(&name) || media_probe::kind_of_name(&name).is_none() {
        return Err(invalid(format!(
            "Layer \"{layer}\": not a media file: {}",
            m.asset
        )));
    }
    m.asset = assets::url_of(&name);
    if m.volume > MEDIA_VOLUME_MAX {
        return Err(invalid(format!(
            "Layer \"{layer}\": volume must be 0 to {MEDIA_VOLUME_MAX}"
        )));
    }
    if let Some(out) = m.out_ms {
        if out <= m.in_ms {
            return Err(invalid(format!(
                "Layer \"{layer}\": the out point must be after the in point"
            )));
        }
    }
    Ok(())
}

/// Ruler guides: at most `SCREEN_GUIDES_MAX` per axis, each within `-size..=2 * size`. Sorted and
/// deduplicated in place (the file stays stable and git-friendly).
fn validate_guides(g: &mut ScreenGuides, width: u32, height: u32) -> ScreenResult<()> {
    for (axis, list, size) in [("x", &mut g.x, width), ("y", &mut g.y, height)] {
        if list.len() > SCREEN_GUIDES_MAX {
            return Err(invalid(format!(
                "Too many {axis} guides ({}, at most {SCREEN_GUIDES_MAX})",
                list.len()
            )));
        }
        let size = i64::from(size);
        if let Some(bad) = list
            .iter()
            .find(|&&v| !(-size..=2 * size).contains(&i64::from(v)))
        {
            return Err(invalid(format!(
                "Guide {axis} = {bad} is out of range ({} to {})",
                -size,
                2 * size
            )));
        }
        list.sort_unstable();
        list.dedup();
    }
    Ok(())
}

/// Named safe areas: at most `SCREEN_SAFE_AREAS_MAX`, unique valid ids, names trimmed to
/// 1..=`SAFE_AREA_NAME_MAX` characters, rects like a layer's (they must overlap the screen).
fn validate_safe_areas(areas: &mut [SafeArea], width: u32, height: u32) -> ScreenResult<()> {
    if areas.len() > SCREEN_SAFE_AREAS_MAX {
        return Err(invalid(format!(
            "Too many safe areas ({}, at most {SCREEN_SAFE_AREAS_MAX})",
            areas.len()
        )));
    }
    let mut ids = HashSet::new();
    for a in areas {
        if !is_id(&a.id) {
            return Err(invalid(format!("Invalid safe area id: {}", a.id)));
        }
        if !ids.insert(a.id.clone()) {
            return Err(invalid(format!("Duplicate safe area id: {}", a.id)));
        }
        let name = a.name.trim();
        if name.is_empty() || name.chars().count() > SAFE_AREA_NAME_MAX {
            return Err(invalid(format!(
                "Safe area name must be 1 to {SAFE_AREA_NAME_MAX} characters"
            )));
        }
        a.name = name.to_string();
        validate_rect(&a.rect, width, height)
            .map_err(|e| invalid(format!("Safe area \"{}\": {e}", a.name)))?;
    }
    Ok(())
}

/// Hard checks: nothing is written when one fails. Returns the document with names trimmed.
pub fn validate(mut doc: ScreenDoc) -> ScreenResult<ScreenDoc> {
    validate_id(&doc.id)?;
    let m = &mut doc.meta;
    m.name = validate_name(&m.name)?;
    validate_size(m.width, m.height)?;
    validate_background(&m.background)?;
    if m.layers.len() > SCREEN_LAYERS_MAX {
        return Err(invalid(format!(
            "Too many layers ({}, at most {SCREEN_LAYERS_MAX})",
            m.layers.len()
        )));
    }
    // Anchored layers are placed before their boxes are checked (always inside: see anchors).
    anchors::place_all(m);
    let mut ids = HashSet::new();
    for l in &mut m.layers {
        if !is_id(&l.id) {
            return Err(invalid(format!("Invalid layer id: {}", l.id)));
        }
        if !ids.insert(l.id.clone()) {
            return Err(invalid(format!("Duplicate layer id: {}", l.id)));
        }
        let name = l.name.trim();
        if name.is_empty() || name.chars().count() > LAYER_NAME_MAX {
            return Err(invalid(format!(
                "Layer name must be 1 to {LAYER_NAME_MAX} characters"
            )));
        }
        l.name = name.to_string();
        validate_rect(&l.rect, m.width, m.height)
            .map_err(|e| invalid(format!("Layer \"{}\": {e}", l.name)))?;
        match &mut l.kind {
            ScreenLayerKind::Template {
                template_id,
                fields,
                vars,
                bindings,
                variant,
                opacity,
                ..
            } => {
                match *opacity {
                    Some(o) if o > 100 => {
                        return Err(invalid(format!(
                            "Layer \"{}\": opacity must be 0 to 100",
                            l.name
                        )))
                    }
                    // 100 % is the default: stored as absent.
                    Some(100) => *opacity = None,
                    _ => {}
                }
                if !is_id(template_id) {
                    return Err(invalid(format!(
                        "Layer \"{}\": invalid template id: {template_id}",
                        l.name
                    )));
                }
                if let Some(v) = variant {
                    if !templates::is_variant_id(v) {
                        return Err(invalid(format!(
                            "Layer \"{}\": invalid variant id: {v}",
                            l.name
                        )));
                    }
                }
                validate_values(&l.name, "field", fields)?;
                validate_values(&l.name, "variable", vars)?;
                validate_bindings(&l.name, bindings)?;
            }
            ScreenLayerKind::Media { media, opacity } => {
                match *opacity {
                    Some(o) if o > 100 => {
                        return Err(invalid(format!(
                            "Layer \"{}\": opacity must be 0 to 100",
                            l.name
                        )))
                    }
                    Some(100) => *opacity = None,
                    _ => {}
                }
                validate_media(&l.name, media)?;
            }
        }
    }
    validate_guides(&mut m.guides, m.width, m.height)?;
    validate_safe_areas(&mut m.safe_areas, m.width, m.height)?;
    Ok(doc)
}

// ---- Disk operations (uncached; `ScreenStore` and `screen_live` wrap them) ---------------------

pub fn screens_root(project: &Path) -> PathBuf {
    project.join(SCREENS_DIR)
}

/// `screens/<id>.json` for a valid id.
pub fn screen_path(project: &Path, id: &str) -> ScreenResult<PathBuf> {
    validate_id(id)?;
    Ok(screens_root(project).join(format!("{id}.json")))
}

/// Reads and decodes a screen file (no validation, nothing on disk changes).
fn read_meta(path: &Path, id: &str) -> ScreenResult<ScreenMeta> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ScreenError::NotFound(id.to_string()))
        }
        Err(e) => return Err(io_err(path)(e)),
    };
    Ok(persist::decode::<ScreenMeta>(path, &bytes)?)
}

/// Reads and validates `screens/<id>.json` (anchored layers placed). Never changes anything on
/// disk.
pub fn get(project: &Path, id: &str) -> ScreenResult<ScreenDoc> {
    get_after(project, id, None).map(|(doc, _)| doc)
}

/// [`get`] for a file edited outside: with the previous version `prev` (as applied), a layer it
/// moved by hand releases the moved anchor axis ([`anchors::release_moved`]). Also tells whether
/// an anchor was released (the document then differs from the file).
pub fn get_after(
    project: &Path,
    id: &str,
    prev: Option<&ScreenMeta>,
) -> ScreenResult<(ScreenDoc, bool)> {
    let path = screen_path(project, id)?;
    let mut meta = read_meta(&path, id)?;
    let released = prev.is_some_and(|p| anchors::release_moved(p, &mut meta));
    let doc = validate(ScreenDoc {
        id: id.to_string(),
        meta,
    })?;
    Ok((doc, released))
}

fn write(project: &Path, doc: &ScreenDoc) -> ScreenResult<()> {
    let root = screens_root(project);
    fs::create_dir_all(&root).map_err(io_err(&root))?;
    let path = screen_path(project, &doc.id)?;
    let v = Versioned {
        schema_version: ScreenMeta::VERSION,
        meta: &doc.meta,
    };
    fsx::write_json_atomic(&path, &v).map_err(io_err(&path))
}

/// Validates and writes `doc`, setting `updatedAt`. An unchanged document is not written (its
/// `updatedAt` is kept). A newer-schema file is never overwritten. With `create: false` the file
/// must already exist.
pub fn save(project: &Path, doc: ScreenDoc, create: bool) -> ScreenResult<ScreenDoc> {
    save_reporting(project, doc, create).map(|(doc, _)| doc)
}

/// [`save`], also telling whether the file was written.
pub fn save_reporting(
    project: &Path,
    doc: ScreenDoc,
    create: bool,
) -> ScreenResult<(ScreenDoc, bool)> {
    let path = screen_path(project, &doc.id)?;
    let old = read_meta(&path, &doc.id);
    let mut doc = doc;
    if let Ok(old) = &old {
        // Anchors: compare with the previous saved version as it was placed.
        let mut prev = old.clone();
        anchors::place_all(&mut prev);
        anchors::release_moved(&prev, &mut doc.meta);
    }
    let mut doc = validate(doc)?;
    match old {
        Ok(old) => {
            let mut same = old.clone();
            same.updated_at = doc.meta.updated_at.clone();
            if same == doc.meta {
                doc.meta.updated_at = old.updated_at;
                return Ok((doc, false));
            }
        }
        Err(ScreenError::NotFound(_)) if create => {}
        Err(e @ ScreenError::NotFound(_)) => return Err(e),
        // A corrupt file may be replaced by a valid document; a newer one never.
        Err(ScreenError::Persist(PersistError::Corrupt { .. })) => {}
        Err(e) => return Err(e),
    }
    doc.meta.updated_at = fsx::now_rfc3339();
    write(project, &doc)?;
    Ok((doc, true))
}

/// A fresh screen id not taken in `screens/`.
fn new_screen_id(project: &Path) -> String {
    let root = screens_root(project);
    loop {
        let id = templates::random_id();
        if !root.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

/// A fresh layer id, unique among `taken`.
pub fn new_layer_id(taken: &HashSet<String>) -> String {
    loop {
        let id = templates::random_id();
        if !taken.contains(&id) {
            return id;
        }
    }
}

pub fn create(project: &Path, new: &NewScreen) -> ScreenResult<ScreenDoc> {
    let doc = ScreenDoc {
        id: new_screen_id(project),
        meta: ScreenMeta {
            name: new.name.clone(),
            width: new.width,
            height: new.height,
            background: new.background.clone(),
            ..ScreenMeta::default()
        },
    };
    save(project, doc, true)
}

pub fn rename(project: &Path, id: &str, name: &str) -> ScreenResult<ScreenDoc> {
    let mut doc = get(project, id)?;
    doc.meta.name = validate_name(name)?;
    save(project, doc, false)
}

/// A copy with a new screen id and new layer ids.
pub fn duplicate(project: &Path, id: &str, name: &str) -> ScreenResult<ScreenDoc> {
    let name = validate_name(name)?;
    let src = get(project, id)?;
    duplicate_doc(project, src, &name)
}

/// A copy of `src` with a new screen id and new layer ids.
pub fn duplicate_doc(project: &Path, src: ScreenDoc, name: &str) -> ScreenResult<ScreenDoc> {
    let name = validate_name(name)?;
    let mut taken = HashSet::new();
    let layers = src
        .meta
        .layers
        .into_iter()
        .map(|l| {
            let id = new_layer_id(&taken);
            taken.insert(id.clone());
            ScreenLayer { id, ..l }
        })
        .collect();
    let doc = ScreenDoc {
        id: new_screen_id(project),
        meta: ScreenMeta {
            name,
            layers,
            ..src.meta
        },
    };
    save(project, doc, true)
}

pub fn delete(project: &Path, id: &str) -> ScreenResult<()> {
    let path = screen_path(project, id)?;
    match fsx::remove_file_own(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(ScreenError::NotFound(id.into())),
        Err(e) => Err(io_err(&path)(e)),
    }
}

pub fn summary(doc: &ScreenDoc) -> ScreenSummary {
    ScreenSummary {
        id: doc.id.clone(),
        name: doc.meta.name.clone(),
        width: doc.meta.width,
        height: doc.meta.height,
        background: doc.meta.background.clone(),
        layer_count: doc.meta.layers.len() as u32,
        updated_at: doc.meta.updated_at.clone(),
    }
}

/// Ids of `screens/<id>.json` files (sorted).
fn file_ids(project: &Path) -> ScreenResult<Vec<String>> {
    let root = screens_root(project);
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(&root)(e)),
    };
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(io_err(&root))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if is_id(id) && entry.file_type().is_ok_and(|t| t.is_file()) {
            ids.push(id.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

/// Every valid screen, newest first (then by name, id). Files that fail to load are logged and
/// skipped, never changed.
pub fn list_uncached(project: &Path) -> ScreenResult<Vec<ScreenSummary>> {
    let mut out = Vec::new();
    for id in file_ids(project)? {
        match get(project, &id) {
            Ok(doc) => out.push(summary(&doc)),
            Err(e) => tracing::warn!("skipping screen {id}: {e}"),
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

/// Every screen for opening the project. A corrupt file (not JSON, wrong shape) is renamed to
/// `<file>.corrupt-<timestamp>` and reported in the second list; files from a newer version or
/// failing validation are logged and skipped (left alone).
pub fn load_all(project: &Path) -> (Vec<ScreenDoc>, Vec<Recovered>) {
    let mut docs = Vec::new();
    let mut recovered = Vec::new();
    let ids = match file_ids(project) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("reading screens failed: {e}");
            return (docs, recovered);
        }
    };
    for id in ids {
        match get(project, &id) {
            Ok(doc) => docs.push(doc),
            Err(ScreenError::Persist(e @ PersistError::Corrupt { .. })) => {
                let Ok(path) = screen_path(project, &id) else {
                    continue;
                };
                tracing::warn!("{e}; setting it aside");
                match persist::quarantine(&path) {
                    Ok(r) => recovered.push(r),
                    Err(e) => tracing::warn!("could not set aside {}: {e}", path.display()),
                }
            }
            Err(e) => tracing::warn!("skipping screen {id}: {e}"),
        }
    }
    (docs, recovered)
}

/// Rewrites retired font names ([`vars::FONT_ALIASES`]) in the layers' variable overrides, for
/// the variables their template declares as fonts (a layer whose template is missing is left
/// alone). True when anything changed.
pub fn normalize_fonts(doc: &mut ScreenDoc, templates: &TemplateMap) -> bool {
    let mut changed = false;
    for l in &mut doc.meta.layers {
        let ScreenLayerKind::Template {
            template_id, vars, ..
        } = &mut l.kind
        else {
            continue;
        };
        let Some(t) = templates.get(template_id.as_str()) else {
            continue;
        };
        for d in &t.meta.variables {
            if matches!(d.kind, VariableKind::Font) {
                if let Some(v) = vars.get_mut(&d.key) {
                    changed |= vars::normalize_font(v);
                }
            }
        }
    }
    changed
}

/// [`normalize_fonts`] over freshly loaded screens, writing the changed ones (on open). A screen
/// that cannot be written keeps its normalized values in memory (they render the same).
pub fn rewrite_retired_fonts(project: &Path, docs: &mut [ScreenDoc], templates: &TemplateMap) {
    for doc in docs.iter_mut() {
        if !normalize_fonts(doc, templates) {
            continue;
        }
        match save(project, doc.clone(), false) {
            Ok(saved) => *doc = saved,
            Err(e) => tracing::warn!("screen {}: retired font names not rewritten: {e}", doc.id),
        }
    }
}

/// The image field the brand kit's logo fills when a layer leaves it empty.
pub const LOGO_FIELD: &str = "logo";

// ---- Templates as screens use them -----------------------------------------------------------

/// One variant of a loaded template: its variable defaults and the shared definition every
/// layer showing it points to.
#[derive(Debug)]
pub struct LoadedVariant {
    pub id: String,
    pub name: String,
    pub vars: Fields,
    pub def: Arc<TemplateDef>,
}

/// A template loaded for screen layers: its metadata plus one definition per variant.
#[derive(Debug)]
pub struct LoadedTemplate {
    pub meta: TemplateMeta,
    /// In the template's order; never empty for a validated template.
    pub variants: Vec<LoadedVariant>,
}

impl LoadedTemplate {
    pub fn from_doc(doc: &TemplateDoc) -> Self {
        let variants = doc
            .variants
            .iter()
            .map(|v| LoadedVariant {
                id: v.id.clone(),
                name: v.name.clone(),
                vars: v.vars.clone(),
                def: Arc::new(TemplateDef {
                    html: v.html.clone(),
                    css: v.css.clone(),
                    fields: doc.meta.fields.clone(),
                    design: doc.meta.design.map(|d| DesignSize { w: d.w, h: d.h }),
                }),
            })
            .collect();
        Self {
            meta: doc.meta.clone(),
            variants,
        }
    }

    pub fn has_variant(&self, id: &str) -> bool {
        self.variants.iter().any(|v| v.id == id)
    }

    /// The template's default variant (the first one if `defaultVariant` is not in the list).
    pub fn default_variant(&self) -> Option<&LoadedVariant> {
        self.variants
            .iter()
            .find(|v| v.id == self.meta.default_variant)
            .or_else(|| self.variants.first())
    }

    /// The variant a layer shows: `wanted` (`None` = the default). The bool is true when
    /// `wanted` is not in the template (the default is used).
    pub fn variant(&self, wanted: Option<&str>) -> Option<(&LoadedVariant, bool)> {
        match wanted.and_then(|w| self.variants.iter().find(|v| v.id == w)) {
            Some(v) => Some((v, false)),
            None => self.default_variant().map(|v| (v, wanted.is_some())),
        }
    }
}

/// Loads and validates a template for screen layers. `Ok(None)` when it does not exist; `Err`
/// with a message when it exists but cannot be used.
pub fn load_template(project: &Path, id: &str) -> Result<Option<Arc<LoadedTemplate>>, String> {
    match templates::get(project, id) {
        Ok(doc) => {
            templates::validate_doc(&doc).map_err(|e| e.to_string())?;
            Ok(Some(Arc::new(LoadedTemplate::from_doc(&doc))))
        }
        Err(templates::TemplateError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Template ids used by a document's layers.
pub fn template_ids(doc: &ScreenDoc) -> impl Iterator<Item = &str> {
    doc.meta.layers.iter().filter_map(|l| l.kind.template_id())
}

/// Media asset names (without `/assets/`) used by a document's media layers.
pub fn media_names(doc: &ScreenDoc) -> impl Iterator<Item = &str> {
    doc.meta
        .layers
        .iter()
        .filter_map(|l| l.kind.media())
        .map(|m| media_asset_name(&m.asset))
}

/// What the probe knows about media files, by asset name: `None` = the file is missing. A name
/// not in the map is unknown (no duration, no warning).
pub type MediaMap = HashMap<String, Option<MediaInfo>>;

/// The probes of the media files `doc` uses (one `stat` each; a changed file is probed again,
/// headers only). Touches the disk.
pub fn media_for(project: &Path, doc: &ScreenDoc) -> MediaMap {
    let root = assets::assets_root(project);
    media_names(doc)
        .map(|n| (n.to_string(), media_probe::info_cached(&root, n)))
        .collect()
}

pub type TemplateMap = HashMap<String, Arc<LoadedTemplate>>;

/// Source ids bound by a document's layers (may repeat).
pub fn source_ids(doc: &ScreenDoc) -> impl Iterator<Item = &str> {
    doc.meta.layers.iter().flat_map(|l| {
        let b = match &l.kind {
            ScreenLayerKind::Template { bindings, .. } => Some(bindings),
            ScreenLayerKind::Media { .. } => None,
        };
        b.into_iter()
            .flat_map(|b| b.values().map(|b| b.source_id.as_str()))
    })
}

/// The layers of `doc` bound to source `id`, each with its bound field keys.
pub fn bound_layers(doc: &ScreenDoc, id: &str) -> Vec<(String, Vec<String>)> {
    doc.meta
        .layers
        .iter()
        .filter_map(|l| {
            let ScreenLayerKind::Template { bindings, .. } = &l.kind else {
                return None;
            };
            let keys: Vec<String> = bindings
                .iter()
                .filter(|(_, b)| b.source_id == id)
                .map(|(k, _)| k.clone())
                .collect();
            (!keys.is_empty()).then(|| (l.id.clone(), keys))
        })
        .collect()
}

/// Whether any layer of `doc` is bound to source `id`.
pub fn binds_source(doc: &ScreenDoc, id: &str) -> bool {
    source_ids(doc).any(|s| s == id)
}

/// What the live state should show for `doc`, and the soft problems found on the way. Layers
/// whose template is not in `templates` are left out (`TemplateMissing`); field values the
/// template doesn't have and variable values that are unknown or invalid are dropped
/// (`InvalidValue`), so template defaults apply. A bound field takes the current row's value of
/// its source column from `sources`; a missing source, column or row falls back to the layer's
/// own value (`SourceMissing`), and a binding for a field the template doesn't have is ignored
/// (`InvalidValue`).
///
/// Variables resolve as: layer override → `brand` (its keys map to the variables `accent`,
/// `secondary`, `text`, `plate`, `font-heading`, `font-body`; only for templates that declare
/// them, and only values valid for that definition) → the variant's value → the definition's
/// default. An `image` field with the key `logo` whose value is empty takes `brand.logo`. Row
/// marks (`_winner`, `_highlight`) are set last (`fieldvals::apply_row_marks`).
pub fn plan(
    doc: &ScreenDoc,
    templates: &TemplateMap,
    sources: &SourceMap,
    brand: &BrandKit,
) -> (ScreenPlan, Vec<ScreenWarning>) {
    plan_with(doc, templates, sources, brand, &MediaMap::new())
}

/// A media layer's plan: kind and duration from `media` (by extension when the file is unknown
/// there), in/out clamped to a known duration. `None` (with an `assetMissing` warning) when
/// `media` says the file is missing.
fn plan_media(
    l: &ScreenLayer,
    config: &MediaLayerConfig,
    opacity: Option<u8>,
    media: &MediaMap,
    warnings: &mut Vec<ScreenWarning>,
) -> Option<LayerPlan> {
    let name = media_asset_name(&config.asset);
    let info = match media.get(name) {
        Some(None) => {
            warnings.push(ScreenWarning {
                layer_id: l.id.clone(),
                code: ScreenWarningCode::AssetMissing,
                message: format!("The file \u{201c}{name}\u{201d} is missing"),
            });
            return None;
        }
        Some(Some(i)) => Some(i),
        None => None,
    };
    let kind = info
        .map(|i| i.kind)
        .or_else(|| media_probe::kind_of_name(name))
        .unwrap_or(MediaKind::Video);
    let duration_ms = info
        .and_then(|i| i.duration_ms)
        .filter(|_| kind != MediaKind::Image);
    let mut config = config.clone();
    if let Some(d) = duration_ms {
        config.in_ms = config.in_ms.min(d.saturating_sub(1));
        config.out_ms = config
            .out_ms
            .map(|o| o.min(d))
            .filter(|o| *o > config.in_ms);
    }
    Some(LayerPlan {
        id: l.id.clone(),
        name: l.name.clone(),
        content: LayerContent {
            template: media_template(),
            variant: String::new(),
            fields: Fields::new(),
            variables: Vec::new(),
            var_values: Fields::new(),
            vars: Fields::new(),
            auto_out_ms: None,
            rect: Some(l.rect),
            sizing: ContentSizing::Reflow,
            opacity: opacity.unwrap_or(100).min(100),
            media: Some(MediaContent {
                config,
                kind,
                duration_ms,
            }),
        },
    })
}

/// [`plan`] with what the probe knows about the media files (`media_for`).
pub fn plan_with(
    doc: &ScreenDoc,
    templates: &TemplateMap,
    sources: &SourceMap,
    brand: &BrandKit,
    media: &MediaMap,
) -> (ScreenPlan, Vec<ScreenWarning>) {
    let mut warnings = Vec::new();
    let mut layers = Vec::new();
    for l in &doc.meta.layers {
        let (template_id, fields, overrides, bindings, variant, sizing, opacity) = match &l.kind {
            ScreenLayerKind::Template {
                template_id,
                fields,
                vars,
                bindings,
                variant,
                sizing,
                opacity,
            } => (
                template_id,
                fields,
                vars,
                bindings,
                variant,
                sizing,
                opacity,
            ),
            ScreenLayerKind::Media { media: m, opacity } => {
                if let Some(p) = plan_media(l, m, *opacity, media, &mut warnings) {
                    layers.push(p);
                }
                continue;
            }
        };
        let warn = |code, message: String| ScreenWarning {
            layer_id: l.id.clone(),
            code,
            message,
        };
        let Some((t, (shown, variant_missing))) = templates
            .get(template_id)
            .and_then(|t| t.variant(variant.as_deref()).map(|v| (t, v)))
        else {
            warnings.push(warn(
                ScreenWarningCode::TemplateMissing,
                "The template is missing or can't be opened".to_string(),
            ));
            continue;
        };
        if variant_missing {
            warnings.push(warn(
                ScreenWarningCode::VariantMissing,
                format!(
                    "Variant \u{201c}{}\u{201d} isn't in the template \u{201c}{}\u{201d}: showing \u{201c}{}\u{201d}",
                    variant.as_deref().unwrap_or_default(),
                    t.meta.name,
                    shown.name
                ),
            ));
        }
        for k in fields
            .keys()
            .filter(|k| !t.meta.fields.iter().any(|f| &f.key == *k))
        {
            warnings.push(warn(
                ScreenWarningCode::InvalidValue,
                format!("The template has no field \"{k}\""),
            ));
        }
        for k in overrides
            .keys()
            .filter(|k| !t.meta.variables.iter().any(|v| &v.key == *k))
        {
            warnings.push(warn(
                ScreenWarningCode::InvalidValue,
                format!("The template has no variable \"{k}\""),
            ));
        }
        let mut field_values = Fields::new();
        for f in &t.meta.fields {
            let v = match fields.get(&f.key) {
                None => fieldvals::wire_default(f),
                Some(raw) => match fieldvals::own_value(f, raw) {
                    Ok((v, dropped)) => {
                        if dropped {
                            warnings.push(warn(
                                ScreenWarningCode::InvalidValue,
                                format!(
                                    "Field \"{}\": cells of unknown columns were dropped",
                                    f.key
                                ),
                            ));
                        }
                        v
                    }
                    Err(msg) => {
                        warnings.push(warn(
                            ScreenWarningCode::InvalidValue,
                            format!("Field \"{}\": {msg}", f.key),
                        ));
                        fieldvals::wire_default(f)
                    }
                },
            };
            field_values.insert(f.key.clone(), v);
        }
        for (k, b) in bindings {
            let def = t.meta.fields.iter().find(|f| &f.key == k);
            let (Some(def), Some(slot)) = (def, field_values.get_mut(k)) else {
                warnings.push(warn(
                    ScreenWarningCode::InvalidValue,
                    format!("The template has no field \"{k}\" to bind"),
                ));
                continue;
            };
            let schedule = sources
                .get(&b.source_id)
                .is_some_and(|c| c.kind == SourceKindName::Schedule);
            let misuse = match (def.kind, b.column.is_empty()) {
                // A timer shows a schedule item's countdown.
                (FieldType::Timer, false) if schedule && b.column == "countdown" => None,
                (FieldType::Clock | FieldType::Timer, _) => Some(
                    "clock and timer fields cannot be bound to a source (a timer can show a schedule's countdown)",
                ),
                (FieldType::List, false) => Some("a list field binds a whole source, not a column"),
                (FieldType::List, true) => None,
                (_, true) => Some("the binding has no column"),
                (_, false) => None,
            };
            if let Some(m) = misuse {
                warnings.push(warn(
                    ScreenWarningCode::InvalidValue,
                    format!("Field \"{k}\": {m}"),
                ));
                continue;
            }
            let missing = match sources.get(&b.source_id) {
                None => Some("its source was deleted".to_string()),
                Some(c) if def.kind == FieldType::List => {
                    let (list, notes) = fieldvals::list_from_rows(
                        &def.columns,
                        &c.columns,
                        &c.rows,
                        b.sort.as_ref(),
                        b.limit,
                    );
                    *slot = list;
                    for n in notes {
                        warnings.push(warn(
                            ScreenWarningCode::InvalidValue,
                            format!("Field \"{k}\": {n}"),
                        ));
                    }
                    None
                }
                Some(c) if !c.columns.contains(&b.column) => {
                    Some(format!("the source has no column \"{}\"", b.column))
                }
                Some(c) => {
                    let row = match b.row.unwrap_or_default() {
                        RowSelector::Current => &c.row,
                        RowSelector::Now => &c.now_row,
                        RowSelector::Next => &c.next_row,
                    };
                    match row {
                        // No NOW / UP NEXT item right now: the field is empty (a countdown
                        // stopped at zero), not the layer's own value.
                        None if c.kind == SourceKindName::Schedule => {
                            *slot = if def.kind == FieldType::Timer {
                                schedule::countdown_zero()
                            } else {
                                String::new()
                            };
                            None
                        }
                        None => Some("the source has no rows".to_string()),
                        Some(row) => {
                            *slot = row.get(&b.column).cloned().unwrap_or_default();
                            None
                        }
                    }
                }
            };
            if let Some(m) = missing {
                warnings.push(warn(
                    ScreenWarningCode::SourceMissing,
                    format!("Field \"{k}\": {m}"),
                ));
            }
        }
        if let Some(logo) = brand.logo.as_deref().filter(|l| !l.is_empty()) {
            let is_logo = t
                .meta
                .fields
                .iter()
                .any(|f| f.key == LOGO_FIELD && f.kind == FieldType::Image);
            if let (true, Some(v)) = (is_logo, field_values.get_mut(LOGO_FIELD)) {
                if v.trim().is_empty() {
                    *v = logo.to_string();
                }
            }
        }
        fieldvals::apply_row_marks(&t.meta.fields, &mut field_values);
        let mut var_values = Fields::new();
        let mut css = Fields::new();
        for def in &t.meta.variables {
            let chosen = overrides
                .get(&def.key)
                .and_then(|v| match vars::css_value(def, v) {
                    Ok(c) => Some((v.clone(), c)),
                    Err(msg) => {
                        warnings.push(warn(
                            ScreenWarningCode::InvalidValue,
                            format!("Variable \"{}\": {msg}", def.key),
                        ));
                        None
                    }
                });
            // Layer override, then the brand kit, then the variant's default, then the
            // definition's.
            let pair = chosen
                .or_else(|| {
                    brand
                        .var(&def.key)
                        .and_then(|v| vars::css_value(def, v).ok().map(|c| (v.to_string(), c)))
                })
                .or_else(|| {
                    shown
                        .vars
                        .get(&def.key)
                        .and_then(|v| vars::css_value(def, v).ok().map(|c| (v.clone(), c)))
                })
                .or_else(|| {
                    vars::css_value(def, &def.default)
                        .ok()
                        .map(|c| (def.default.clone(), c))
                });
            if let Some((v, c)) = pair {
                var_values.insert(def.key.clone(), v);
                css.insert(def.key.clone(), c);
            }
        }
        layers.push(LayerPlan {
            id: l.id.clone(),
            name: l.name.clone(),
            content: LayerContent {
                template: shown.def.clone(),
                variant: shown.id.clone(),
                fields: field_values,
                variables: t.meta.variables.clone(),
                var_values,
                vars: css,
                auto_out_ms: t.meta.auto_out_ms,
                rect: Some(l.rect),
                sizing: sizing.unwrap_or_default(),
                opacity: opacity.unwrap_or(100).min(100),
                media: None,
            },
        });
    }
    let plan = ScreenPlan {
        info: ScreenInfo {
            id: doc.id.clone(),
            name: doc.meta.name.clone(),
            width: doc.meta.width,
            height: doc.meta.height,
            background: doc.meta.background.clone(),
        },
        layers,
    };
    (plan, warnings)
}

// ---- Store -----------------------------------------------------------------------------------

#[derive(Default)]
struct SummaryCache {
    project: Option<PathBuf>,
    list: Option<Arc<Vec<ScreenSummary>>>,
    generation: u64,
}

/// What the live state of the open project's screens was built from.
#[derive(Default)]
struct Applied {
    project: Option<PathBuf>,
    /// Last good document per screen id.
    docs: BTreeMap<String, ScreenDoc>,
    /// Templates used by those documents (absent = missing or not loaded yet).
    templates: TemplateMap,
    /// Screens whose remembered document is newer than their file (Control field edits), written
    /// by the content flusher.
    dirty: BTreeSet<String>,
    /// The project's brand kit (`settings.brand`) every plan resolves against.
    brand: Arc<BrandKit>,
    /// The latest `apply_style` undo step (in memory only).
    style_undo: Option<StyleUndo>,
}

/// The open project's screens: a summary cache (dropped on every write, watcher change and
/// project switch), the lock that serializes screen writes and live applies, and the documents
/// and templates the live state was built from. Data locks are never held across file I/O; the
/// write lock is (it only orders writers).
#[derive(Default)]
pub struct ScreenStore {
    cache: Mutex<SummaryCache>,
    applied: Mutex<Applied>,
    writes: Mutex<()>,
    /// Told (after the change, no lock held) when the set of screens or a screen size changed:
    /// display outputs follow it. Must not block.
    docs_listener: RwLock<Option<DocsListener>>,
}

/// See `ScreenStore::set_docs_listener`.
pub type DocsListener = Arc<dyn Fn() + Send + Sync>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ScreenStore {
    /// Sets the listener told when screens are added or removed or a size changes.
    pub fn set_docs_listener(&self, l: Option<DocsListener>) {
        *self
            .docs_listener
            .write()
            .unwrap_or_else(PoisonError::into_inner) = l;
    }

    fn docs_changed(&self) {
        let l = self
            .docs_listener
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(l) = l {
            l();
        }
    }

    /// Every remembered screen's size (display outputs: which screens exist, pattern sizes).
    pub fn sizes(&self) -> HashMap<String, (u32, u32)> {
        lock(&self.applied)
            .docs
            .values()
            .map(|d| (d.id.clone(), (d.meta.width, d.meta.height)))
            .collect()
    }

    /// Serializes screen writes and live applies (and project switches against them).
    pub fn write_lock(&self) -> MutexGuard<'_, ()> {
        lock(&self.writes)
    }

    /// Drops the summary cache.
    pub fn invalidate(&self) {
        let mut c = lock(&self.cache);
        c.list = None;
        c.generation += 1;
    }

    /// Forgets everything (project closed, or about to be replaced). Caller holds `write_lock`.
    pub fn forget(&self) {
        {
            let mut c = lock(&self.cache);
            c.project = None;
            c.list = None;
            c.generation += 1;
        }
        *lock(&self.applied) = Applied::default();
        self.docs_changed();
    }

    /// Installs what a freshly loaded project's live state was built from. Caller holds
    /// `write_lock`.
    pub fn install(
        &self,
        project: &Path,
        docs: Vec<ScreenDoc>,
        templates: TemplateMap,
        brand: BrandKit,
    ) {
        self.invalidate();
        *lock(&self.applied) = Applied {
            project: Some(project.to_path_buf()),
            docs: docs.into_iter().map(|d| (d.id.clone(), d)).collect(),
            templates,
            dirty: BTreeSet::new(),
            brand: Arc::new(brand),
            style_undo: None,
        };
        self.docs_changed();
    }

    /// The brand kit plans resolve against.
    pub fn brand(&self) -> Arc<BrandKit> {
        lock(&self.applied).brand.clone()
    }

    /// Replaces the brand kit (the caller re-applies the screens). Caller holds `write_lock`.
    pub fn set_brand(&self, brand: BrandKit) {
        lock(&self.applied).brand = Arc::new(brand);
    }

    /// Replaces the latest style undo step.
    pub fn set_style_undo(&self, undo: Option<StyleUndo>) {
        lock(&self.applied).style_undo = undo;
    }

    /// Takes the style undo step if its token is `token` (it is consumed); `None` otherwise
    /// (the step is kept).
    pub fn take_style_undo(&self, token: &str) -> Option<StyleUndo> {
        let mut a = lock(&self.applied);
        if a.style_undo.as_ref().is_some_and(|u| u.token == token) {
            a.style_undo.take()
        } else {
            None
        }
    }

    /// The open project's folder as the store knows it.
    pub fn project(&self) -> Option<PathBuf> {
        lock(&self.applied).project.clone()
    }

    /// The project folder moved; the documents are unchanged.
    pub fn relocate(&self, project: &Path) {
        {
            let mut c = lock(&self.cache);
            c.project = None;
            c.list = None;
            c.generation += 1;
        }
        let mut a = lock(&self.applied);
        if a.project.is_some() {
            a.project = Some(project.to_path_buf());
        }
    }

    /// Whether the live state was built for `project` (it is still the open one).
    pub fn is_project(&self, project: &Path) -> bool {
        lock(&self.applied).project.as_deref() == Some(project)
    }

    /// Summaries, newest first. Served from memory after the first call for a project.
    pub fn list(&self, project: &Path) -> ScreenResult<Vec<ScreenSummary>> {
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

    /// The templates `doc` needs: from memory, else loaded from disk (and remembered). Missing or
    /// unusable ones are left out. Caller holds `write_lock`; the data lock is not held while
    /// reading files.
    pub fn templates_for(&self, project: &Path, doc: &ScreenDoc) -> TemplateMap {
        let mut have = TemplateMap::new();
        let mut need = Vec::new();
        {
            let a = lock(&self.applied);
            for id in template_ids(doc) {
                match a.templates.get(id) {
                    Some(t) => {
                        have.insert(id.to_string(), t.clone());
                    }
                    None => need.push(id.to_string()),
                }
            }
        }
        need.sort();
        need.dedup();
        let mut loaded = Vec::new();
        for id in need {
            match load_template(project, &id) {
                Ok(Some(t)) => loaded.push((id, t)),
                Ok(None) => {}
                Err(e) => tracing::warn!("template {id} cannot be used: {e}"),
            }
        }
        if !loaded.is_empty() {
            let mut a = lock(&self.applied);
            for (id, t) in loaded {
                a.templates.insert(id.clone(), t.clone());
                have.insert(id, t);
            }
        }
        have
    }

    /// Remembers `doc` as the screen's last good document.
    pub fn set_doc(&self, doc: ScreenDoc) {
        let size = (doc.meta.width, doc.meta.height);
        let before = lock(&self.applied)
            .docs
            .insert(doc.id.clone(), doc)
            .map(|d| (d.meta.width, d.meta.height));
        if before != Some(size) {
            self.docs_changed();
        }
    }

    pub fn doc(&self, id: &str) -> Option<ScreenDoc> {
        lock(&self.applied).docs.get(id).cloned()
    }

    pub fn remove_doc(&self, id: &str) -> Option<ScreenDoc> {
        let removed = {
            let mut a = lock(&self.applied);
            a.dirty.remove(id);
            a.docs.remove(id)
        };
        if removed.is_some() {
            self.docs_changed();
        }
        removed
    }

    /// The remembered document is newer than the file (written by the content flusher).
    pub fn mark_dirty(&self, id: &str) {
        let mut a = lock(&self.applied);
        if a.docs.contains_key(id) {
            a.dirty.insert(id.to_string());
        }
    }

    /// The file is current again (saved, or replaced from outside).
    pub fn clear_dirty(&self, id: &str) {
        lock(&self.applied).dirty.remove(id);
    }

    pub fn is_dirty(&self, id: &str) -> bool {
        lock(&self.applied).dirty.contains(id)
    }

    /// Takes the dirty set: the documents to write and the project folder.
    pub fn take_dirty(&self) -> Option<(PathBuf, Vec<ScreenDoc>)> {
        let mut a = lock(&self.applied);
        if a.dirty.is_empty() {
            return None;
        }
        let project = a.project.clone()?;
        let ids = std::mem::take(&mut a.dirty);
        let docs = ids
            .iter()
            .filter_map(|id| a.docs.get(id).cloned())
            .collect();
        Some((project, docs))
    }

    /// Documents with a layer bound to source `id`.
    pub fn source_users(&self, id: &str) -> Vec<ScreenDoc> {
        lock(&self.applied)
            .docs
            .values()
            .filter(|d| binds_source(d, id))
            .cloned()
            .collect()
    }

    /// Source id → ids of the screens bound to it (sorted).
    pub fn source_usage(&self) -> HashMap<String, Vec<String>> {
        let a = lock(&self.applied);
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        for d in a.docs.values() {
            let mut ids: Vec<&str> = source_ids(d).collect();
            ids.sort_unstable();
            ids.dedup();
            for s in ids {
                out.entry(s.to_string()).or_default().push(d.id.clone());
            }
        }
        out
    }

    /// Where a drag to `rect` puts a layer of the remembered document, and its anchor
    /// afterwards ([`anchors::moved`]: a move releases the moved axis, a resize stays anchored).
    /// `rect` and no anchor when the screen or layer is not remembered.
    pub fn moved_layer_rect(
        &self,
        screen_id: &str,
        layer_id: &str,
        rect: Rect,
    ) -> (Rect, Option<LayerAnchor>) {
        let a = lock(&self.applied);
        let Some(d) = a.docs.get(screen_id) else {
            return (rect, None);
        };
        match d.meta.layers.iter().find(|l| l.id == layer_id) {
            Some(l) => anchors::moved(l.rect, l.anchor, rect, d.meta.width, d.meta.height),
            None => (rect, None),
        }
    }

    /// Updates a layer's box and anchor in the remembered document (a drag; the file is saved on
    /// drop).
    pub fn set_layer_rect(
        &self,
        screen_id: &str,
        layer_id: &str,
        rect: Rect,
        anchor: Option<LayerAnchor>,
    ) {
        let mut a = lock(&self.applied);
        if let Some(l) = a
            .docs
            .get_mut(screen_id)
            .and_then(|d| d.meta.layers.iter_mut().find(|l| l.id == layer_id))
        {
            l.rect = rect;
            l.anchor = anchor;
        }
    }

    /// Replaces (`Some`) or forgets (`None`) a template and returns the documents using it.
    pub fn set_template(&self, id: &str, t: Option<Arc<LoadedTemplate>>) -> Vec<ScreenDoc> {
        let mut a = lock(&self.applied);
        let users: Vec<ScreenDoc> = a
            .docs
            .values()
            .filter(|d| template_ids(d).any(|t| t == id))
            .cloned()
            .collect();
        match t {
            // Only templates in use are kept in memory.
            Some(t) if !users.is_empty() || a.templates.contains_key(id) => {
                a.templates.insert(id.to_string(), t);
            }
            Some(_) => {}
            None => {
                a.templates.remove(id);
            }
        }
        users
    }

    /// Templates currently in memory (a copy of the map; the values are shared).
    pub fn templates(&self) -> TemplateMap {
        lock(&self.applied).templates.clone()
    }

    /// Ids of the remembered documents.
    pub fn doc_ids(&self) -> Vec<String> {
        lock(&self.applied).docs.keys().cloned().collect()
    }
}

/// Loads every template the documents use. Missing or unusable ones are left out (logged).
pub fn load_templates_for(project: &Path, docs: &[ScreenDoc]) -> TemplateMap {
    let mut ids: Vec<&str> = docs.iter().flat_map(template_ids).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut map = TemplateMap::new();
    for id in ids {
        match load_template(project, id) {
            Ok(Some(t)) => {
                map.insert(id.to_string(), t);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("template {id} cannot be used: {e}"),
        }
    }
    map
}

#[cfg(test)]
mod tests;
