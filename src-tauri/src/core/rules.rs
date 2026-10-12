//! The live part of the schedule engine, the trigger detection of flows, and the recipe-instance
//! store. Everything is event-driven: no polling, no task at rest unless something is due.
//!
//! # One planner task
//! A single sleeping tokio task (the *planner*) waits for the earliest of:
//! - the next boundary of every *watched* schedule (a source bound by a screen layer, the source
//!   of a recipe, or named by a flow trigger): an effective start or end still ahead;
//! - `trigger.beforeNext` times (UP NEXT's effective start − N minutes);
//! - `trigger.timeOfDay` times (the next matching local time);
//! - `trigger.timerZero` times (a running countdown's zero, read from the live layer).
//!
//! It wakes, re-plans the watched schedules, fires what is due, and sleeps again until the next
//! time (re-planned after every edit, action, deploy, arm, live change and project open). No
//! task exists while nothing is ahead. One task for everything (instead of one per schedule)
//! keeps the bookkeeping in one place; the CPU cost is the same (zero between wakes). Flow waits
//! (Delay, Wait until) park inside their instance (`core::flow_engine`), not here.
//!
//! # Schedules
//! Re-planning a schedule computes its view and binding rows at the current time. When the
//! rows changed, the bound screens are re-applied with their bound fields forced on air (like
//! auto-advance `update`: automation bypasses `onTake`); edits from the Sources page follow the
//! normal live rules instead. A changed view emits `schedule-changed`. Operator actions
//! (`schedule_action`, also a flow's Schedule action) change the source in memory (written by the
//! content flusher), keep ONE level of undo, and apply at once.
//!
//! # Trigger detection
//! The trigger nodes come from the engine's index (armed, deployed flows); the once-only
//! bookkeeping is keyed by `flowId/nodeId`.
//! - `itemStatus`: an item newly gets the status (compared with the previous plan of that
//!   schedule; the first plan after opening or creating it is a baseline and fires nothing). A
//!   trigger fires at most once per item and status. Data: `item` (the row, with `_id`).
//! - `beforeNext`: when UP NEXT's start − N minutes has passed and the item hasn't started; once
//!   per item. Items already in that window when the trigger is loaded, deployed or armed don't
//!   fire. Data: `item`.
//! - `timeOfDay`: at the next matching local time after the trigger was loaded or deployed.
//!   Data: `at`.
//! - `timerZero`: when a running countdown (any `end`) of that layer's timer field reaches zero;
//!   once per zero time. A zero already passed when the trigger is loaded doesn't fire.
//! - `layer`: when the layer goes on or off air (from any path except PANIC).
//! - `slideshowWrap`: when an auto-advancing source completes a cycle (`core::automation`).
//!
//! Detection runs while Automation is off too (statuses and times move on), but the engine
//! drops automatic firings then, so nothing catches up when it is switched back on.
//!
//! `take` with `forMs` schedules a clear (one sleeping task per layer); it only clears the layer
//! if it is still in the same on-air period. PANIC and Automation off cancel pending clears; PANIC
//! fires no `layer` off-air trigger. Closing or switching the project stops everything.
//!
//! Locking: the rules lock is a data lock, held briefly and never while calling the hub, the
//! screen store, the source store or the engine. Re-planning a schedule requires the screen
//! write lock (taken by the planner tick; held by the callers of the edit paths).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;

use crate::core::events::names;
use crate::core::fieldvals;
use crate::core::flow_engine::{schedule_row_of, Trig, TrigRef, TriggerKey, Val};
use crate::core::fsx;
use crate::core::localtime::{self, Now};
use crate::core::model::{
    ItemStatus, NowNext, RecipeInstance, ScheduleAction, ScheduleView, SourceKind,
};
use crate::core::protocol::TimerDir;
use crate::core::schedule::{self, Plan};
use crate::core::screens;
use crate::core::sources::{self, Current};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

static GEN: AtomicU64 = AtomicU64::new(1);

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A zero time this far in the past still fires (the planner woke a little late).
const ZERO_GRACE_MS: i64 = 2_000;

/// What the engine remembers of one schedule.
#[derive(Default)]
struct Track {
    statuses: HashMap<String, ItemStatus>,
    current: Option<Current>,
    /// The last view emitted (with `computedAt` zeroed).
    view: Option<ScheduleView>,
    /// One level of undo for operator actions.
    undo: Option<Plan>,
}

