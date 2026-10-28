//! `panel.json`: remote control's button panel (protocol 2 §4.3, §5 `press_button`). Built in
//! the app (Simple and Advanced, also without remote control), saved with the project; the
//! control site only sees labels, colors, `confirm` and `on_air`.
//!
//! Like `controls.json` (`core::controls`): in memory with the open project, written coalesced
//! and atomically by the store flusher, watched (an invalid file keeps the last good state),
//! backed up and part of the settings export. Every change emits `panel-changed` and wakes
//! remote control.
//!
//! A step whose target is missing (a deleted layer, an edited row) is a [`PanelIssue`], never a
//! load failure; pressing the button stops at that step. A pressed button runs its steps in
//! order through the same paths as the local controls (Studio mode cues like Control); the
//! first failure stops it, steps before it stay done.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

use crate::core::control_actions::ControlInput;
use crate::core::events::names;
use crate::core::live::FieldDef;
use crate::core::model::{
    ControlTarget, FileChange, FileKind, FilesChanged, PanelAction, PanelButton, PanelDoc,
    PanelIssue, PanelRow, PanelSourceRows, PanelTakeMode, PanelTimerOp, PanelView, ScreenLayerKind,
    SourceStep, StudioOp, TimerAction, PANEL_ACTIONS_MAX, PANEL_BUTTONS_MAX, PANEL_LABEL_MAX,
};
use crate::core::persist::{self, PersistError, Store};
use crate::core::protocol::{FieldType, TakeTransition, TakeTransitionKind, VariantTransition};
use crate::core::remote::state::{cues, row_ids, row_label};
use crate::core::state::AppState;
use crate::core::templates::{is_id, is_key};
use crate::error::{AppError, AppResult};

/// The file name inside the project.
pub const PANEL_FILE: &str = "panel.json";

impl Store for PanelDoc {
    const FILE: &'static str = PANEL_FILE;
    const VERSION: u32 = 1;
}

/// Most bytes of one `setFields` step's values.
const VALUES_BYTES_MAX: usize = 64 * 1024;

/// Serializes saves and watcher reloads (validate → replace → announce).
static SAVE: Mutex<()> = Mutex::new(());

/// A row id as `row_ids` makes it (hex, optionally `-<n>`).
fn is_row_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Why a button is invalid, or `None`.
fn button_problem(b: &PanelButton) -> Option<String> {
    if !is_id(&b.id) {
        return Some(format!("Invalid button id: {}", b.id));
    }
    let n = b.label.trim().chars().count();
    if n == 0 || n > PANEL_LABEL_MAX {
        return Some(format!(
            "A button label needs 1 to {PANEL_LABEL_MAX} characters"
        ));
    }
    if b.actions.is_empty() || b.actions.len() > PANEL_ACTIONS_MAX {
        return Some(format!("A button needs 1 to {PANEL_ACTIONS_MAX} actions"));
    }
    for a in &b.actions {
        let ids_ok = match a {
            PanelAction::Take { screen, layer } | PanelAction::Clear { screen, layer } => {
                is_id(screen) && is_id(layer)
            }
            PanelAction::RunCue { flow } => is_id(flow),
            PanelAction::SetFields {
                screen,
                layer,
                values,
            } => {
                let bytes: usize = values.iter().map(|(k, v)| k.len() + v.len()).sum();
                if values.is_empty() || values.len() > 64 || bytes > VALUES_BYTES_MAX {
                    return Some("A Set fields step needs 1 to 64 values".into());
                }
                is_id(screen) && is_id(layer) && values.keys().all(|k| is_key(k))
            }
            PanelAction::SetVariant {
                screen,
                layer,
                variant,
            } => is_id(screen) && is_id(layer) && is_id(variant),
            PanelAction::Timer {
                screen,
                layer,
                field,
                ..
            } => is_id(screen) && is_id(layer) && is_key(field),
            PanelAction::SelectRow { source, row } => is_id(source) && is_row_id(row),
            PanelAction::StudioTake { screen, .. } => is_id(screen),
        };
        if !ids_ok {
            return Some("A button step names an invalid target".into());
        }
    }
    None
}

