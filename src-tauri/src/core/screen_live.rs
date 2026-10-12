//! Project screens and the live state: the operations behind the screen and Control commands,
//! template and source changes and the file watcher. Every path that changes a screen ends in
//! [`apply_doc`]: plan the document against its templates and sources (`screens::plan`) and hand
//! the plan to `LiveHub::apply_screen`, which applies the live-edit rules.
//!
//! Locking: each operation takes the screen store's write lock (it serializes writers of screens
//! and sources, never readers), then checks that the project it read is still the open one. Data
//! locks are held only briefly. The hub's lock is taken last, inside `LiveHub`.
//!
//! # Persisted Control edits
//! `set_layer_fields` patches the remembered document in memory, applies it and marks the screen
//! dirty; the content flusher (`state`) writes it at most once per `CONTENT_FLUSH_DELAY`, and every
//! explicit flush (project close or switch, snapshot, exit) writes it too ([`flush_dirty`]). Until
//! then `get_screen` returns the in-memory document; a save replaces it.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use crate::core::events::{names, EventSink};
use crate::core::fieldvals;
use crate::core::live::{
    is_reserved_screen, LiveError, LiveLayerView, LiveScreenView, LiveState, VariantSwitch,
};
use crate::core::live_hub::LiveHub;
use crate::core::localtime::Now;
use crate::core::model::{
    BrandKit, FileChange, FileKind, FilesChanged, LiveEdit, NewScreen, Rect, ScreenDoc,
    ScreenLayerKind, ScreenState, ScreenSummary, ScreenWarning, SourceDoc, TemplateDoc,
    VariantTransition,
};
use crate::core::persist::PersistError;
use crate::core::protocol::Fields;
use crate::core::screens::{self, LoadedTemplate, ScreenError, ScreenStore, TemplateMap};
use crate::core::sources::{self, Current, SourceMap, SourceStore};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

/// Plans `doc` against its templates (loading the ones not in memory) and sources, remembers it
/// as the screen's last good document and applies it to the live state. Caller holds the screen
/// write lock.
pub(crate) fn apply_doc(
    store: &ScreenStore,
    sources: &SourceStore,
    hub: &LiveHub,
    dir: &Path,
    doc: &ScreenDoc,
    mode: LiveEdit,
) -> Result<(LiveScreenView, Vec<ScreenWarning>), LiveError> {
    apply_doc_forcing(store, sources, hub, dir, doc, mode, &[])
}

/// [`apply_doc`], then the planned values of `force` (layer id → field keys) go on air at once
/// even under `onTake` (`LiveHub::force_fields`; automation: timers, auto-advance). Caller holds
/// the screen write lock.
pub(crate) fn apply_doc_forcing(
    store: &ScreenStore,
    sources: &SourceStore,
    hub: &LiveHub,
    dir: &Path,
    doc: &ScreenDoc,
    mode: LiveEdit,
    force: &[(String, Vec<String>)],
) -> Result<(LiveScreenView, Vec<ScreenWarning>), LiveError> {
    apply_doc_with(store, sources, hub, dir, doc, mode, force, &[])
}

/// [`apply_doc_forcing`] with variant switches for some layers (`LiveState::apply_screen_with`).
/// Caller holds the screen write lock.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_doc_with(
    store: &ScreenStore,
    sources: &SourceStore,
    hub: &LiveHub,
    dir: &Path,
    doc: &ScreenDoc,
    mode: LiveEdit,
    force: &[(String, Vec<String>)],
    switches: &[VariantSwitch],
) -> Result<(LiveScreenView, Vec<ScreenWarning>), LiveError> {
    let templates = store.templates_for(dir, doc);
    let current = sources.current(screens::source_ids(doc));
    let media = screens::media_for(dir, doc);
    let (plan, warnings) = screens::plan_with(doc, &templates, &current, &store.brand(), &media);
    let forced: Vec<(&str, Fields)> = force
        .iter()
        .filter_map(|(layer_id, keys)| {
            let p = plan.layers.iter().find(|p| p.id == *layer_id)?;
            let f: Fields = keys
                .iter()
                .filter_map(|k| p.content.fields.get(k).map(|v| (k.clone(), v.clone())))
                .collect();
            (!f.is_empty()).then_some((layer_id.as_str(), f))
        })
        .collect();
    store.set_doc(doc.clone());
    let mut view = hub.apply_screen_with(plan, mode, switches)?;
    if !forced.is_empty() {
        for (layer_id, f) in &forced {
            hub.force_fields(&doc.id, layer_id, f)?;
        }
        view = hub.screen_view(&doc.id)?;
    }
    Ok((view, warnings))
}