struct Wake {
    at: i64,
    /// Real server ms of `at` (a changed test clock re-plans).
    real_at: i64,
    gen: u64,
    abort: AbortHandle,
}

struct PendingClear {
    gen: u64,
    abort: AbortHandle,
}

#[derive(Default)]
struct Eng {
    project: Option<PathBuf>,
    instances: Vec<RecipeInstance>,
    /// Instances changed in memory only (written by the content flusher).
    dirty: BTreeSet<String>,
    sched: HashMap<String, Track>,
    /// (trigger key, item id, status) of `itemStatus` firings.
    fired_status: HashSet<(String, String, &'static str)>,
    /// (trigger key, item id) of `beforeNext` firings.
    fired_before: HashSet<(String, String)>,
    /// Trigger key → next `timeOfDay` time (schedule-frame epoch ms).
    tod_next: HashMap<String, i64>,
    /// Trigger key → the zero time it last fired for.
    zero_fired: HashMap<String, i64>,
    /// On-air state of the layers `layer` triggers watch.
    layer_seen: HashMap<(String, String), bool>,
    wake: Option<Wake>,
    clears: HashMap<(String, String), PendingClear>,
    /// During PANIC: layer changes update the baseline but fire nothing.
    suppress_layers: bool,
}

/// The schedule planner, trigger detection and recipe instances (see the module docs). Owned by
/// `AppState`.
#[derive(Default)]
pub struct Rules {
    inner: Mutex<Eng>,
}

/// A trigger to fire, with its data outputs.
struct Fire {
    t: TrigRef,
    data: Vec<(&'static str, Val)>,
}

impl Rules {
    /// Aborts the planner and every pending clear.
    pub(crate) fn stop_all(&self) {
        let mut e = lock(&self.inner);
        if let Some(w) = e.wake.take() {
            w.abort.abort();
        }
        for (_, c) in e.clears.drain() {
            c.abort.abort();
        }
    }

    /// Forgets the project (closed). Call [`Rules::stop_all`] first.
    pub(crate) fn forget(&self) {
        self.stop_all();
        *lock(&self.inner) = Eng::default();
    }

    /// The project folder moved.
    pub(crate) fn relocate(&self, dir: &Path) {
        let mut e = lock(&self.inner);
        if e.project.is_some() {
            e.project = Some(dir.to_path_buf());
        }
    }

    /// Number of tasks alive (planner + pending clears), for tests and diagnostics.
    pub fn task_count(&self) -> usize {
        let e = lock(&self.inner);
        usize::from(e.wake.is_some()) + e.clears.len()
    }

    /// When the planner wakes next (schedule-frame epoch ms), if it sleeps.
    pub fn next_wake(&self) -> Option<i64> {
        lock(&self.inner).wake.as_ref().map(|w| w.at)
    }

    /// The installed instances (creation order).
    pub fn instances(&self) -> Vec<RecipeInstance> {
        lock(&self.inner).instances.clone()
    }

    pub(crate) fn project(&self) -> Option<PathBuf> {
        lock(&self.inner).project.clone()
    }

    pub(crate) fn instance(&self, id: &str) -> Option<RecipeInstance> {
        lock(&self.inner)
            .instances
            .iter()
            .find(|i| i.id == id)
            .cloned()
    }

    /// Adds or replaces an instance (already written to disk).
    pub(crate) fn put_instance(&self, inst: RecipeInstance) {
        let mut e = lock(&self.inner);
        e.dirty.remove(&inst.id);
        match e.instances.iter_mut().find(|i| i.id == inst.id) {
            Some(slot) => *slot = inst,
            None => e.instances.push(inst),
        }
    }

    pub(crate) fn remove_instance(&self, id: &str) -> Option<RecipeInstance> {
        let mut e = lock(&self.inner);
        e.dirty.remove(id);
        let pos = e.instances.iter().position(|i| i.id == id)?;
        Some(e.instances.remove(pos))
    }

    /// Takes the instances changed in memory only, with the project folder.
    pub(crate) fn take_dirty(&self) -> Option<(PathBuf, Vec<RecipeInstance>)> {
        let mut e = lock(&self.inner);
        if e.dirty.is_empty() {
            return None;
        }
        let project = e.project.clone()?;
        let ids = std::mem::take(&mut e.dirty);
        let list = e
            .instances
            .iter()
            .filter(|i| ids.contains(&i.id))
            .cloned()
            .collect();
        Some((project, list))
    }