/// Validates a whole panel (saves refuse the first problem).
pub fn validate(doc: &PanelDoc) -> AppResult<()> {
    if doc.buttons.len() > PANEL_BUTTONS_MAX {
        return Err(AppError::Invalid(format!(
            "A button panel holds at most {PANEL_BUTTONS_MAX} buttons"
        )));
    }
    let mut seen = HashSet::new();
    for b in &doc.buttons {
        if let Some(p) = button_problem(b) {
            return Err(AppError::Invalid(p));
        }
        if !seen.insert(b.id.as_str()) {
            return Err(AppError::Invalid(format!(
                "Two buttons have the id {}",
                b.id
            )));
        }
    }
    Ok(())
}

/// A loaded file: invalid or duplicate buttons are dropped (logged), at most 48 kept, labels
/// trimmed.
pub fn sanitize(mut doc: PanelDoc) -> PanelDoc {
    let mut seen = HashSet::new();
    doc.buttons.retain(|b| match button_problem(b) {
        Some(p) => {
            tracing::warn!("{PANEL_FILE}: button dropped: {p}");
            false
        }
        None => seen.insert(b.id.clone()),
    });
    doc.buttons.truncate(PANEL_BUTTONS_MAX);
    for b in &mut doc.buttons {
        b.label = b.label.trim().to_string();
    }
    doc
}

/// What the editor's pickers offer (`panel_targets`): screens with their layers (top first),
/// the cues and the table sources with their rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelTargets {
    pub screens: Vec<PanelScreen>,
    pub cues: Vec<PanelRow>,
    pub sources: Vec<PanelSourceRows>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelScreen {
    pub id: String,
    pub name: String,
    pub layers: Vec<PanelLayer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelLayer {
    pub id: String,
    pub name: String,
    /// A video / image / audio layer: only take and clear (on the show machine).
    pub media: bool,
    pub fields: Vec<FieldDef>,
    /// Field keys fed by a data source (not settable by a button).
    pub bound: Vec<String>,
    /// The template's looks (`id`, `label` = the name).
    pub variants: Vec<PanelRow>,
}

/// A step that failed: its 1-based number, the button's step count and the protocol's code
/// and words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PressFailed {
    pub step: u32,
    pub steps: u32,
    pub code: &'static str,
    pub message: String,
}

fn missing(what: &str) -> (&'static str, String) {
    ("not_found", format!("{what} no longer exists."))
}

impl AppState {
    /// The open project's panel and its missing targets.
    pub fn get_panel(&self) -> AppResult<PanelView> {
        let doc = (*self.panel_doc()?).clone();
        let issues = self.panel_issues(&doc);
        Ok(PanelView { doc, issues })
    }

    /// Validates and stores the panel (`panel.json`, written coalesced); emits
    /// `panel-changed` and wakes remote control when something changed.
    pub fn save_panel(&self, doc: PanelDoc) -> AppResult<PanelView> {
        validate(&doc)?;
        let doc = sanitize(doc);
        let _g = SAVE.lock().unwrap_or_else(PoisonError::into_inner);
        if self.replace_panel(Arc::new(doc), true)? {
            drop(_g);
            self.panel_announce();
        }
        self.get_panel()
    }

    /// Emits `panel-changed` (an empty panel without a project) and wakes remote control.
    pub(crate) fn panel_announce(&self) {
        let view = self.get_panel().unwrap_or_else(|_| PanelView {
            doc: PanelDoc::default(),
            issues: Vec::new(),
        });
        match serde_json::to_value(&view) {
            Ok(v) => self.events().emit(names::PANEL_CHANGED, v),
            Err(e) => tracing::warn!("panel-changed payload: {e}"),
        }
        self.remote_touch();
    }

    /// The watcher saw `panel.json` change outside Kinetra: reload it. Invalid: the last good
    /// state is kept and the change reported as `invalid`. Removed: no buttons.
    pub(crate) fn reload_panel(&self, dir: &Path) -> Option<FilesChanged> {
        let changed = |change, message| FilesChanged {
            kind: FileKind::Panel,
            id: PANEL_FILE.to_string(),
            change,
            message,
        };
        let g = SAVE.lock().unwrap_or_else(PoisonError::into_inner);
        let (doc, change) = match persist::load_strict::<PanelDoc>(dir) {
            Ok(doc) => match validate(&doc) {
                Ok(()) => (sanitize(doc), FileChange::Changed),
                Err(e) => {
                    tracing::warn!("{PANEL_FILE} changed on disk but is not valid: {e}");
                    return Some(changed(FileChange::Invalid, Some(e.to_string())));
                }
            },
            Err(PersistError::Missing(_)) => (PanelDoc::default(), FileChange::Removed),
            Err(e) => {
                tracing::warn!("{PANEL_FILE} changed on disk but is not valid: {e}");
                return Some(changed(FileChange::Invalid, Some(e.to_string())));
            }
        };
        match self.replace_panel(Arc::new(doc), false) {
            Ok(true) => {}
            Ok(false) | Err(_) => return None,
        }
        drop(g);
        self.panel_announce();
        Some(changed(change, None))
    }