fn source_map(docs: &[SourceDoc], now: &Now) -> SourceMap {
    docs.iter()
        .map(|d| (d.id.clone(), Current::of(d, now)))
        .collect()
}

/// What a project being opened starts from.
pub struct ProjectLive {
    pub live: LiveState,
    pub docs: Vec<ScreenDoc>,
    pub templates: TemplateMap,
    pub sources: Vec<SourceDoc>,
}

/// The live state of a project being opened: the reserved screens plus every screen in
/// `screens/`, all off air, with bound fields resolved from `sources/`. Corrupt screen and source
/// files are set aside (`store-recovered`). Touches the disk.
pub fn build_project_live(
    dir: &Path,
    events: &dyn EventSink,
    now: &Now,
    brand: &BrandKit,
) -> ProjectLive {
    let (sources, recovered) = sources::load_all(dir);
    for r in recovered {
        events.emit(names::STORE_RECOVERED, r.payload());
    }
    let (mut docs, recovered) = screens::load_all(dir);
    for r in recovered {
        events.emit(names::STORE_RECOVERED, r.payload());
    }
    let templates = screens::load_templates_for(dir, &docs);
    screens::rewrite_retired_fonts(dir, &mut docs, &templates);
    let current = source_map(&sources, now);
    let mut live = LiveState::for_project();
    for doc in &docs {
        let media = screens::media_for(dir, doc);
        let (plan, _) = screens::plan_with(doc, &templates, &current, brand, &media);
        live.apply_screen(plan, LiveEdit::OnTake);
    }
    ProjectLive {
        live,
        docs,
        templates,
        sources,
    }
}

/// Re-reads every source and screen after the project's files were replaced (backup restore),
/// applying them by the live rules: on-air layers stay up, removed screens go away. Unsaved
/// in-memory changes are dropped. Touches the disk.
pub fn resync(
    store: &ScreenStore,
    sources: &SourceStore,
    hub: &LiveHub,
    dir: &Path,
    events: &dyn EventSink,
    mode: LiveEdit,
    brand: &BrandKit,
) {
    let _w = store.write_lock();
    let (source_docs, recovered) = sources::load_all(dir);
    for r in recovered {
        events.emit(names::STORE_RECOVERED, r.payload());
    }
    let current = source_map(&source_docs, &sources.clock().now());
    sources.install(dir, source_docs);
    let (mut docs, recovered) = screens::load_all(dir);
    for r in recovered {
        events.emit(names::STORE_RECOVERED, r.payload());
    }
    let templates = screens::load_templates_for(dir, &docs);
    screens::rewrite_retired_fonts(dir, &mut docs, &templates);
    let keep: HashSet<&str> = docs.iter().map(|d| d.id.as_str()).collect();
    for id in hub.user_screen_ids() {
        if !keep.contains(id.as_str()) {
            hub.remove_screen(&id);
        }
    }
    for doc in &docs {
        let media = screens::media_for(dir, doc);
        let (plan, _) = screens::plan_with(doc, &templates, &current, brand, &media);
        if let Err(e) = hub.apply_screen(plan, mode) {
            tracing::warn!("applying screen {}: {e}", doc.id);
        }
    }
    store.install(dir, docs, templates, brand.clone());
}