    pub(crate) fn mark_dirty(&self, id: &str) {
        let mut e = lock(&self.inner);
        if e.instances.iter().any(|i| i.id == id) {
            e.dirty.insert(id.to_string());
        }
    }
}

/// The next local `HH:MM` on one of `days` (0 = Monday; empty = every day) strictly after `now`.
pub fn next_time_of_day(at: &str, days: &[u8], now: &Now) -> Option<i64> {
    let (h, m) = localtime::parse_hhmm(at)?;
    let today = now.local().date();
    for d in 0..=8 {
        let date = today.checked_add(time::Duration::days(d))?;
        let wd = date.weekday().number_days_from_monday();
        if !days.is_empty() && !days.contains(&wd) {
            continue;
        }
        let t = time::Time::from_hms(h, m, 0).ok()?;
        let ms = now.zone.to_epoch(time::PrimitiveDateTime::new(date, t));
        if ms > now.ms {
            return Some(ms);
        }
    }
    None
}

impl AppState {
    fn clock_now(&self) -> Now {
        self.sources().clock().now()
    }

    // ---- Project lifecycle ---------------------------------------------------------------------

    /// Loads the project's recipe instances and takes the baseline of every schedule, layer and
    /// timed trigger (nothing fires for what is already the case), then plans the first wake.
    /// Called after a project opened or was restored, after the engine got its flows. Touches the
    /// disk.
    pub(crate) fn rules_project_opened(&self) {
        let Ok(dir) = self.project_dir() else { return };
        self.rules().stop_all();
        let (list, recovered) = crate::core::recipes::load_all(&dir);
        for r in recovered {
            self.events().emit(names::STORE_RECOVERED, r.payload());
        }
        *lock(&self.rules().inner) = Eng {
            project: Some(dir.clone()),
            instances: list,
            ..Eng::default()
        };
        if let Ok((dir, _w)) = self.screen_guard() {
            for id in self.sources().schedule_ids() {
                let _ = self.refresh_schedule(&dir, &id, false, true, false);
            }
        }
        self.rules_baseline();
        self.replan();
    }

    /// Baselines the timed and layer triggers of every armed flow (after a load, a deploy, an
    /// arm or Automation back on): nothing that is already due or already the case fires.
    pub(crate) fn rules_baseline(&self) {
        let now = self.clock_now();
        let index = self.flow_engine().index();
        let mut before = Vec::new();
        let mut tod = HashMap::new();
        let prev_tod = lock(&self.rules().inner).tod_next.clone();
        let mut zero = Vec::new();
        let mut layers = Vec::new();
        for t in index.timed().iter().chain(index.layers()) {
            let key = t.key();
            match t.trig() {
                Trig::BeforeNext { source, minutes } => {
                    if let Some((item, start)) = self.next_item(source, &now) {
                        let at = start - i64::from(minutes) * 60_000;
                        if at <= now.ms {
                            before.push((key, item));
                        }
                    }
                }
                Trig::TimeOfDay { at, days } => {
                    // A time just due but not fired yet (the planner wakes a few ms late) is
                    // kept, so a deploy of another flow doesn't skip it; only if the trigger
                    // still names that time (an edited time of day never keeps the old one).
                    let pending = prev_tod.get(&key).copied().filter(|&v| {
                        v <= now.ms
                            && now.ms - v < ZERO_GRACE_MS
                            && next_time_of_day(at, &days, &Now { ms: v - 1, ..now }) == Some(v)
                    });
                    if let Some(next) = pending.or_else(|| next_time_of_day(at, &days, &now)) {
                        tod.insert(key, next);
                    }
                }
                Trig::TimerZero {
                    screen,
                    layer,
                    key: field,
                } => {
                    if let Some(z) = self.timer_zero(screen, layer, field, &now) {
                        if z <= now.ms {
                            zero.push((key, z));
                        }
                    }
                }
                Trig::Layer { screen, layer, .. } => {
                    let on = self.live().is_on_air(screen, layer);
                    layers.push(((screen.to_string(), layer.to_string()), on));
                }
                Trig::ItemStatus { .. } | Trig::Other => {}
            }
        }
        let mut e = lock(&self.rules().inner);
        e.fired_before.extend(before);
        e.tod_next = tod;
        for (k, z) in zero {
            e.zero_fired.insert(k, z);
        }
        for (k, on) in layers {
            e.layer_seen.insert(k, on);
        }
    }

