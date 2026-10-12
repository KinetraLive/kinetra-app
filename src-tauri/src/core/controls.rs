//! `controls.json`: the project's MIDI mappings (they follow the project to another machine; the
//! device setup is per machine in `prefs.json` `midi`).
//!
//! The document lives in memory with the open project (`state::OpenProject`, an `Arc` so the MIDI
//! input path reads it without copying), is written coalesced and atomically by the store flusher,
//! watched (a hand edit reloads; an invalid file keeps the last good state), backed up and part
//! of the settings export.
//!
//! Every change (save, watcher reload, project open / close / reload / restore) emits
//! `controls-changed` and calls the in-core listeners ([`AppState::on_controls_changed`], e.g. the
//! MIDI input's lookup table), with no `AppState` lock held.
//!
//! Conflicts: two mappings whose sources overlap ([`MidiSource::overlaps`]) conflict. Both are
//! kept and shown; at runtime neither runs ([`conflicts`], [`matching`]).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use crate::core::control_actions::{parse_command, Pacer};
use crate::core::events::names;
use crate::core::model::{
    ControlMapping, ControlTarget, ControlsDoc, FileChange, FileKind, FilesChanged, MidiKind,
    MidiSource, ValueTarget, CONTROL_MAPPINGS_CAP,
};
use crate::core::persist::{self, PersistError, Store};
use crate::core::state::{is_midi_name, AppState};
use crate::core::templates::{is_id, is_key};
use crate::error::{AppError, AppResult};

/// The file name inside the project.
pub const CONTROLS_FILE: &str = "controls.json";

impl Store for ControlsDoc {
    const FILE: &'static str = CONTROLS_FILE;
    const VERSION: u32 = 1;
}

/// Called with the new mappings after every change (see the module docs).
pub type ControlsListener = Arc<dyn Fn(&Arc<ControlsDoc>) + Send + Sync>;

/// Runtime state behind the controls: change listeners and the pacing of value targets.
#[derive(Default)]
pub struct ControlRuntime {
    listeners: RwLock<Vec<ControlsListener>>,
    pub(crate) pacer: Pacer,
    /// Serializes saves and watcher reloads (validate → replace → announce).
    save: Mutex<()>,
}

/// Why a mapping is invalid, or `None` when it is fine.
fn mapping_problem(m: &ControlMapping) -> Option<String> {
    if !is_id(&m.id) {
        return Some(format!("Invalid mapping id: {}", m.id));
    }
    let s = &m.source;
    if let Some(d) = &s.device {
        if !is_midi_name(d) {
            return Some("A MIDI device name is too long or empty".into());
        }
    }
    if s.channel.is_some_and(|c| !(1..=16).contains(&c)) {
        return Some("MIDI channel must be 1 to 16".into());
    }
    if s.number > 127 {
        let what = match s.kind {
            MidiKind::Note => "Note",
            MidiKind::Cc => "CC",
        };
        return Some(format!("{what} number must be 0 to 127"));
    }
    match &m.target {
        ControlTarget::Command { command_id } => {
            if parse_command(command_id).is_none() {
                return Some(format!("Unknown action: {command_id}"));
            }
        }
        ControlTarget::Value {
            screen_id,
            layer_id,
            target,
        } => {
            if !is_id(screen_id) || !is_id(layer_id) {
                return Some("Invalid layer".into());
            }
            match target {
                ValueTarget::Opacity => {}
                ValueTarget::Var { key } | ValueTarget::Field { key } => {
                    if !is_key(key) {
                        return Some(format!("Invalid key: {key}"));
                    }
                }
            }
        }
    }
    None
}

/// Checks a document from the UI: ≤ CONTROL_MAPPINGS_CAP mappings, unique ids, channels 1–16 or
/// any, numbers 0–127, device names, known command ids and target shapes. Conflicts are allowed
/// (they are shown, not rejected).
pub fn validate(doc: &ControlsDoc) -> AppResult<()> {
    if doc.mappings.len() > CONTROL_MAPPINGS_CAP {
        return Err(AppError::Invalid(format!(
            "Too many MIDI mappings ({}, at most {CONTROL_MAPPINGS_CAP})",
            doc.mappings.len()
        )));
    }
    let mut ids = HashSet::new();
    for m in &doc.mappings {
        if let Some(p) = mapping_problem(m) {
            return Err(AppError::Invalid(p));
        }
        if !ids.insert(m.id.as_str()) {
            return Err(AppError::Invalid(format!("Duplicate mapping id: {}", m.id)));
        }
    }
    Ok(())
}

/// A document from disk at project open: invalid and duplicate mappings are dropped (logged) and
/// at most CONTROL_MAPPINGS_CAP kept, so one bad hand edit never loses the rest.
pub fn sanitize(doc: ControlsDoc) -> ControlsDoc {
    let mut ids = HashSet::new();
    let mut mappings = Vec::new();
    for m in doc.mappings {
        if mappings.len() == CONTROL_MAPPINGS_CAP {
            tracing::warn!(
                "{CONTROLS_FILE}: more than {CONTROL_MAPPINGS_CAP} mappings; the rest are ignored"
            );
            break;
        }
        if let Some(p) = mapping_problem(&m) {
            tracing::warn!("{CONTROLS_FILE}: mapping {} ignored: {p}", m.id);
            continue;
        }
        if !ids.insert(m.id.clone()) {
            tracing::warn!("{CONTROLS_FILE}: duplicate mapping {} ignored", m.id);
            continue;
        }
        mappings.push(m);
    }
    ControlsDoc { mappings }
}