fn background_error(events: &dyn EventSink, message: String) {
    tracing::error!("{message}");
    events.emit(
        names::BACKGROUND_ERROR,
        serde_json::json!({ "message": message }),
    );
}

/// Writes every screen and source changed in memory only (Control field edits, source steps).
/// Returns the number of files written. A file that is gone or now from a newer Kinetra is left
/// alone (the change stays in memory only); other failures are reported (`background-error`) and
/// retried on the next flush.
///
/// The disk writes happen WITHOUT the screen write lock (only the snapshot and the commit take
/// it), so Control edits and live applies never wait for a file sync. If the document changed in
/// memory while it was being written (a newer edit, or an explicit save racing this write), it
/// is marked dirty again and the next pass writes the newer version.
pub(crate) fn flush_dirty(
    store: &ScreenStore,
    sources: &SourceStore,
    events: &dyn EventSink,
) -> u64 {
    let (screen_batch, source_batch) = {
        let _w = store.write_lock();
        (store.take_dirty(), sources.take_dirty())
    };
    let mut written = 0;
    if let Some((dir, docs)) = screen_batch {
        for doc in docs {
            let id = doc.id.clone();
            let result = screens::save_reporting(&dir, doc.clone(), false);
            let _w = store.write_lock();
            let unchanged = store.doc(&id).as_ref() == Some(&doc);
            match result {
                Ok((saved, wrote)) => {
                    written += u64::from(wrote);
                    if unchanged {
                        store.set_doc(saved);
                    } else {
                        store.mark_dirty(&id);
                    }
                    store.invalidate();
                }
                Err(ScreenError::NotFound(_)) => {}
                Err(e @ ScreenError::Persist(PersistError::NewerVersion { .. })) => {
                    background_error(
                        events,
                        format!(
                            "Could not save the screen \u{201c}{}\u{201d}: {e}",
                            doc.meta.name
                        ),
                    );
                }
                Err(e) => {
                    store.mark_dirty(&id);
                    background_error(
                        events,
                        format!(
                            "Could not save the screen \u{201c}{}\u{201d}: {e}",
                            doc.meta.name
                        ),
                    );
                }
            }
        }
    }
    if let Some((dir, docs)) = source_batch {
        for doc in docs {
            let result = sources::write_index(&dir, &doc);
            let _w = store.write_lock();
            match result {
                Ok(wrote) => {
                    written += u64::from(wrote);
                    if sources.doc(&doc.id).as_ref() != Some(&doc) {
                        sources.mark_dirty(&doc.id);
                    }
                }
                Err(e) => {
                    sources.mark_dirty(&doc.id);
                    background_error(
                        events,
                        format!(
                            "Could not save the source \u{201c}{}\u{201d}: {e}",
                            doc.meta.name
                        ),
                    );
                }
            }
        }
    }
    written
}

pub(crate) fn emit_files_changed(events: &dyn EventSink, change: &FilesChanged) {
    match serde_json::to_value(change) {
        Ok(v) => events.emit(names::PROJECT_FILES_CHANGED, v),
        Err(e) => tracing::warn!("project-files-changed payload: {e}"),
    }
}

impl AppState {
    /// `liveEdit` of the open project (`onTake` when none is open).
    pub fn live_edit(&self) -> LiveEdit {
        self.ui_state()
            .map(|u| u.control.live_edit)
            .unwrap_or_default()
    }

    /// Takes the screen write lock for the open project; `NoProject` if there is none or it
    /// changed meanwhile.
    pub(crate) fn screen_guard(
        &self,
    ) -> AppResult<(std::path::PathBuf, std::sync::MutexGuard<'_, ()>)> {
        let dir = self.project_dir()?;
        let guard = self.screens().write_lock();
        if !self.screens().is_project(&dir) {
            return Err(AppError::NoProject);
        }
        Ok((dir, guard))
    }

