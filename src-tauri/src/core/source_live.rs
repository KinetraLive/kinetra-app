//! Project sources and the live state: the operations behind the source commands and the watcher.
//! A source change (save, step, delete, external edit) re-applies every screen bound to it through
//! `screen_live::apply_doc`, so the live rules hold: an off-air layer updates now; an on-air layer
//! under `onTake` stages the change for the next take; `immediate` applies it now.
//!
//! Reads (`list_sources`, `get_source`) are served from memory, with the in-memory `index` of
//! stepped sources. Writers take the screen store's write lock (see `screen_live`).
//!
//! A change of a text source's current row (step, save, reload; auto-advance in `automation`)
//! fires the flows' `trigger.sourceRow`.
//!
//! A step changes only the in-memory `index`; the content flusher (`state`) writes it at most once
//! per `CONTENT_FLUSH_DELAY`, and explicit flushes (close, switch, snapshot, exit) write it too.

use std::path::Path;

use crate::core::model::{
    FileChange, FileKind, FilesChanged, SourceDoc, SourceKind, SourceStep, SourceSummary,
};
use crate::core::protocol::Fields;
use crate::core::sources::{self, SourceError};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

impl AppState {
    pub(crate) fn source_summary(&self, doc: &SourceDoc) -> SourceSummary {
        let used_by = self
            .screens()
            .source_usage()
            .remove(&doc.id)
            .unwrap_or_default();
        let now = self.sources().clock().now();
        let mut s = sources::summary(doc, used_by, &now);
        s.next_at = self.automation().next_at(&doc.id);
        s.api = self.api_status(doc);
        s.poll = self.poll_view(doc);
        s
    }

    /// Every source of the open project, by name (case-insensitive), then id. From memory.
    pub fn list_sources(&self) -> AppResult<Vec<SourceSummary>> {
        self.project_dir()?;
        let mut usage = self.screens().source_usage();
        let now = self.sources().clock().now();
        let mut list: Vec<SourceSummary> = self
            .sources()
            .docs()
            .iter()
            .map(|d| {
                let mut s = sources::summary(d, usage.remove(&d.id).unwrap_or_default(), &now);
                s.next_at = self.automation().next_at(&d.id);
                s.api = self.api_status(d);
                s.poll = self.poll_view(d);
                s
            })
            .collect();
        list.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(list)
    }

    /// A source with its current (in-memory) index. A file not loaded (e.g. from a newer
    /// Kinetra) is read from disk to report why.
    pub fn get_source(&self, id: &str) -> AppResult<SourceDoc> {
        let dir = self.project_dir()?;
        sources::validate_id(id)?;
        if let Some(doc) = self.sources().doc(id) {
            return Ok(self.with_stored(doc));
        }
        Ok(self.with_stored(sources::get(&dir, id)?))
    }

    /// A new text source with one column (`text`, "Text") and no rows.
    pub fn create_source(&self, name: &str) -> AppResult<SourceDoc> {
        let (dir, _w) = self.screen_guard()?;
        let doc = sources::create(&dir, name)?;
        self.sources().put(doc.clone());
        self.reapply_source(&dir, &doc.id);
        Ok(doc)
    }

    /// A new schedule source (no items, default labels, ripple off).
    pub fn create_schedule(&self, name: &str) -> AppResult<SourceDoc> {
        let (dir, _w) = self.screen_guard()?;
        let doc = sources::create_schedule(&dir, name)?;
        self.sources().put(doc.clone());
        self.reapply_source(&dir, &doc.id);
        self.schedule_edited(&dir, &doc.id);
        Ok(doc)
    }