/// Groups of conflicting mappings: each group is the mappings connected by overlapping sources
/// (two or more), with the first mapping's source and the ids in document order. Groups are in
/// the order of their first mapping. O(n²) over ≤ 512 mappings, only on changes.
pub fn conflicts(doc: &ControlsDoc) -> Vec<(MidiSource, Vec<String>)> {
    let n = doc.mappings.len();
    let mut group: Vec<usize> = (0..n).collect();
    fn root(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if doc.mappings[i].source.overlaps(&doc.mappings[j].source) {
                let (a, b) = (root(&mut group, i), root(&mut group, j));
                if a != b {
                    group[a.max(b)] = a.min(b);
                }
            }
        }
    }
    let mut out: Vec<(usize, MidiSource, Vec<String>)> = Vec::new();
    for i in 0..n {
        let r = root(&mut group, i);
        match out.iter_mut().find(|(g, _, _)| *g == r) {
            Some((_, _, ids)) => ids.push(doc.mappings[i].id.clone()),
            None => out.push((
                r,
                doc.mappings[i].source.clone(),
                vec![doc.mappings[i].id.clone()],
            )),
        }
    }
    out.into_iter()
        .filter(|(_, _, ids)| ids.len() > 1)
        .map(|(_, s, ids)| (s, ids))
        .collect()
}

/// The mappings a MIDI message triggers (`channel` 1–16). More than one = a conflict: run none.
pub fn matching<'a>(
    doc: &'a ControlsDoc,
    device: &str,
    channel: u8,
    kind: MidiKind,
    number: u8,
) -> Vec<&'a ControlMapping> {
    doc.mappings
        .iter()
        .filter(|m| m.source.matches(device, channel, kind, number))
        .collect()
}

fn read<T>(l: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

impl AppState {
    /// The open project's mappings (a copy for the UI), or `NoProject`.
    pub fn get_controls(&self) -> AppResult<ControlsDoc> {
        Ok((*self.controls_doc()?).clone())
    }

    /// Validates and stores the mappings (`controls.json`, written coalesced); emits
    /// `controls-changed` and calls the listeners when something changed. Returns the stored
    /// document.
    pub fn save_controls(&self, doc: ControlsDoc) -> AppResult<ControlsDoc> {
        validate(&doc)?;
        let _g = self
            .control_runtime()
            .save
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let doc = Arc::new(doc);
        if self.replace_controls(doc.clone(), true)? {
            self.controls_announce();
        }
        Ok((*doc).clone())
    }

    /// Adds a listener told about every change of the mappings (the MIDI input's lookup). It is
    /// called at once with the current mappings (empty without a project). It must not block.
    pub fn on_controls_changed(&self, listener: ControlsListener) {
        self.control_runtime()
            .listeners
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(listener.clone());
        let doc = self.controls_doc().unwrap_or_default();
        listener(&doc);
    }

    /// Emits `controls-changed` with the current mappings (none without a project) and calls the
    /// listeners. No lock held.
    pub(crate) fn controls_announce(&self) {
        let doc = self.controls_doc().unwrap_or_default();
        match serde_json::to_value(&*doc) {
            Ok(v) => self.events().emit(names::CONTROLS_CHANGED, v),
            Err(e) => tracing::warn!("controls-changed payload: {e}"),
        }
        let listeners = read(&self.control_runtime().listeners).clone();
        for l in listeners {
            l(&doc);
        }
    }

    /// The watcher saw `controls.json` change outside Kinetra: reload it. Invalid (unreadable,
    /// a newer schema, or a mapping that doesn't validate): the last good state is kept and the
    /// change reported as `invalid`. Removed: no mappings.
    pub(crate) fn reload_controls(&self, dir: &Path) -> Option<FilesChanged> {
        let changed = |change, message| FilesChanged {
            kind: FileKind::Controls,
            id: CONTROLS_FILE.to_string(),
            change,
            message,
        };
        let _g = self
            .control_runtime()
            .save
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (doc, change) = match persist::load_strict::<ControlsDoc>(dir) {
            Ok(doc) => match validate(&doc) {
                Ok(()) => (doc, FileChange::Changed),
                Err(e) => {
                    tracing::warn!("{CONTROLS_FILE} changed on disk but is not valid: {e}");
                    return Some(changed(FileChange::Invalid, Some(e.to_string())));
                }
            },
            Err(PersistError::Missing(_)) => (ControlsDoc::default(), FileChange::Removed),
            Err(e) => {
                tracing::warn!("{CONTROLS_FILE} changed on disk but is not valid: {e}");
                return Some(changed(FileChange::Invalid, Some(e.to_string())));
            }
        };
        match self.replace_controls(Arc::new(doc), false) {
            Ok(true) => {}
            // Same as what is in memory (or the project closed): nothing to announce.
            Ok(false) | Err(_) => return None,
        }
        drop(_g);
        self.controls_announce();
        Some(changed(change, None))
    }
}

#[cfg(test)]
mod tests;
