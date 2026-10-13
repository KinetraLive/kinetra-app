//! Sources stored in the project: `sources/<id>.json` = `{ "schemaVersion": 1, ...SourceMeta }`.
//! Kinds: `text`, a table with columns and one row per item, stepped with Next/Prev (or
//! auto-advanced as a slideshow); `schedule`, a run of show whose statuses are computed
//! from the clock (`core::schedule`, `core::rules`).
//!
//! - Ids are 10-character nanoids like screens; ids from the UI are validated with
//!   `templates::is_id` (no separators, `.` or `..`).
//! - Everything is validated before anything is written ([`validate`]); writes are atomic
//!   (`fsx`, recorded as own writes so the watcher ignores them), and a file from a newer schema is
//!   never overwritten.
//! - [`SourceStore`] holds every valid source of the open project in memory (sources are small and
//!   screen plans read their current rows on every apply), so `list` and `get` never touch the
//!   disk. Corrupt files are set aside only when the project opens, like screens.
//! - A step changes only the in-memory `index` and marks the source dirty; the content flusher
//!   (`core::source_live`) writes it coalesced.
//!
//! Locking: writers (and the flusher) hold the screen store's write lock, which serializes every
//! screen and source write together with the live applies that depend on them. The data lock here
//! is held only briefly, never across file I/O.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;

use crate::core::api_map;
use crate::core::assets;
use crate::core::fsx;
use crate::core::localtime::{Clock, Now};
use crate::core::model::{
    AutoAdvance, ColumnKind, ScheduleLabels, SourceColumn, SourceDoc, SourceKind, SourceKindName,
    SourceMeta, SourceStep, SourceSummary, AUTO_ADVANCE_RANGE, SCHEDULE_COLUMNS, SOURCE_CELL_MAX,
    SOURCE_COLUMNS_MAX, SOURCE_NAME_MAX, SOURCE_ROWS_MAX,
};
use crate::core::persist::{self, PersistError, Recovered, Store};
use crate::core::polls;
use crate::core::protocol::Fields;
use crate::core::schedule;
use crate::core::templates::{self, is_id, is_key};
use crate::error::AppError;

pub const SOURCES_DIR: &str = "sources";
/// Longest column label, in characters.
pub const COLUMN_LABEL_MAX: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("{0}")]
    Invalid(String),
    #[error("Source not found: {0}")]
    NotFound(String),
    #[error(transparent)]
    Persist(#[from] PersistError),
    #[error("{}: {source}", fsx::display_path(path))]
    Io { path: PathBuf, source: io::Error },
}

impl From<SourceError> for AppError {
    fn from(e: SourceError) -> Self {
        match e {
            SourceError::Invalid(m) => AppError::Invalid(m),
            SourceError::NotFound(_) => AppError::NotFound(e.to_string()),
            SourceError::Persist(p) => p.into(),
            SourceError::Io { path, source } => AppError::Io(io::Error::new(
                source.kind(),
                format!("{}: {source}", fsx::display_path(&path)),
            )),
        }
    }
}

pub type SourceResult<T> = Result<T, SourceError>;

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> SourceError + '_ {
    move |source| SourceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn invalid(msg: impl Into<String>) -> SourceError {
    SourceError::Invalid(msg.into())
}

// ---- The source file as a versioned store ---------------------------------------------------

// Here rather than a derive so `model.rs` (mirrored in TS) stays untouched.
impl Default for SourceMeta {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: SourceKind::Text {
                columns: Vec::new(),
                rows: Vec::new(),
                index: 0,
                auto: None,
            },
            updated_at: String::new(),
        }
    }
}

impl Store for SourceMeta {
    /// Unused: source files are named after their id (see [`source_path`]).
    const FILE: &'static str = "source.json";
    const VERSION: u32 = 1;
}

#[derive(Serialize)]
struct Versioned<'a> {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    #[serde(flatten)]
    meta: &'a SourceMeta,
}

// ---- Validation ----------------------------------------------------------------------------

pub fn validate_id(id: &str) -> SourceResult<()> {
    if is_id(id) {
        Ok(())
    } else {
        Err(invalid(format!("Invalid source id: {id}")))
    }
}