    /// Applies `doc` by the live rules. Caller holds the screen write lock.
    pub(crate) fn apply_screen_doc(
        &self,
        dir: &Path,
        doc: &ScreenDoc,
    ) -> Result<(LiveScreenView, Vec<ScreenWarning>), LiveError> {
        apply_doc(
            self.screens(),
            self.sources(),
            self.live(),
            dir,
            doc,
            self.live_edit(),
        )
    }

    /// [`apply_doc_forcing`] with the project's live-edit mode. Caller holds the screen write lock.
    pub(crate) fn apply_screen_doc_forcing(
        &self,
        dir: &Path,
        doc: &ScreenDoc,
        force: &[(String, Vec<String>)],
    ) -> Result<(LiveScreenView, Vec<ScreenWarning>), LiveError> {
        apply_doc_forcing(
            self.screens(),
            self.sources(),
            self.live(),
            dir,
            doc,
            self.live_edit(),
            force,
        )
    }

    fn apply(&self, dir: &Path, doc: ScreenDoc) -> AppResult<ScreenState> {
        let (live, warnings) = self.apply_screen_doc(dir, &doc)?;
        Ok(ScreenState {
            doc,
            live,
            warnings,
        })
    }

    /// The screen as the user last changed it: the in-memory document when it has unsaved
    /// Control edits, else the file. Caller holds the screen write lock.
    fn current_doc(&self, dir: &Path, id: &str) -> AppResult<ScreenDoc> {
        if self.screens().is_dirty(id) {
            if let Some(doc) = self.screens().doc(id) {
                return Ok(doc);
            }
        }
        Ok(screens::get(dir, id)?)
    }

    /// Summaries of the open project's screens, newest first (cached).
    pub fn list_screen_docs(&self) -> AppResult<Vec<ScreenSummary>> {
        let dir = self.project_dir()?;
        Ok(self.screens().list(&dir)?)
    }

    /// The screen as saved (or with its unsaved Control edits), its live state and soft warnings.
    /// Touches the disk.
    pub fn get_screen(&self, id: &str) -> AppResult<ScreenState> {
        let (dir, _w) = self.screen_guard()?;
        let doc = self.current_doc(&dir, id)?;
        let templates = self.screens().templates_for(&dir, &doc);
        let current = self.sources().current(screens::source_ids(&doc));
        let media = screens::media_for(&dir, &doc);
        let (plan, warnings) =
            screens::plan_with(&doc, &templates, &current, &self.screens().brand(), &media);
        let live = match self.live().screen_view(id) {
            Ok(v) => v,
            // Valid on disk but not live (e.g. fixed while the watcher was not running).
            Err(_) => {
                self.screens().set_doc(doc.clone());
                self.live().apply_screen(plan, self.live_edit())?
            }
        };
        Ok(ScreenState {
            doc,
            live,
            warnings,
        })
    }

    /// Creates `screens/<new id>.json` with no layers and adds it to the live state.
    pub fn create_screen(&self, new: &NewScreen) -> AppResult<ScreenState> {
        let (dir, _w) = self.screen_guard()?;
        let doc = screens::create(&dir, new);
        self.screens().invalidate();
        self.apply(&dir, doc?)
    }

    /// Validates, writes (if changed) and applies by the live rules. The screen must exist.
    /// Replaces any unsaved Control edits.
    pub fn save_screen(&self, doc: ScreenDoc) -> AppResult<ScreenState> {
        self.write_screen(doc, false)
    }

    /// Like [`AppState::save_screen`], but creates the file when the id is new (fixture, import).
    pub fn put_screen(&self, doc: ScreenDoc) -> AppResult<ScreenState> {
        self.write_screen(doc, true)
    }

    fn write_screen(&self, mut doc: ScreenDoc, create: bool) -> AppResult<ScreenState> {
        let (dir, _w) = self.screen_guard()?;
        self.start_new_layers_on_style(&dir, &mut doc);
        let saved = screens::save(&dir, doc, create);
        self.screens().invalidate();
        let saved = saved?;
        self.screens().clear_dirty(&saved.id);
        let binds_schedule = self.binds_schedule(&saved);
        let state = self.apply(&dir, saved)?;
        if binds_schedule {
            // A newly bound schedule is watched by the planner from now on.
            self.replan();
        }
        Ok(state)
    }