    /// Validates and writes (if changed) the whole document, `index` included (clamped), then
    /// re-applies the screens bound to it. The source must exist. A schedule is re-planned
    /// (statuses, boundaries, rules); its Undo is cleared.
    ///
    /// API sources: the rows cache and `fetchedAt` are the poller's (the document's are
    /// ignored; rows are cut to the new columns); keychain values of secret headers that are
    /// gone are deleted. The poller follows the new settings.
    pub fn save_source(&self, mut doc: SourceDoc) -> AppResult<SourceDoc> {
        let (saved, dropped) = {
            let (dir, _w) = self.screen_guard()?;
            let old = self.sources().doc(&doc.id);
            if let (
                SourceKind::Api {
                    rows, fetched_at, ..
                },
                Some(SourceKind::Api {
                    rows: have,
                    fetched_at: at,
                    ..
                }),
            ) = (&mut doc.meta.kind, old.as_ref().map(|d| &d.meta.kind))
            {
                rows.clone_from(have);
                fetched_at.clone_from(at);
            }
            // A poll source: the link and the server's data are the core's.
            if let (
                SourceKind::Poll { link, cache, gone },
                Some(SourceKind::Poll {
                    link: l0,
                    cache: c0,
                    gone: g0,
                }),
            ) = (&mut doc.meta.kind, old.as_ref().map(|d| &d.meta.kind))
            {
                link.clone_from(l0);
                cache.clone_from(c0);
                *gone = *g0;
            }
            let before = self.row_mark(&doc.id);
            let saved = sources::save(&dir, doc, false)?;
            self.sources().put(saved.clone());
            self.reapply_source(&dir, &saved.id);
            self.schedule_edited(&dir, &saved.id);
            self.row_maybe_changed(&saved.id, before);
            let kept = Self::secret_ids(&saved);
            let dropped: Vec<String> = old
                .as_ref()
                .map(Self::secret_ids)
                .unwrap_or_default()
                .into_iter()
                .filter(|s| !kept.contains(s))
                .collect();
            (saved, dropped)
        };
        self.delete_secrets(&dropped);
        self.api_reconcile();
        Ok(self.with_stored(saved))
    }

    pub fn rename_source(&self, id: &str, name: &str) -> AppResult<SourceSummary> {
        let (dir, _w) = self.screen_guard()?;
        let mut doc = self.loaded_source(id)?;
        doc.meta.name = sources::validate_name(name)?;
        let saved = sources::save(&dir, doc, false)?;
        self.sources().put(saved.clone());
        Ok(self.source_summary(&saved))
    }

    /// A copy (current index included) under a new id; nothing is bound to it yet. An API
    /// source's secret headers get new keychain entries holding copies of the values.
    pub fn duplicate_source(&self, id: &str, name: &str) -> AppResult<SourceDoc> {
        let doc = {
            let (dir, _w) = self.screen_guard()?;
            let mut src = self.loaded_source(id)?;
            self.copy_secrets(&mut src)?;
            let copies = Self::secret_ids(&src);
            let doc = match sources::duplicate(&dir, src, name) {
                Ok(d) => d,
                Err(e) => {
                    self.delete_secrets(&copies);
                    return Err(e.into());
                }
            };
            self.sources().put(doc.clone());
            doc
        };
        self.api_reconcile();
        Ok(self.with_stored(doc))
    }

    /// Deletes the file; bound layers fall back to their own values (`sourceMissing`).
    /// An API source's keychain values are deleted and its polling stops.
    pub fn delete_source(&self, id: &str) -> AppResult<()> {
        let (result, secrets) = {
            let (dir, _w) = self.screen_guard()?;
            let mut result = sources::delete(&dir, id);
            let mut secrets = Vec::new();
            if matches!(result, Ok(()) | Err(SourceError::NotFound(_))) {
                let had = self.sources().remove(id);
                secrets = had.as_ref().map(Self::secret_ids).unwrap_or_default();
                self.reapply_source(&dir, id);
                self.schedule_edited(&dir, id);
                if result.is_err() && had.is_some() {
                    result = Ok(());
                }
            }
            (result, secrets)
        };
        self.delete_secrets(&secrets);
        self.api_reconcile();
        Ok(result?)
    }