/// Trimmed, 1..=`SOURCE_NAME_MAX` characters.
pub fn validate_name(name: &str) -> SourceResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > SOURCE_NAME_MAX {
        return Err(invalid(format!(
            "Source name must be 1 to {SOURCE_NAME_MAX} characters"
        )));
    }
    Ok(name.to_string())
}

/// `index` clamped to `rows` (0 when there are none).
pub fn clamp_index(index: u32, rows: usize) -> u32 {
    match rows {
        0 => 0,
        n => index.min(u32::try_from(n - 1).unwrap_or(u32::MAX)),
    }
}

/// Auto-advance interval in `AUTO_ADVANCE_RANGE`.
pub fn validate_auto(a: &AutoAdvance) -> SourceResult<()> {
    if AUTO_ADVANCE_RANGE.contains(&a.every_ms) {
        Ok(())
    } else {
        Err(invalid(format!(
            "Auto-advance interval must be between {} and {} seconds",
            AUTO_ADVANCE_RANGE.start() / 1000,
            AUTO_ADVANCE_RANGE.end() / 1000
        )))
    }
}

/// The table of a table-shaped source (text and API sources): columns, rows, index, auto.
pub struct Table<'a> {
    pub columns: &'a Vec<SourceColumn>,
    pub rows: &'a Vec<Fields>,
    pub index: u32,
    pub auto: Option<AutoAdvance>,
}

/// [`Table`], mutable.
pub struct TableMut<'a> {
    pub columns: &'a mut Vec<SourceColumn>,
    pub rows: &'a mut Vec<Fields>,
    pub index: &'a mut u32,
    pub auto: &'a mut Option<AutoAdvance>,
}

impl SourceKind {
    /// Text and API sources are tables (rows, index, auto-advance); schedules are not.
    pub fn table(&self) -> Option<Table<'_>> {
        match self {
            SourceKind::Text {
                columns,
                rows,
                index,
                auto,
            }
            | SourceKind::Api {
                columns,
                rows,
                index,
                auto,
                ..
            } => Some(Table {
                columns,
                rows,
                index: *index,
                auto: *auto,
            }),
            SourceKind::Schedule { .. } | SourceKind::Poll { .. } => None,
        }
    }

    pub fn table_mut(&mut self) -> Option<TableMut<'_>> {
        match self {
            SourceKind::Text {
                columns,
                rows,
                index,
                auto,
            }
            | SourceKind::Api {
                columns,
                rows,
                index,
                auto,
                ..
            } => Some(TableMut {
                columns,
                rows,
                index,
                auto,
            }),
            SourceKind::Schedule { .. } | SourceKind::Poll { .. } => None,
        }
    }
}

/// The source's auto-advance settings (table sources only).
pub fn auto_of(doc: &SourceDoc) -> Option<AutoAdvance> {
    doc.meta.kind.table().and_then(|t| t.auto)
}

/// Number of rows (schedule: items) and the current index (schedule: 0).
pub fn position(doc: &SourceDoc) -> (usize, u32) {
    match &doc.meta.kind {
        SourceKind::Schedule { items, .. } => (items.len(), 0),
        SourceKind::Poll { .. } => (
            polls::data_of(&doc.meta.kind).map_or(0, |d| d.options.len()),
            0,
        ),
        k => k.table().map_or((0, 0), |t| (t.rows.len(), t.index)),
    }
}

pub fn kind_name(doc: &SourceDoc) -> SourceKindName {
    match &doc.meta.kind {
        SourceKind::Text { .. } => SourceKindName::Text,
        SourceKind::Schedule { .. } => SourceKindName::Schedule,
        SourceKind::Api { .. } => SourceKindName::Api,
        SourceKind::Poll { .. } => SourceKindName::Poll,
    }
}

pub fn is_poll(doc: &SourceDoc) -> bool {
    matches!(doc.meta.kind, SourceKind::Poll { .. })
}

pub fn is_schedule(doc: &SourceDoc) -> bool {
    matches!(doc.meta.kind, SourceKind::Schedule { .. })
}