    // ---- Schedules -----------------------------------------------------------------------------

    /// The UP NEXT item of a schedule and its effective start.
    fn next_item(&self, source_id: &str, now: &Now) -> Option<(String, i64)> {
        let doc = self.sources().doc(source_id)?;
        let SourceKind::Schedule {
            items,
            ripple,
            next_override,
            ..
        } = &doc.meta.kind
        else {
            return None;
        };
        let c = schedule::compute(items, *ripple, next_override.as_deref(), now);
        let id = c.next_id?;
        let e = c.items.iter().find(|e| items[e.index].id == id)?;
        Some((id, e.start_ms))
    }

    /// A schedule source was created, saved, deleted or reloaded from disk: re-plan it (the
    /// screens were re-applied by the live rules already), drop its undo, fire status triggers
    /// and re-plan the wake. Caller holds the screen write lock.
    pub(crate) fn schedule_edited(&self, dir: &Path, id: &str) {
        let is_schedule = self
            .sources()
            .doc(id)
            .is_some_and(|d| sources::is_schedule(&d));
        let tracked = lock(&self.rules().inner).sched.contains_key(id);
        if !is_schedule && !tracked {
            return;
        }
        if let Some(t) = lock(&self.rules().inner).sched.get_mut(id) {
            t.undo = None;
        }
        let fires = self.refresh_schedule(dir, id, false, false, true);
        self.fire_all(fires);
        self.replan_later();
    }

    /// Re-plans one schedule at the current time (see the module docs): `apply` = re-apply the
    /// bound screens with their bound fields forced on air when the rows changed; `baseline` =
    /// fire nothing; `emit_source` = emit `source-changed` even when NOW / UP NEXT did not change
    /// (after edits and actions; otherwise only when they did). Returns the status triggers to
    /// fire. Caller holds the screen write lock.
    fn refresh_schedule(
        &self,
        dir: &Path,
        id: &str,
        apply: bool,
        baseline: bool,
        emit_source: bool,
    ) -> Vec<Fire> {
        let now = self.clock_now();
        let doc = match self.sources().doc(id) {
            Some(d) if sources::is_schedule(&d) => d,
            _ => {
                lock(&self.rules().inner).sched.remove(id);
                return Vec::new();
            }
        };
        let SourceKind::Schedule {
            items,
            ripple,
            next_override,
            labels,
        } = &doc.meta.kind
        else {
            return Vec::new();
        };
        let c = schedule::compute(items, *ripple, next_override.as_deref(), &now);
        let current = Current::of(&doc, &now);
        let statuses = schedule::statuses(items, &c);
        let (prev, rows_changed, view, view_changed, ids_changed) = {
            let mut e = lock(&self.rules().inner);
            let t = e.sched.entry(id.to_string()).or_default();
            let mut view = schedule::view(id, items, &c, &now, t.undo.is_some());
            let computed_at = view.computed_at;
            view.computed_at = 0;
            let view_changed = t.view.as_ref() != Some(&view);
            let ids_changed = t
                .view
                .as_ref()
                .is_none_or(|v| v.now_id != view.now_id || v.next_id != view.next_id);
            let rows_changed = t.current.as_ref() != Some(&current);
            let prev = (!t.statuses.is_empty() || t.view.is_some())
                .then(|| std::mem::replace(&mut t.statuses, statuses.clone()));
            t.statuses = statuses.clone();
            t.current = Some(current);
            t.view = Some(view.clone());
            view.computed_at = computed_at;
            (prev, rows_changed, view, view_changed, ids_changed)
        };
        if apply && rows_changed {
            for sdoc in self.screens().source_users(id) {
                let force = screens::bound_layers(&sdoc, id);
                if let Err(e) = self.apply_screen_doc_forcing(dir, &sdoc, &force) {
                    tracing::warn!("schedule {id}: applying screen {}: {e}", sdoc.id);
                }
            }
        }
        if view_changed {
            match serde_json::to_value(&view) {
                Ok(v) => self.events().emit(names::SCHEDULE_CHANGED, v),
                Err(e) => tracing::warn!("schedule-changed payload: {e}"),
            }
        }
        if ids_changed || emit_source {
            self.emit_source_changed(id);
        }
        let Some(prev) = prev.filter(|_| !baseline) else {
            return Vec::new();
        };
        let changed: Vec<(&String, ItemStatus)> = statuses
            .iter()
            .filter(|(item, s)| prev.get(*item) != Some(*s))
            .map(|(item, s)| (item, *s))
            .collect();
        if changed.is_empty() {
            return Vec::new();
        }
        let index = self.flow_engine().index();
        let triggers = index.get(&TriggerKey::ItemStatus(id.to_string()));
        if triggers.is_empty() {
            return Vec::new();
        }
        let mut fires = Vec::new();
        let mut e = lock(&self.rules().inner);
        for t in triggers {
            let Trig::ItemStatus { status, item, .. } = t.trig() else {
                continue;
            };
            let key = t.key();
            for (item_id, s) in &changed {
                if *s != status || item.is_some_and(|w| w != item_id.as_str()) {
                    continue;
                }
                if e.fired_status.insert((
                    key.clone(),
                    (*item_id).clone(),
                    schedule::status_name(*s),
                )) {
                    let row = schedule_row_of(items, labels, &c, &now, item_id).unwrap_or_default();
                    fires.push(Fire {
                        t: t.clone(),
                        data: vec![("item", Val::Row(row))],
                    });
                }
            }
        }
        fires
    }