    /// Whether a layer of `doc` is bound to a schedule source.
    fn binds_schedule(&self, doc: &ScreenDoc) -> bool {
        screens::source_ids(doc).any(|id| {
            self.sources()
                .doc(id)
                .is_some_and(|d| sources::is_schedule(&d))
        })
    }

    pub fn rename_screen(&self, id: &str, name: &str) -> AppResult<ScreenSummary> {
        let (dir, _w) = self.screen_guard()?;
        let mut doc = self.current_doc(&dir, id)?;
        doc.meta.name = screens::validate_name(name)?;
        let saved = screens::save(&dir, doc, false);
        self.screens().invalidate();
        let saved = saved?;
        self.screens().clear_dirty(id);
        let state = self.apply(&dir, saved)?;
        Ok(screens::summary(&state.doc))
    }

    /// Copies a screen (with its unsaved Control edits) with new screen and layer ids; every layer
    /// of the copy starts off air.
    pub fn duplicate_screen(&self, id: &str, name: &str) -> AppResult<ScreenState> {
        let (dir, _w) = self.screen_guard()?;
        let src = self.current_doc(&dir, id)?;
        let doc = screens::duplicate_doc(&dir, src, name);
        self.screens().invalidate();
        self.apply(&dir, doc?)
    }

    /// Deletes the file and drops the screen from the live state (its renderers get
    /// `unknownScreen`).
    pub fn delete_screen(&self, id: &str) -> AppResult<()> {
        let (dir, _w) = self.screen_guard()?;
        let result = screens::delete(&dir, id);
        self.screens().invalidate();
        if matches!(result, Ok(()) | Err(ScreenError::NotFound(_))) {
            let had_doc = self.screens().remove_doc(id).is_some();
            let had_live = self.live().remove_screen(id);
            if result.is_err() && (had_doc || had_live) {
                return Ok(());
            }
        }
        Ok(result?)
    }

    /// Moves or resizes a layer's box in memory and broadcasts it (no disk write), by the live
    /// rules. In memory only, so it is safe on the UI thread. An anchored layer: a move releases
    /// the moved axis, a resize stays anchored (the box is placed by its anchor; `anchors`).
    pub fn set_layer_rect(
        &self,
        screen_id: &str,
        layer_id: &str,
        rect: Rect,
    ) -> AppResult<LiveLayerView> {
        self.project_dir()?;
        if is_reserved_screen(screen_id) {
            return Err(AppError::NotFound(format!("Screen not found: {screen_id}")));
        }
        let (w, h) = self
            .live()
            .screen_size(screen_id)
            .ok_or_else(|| LiveError::UnknownScreen(screen_id.to_string()))?;
        screens::validate_rect(&rect, w, h)?;
        let (rect, anchor) = self.screens().moved_layer_rect(screen_id, layer_id, rect);
        let view = self
            .live()
            .set_rect(screen_id, layer_id, rect, self.live_edit())?;
        self.screens()
            .set_layer_rect(screen_id, layer_id, rect, anchor);
        Ok(view)
    }

    /// Control: sets a layer's field values in its screen and applies them by the live rules (off
    /// air now; on air under `onTake` staged until the next show). Unknown keys, bound fields and
    /// values over 64 KiB are `invalid`. The screen is written by the content flusher (coalesced);
    /// until then `get_screen` returns the edited document. May load the layer's template from
    /// disk.
    pub fn set_layer_fields(
        &self,
        screen_id: &str,
        layer_id: &str,
        fields: Fields,
    ) -> AppResult<LiveLayerView> {
        self.set_layer_fields_by(screen_id, layer_id, fields, false)
    }