pub fn is_api(doc: &SourceDoc) -> bool {
    matches!(doc.meta.kind, SourceKind::Api { .. })
}

/// An image cell: empty, an asset URL (`/assets/<file>`) or an http(s) URL.
pub fn is_image_value(v: &str) -> bool {
    v.is_empty()
        || v.strip_prefix(assets::URL_PREFIX)
            .is_some_and(assets::is_image_name)
        || v.starts_with("https://")
        || v.starts_with("http://")
}

/// The column a slideshow row's own duration (seconds) comes from with `itemDurations`.
pub const DURATION_COLUMN: &str = "duration";

/// How long a row stays up under auto-advance: the row's `duration` cell (seconds, clamped to
/// `AUTO_ADVANCE_RANGE`) with `itemDurations`, else `everyMs`.
pub fn row_duration_ms(a: &AutoAdvance, row: Option<&Fields>) -> u32 {
    if a.item_durations {
        let own = row
            .and_then(|r| r.get(DURATION_COLUMN))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|s| s.is_finite() && *s > 0.0)
            .map(|s| (s * 1000.0).round().min(f64::from(u32::MAX)) as u32);
        if let Some(ms) = own {
            return ms.clamp(*AUTO_ADVANCE_RANGE.start(), *AUTO_ADVANCE_RANGE.end());
        }
    }
    a.every_ms
}

/// Hard checks: nothing is written when one fails. Returns the document with the name and column
/// labels trimmed and `index` clamped (schedules: texts trimmed, a dangling `nextOverride`
/// cleared).
pub fn validate(mut doc: SourceDoc) -> SourceResult<SourceDoc> {
    validate_id(&doc.id)?;
    let m = &mut doc.meta;
    m.name = validate_name(&m.name)?;
    let (columns, rows, index, auto) = match &mut m.kind {
        SourceKind::Text {
            columns,
            rows,
            index,
            auto,
        } => (columns, rows, index, auto),
        SourceKind::Schedule {
            items,
            labels,
            next_override,
            ..
        } => {
            schedule::validate(items, labels, next_override).map_err(invalid)?;
            return Ok(doc);
        }
        SourceKind::Api {
            request,
            format,
            mapping,
            poll,
            columns,
            rows,
            index,
            auto,
            ..
        } => {
            api_map::validate_settings(request, *format, mapping, poll).map_err(invalid)?;
            if let Some(a) = auto {
                validate_auto(a)?;
            }
            *columns = api_map::columns_of(mapping);
            api_map::sanitize_rows(columns, rows);
            *index = clamp_index(*index, rows.len());
            return Ok(doc);
        }
        SourceKind::Poll { link, cache, .. } => {
            polls::validate(link, cache).map_err(invalid)?;
            return Ok(doc);
        }
    };
    if let Some(a) = auto {
        validate_auto(a)?;
    }
    if columns.is_empty() || columns.len() > SOURCE_COLUMNS_MAX {
        return Err(invalid(format!(
            "A source needs 1 to {SOURCE_COLUMNS_MAX} columns"
        )));
    }
    let mut keys = HashSet::new();
    for c in columns.iter_mut() {
        if !is_key(&c.key) {
            return Err(invalid(format!(
                "Invalid column key \"{}\": use a-z, 0-9, _ and -, starting with a letter",
                c.key
            )));
        }
        if !keys.insert(c.key.clone()) {
            return Err(invalid(format!("Duplicate column key: {}", c.key)));
        }
        let label = c.label.trim();
        if label.is_empty() || label.chars().count() > COLUMN_LABEL_MAX {
            return Err(invalid(format!(
                "Column labels must be 1 to {COLUMN_LABEL_MAX} characters"
            )));
        }
        c.label = label.to_string();
    }
    if rows.len() > SOURCE_ROWS_MAX {
        return Err(invalid(format!(
            "Too many rows ({}, at most {SOURCE_ROWS_MAX})",
            rows.len()
        )));
    }
    let images: HashSet<&str> = columns
        .iter()
        .filter(|c| c.kind == ColumnKind::Image)
        .map(|c| c.key.as_str())
        .collect();
    for (i, row) in rows.iter().enumerate() {
        for (k, v) in row {
            if !keys.contains(k) {
                return Err(invalid(format!("Row {}: unknown column \"{k}\"", i + 1)));
            }
            if v.len() > SOURCE_CELL_MAX {
                return Err(invalid(format!(
                    "Row {}, column \"{k}\" is too long (at most {} KB)",
                    i + 1,
                    SOURCE_CELL_MAX / 1024
                )));
            }
            if images.contains(k.as_str()) && !is_image_value(v) {
                return Err(invalid(format!(
                    "Row {}, column \"{k}\": use a project image or an http(s) URL",
                    i + 1
                )));
            }
        }
    }
    *index = clamp_index(*index, rows.len());
    Ok(doc)
}