    /// The schedule's items with computed statuses. `invalid` for a text source.
    pub fn schedule_view(&self, source_id: &str) -> AppResult<ScheduleView> {
        self.project_dir()?;
        let doc = self.loaded_source(source_id)?;
        let SourceKind::Schedule {
            items,
            ripple,
            next_override,
            ..
        } = &doc.meta.kind
        else {
            return Err(AppError::Invalid(format!(
                "\u{201c}{}\u{201d} is not a schedule",
                doc.meta.name
            )));
        };
        let now = self.clock_now();
        let c = schedule::compute(items, *ripple, next_override.as_deref(), &now);
        let can_undo = lock(&self.rules().inner)
            .sched
            .get(source_id)
            .is_some_and(|t| t.undo.is_some());
        Ok(schedule::view(source_id, items, &c, &now, can_undo))
    }

    /// An operator action (see `ScheduleAction`; also a flow's Schedule action, which the
    /// operator's Undo reverts the same way): changed in memory (written by the content
    /// flusher), re-planned and applied on air at once. Returns the new view.
    pub fn schedule_action(
        &self,
        source_id: &str,
        action: ScheduleAction,
    ) -> AppResult<ScheduleView> {
        {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.loaded_source(source_id)?;
            let SourceKind::Schedule {
                items,
                ripple,
                next_override,
                ..
            } = &mut doc.meta.kind
            else {
                return Err(AppError::Invalid(format!(
                    "\u{201c}{}\u{201d} is not a schedule",
                    doc.meta.name
                )));
            };
            let now = self.clock_now();
            let before = Plan {
                items: items.clone(),
                ripple: *ripple,
                next_override: next_override.clone(),
            };
            let (after, undo) = match &action {
                ScheduleAction::Undo => {
                    let prev = lock(&self.rules().inner)
                        .sched
                        .get_mut(source_id)
                        .and_then(|t| t.undo.take())
                        .ok_or_else(|| AppError::Invalid("Nothing to undo".into()))?;
                    (prev, None)
                }
                a => {
                    let mut p = before.clone();
                    schedule::apply_action(&mut p, a, &now).map_err(|m| {
                        if m.starts_with("Schedule item not found") {
                            AppError::NotFound(m)
                        } else {
                            AppError::Invalid(m)
                        }
                    })?;
                    (p, Some(before))
                }
            };
            *items = after.items;
            *ripple = after.ripple;
            *next_override = after.next_override;
            doc.meta.updated_at = fsx::now_rfc3339();
            let doc = sources::validate(doc)?;
            self.sources().put_dirty(doc);
            self.schedule_content_flush();
            lock(&self.rules().inner)
                .sched
                .entry(source_id.to_string())
                .or_default()
                .undo = undo;
            let fires = self.refresh_schedule(&dir, source_id, true, false, true);
            self.fire_all(fires);
        }
        self.replan();
        self.schedule_view(source_id)
    }

    // ---- Planner -------------------------------------------------------------------------------