    /// Automation (flows' Set fields): like [`AppState::set_layer_fields`], but in Studio mode the
    /// values go straight to Program (the Preview diff stays on top) instead of into the diff.
    pub fn set_layer_fields_auto(
        &self,
        screen_id: &str,
        layer_id: &str,
        fields: Fields,
    ) -> AppResult<LiveLayerView> {
        self.set_layer_fields_by(screen_id, layer_id, fields, true)
    }

    fn set_layer_fields_by(
        &self,
        screen_id: &str,
        layer_id: &str,
        fields: Fields,
        automation: bool,
    ) -> AppResult<LiveLayerView> {
        let (dir, _w) = self.screen_guard()?;
        let not_found = || AppError::NotFound(format!("Screen not found: {screen_id}"));
        if is_reserved_screen(screen_id) {
            return Err(not_found());
        }
        let mut doc = self.screens().doc(screen_id).ok_or_else(not_found)?;
        let templates = self.screens().templates_for(&dir, &doc);
        let layer = doc
            .meta
            .layers
            .iter_mut()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))?;
        let ScreenLayerKind::Template {
            template_id,
            fields: values,
            bindings,
            ..
        } = &mut layer.kind
        else {
            return Err(AppError::Invalid("A media layer has no fields".to_string()));
        };
        let t = templates.get(template_id.as_str()).ok_or_else(|| {
            AppError::Invalid(
                "The layer's template is missing, so its fields can't be set".to_string(),
            )
        })?;
        let mut fields = fields;
        for (k, v) in fields.iter_mut() {
            let Some(def) = t.meta.fields.iter().find(|f| &f.key == k) else {
                return Err(LiveError::UnknownField(k.clone()).into());
            };
            if bindings.contains_key(k) {
                return Err(AppError::Invalid(format!(
                    "Field \"{k}\" is bound to a source: change the source instead"
                )));
            }
            if v.len() > screens::VALUE_MAX {
                return Err(AppError::Invalid(format!(
                    "Field \"{k}\" is too long (at most {} KB)",
                    screens::VALUE_MAX / 1024
                )));
            }
            // Lists and timers are stored canonically; a clock has no value.
            *v = fieldvals::edit_value(def, v)
                .map_err(|m| AppError::Invalid(format!("Field \"{k}\": {m}")))?;
        }
        let changed = fields.iter().any(|(k, v)| values.get(k) != Some(v));
        let keys: Vec<String> = fields.keys().cloned().collect();
        values.extend(fields);
        if automation && self.live().is_studio() {
            let force = [(layer_id.to_string(), keys)];
            self.apply_screen_doc_forcing(&dir, &doc, &force)?;
        } else {
            self.apply_screen_doc(&dir, &doc)?;
        }
        if changed {
            self.screens().mark_dirty(screen_id);
            self.schedule_content_flush();
        }
        Ok(self.live().layer_view(screen_id, layer_id)?)
    }

    /// Shows another variant of a layer's template (`variant` must be one of the template's
    /// variants: `notFound` otherwise; `invalid` when the template is missing). Saved to the
    /// screen (only when it is not the default; coalesced like `set_layer_fields`) and applied by
    /// the live rules: off air at once; on air under `onTake` staged for the next take; on air
    /// under `immediate` swapped now with `transition`. Field values are untouched.
    pub fn set_layer_variant(
        &self,
        screen_id: &str,
        layer_id: &str,
        variant: &str,
        transition: VariantTransition,
    ) -> AppResult<ScreenState> {
        self.switch_layer_variant(screen_id, layer_id, variant, transition, false)
    }

    /// Automation (flows): like [`AppState::set_layer_variant`], but an on-air layer swaps at
    /// once even under `onTake` (its other staged changes stay staged).
    pub fn set_layer_variant_direct(
        &self,
        screen_id: &str,
        layer_id: &str,
        variant: &str,
        transition: VariantTransition,
    ) -> AppResult<ScreenState> {
        self.switch_layer_variant(screen_id, layer_id, variant, transition, true)
    }

    fn switch_layer_variant(
        &self,
        screen_id: &str,
        layer_id: &str,
        variant: &str,
        transition: VariantTransition,
        direct: bool,
    ) -> AppResult<ScreenState> {
        let (dir, _w) = self.screen_guard()?;
        let not_found = || AppError::NotFound(format!("Screen not found: {screen_id}"));
        if is_reserved_screen(screen_id) {
            return Err(not_found());
        }
        let mut doc = self.screens().doc(screen_id).ok_or_else(not_found)?;
        let templates = self.screens().templates_for(&dir, &doc);
        let layer = doc
            .meta
            .layers
            .iter_mut()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))?;
        let ScreenLayerKind::Template {
            template_id,
            variant: current,
            ..
        } = &mut layer.kind
        else {
            return Err(AppError::Invalid(
                "A media layer has no variants".to_string(),
            ));
        };
        let t = templates.get(template_id.as_str()).ok_or_else(|| {
            AppError::Invalid(
                "The layer's template is missing, so its variant can't be changed".to_string(),
            )
        })?;
        if !t.has_variant(variant) {
            return Err(AppError::NotFound(format!(
                "Variant \u{201c}{variant}\u{201d} isn't in the template \u{201c}{}\u{201d}",
                t.meta.name
            )));
        }
        let is_default = t.default_variant().is_some_and(|d| d.id == variant);
        let next = (!is_default).then(|| variant.to_string());
        let changed = *current != next;
        *current = next;
        let switch = VariantSwitch {
            layer_id: layer_id.to_string(),
            transition,
            direct,
        };
        let (live, warnings) = apply_doc_with(
            self.screens(),
            self.sources(),
            self.live(),
            &dir,
            &doc,
            self.live_edit(),
            &[],
            std::slice::from_ref(&switch),
        )?;
        if changed {
            self.screens().mark_dirty(screen_id);
            self.schedule_content_flush();
        }
        Ok(ScreenState {
            doc,
            live,
            warnings,
        })
    }

    /// Every on-air layer of the screen plays out (animated).
    pub fn live_clear(&self, screen_id: &str) -> AppResult<LiveScreenView> {
        self.project_dir()?;
        Ok(self.live().clear(screen_id)?)
    }

    /// Every on-air layer of every user screen plays out (animated).
    pub fn live_clear_all(&self) -> AppResult<Vec<LiveScreenView>> {
        self.project_dir()?;
        Ok(self.live().clear_all())
    }

    /// PANIC: see `LiveHub::panic`. Also disarms every auto-advance, stops every macro and
    /// cancels pending rule clears (`take … for N s`).
    pub fn live_panic(&self) -> AppResult<Vec<LiveScreenView>> {
        self.project_dir()?;
        self.stop_automation();
        // Pending rule clears are cancelled; the cut fires no layer rules.
        self.rules_panic_begin();
        let views = self.live().panic();
        self.rules_panic_end();
        // Nothing is on air any more; this also drops what a retake had kept armed.
        self.rearm();
        Ok(views)
    }

    /// A template was saved through Kinetra: every screen layer using it follows (one `layer` per
    /// affected off-air layer when HTML/CSS/field types changed; on-air layers staged under
    /// `onTake`).
    pub fn template_saved(&self, doc: &TemplateDoc) {
        let t = Arc::new(LoadedTemplate::from_doc(doc));
        self.template_changed(&doc.id, Some(t));
    }

    /// A template was deleted: its layers become `TemplateMissing` (on-air ones animate out and
    /// are then removed from the output).
    pub fn template_deleted(&self, id: &str) {
        self.template_changed(id, None);
    }

    fn template_changed(&self, id: &str, t: Option<Arc<LoadedTemplate>>) {
        let Ok((dir, _w)) = self.screen_guard() else {
            return;
        };
        self.reapply_template(&dir, id, t);
    }

    /// Caller holds the screen write lock.
    fn reapply_template(&self, dir: &Path, id: &str, t: Option<Arc<LoadedTemplate>>) {
        let users = self.screens().set_template(id, t);
        for doc in users {
            if let Err(e) = self.apply_screen_doc(dir, &doc) {
                tracing::warn!(
                    "applying screen {} after template {id} changed: {e}",
                    doc.id
                );
            }
        }
    }

    /// The watcher's handler: reloads each changed template, source or screen from disk and
    /// applies it by the live rules, then emits `project-files-changed`. An invalid file keeps the
    /// last good live state (it is not set aside: the user may be in the middle of editing it).
    pub fn handle_file_changes(&self, dir: &Path, batch: Vec<(FileKind, String)>) {
        for (kind, id) in batch {
            if self.project_dir().ok().as_deref() != Some(dir) {
                return;
            }
            let change = match kind {
                FileKind::Template => self.reload_template(dir, &id),
                FileKind::Screen => self.reload_screen(dir, &id),
                FileKind::Source => self.reload_source(dir, &id),
                FileKind::Asset => Some(self.asset_file_changed(dir, &id)),
                FileKind::Flow => self.reload_flow(dir, &id),
                FileKind::Controls => self.reload_controls(dir),
            };
            if let Some(c) = change {
                emit_files_changed(&**self.events(), &c);
            }
        }
    }

    fn reload_template(&self, dir: &Path, id: &str) -> Option<FilesChanged> {
        let _w = self.screens().write_lock();
        if !self.screens().is_project(dir) {
            return None;
        }
        self.templates().invalidate();
        let changed = |change, message| FilesChanged {
            kind: FileKind::Template,
            id: id.to_string(),
            change,
            message,
        };
        let (change, t) = match screens::load_template(dir, id) {
            Ok(Some(t)) => (FileChange::Changed, Some(t)),
            Ok(None) => (FileChange::Removed, None),
            Err(msg) => {
                tracing::warn!("template {id} changed on disk but is not valid: {msg}");
                return Some(changed(FileChange::Invalid, Some(msg)));
            }
        };
        self.reapply_template(dir, id, t);
        Some(changed(change, None))
    }

    fn reload_screen(&self, dir: &Path, id: &str) -> Option<FilesChanged> {
        let _w = self.screens().write_lock();
        if !self.screens().is_project(dir) {
            return None;
        }
        self.screens().invalidate();
        let changed = |change, message| FilesChanged {
            kind: FileKind::Screen,
            id: id.to_string(),
            change,
            message,
        };
        let prev = self.screens().doc(id);
        match screens::get_after(dir, id, prev.as_ref().map(|d| &d.meta)) {
            // Exactly what is applied already (typically our own write, seen late under load
            // after the own-write memory expired): nothing changed, nothing to announce.
            Ok((doc, _)) if prev.as_ref() == Some(&doc) => None,
            // The file edited outside wins over unsaved Control edits.
            Ok((doc, released)) => {
                self.screens().clear_dirty(id);
                let applied = self.apply_screen_doc(dir, &doc);
                if released {
                    // A layer moved by hand let go of its anchor axis: the file is written back
                    // (until then `get_screen` returns this document).
                    self.screens().mark_dirty(id);
                    self.schedule_content_flush();
                }
                if self.binds_schedule(&doc) {
                    self.replan();
                }
                match applied {
                    Ok(_) => Some(changed(FileChange::Changed, None)),
                    Err(e) => Some(changed(FileChange::Invalid, Some(e.to_string()))),
                }
            }
            Err(ScreenError::NotFound(_)) => {
                self.screens().remove_doc(id);
                self.live().remove_screen(id);
                Some(changed(FileChange::Removed, None))
            }
            Err(e) => {
                tracing::warn!("screen {id} changed on disk but is not valid: {e}");
                Some(changed(FileChange::Invalid, Some(e.to_string())))
            }
        }
    }
}

#[cfg(test)]
mod tests;