// ---- Reading a source ------------------------------------------------------------------------

/// The current row with every column filled in (missing cells read as empty); empty when there
/// are no rows. Schedules: the NOW item's columns at `now` (empty when nothing is on).
pub fn current_row(doc: &SourceDoc, now: &Now) -> Fields {
    match doc.meta.kind.table() {
        Some(t) => {
            let Some(row) = t.rows.get(t.index as usize) else {
                return Fields::new();
            };
            t.columns
                .iter()
                .map(|c| (c.key.clone(), row.get(&c.key).cloned().unwrap_or_default()))
                .collect()
        }
        None => Current::of(doc, now).now_row.unwrap_or_default(),
    }
}

/// What a screen plan needs from one source: its column keys, current row and (for `list` field
/// bindings) every row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub kind: SourceKindName,
    pub columns: Vec<String>,
    /// `None` when the source has no rows. Schedules: the NOW item.
    pub row: Option<Fields>,
    /// Every row, with reserved keys: text sources mark the current row `_current: "true"` and,
    /// with auto-advance, give every row `_duration` (whole seconds); schedule rows (in view
    /// order) carry `_status`.
    pub rows: Vec<Fields>,
    /// Schedules: the NOW and UP NEXT items (text sources: the current row).
    pub now_row: Option<Fields>,
    pub next_row: Option<Fields>,
}

impl Current {
    pub fn of(doc: &SourceDoc, now: &Now) -> Self {
        if let Some(Table {
            columns,
            rows,
            index,
            auto,
        }) = doc.meta.kind.table()
        {
            let row = (!rows.is_empty()).then(|| current_row(doc, now));
            let marked = rows
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let mut f = r.clone();
                    if i == index as usize {
                        f.insert("_current".into(), "true".into());
                    }
                    if let Some(a) = &auto {
                        let secs = (row_duration_ms(a, Some(r)) + 500) / 1000;
                        f.insert("_duration".into(), secs.max(1).to_string());
                    }
                    f
                })
                .collect();
            return Self {
                kind: kind_name(doc),
                columns: columns.iter().map(|c| c.key.clone()).collect(),
                now_row: row.clone(),
                next_row: row.clone(),
                row,
                rows: marked,
            };
        }
        match &doc.meta.kind {
            SourceKind::Poll { .. } => {
                let rows = polls::data_of(&doc.meta.kind)
                    .map(|d| polls::rows(&d))
                    .unwrap_or_default();
                let row = rows.first().map(|r| {
                    let mut r = r.clone();
                    r.retain(|k, _| !k.starts_with('_'));
                    r
                });
                Self {
                    kind: SourceKindName::Poll,
                    columns: polls::POLL_COLUMNS
                        .iter()
                        .map(|(k, _, _)| (*k).to_string())
                        .collect(),
                    now_row: row.clone(),
                    next_row: row.clone(),
                    row,
                    rows,
                }
            }
            // Tables are handled above.
            SourceKind::Text { .. } | SourceKind::Api { .. } => Self {
                kind: kind_name(doc),
                columns: Vec::new(),
                row: None,
                rows: Vec::new(),
                now_row: None,
                next_row: None,
            },
            SourceKind::Schedule {
                items,
                ripple,
                next_override,
                labels,
            } => {
                let c = schedule::compute(items, *ripple, next_override.as_deref(), now);
                let rows = schedule::rows(items, labels, &c, now);
                let pick = |id: &Option<String>| {
                    let id = id.as_deref()?;
                    let pos = c.items.iter().position(|e| items[e.index].id == id)?;
                    let mut r = rows[pos].clone();
                    r.remove("_status");
                    Some(r)
                };
                let now_row = pick(&c.now_id);
                Self {
                    kind: SourceKindName::Schedule,
                    columns: SCHEDULE_COLUMNS
                        .iter()
                        .map(|(k, _)| k.to_string())
                        .collect(),
                    row: now_row.clone(),
                    next_row: pick(&c.next_id),
                    now_row,
                    rows,
                }
            }
        }
    }
}