    /// Next / Prev / go to a row (clamped, no wrap). An unchanged index does nothing. Otherwise
    /// the bound screens are re-applied and the index is written coalesced.
    pub fn step_source(&self, id: &str, step: SourceStep) -> AppResult<SourceSummary> {
        let (dir, _w) = self.screen_guard()?;
        sources::validate_id(id)?;
        if self
            .sources()
            .doc(id)
            .is_some_and(|d| sources::is_schedule(&d))
        {
            return Err(AppError::Invalid(
                "A schedule has no rows to step: use its actions (Start now, Set next, ...)".into(),
            ));
        }
        let (doc, changed) = self
            .sources()
            .step(id, step)
            .ok_or_else(|| AppError::from(SourceError::NotFound(id.to_string())))?;
        if changed {
            self.schedule_content_flush();
            self.reapply_source(&dir, id);
            self.flow_source_row_changed(id);
        }
        Ok(self.source_summary(&doc))
    }

    /// The current row of a text source watched by a `trigger.sourceRow` (index and cells), to
    /// tell a save or reload that changed it. `None` when no flow watches it (cheap).
    fn row_mark(&self, id: &str) -> Option<(u32, Fields)> {
        if !self.flow_engine().watches_source_row(id) {
            return None;
        }
        let doc = self
            .sources()
            .doc(id)
            .filter(|d| !sources::is_schedule(d))?;
        let now = self.sources().clock().now();
        Some((sources::position(&doc).1, sources::current_row(&doc, &now)))
    }

    /// Fires `trigger.sourceRow` if a save or reload changed the current row.
    fn row_maybe_changed(&self, id: &str, before: Option<(u32, Fields)>) {
        let Some(before) = before else { return };
        if self.row_mark(id).is_some_and(|after| after != before) {
            self.flow_source_row_changed(id);
        }
    }

    pub(crate) fn loaded_source(&self, id: &str) -> AppResult<SourceDoc> {
        sources::validate_id(id)?;
        self.sources()
            .doc(id)
            .ok_or_else(|| SourceError::NotFound(id.to_string()).into())
    }

    /// Re-applies every screen bound to source `id`. Caller holds the screen write lock.
    pub(crate) fn reapply_source(&self, dir: &Path, id: &str) {
        for doc in self.screens().source_users(id) {
            if let Err(e) = self.apply_screen_doc(dir, &doc) {
                tracing::warn!("applying screen {} after source {id} changed: {e}", doc.id);
            }
        }
    }

    /// The watcher's reload of `sources/<id>.json`.
    pub(crate) fn reload_source(&self, dir: &Path, id: &str) -> Option<FilesChanged> {
        let _w = self.screens().write_lock();
        if !self.screens().is_project(dir) {
            return None;
        }
        let changed = |change, message| FilesChanged {
            kind: FileKind::Source,
            id: id.to_string(),
            change,
            message,
        };
        match sources::get(dir, id) {
            // Exactly what is in memory already (typically our own write, seen late under load
            // after the own-write memory expired): nothing changed, nothing to announce.
            Ok(doc) if self.sources().doc(id).as_ref() == Some(&doc) => None,
            // The file edited outside wins over an unsaved step.
            Ok(doc) => {
                let before = self.row_mark(id);
                self.sources().put(doc);
                self.reapply_source(dir, id);
                self.schedule_edited(dir, id);
                self.row_maybe_changed(id, before);
                self.api_reconcile();
                Some(changed(FileChange::Changed, None))
            }
            Err(SourceError::NotFound(_)) => {
                self.sources().remove(id);
                self.reapply_source(dir, id);
                self.schedule_edited(dir, id);
                self.api_reconcile();
                Some(changed(FileChange::Removed, None))
            }
            Err(e) => {
                tracing::warn!("source {id} changed on disk but is not valid: {e}");
                Some(changed(FileChange::Invalid, Some(e.to_string())))
            }
        }
    }
}

#[cfg(test)]
mod tests;