    /// Everything the editor's pickers offer.
    pub fn panel_targets(&self) -> AppResult<PanelTargets> {
        let dir = self.project_dir()?;
        let mut screens = Vec::new();
        for view in self.live().user_screen_views() {
            let Some(doc) = self.screens().doc(&view.screen.id) else {
                continue;
            };
            let loaded = self.screens().templates_for(&dir, &doc);
            let layers = view
                .layers
                .iter()
                .rev()
                .filter_map(|lv| {
                    let dl = doc.meta.layers.iter().find(|l| l.id == lv.id)?;
                    let (media, bound, variants) = match &dl.kind {
                        ScreenLayerKind::Media { .. } => (true, Vec::new(), Vec::new()),
                        ScreenLayerKind::Template {
                            template_id,
                            bindings,
                            ..
                        } => (
                            false,
                            bindings.keys().cloned().collect(),
                            loaded
                                .get(template_id.as_str())
                                .map(|t| {
                                    t.variants
                                        .iter()
                                        .map(|v| PanelRow {
                                            id: v.id.clone(),
                                            label: v.name.clone(),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        ),
                    };
                    Some(PanelLayer {
                        id: lv.id.clone(),
                        name: dl.name.clone(),
                        media,
                        fields: if media {
                            Vec::new()
                        } else {
                            lv.field_defs.clone()
                        },
                        bound,
                        variants,
                    })
                })
                .collect();
            screens.push(PanelScreen {
                id: view.screen.id.clone(),
                name: view.screen.name.clone(),
                layers,
            });
        }
        let cues = cues(self)
            .into_iter()
            .map(|c| PanelRow {
                id: c.id,
                label: c.name,
            })
            .collect();
        Ok(PanelTargets {
            screens,
            cues,
            sources: self.panel_rows()?,
        })
    }

    /// The table sources (text and API) with their rows, for the editor's row picker.
    pub fn panel_rows(&self) -> AppResult<Vec<PanelSourceRows>> {
        self.project_dir()?;
        let mut out: Vec<PanelSourceRows> = self
            .sources()
            .docs()
            .into_iter()
            .filter_map(|d| {
                let t = d.meta.kind.table()?;
                let ids = row_ids(t.columns, t.rows);
                let rows = ids
                    .into_iter()
                    .zip(t.rows)
                    .enumerate()
                    .map(|(i, (id, row))| PanelRow {
                        id,
                        label: row_label(t.columns, row, i),
                    })
                    .collect();
                Some(PanelSourceRows {
                    id: d.id.clone(),
                    name: d.meta.name.clone(),
                    rows,
                })
            })
            .collect();
        out.sort_by_key(|s| s.name.to_lowercase());
        Ok(out)
    }

    /// Every step whose target is missing now.
    fn panel_issues(&self, doc: &PanelDoc) -> Vec<PanelIssue> {
        let mut out = Vec::new();
        for b in &doc.buttons {
            for (i, a) in b.actions.iter().enumerate() {
                if let Err((_, message)) = self.panel_target(a) {
                    out.push(PanelIssue {
                        button: b.id.clone(),
                        step: u32::try_from(i + 1).unwrap_or(u32::MAX),
                        message,
                    });
                }
            }
        }
        out
    }

    /// A step's target exists and can be used (the press-time checks short of running it).
    fn panel_target(&self, a: &PanelAction) -> Result<(), (&'static str, String)> {
        let layer =
            |screen: &str, layer: &str| -> Result<ScreenLayerKind, (&'static str, String)> {
                let doc = self
                    .screens()
                    .doc(screen)
                    .ok_or_else(|| missing("A screen of this button"))?;
                let l = doc
                    .meta
                    .layers
                    .iter()
                    .find(|l| l.id == layer)
                    .ok_or_else(|| missing("A layer of this button"))?;
                if matches!(l.kind, ScreenLayerKind::Media { .. }) {
                    return Err((
                        "read_only",
                        "Videos and images can only be taken on the show machine.".into(),
                    ));
                }
                Ok(l.kind.clone())
            };
        match a {
            PanelAction::Take { screen, layer: l } | PanelAction::Clear { screen, layer: l } => {
                layer(screen, l).map(drop)
            }
            PanelAction::RunCue { flow } => {
                if cues(self).iter().any(|c| c.id == *flow) {
                    Ok(())
                } else {
                    Err(missing("A cue of this button"))
                }
            }
            PanelAction::SetFields {
                screen,
                layer: l,
                values,
            } => {
                let kind = layer(screen, l)?;
                let ScreenLayerKind::Template { bindings, .. } = kind else {
                    return Ok(());
                };
                let defs = self
                    .live()
                    .layer_view(screen, l)
                    .map(|v| v.field_defs)
                    .unwrap_or_default();
                for k in values.keys() {
                    let Some(d) = defs.iter().find(|d| d.key == *k) else {
                        return Err(("invalid", format!("This graphic has no field \"{k}\".")));
                    };
                    if bindings.contains_key(k) {
                        return Err((
                            "bound",
                            "This field comes from a data source on the show machine.".into(),
                        ));
                    }
                    if matches!(d.kind, FieldType::Clock | FieldType::Timer) {
                        return Err(("read_only", "This field can't be set by a button.".into()));
                    }
                }
                Ok(())
            }
            PanelAction::SetVariant {
                screen,
                layer: l,
                variant,
            } => {
                let ScreenLayerKind::Template { template_id, .. } = layer(screen, l)? else {
                    return Err(missing("A look of this button"));
                };
                let has = match (self.project_dir().ok(), self.screens().doc(screen)) {
                    (Some(dir), Some(doc)) => self
                        .screens()
                        .templates_for(&dir, &doc)
                        .get(template_id.as_str())
                        .is_some_and(|t| t.variants.iter().any(|v| v.id == *variant)),
                    _ => false,
                };
                if has {
                    Ok(())
                } else {
                    Err(missing("A look of this button"))
                }
            }
            PanelAction::Timer {
                screen,
                layer: l,
                field,
                ..
            } => {
                layer(screen, l)?;
                let is_timer = self.live().layer_view(screen, l).ok().is_some_and(|v| {
                    v.field_defs
                        .iter()
                        .any(|d| d.key == *field && d.kind == FieldType::Timer)
                });
                if is_timer {
                    Ok(())
                } else {
                    Err(missing("A timer of this button"))
                }
            }
            PanelAction::SelectRow { source, row } => {
                let doc = self
                    .sources()
                    .doc(source)
                    .ok_or_else(|| missing("A data source of this button"))?;
                let t = doc
                    .meta
                    .kind
                    .table()
                    .ok_or_else(|| missing("A data source of this button"))?;
                if row_ids(t.columns, t.rows).iter().any(|r| r == row) {
                    Ok(())
                } else {
                    Err((
                        "not_found",
                        "A row of this button changed or was deleted.".into(),
                    ))
                }
            }
            PanelAction::StudioTake { screen, .. } => {
                if self.screens().doc(screen).is_some() {
                    Ok(())
                } else {
                    Err(missing("A screen of this button"))
                }
            }
        }
    }

    /// Whether pressing `b` can change what's on air (protocol 2 §4.3 `on_air`): a take,
    /// clear, cue or Studio take always; a field, look or timer change of a layer on air; a
    /// row of a source that feeds a layer on air. Unsure (a missing target): true.
    pub fn panel_button_on_air(&self, b: &PanelButton) -> bool {
        b.actions.iter().any(|a| match a {
            PanelAction::Take { .. }
            | PanelAction::Clear { .. }
            | PanelAction::RunCue { .. }
            | PanelAction::StudioTake { .. } => true,
            PanelAction::SetFields { screen, layer, .. }
            | PanelAction::SetVariant { screen, layer, .. }
            | PanelAction::Timer { screen, layer, .. } => {
                self.live().layer_view(screen, layer).is_err() || self.live().visible(screen, layer)
            }
            PanelAction::SelectRow { source, .. } => {
                self.screens().source_users(source).iter().any(|d| {
                    crate::core::screens::bound_layers(d, source)
                        .iter()
                        .any(|(l, _)| self.live().visible(&d.id, l))
                })
            }
        })
    }

    /// Runs a button's steps in order through the local paths; stops at the first failure.
    pub fn press_panel_button(&self, id: &str) -> Result<(), PressFailed> {
        let doc = self.panel_doc().map_err(|_| PressFailed {
            step: 0,
            steps: 0,
            code: "not_found",
            message: "That button doesn't exist any more.".into(),
        })?;
        let Some(b) = doc.buttons.iter().find(|b| b.id == id) else {
            return Err(PressFailed {
                step: 0,
                steps: 0,
                code: "not_found",
                message: "That button doesn't exist any more.".into(),
            });
        };
        let steps = u32::try_from(b.actions.len()).unwrap_or(u32::MAX);
        for (i, a) in b.actions.iter().enumerate() {
            let step = u32::try_from(i + 1).unwrap_or(u32::MAX);
            let fail = |(code, message): (&'static str, String)| PressFailed {
                step,
                steps,
                code,
                message,
            };
            self.panel_target(a).map_err(fail)?;
            self.panel_run(a).map_err(|e| fail(error_code(&e)))?;
        }
        Ok(())
    }

    fn panel_run(&self, a: &PanelAction) -> AppResult<()> {
        let press = |id: String| {
            self.run_control(
                &ControlTarget::Command { command_id: id },
                ControlInput::Press,
            )
        };
        match a {
            PanelAction::Take { screen, layer } => press(format!("live.take:{screen}/{layer}")),
            PanelAction::Clear { screen, layer } => press(format!("live.clear:{screen}/{layer}")),
            PanelAction::RunCue { flow } => press(format!("cue.go:{flow}")),
            PanelAction::SetFields {
                screen,
                layer,
                values,
            } => self
                .set_layer_fields(screen, layer, values.clone())
                .map(drop),
            PanelAction::SetVariant {
                screen,
                layer,
                variant,
            } => self
                .set_layer_variant(screen, layer, variant, VariantTransition::default())
                .map(drop),
            PanelAction::Timer {
                screen,
                layer,
                field,
                op,
            } => {
                let action = match op {
                    PanelTimerOp::Start => TimerAction::Start,
                    PanelTimerOp::Pause => TimerAction::Pause,
                    PanelTimerOp::Reset => TimerAction::Reset,
                };
                self.set_timer(screen, layer, field, action).map(drop)
            }
            PanelAction::SelectRow { source, row } => {
                let index = self
                    .sources()
                    .doc(source)
                    .and_then(|d| {
                        let t = d.meta.kind.table()?;
                        row_ids(t.columns, t.rows).iter().position(|r| r == row)
                    })
                    .and_then(|i| u32::try_from(i).ok())
                    .ok_or_else(|| AppError::NotFound(format!("Row not found: {row}")))?;
                self.step_source(source, SourceStep::Index { index })
                    .map(drop)
            }
            PanelAction::StudioTake { screen, mode } => {
                if !self.live().is_studio() {
                    return Err(AppError::Busy(
                        "Studio mode is off on the show machine.".into(),
                    ));
                }
                let transition = (*mode == PanelTakeMode::Cut).then(|| TakeTransition {
                    kind: TakeTransitionKind::Cut,
                    ..TakeTransition::default()
                });
                self.apply_studio_op(screen, StudioOp::Take { transition })
                    .map(drop)
            }
        }
    }
}

/// A core error in the protocol's codes (operator-facing words only).
fn error_code(e: &AppError) -> (&'static str, String) {
    match e {
        AppError::NotFound(_) => (
            "not_found",
            "Something this button uses no longer exists.".into(),
        ),
        AppError::Invalid(m) => ("invalid", m.clone()),
        AppError::Busy(m) => ("busy", m.clone()),
        AppError::NoProject => ("disabled", "No show is open on the show machine.".into()),
        _ => (
            "internal",
            "Something went wrong on the show machine.".into(),
        ),
    }
}

#[cfg(test)]
mod tests;