/// Source id → current row, for `screens::plan`.
pub type SourceMap = HashMap<String, Current>;

/// The columns a source exposes (schedules: `SCHEDULE_COLUMNS`).
pub fn columns_of(doc: &SourceDoc) -> Vec<SourceColumn> {
    match &doc.meta.kind {
        SourceKind::Text { columns, .. } | SourceKind::Api { columns, .. } => columns.clone(),
        SourceKind::Poll { .. } => polls::columns(),
        SourceKind::Schedule { .. } => SCHEDULE_COLUMNS
            .iter()
            .map(|(k, l)| SourceColumn {
                key: (*k).to_string(),
                label: (*l).to_string(),
                kind: ColumnKind::Text,
            })
            .collect(),
    }
}

pub fn summary(doc: &SourceDoc, used_by: Vec<String>, now: &Now) -> SourceSummary {
    let (rows, index) = position(doc);
    let (now_id, next_id) = match &doc.meta.kind {
        SourceKind::Text { .. } | SourceKind::Api { .. } | SourceKind::Poll { .. } => (None, None),
        SourceKind::Schedule {
            items,
            ripple,
            next_override,
            ..
        } => {
            let c = schedule::compute(items, *ripple, next_override.as_deref(), now);
            (c.now_id, c.next_id)
        }
    };
    SourceSummary {
        id: doc.id.clone(),
        name: doc.meta.name.clone(),
        kind: kind_name(doc),
        row_count: u32::try_from(rows).unwrap_or(u32::MAX),
        index,
        current: current_row(doc, now),
        columns: columns_of(doc),
        used_by,
        updated_at: doc.meta.updated_at.clone(),
        auto: auto_of(doc),
        // Filled in by the automation layer while armed.
        next_at: None,
        now_id,
        next_id,
        // Filled in by the API poller for API sources.
        api: None,
        format: match &doc.meta.kind {
            SourceKind::Api { format, .. } => Some(*format),
            SourceKind::Text { .. } | SourceKind::Schedule { .. } | SourceKind::Poll { .. } => None,
        },
        // Filled in by the poll service for poll sources.
        poll: None,
    }
}

/// The index `step` leads to from `index` over `rows` rows: clamped, no wrap.
pub fn stepped(index: u32, rows: usize, step: SourceStep) -> u32 {
    if rows == 0 {
        return 0;
    }
    let last = i64::try_from(rows - 1).unwrap_or(i64::MAX);
    let next = match step {
        SourceStep::By { delta } => (i64::from(index) + i64::from(delta)).clamp(0, last),
        SourceStep::Index { index } => i64::from(index).min(last),
    };
    u32::try_from(next).unwrap_or(0)
}

// ---- Disk operations -------------------------------------------------------------------------

pub fn sources_root(project: &Path) -> PathBuf {
    project.join(SOURCES_DIR)
}

/// `sources/<id>.json` for a valid id.
pub fn source_path(project: &Path, id: &str) -> SourceResult<PathBuf> {
    validate_id(id)?;
    Ok(sources_root(project).join(format!("{id}.json")))
}

fn read_meta(path: &Path, id: &str) -> SourceResult<SourceMeta> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(SourceError::NotFound(id.to_string()))
        }
        Err(e) => return Err(io_err(path)(e)),
    };
    Ok(persist::decode::<SourceMeta>(path, &bytes)?)
}

/// Reads and validates `sources/<id>.json`. Never changes anything on disk.
pub fn get(project: &Path, id: &str) -> SourceResult<SourceDoc> {
    let path = source_path(project, id)?;
    let meta = read_meta(&path, id)?;
    validate(SourceDoc {
        id: id.to_string(),
        meta,
    })
}