    /// Schedules that the planner keeps up to date (see the module docs).
    fn watched_schedules(&self) -> Vec<String> {
        let usage = self.screens().source_usage();
        let mut named: HashSet<String> = HashSet::new();
        {
            let e = lock(&self.rules().inner);
            for i in &e.instances {
                if let Some(s) = &i.source_id {
                    named.insert(s.clone());
                }
            }
        }
        let index = self.flow_engine().index();
        named.extend(index.schedule_sources().map(str::to_string));
        self.sources()
            .schedule_ids()
            .into_iter()
            .filter(|id| usage.contains_key(id) || named.contains(id))
            .collect()
    }

    /// The zero time (schedule frame) of a running countdown in a layer's timer field.
    fn timer_zero(&self, screen_id: &str, layer_id: &str, key: &str, now: &Now) -> Option<i64> {
        let raw = self
            .live()
            .with_layer(screen_id, layer_id, |l| l.fields.get(key).cloned())
            .ok()
            .flatten()?;
        let s = fieldvals::parse_timer(&raw).ok()?;
        if !s.run || s.dir != TimerDir::Down {
            return None;
        }
        let zero_real = i64::try_from(s.at.saturating_add(s.base)).ok()?;
        Some(zero_real + (now.ms - now.real_ms))
    }