fn write(project: &Path, doc: &SourceDoc) -> SourceResult<()> {
    let root = sources_root(project);
    fs::create_dir_all(&root).map_err(io_err(&root))?;
    let path = source_path(project, &doc.id)?;
    let v = Versioned {
        schema_version: SourceMeta::VERSION,
        meta: &doc.meta,
    };
    fsx::write_json_atomic(&path, &v).map_err(io_err(&path))
}

/// Validates and writes `doc`, setting `updatedAt`. An unchanged document is not written (its
/// `updatedAt` is kept). A newer-schema file is never overwritten. With `create: false` the file
/// must already exist.
pub fn save(project: &Path, doc: SourceDoc, create: bool) -> SourceResult<SourceDoc> {
    let mut doc = validate(doc)?;
    let path = source_path(project, &doc.id)?;
    match read_meta(&path, &doc.id) {
        Ok(old) => {
            let mut same = old.clone();
            same.updated_at = doc.meta.updated_at.clone();
            if same == doc.meta {
                doc.meta.updated_at = old.updated_at;
                return Ok(doc);
            }
        }
        Err(SourceError::NotFound(_)) if create => {}
        Err(e @ SourceError::NotFound(_)) => return Err(e),
        // A corrupt file may be replaced by a valid document; a newer one never.
        Err(SourceError::Persist(PersistError::Corrupt { .. })) => {}
        Err(e) => return Err(e),
    }
    doc.meta.updated_at = fsx::now_rfc3339();
    write(project, &doc)?;
    Ok(doc)
}

/// Writes the in-memory `index` of a stepped source as is (`updatedAt` unchanged: a step is not
/// an edit). Skipped (`Ok(false)`) when the file is gone or now from a newer schema.
pub fn write_index(project: &Path, doc: &SourceDoc) -> SourceResult<bool> {
    let path = source_path(project, &doc.id)?;
    match read_meta(&path, &doc.id) {
        Ok(_) | Err(SourceError::Persist(PersistError::Corrupt { .. })) => {}
        Err(SourceError::NotFound(_))
        | Err(SourceError::Persist(PersistError::NewerVersion { .. })) => return Ok(false),
        Err(e) => return Err(e),
    }
    write(project, doc)?;
    Ok(true)
}

/// A fresh source id not taken in `sources/`.
fn new_source_id(project: &Path) -> String {
    let root = sources_root(project);
    loop {
        let id = templates::random_id();
        if !root.join(format!("{id}.json")).exists() {
            return id;
        }
    }
}

/// A new text source: one column (`text`, "Text"), no rows.
pub fn create(project: &Path, name: &str) -> SourceResult<SourceDoc> {
    let doc = SourceDoc {
        id: new_source_id(project),
        meta: SourceMeta {
            name: validate_name(name)?,
            kind: SourceKind::Text {
                columns: vec![SourceColumn {
                    key: "text".into(),
                    label: "Text".into(),
                    kind: ColumnKind::Text,
                }],
                rows: Vec::new(),
                index: 0,
                auto: None,
            },
            updated_at: String::new(),
        },
    };
    save(project, doc, true)
}

/// A new schedule source: no items, default labels, ripple off.
pub fn create_schedule(project: &Path, name: &str) -> SourceResult<SourceDoc> {
    create_with(
        project,
        name,
        SourceKind::Schedule {
            items: Vec::new(),
            ripple: false,
            next_override: None,
            labels: ScheduleLabels::default(),
        },
    )
}

/// A new source of any kind (validated).
pub fn create_with(project: &Path, name: &str, kind: SourceKind) -> SourceResult<SourceDoc> {
    let doc = SourceDoc {
        id: new_source_id(project),
        meta: SourceMeta {
            name: validate_name(name)?,
            kind,
            updated_at: String::new(),
        },
    };
    save(project, doc, true)
}

/// A copy of `src` under a new id.
pub fn duplicate(project: &Path, src: SourceDoc, name: &str) -> SourceResult<SourceDoc> {
    let doc = SourceDoc {
        id: new_source_id(project),
        meta: SourceMeta {
            name: validate_name(name)?,
            ..src.meta
        },
    };
    save(project, doc, true)
}

pub fn delete(project: &Path, id: &str) -> SourceResult<()> {
    let path = source_path(project, id)?;
    match fsx::remove_file_own(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(SourceError::NotFound(id.into())),
        Err(e) => Err(io_err(&path)(e)),
    }
}

/// Ids of `sources/<id>.json` files (sorted).
fn file_ids(project: &Path) -> SourceResult<Vec<String>> {
    let root = sources_root(project);
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

/// Every source for opening the project. A corrupt file (not JSON, wrong shape) is renamed to
/// `<file>.corrupt-<timestamp>` and reported in the second list; files from a newer version or
/// failing validation are logged and skipped (left alone).
pub fn load_all(project: &Path) -> (Vec<SourceDoc>, Vec<Recovered>) {
    let mut docs = Vec::new();
    let mut recovered = Vec::new();
    let ids = match file_ids(project) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("reading sources failed: {e}");
            return (docs, recovered);
        }
    };
    for id in ids {
        match get(project, &id) {
            Ok(doc) => docs.push(doc),
            Err(SourceError::Persist(e @ PersistError::Corrupt { .. })) => {
                let Ok(path) = source_path(project, &id) else {
                    continue;
                };
                tracing::warn!("{e}; setting it aside");
                match persist::quarantine(&path) {
                    Ok(r) => recovered.push(r),
                    Err(e) => tracing::warn!("could not set aside {}: {e}", path.display()),
                }
            }
            Err(e) => tracing::warn!("skipping source {id}: {e}"),
        }
    }
    (docs, recovered)
}

// ---- Store -----------------------------------------------------------------------------------

#[derive(Default)]
struct Loaded {
    project: Option<PathBuf>,
    docs: HashMap<String, SourceDoc>,
    /// Sources whose in-memory `index` is newer than their file (stepped, not yet flushed).
    dirty: BTreeSet<String>,
}