    /// Re-plans the planner's wake (see the module docs). Never takes the screen write lock.
    pub fn replan(&self) {
        if self.project_dir().is_err() || self.rules().project().is_none() {
            return;
        }
        let now = self.clock_now();
        let mut due: Option<i64> = None;
        let mut at = |t: i64| due = Some(due.map_or(t, |d| d.min(t)));
        for id in self.watched_schedules() {
            if let Some(doc) = self.sources().doc(&id) {
                if let SourceKind::Schedule {
                    items,
                    ripple,
                    next_override,
                    ..
                } = &doc.meta.kind
                {
                    let c = schedule::compute(items, *ripple, next_override.as_deref(), &now);
                    if let Some(t) = schedule::next_boundary(&c, &now) {
                        at(t);
                    }
                }
            }
        }
        let index = self.flow_engine().index();
        if !index.timed().is_empty() {
            let (fired_before, tod, zero_fired) = {
                let e = lock(&self.rules().inner);
                (
                    e.fired_before.clone(),
                    e.tod_next.clone(),
                    e.zero_fired.clone(),
                )
            };
            for t in index.timed() {
                let key = t.key();
                match t.trig() {
                    Trig::BeforeNext { source, minutes } => {
                        if let Some((item, start)) = self.next_item(source, &now) {
                            let when = start - i64::from(minutes) * 60_000;
                            if when > now.ms {
                                at(when);
                            } else if now.ms < start && !fired_before.contains(&(key, item)) {
                                at(now.ms);
                            }
                        }
                    }
                    Trig::TimeOfDay { .. } => {
                        if let Some(t) = tod.get(&key) {
                            at((*t).max(now.ms));
                        }
                    }
                    Trig::TimerZero {
                        screen,
                        layer,
                        key: field,
                    } => {
                        if let Some(z) = self.timer_zero(screen, layer, field, &now) {
                            if z > now.ms - ZERO_GRACE_MS && zero_fired.get(&key) != Some(&z) {
                                at(z.max(now.ms));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.set_wake(due, &now);
    }

    /// [`AppState::replan`] from a path that may hold the screen write lock: it never takes it,
    /// so calling it directly is fine; kept as a name for the intent.
    fn replan_later(&self) {
        self.replan();
    }

    /// Keeps, replaces or drops the planner task.
    fn set_wake(&self, due: Option<i64>, now: &Now) {
        let mut e = lock(&self.rules().inner);
        if e.wake.as_ref().map(|w| (w.at, w.real_at)) == due.map(|d| (d, now.to_real(d))) {
            return;
        }
        if let Some(w) = e.wake.take() {
            w.abort.abort();
        }
        let Some(at) = due else { return };
        let Some(handle) = self.live().runtime_handle() else {
            tracing::warn!("no async runtime: schedules and flow triggers are not planned");
            return;
        };
        let gen = next_gen();
        let delay = Duration::from_millis(u64::try_from(at - now.ms).unwrap_or(0) + 5);
        let weak = self.downgrade();
        let task = handle.spawn(async move {
            tokio::time::sleep(delay).await;
            let Some(st) = weak.upgrade() else { return };
            let _ = tokio::task::spawn_blocking(move || st.planner_tick(gen)).await;
        });
        e.wake = Some(Wake {
            at,
            real_at: now.to_real(at),
            gen,
            abort: task.abort_handle(),
        });
    }

    /// The planner woke: re-plan the watched schedules, fire what is due, sleep again.
    fn planner_tick(&self, gen: u64) {
        {
            let mut e = lock(&self.rules().inner);
            if e.wake.as_ref().is_none_or(|w| w.gen != gen) {
                return;
            }
            e.wake = None;
        }
        let mut fires = Vec::new();
        {
            let Ok((dir, _w)) = self.screen_guard() else {
                return;
            };
            for id in self.watched_schedules() {
                fires.extend(self.refresh_schedule(&dir, &id, true, false, false));
            }
        }
        fires.extend(self.due_timed_triggers());
        self.fire_all(fires);
        self.replan();
    }

    /// `beforeNext`, `timeOfDay` and `timerZero` triggers that are due now (marked as fired).
    fn due_timed_triggers(&self) -> Vec<Fire> {
        let now = self.clock_now();
        let index = self.flow_engine().index();
        let mut fires = Vec::new();
        for t in index.timed() {
            let key = t.key();
            let data = match t.trig() {
                Trig::BeforeNext { source, minutes } => match self.next_item(source, &now) {
                    Some((item, start))
                        if start - i64::from(minutes) * 60_000 <= now.ms && now.ms < start =>
                    {
                        let fresh = lock(&self.rules().inner)
                            .fired_before
                            .insert((key.clone(), item));
                        fresh.then(|| {
                            let row = self
                                .schedule_item_row(source, NowNext::Next, &now)
                                .unwrap_or_default();
                            vec![("item", Val::Row(row))]
                        })
                    }
                    _ => None,
                },
                Trig::TimeOfDay { at, days } => {
                    let mut e = lock(&self.rules().inner);
                    match e.tod_next.get(&key).copied() {
                        Some(when) if when <= now.ms => {
                            match next_time_of_day(at, &days, &now) {
                                Some(n) => e.tod_next.insert(key.clone(), n),
                                None => e.tod_next.remove(&key),
                            };
                            Some(vec![("at", Val::Time(when))])
                        }
                        _ => None,
                    }
                }
                Trig::TimerZero {
                    screen,
                    layer,
                    key: field,
                } => match self.timer_zero(screen, layer, field, &now) {
                    Some(z) if z <= now.ms && z > now.ms - ZERO_GRACE_MS => {
                        let mut e = lock(&self.rules().inner);
                        (e.zero_fired.insert(key.clone(), z) != Some(z)).then(Vec::new)
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some(data) = data {
                fires.push(Fire { t: t.clone(), data });
            }
        }
        fires
    }

    // ---- Event hooks ---------------------------------------------------------------------------

    /// After every live change (the hub's hook): `layer` triggers, and a re-plan when `timerZero`
    /// triggers exist (a timer may have started or stopped). Cheap without them.
    pub(crate) fn rules_live_changed(&self) {
        let index = self.flow_engine().index();
        if index.layers().is_empty() && !index.has_timer_zero() {
            return;
        }
        if !index.layers().is_empty() {
            let mut watched: Vec<(String, String)> = index
                .layers()
                .iter()
                .filter_map(|t| match t.trig() {
                    Trig::Layer { screen, layer, .. } => {
                        Some((screen.to_string(), layer.to_string()))
                    }
                    _ => None,
                })
                .collect();
            watched.sort();
            watched.dedup();
            let states: Vec<((String, String), bool)> = watched
                .into_iter()
                .map(|k| {
                    let on = self.live().is_on_air(&k.0, &k.1);
                    (k, on)
                })
                .collect();
            let mut changed = Vec::new();
            {
                let mut e = lock(&self.rules().inner);
                let suppress = e.suppress_layers;
                for (k, on) in states {
                    let prev = e.layer_seen.insert(k.clone(), on);
                    if !suppress && prev.is_some_and(|p| p != on) {
                        changed.push((k, on));
                    }
                }
            }
            if !changed.is_empty() {
                let fires: Vec<Fire> = index
                    .layers()
                    .iter()
                    .filter(|t| match t.trig() {
                        Trig::Layer {
                            screen,
                            layer,
                            on_air,
                        } => changed
                            .iter()
                            .any(|((s, l), on)| s == screen && l == layer && *on == on_air),
                        _ => false,
                    })
                    .map(|t| Fire {
                        t: t.clone(),
                        data: Vec::new(),
                    })
                    .collect();
                self.fire_all(fires);
            }
        }
        if index.has_timer_zero() {
            self.replan();
        }
    }

    /// An auto-advancing source completed a cycle.
    pub(crate) fn rules_slideshow_wrapped(&self, source_id: &str) {
        let index = self.flow_engine().index();
        let fires: Vec<Fire> = index
            .get(&TriggerKey::SlideshowWrap(source_id.to_string()))
            .iter()
            .map(|t| Fire {
                t: t.clone(),
                data: Vec::new(),
            })
            .collect();
        self.fire_all(fires);
    }

    /// PANIC begins: pending `forMs` clears are cancelled and layer changes fire nothing.
    pub(crate) fn rules_panic_begin(&self) {
        let mut e = lock(&self.rules().inner);
        for (_, c) in e.clears.drain() {
            c.abort.abort();
        }
        e.suppress_layers = true;
    }

    /// PANIC is over: the layer baseline is what is on air now (nothing).
    pub(crate) fn rules_panic_end(&self) {
        let keys: Vec<(String, String)> = lock(&self.rules().inner)
            .layer_seen
            .keys()
            .cloned()
            .collect();
        let states: Vec<_> = keys
            .into_iter()
            .map(|k| {
                let on = self.live().is_on_air(&k.0, &k.1);
                (k, on)
            })
            .collect();
        let mut e = lock(&self.rules().inner);
        for (k, on) in states {
            e.layer_seen.insert(k, on);
        }
        e.suppress_layers = false;
    }

    /// Automation off: pending `forMs` clears are cancelled.
    pub(crate) fn rules_cancel_clears(&self) {
        let mut e = lock(&self.rules().inner);
        for (_, c) in e.clears.drain() {
            c.abort.abort();
        }
    }

    // ---- Firing --------------------------------------------------------------------------------

    /// Hands each firing to the engine (which never runs a node inline: the caller may hold the
    /// screen write lock).
    fn fire_all(&self, fires: Vec<Fire>) {
        for f in fires {
            self.flow_trigger(&f.t, f.data);
        }
    }

    /// Pauses or resumes a source's auto-advance (saved). `invalid` without auto-advance.
    pub fn set_auto_paused(&self, source_id: &str, paused: bool) -> AppResult<()> {
        let doc = self.loaded_source(source_id)?;
        let Some(mut a) = sources::auto_of(&doc) else {
            return Err(AppError::Invalid(format!(
                "\u{201c}{}\u{201d} has no auto-advance to pause",
                doc.meta.name
            )));
        };
        if a.paused != paused {
            a.paused = paused;
            self.set_auto_advance(source_id, Some(a))?;
        }
        Ok(())
    }

    /// `take … for N ms`: clears the layer after `ms`, if it is still in the same on-air period.
    pub(crate) fn clear_later(&self, screen_id: &str, layer_id: &str, ms: u32) {
        let Ok(out_gen) = self.live().with_layer(screen_id, layer_id, |l| l.out_gen) else {
            return;
        };
        let Some(handle) = self.live().runtime_handle() else {
            return;
        };
        let gen = next_gen();
        let key = (screen_id.to_string(), layer_id.to_string());
        let weak = self.downgrade();
        let task_key = key.clone();
        let mut e = lock(&self.rules().inner);
        let task = handle.spawn(async move {
            tokio::time::sleep(Duration::from_millis(u64::from(ms))).await;
            let Some(st) = weak.upgrade() else { return };
            let _ = tokio::task::spawn_blocking(move || {
                let mine = {
                    let mut e = lock(&st.rules().inner);
                    let mine = e.clears.get(&task_key).is_some_and(|c| c.gen == gen);
                    if mine {
                        e.clears.remove(&task_key);
                    }
                    mine
                };
                let (s, l) = &task_key;
                let same = st
                    .live()
                    .with_layer(s, l, |x| x.on_air && x.out_gen == out_gen)
                    .unwrap_or(false);
                if mine && same {
                    let _ = st.live().hide(s, l);
                }
            })
            .await;
        });
        if let Some(old) = e.clears.insert(
            key,
            PendingClear {
                gen,
                abort: task.abort_handle(),
            },
        ) {
            old.abort.abort();
        }
    }
}

#[cfg(test)]
mod tests;