/// The open project's sources, all in memory (see the module docs). Replaced on every project
/// open, close, restore and move.
#[derive(Default)]
pub struct SourceStore {
    loaded: Mutex<Loaded>,
    /// Server time for schedules (skewable in tests).
    clock: Clock,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl SourceStore {
    /// Server time as schedules see it.
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// Ids of the schedule sources.
    pub fn schedule_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.loaded)
            .docs
            .values()
            .filter(|d| is_schedule(d))
            .map(|d| d.id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// The API sources, as the poller's reconcile pass needs them (cheap: no rows cloned).
    pub(crate) fn api_metas(&self) -> Vec<crate::core::api_poll::ApiMeta> {
        lock(&self.loaded)
            .docs
            .values()
            .filter_map(crate::core::api_poll::ApiMeta::of)
            .collect()
    }

    /// Poll sources as `(source id, linked poll id or None, test)`, for the poll service (cheap:
    /// nothing cloned but ids). A gone poll counts as unlinked.
    pub(crate) fn poll_links(&self) -> Vec<(String, Option<String>)> {
        lock(&self.loaded)
            .docs
            .values()
            .filter_map(|d| match &d.meta.kind {
                SourceKind::Poll { link, gone, .. } => Some((
                    d.id.clone(),
                    link.as_ref().filter(|_| !gone).map(|l| l.id.clone()),
                )),
                _ => None,
            })
            .collect()
    }

    /// Replaces a document in memory only and marks it dirty (the content flusher writes it).
    pub fn put_dirty(&self, doc: SourceDoc) {
        let mut l = lock(&self.loaded);
        l.dirty.insert(doc.id.clone());
        l.docs.insert(doc.id.clone(), doc);
    }

    /// Forgets everything (project closed). Flush first: dirty indexes are dropped.
    pub fn forget(&self) {
        *lock(&self.loaded) = Loaded::default();
    }

    /// Installs a freshly loaded project's sources.
    pub fn install(&self, project: &Path, docs: Vec<SourceDoc>) {
        *lock(&self.loaded) = Loaded {
            project: Some(project.to_path_buf()),
            docs: docs.into_iter().map(|d| (d.id.clone(), d)).collect(),
            dirty: BTreeSet::new(),
        };
    }

    /// The project folder moved; the documents are unchanged.
    pub fn relocate(&self, project: &Path) {
        let mut l = lock(&self.loaded);
        if l.project.is_some() {
            l.project = Some(project.to_path_buf());
        }
    }

    pub fn project(&self) -> Option<PathBuf> {
        lock(&self.loaded).project.clone()
    }

    pub fn doc(&self, id: &str) -> Option<SourceDoc> {
        lock(&self.loaded).docs.get(id).cloned()
    }

    /// Every document (for listing).
    pub fn docs(&self) -> Vec<SourceDoc> {
        lock(&self.loaded).docs.values().cloned().collect()
    }

    /// Remembers `doc` as written to disk (its file is current: not dirty).
    pub fn put(&self, doc: SourceDoc) {
        let mut l = lock(&self.loaded);
        l.dirty.remove(&doc.id);
        l.docs.insert(doc.id.clone(), doc);
    }

    pub fn remove(&self, id: &str) -> Option<SourceDoc> {
        let mut l = lock(&self.loaded);
        l.dirty.remove(id);
        l.docs.remove(id)
    }

    /// Moves a source's current row. `None` for an unknown source; `Some((summary-ready doc,
    /// changed))` otherwise. A change marks the source dirty.
    pub fn step(&self, id: &str, step: SourceStep) -> Option<(SourceDoc, bool)> {
        let mut l = lock(&self.loaded);
        let doc = l.docs.get_mut(id)?;
        let Some(t) = doc.meta.kind.table_mut() else {
            return Some((doc.clone(), false));
        };
        let next = stepped(*t.index, t.rows.len(), step);
        let changed = next != *t.index;
        *t.index = next;
        let doc = doc.clone();
        if changed {
            l.dirty.insert(id.to_string());
        }
        Some((doc, changed))
    }

    /// Sources with auto-advance: `(id, settings, steppable, rows)`, where `steppable` = it is
    /// not paused and has a row to step to (more than one row; without `loop` and `shuffle`, not
    /// at the last row yet; a shuffled cycle's end is the automation's to judge).
    pub fn auto_sources(&self) -> Vec<(String, AutoAdvance, bool, usize)> {
        let l = lock(&self.loaded);
        l.docs
            .values()
            .filter_map(|d| {
                let a = auto_of(d)?;
                let (n, i) = position(d);
                let can = !a.paused && n > 1 && (a.looped || a.shuffle || (i as usize) + 1 < n);
                Some((d.id.clone(), a, can, n))
            })
            .collect()
    }

    /// The current rows of the given sources (unknown ids are left out).
    pub fn current<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> SourceMap {
        let now = self.clock.now();
        let docs: Vec<SourceDoc> = {
            let l = lock(&self.loaded);
            let mut seen = HashSet::new();
            ids.into_iter()
                .filter(|id| seen.insert(*id))
                .filter_map(|id| l.docs.get(id).cloned())
                .collect()
        };
        docs.iter()
            .map(|d| (d.id.clone(), Current::of(d, &now)))
            .collect()
    }

    /// Takes the dirty set: the documents whose index must be written, and the project folder.
    pub fn take_dirty(&self) -> Option<(PathBuf, Vec<SourceDoc>)> {
        let mut l = lock(&self.loaded);
        if l.dirty.is_empty() {
            return None;
        }
        let project = l.project.clone()?;
        let ids = std::mem::take(&mut l.dirty);
        let docs = ids
            .iter()
            .filter_map(|id| l.docs.get(id).cloned())
            .collect();
        Some((project, docs))
    }

    /// Marks a source dirty again (its write failed).
    pub fn mark_dirty(&self, id: &str) {
        let mut l = lock(&self.loaded);
        if l.docs.contains_key(id) {
            l.dirty.insert(id.to_string());
        }
    }

    pub fn is_dirty(&self, id: &str) -> bool {
        lock(&self.loaded).dirty.contains(id)
    }
}

#[cfg(test)]
mod tests;
