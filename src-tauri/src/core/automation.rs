//! Automation: Control timers and source auto-advance (rotation); macros are now Cue flows
//! (`core::flow_engine`). All of it runs
//! server-side as tokio tasks on the hub's runtime (Tauri's in the app), so it keeps going
//! whichever page is open, and stops with the project. Automation changes go on air directly:
//! they bypass `liveEdit = onTake` for the keys they change, while other staged changes of the
//! layer stay staged (`LiveHub::force_fields`).
//!
//! # Timers
//! [`AppState::set_timer`] computes the new `TimerState` with server time, saves it in the screen
//! (coalesced, like Control field edits) and puts it on air at once: one `fields` message, no
//! ticks. A countdown with `end: hide` is taken off air at zero by the hub (see `live_hub`).
//!
//! # Auto-advance
//! A text source with `auto` is *armed* while at least one layer bound to it is on air (and it
//! has a row to step to: more than one row, and without `loop` not at the last row yet). Arming is
//! re-evaluated after every live change on any path (the hub's hook: show, hide, clear, PANIC,
//! auto-out, countdown end, screen apply, reset), and after source saves, steps and deletes
//! (they re-apply bound screens) and `set_auto_advance`. One tokio task per armed source sleeps
//! `everyMs` and then steps it (+1; with `loop` the last row wraps to the first):
//! - `update`: the bound screens are re-applied by the live rules and the bound fields of on-air
//!   layers are forced on air (one `fields` message per bound layer);
//! - `retake`: the bound on-air layers go out (`state out`), and after [`RETAKE_GAP`] the source
//!   steps and those layers come back in (each only if nothing took it on or off air meanwhile).
//!
//! Slideshow settings: `paused` keeps the settings but disarms; `itemDurations` makes a row's
//! `duration` cell (seconds) its interval; `shuffle` steps through a random permutation of the
//! rows, reshuffled every cycle (in memory; the next cycle never starts with the row the last one
//! ended on). A completed cycle (with `loop`: the step into the next cycle; without: the step onto
//! the last row) fires the flows' `trigger.slideshowWrap`; every step fires `trigger.sourceRow`.
//!
//! `source-changed` (the `SourceSummary` with `nextAt`) is emitted on every automatic step and
//! whenever a source is armed, re-armed with new settings or disarmed. PANIC and closing the
//! project disarm everything. While Automation is off nothing is armed (rotations hold and resume
//! from the current row when it is switched back on). No task exists while nothing is armed.
//!
//! Locking: the automation lock is held only briefly and never while calling the hub, the screen
//! store or the source store (events are emitted outside it).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;

use crate::core::events::names;
use crate::core::fieldvals;
use crate::core::fsx;
use crate::core::live::{is_reserved_screen, LiveError, LiveLayerView};
use crate::core::model::{
    AutoAdvance, AutoTransition, ScreenLayerKind, SourceDoc, SourceStep, SourceSummary, TimerAction,
};
use crate::core::protocol::FieldType;
use crate::core::protocol::Fields;
use crate::core::screens;
use crate::core::sources;
use crate::core::state::{AppState, WeakState};
use crate::error::{AppError, AppResult};

/// `retake`: time between taking the bound layers out and bringing them back with the next row.
/// Covers typical out-animations (240–600 ms) with a margin; the renderer's hard cap (3 s) is for
/// pathological templates and would make a rotation feel sluggish.
pub const RETAKE_GAP: Duration = Duration::from_millis(900);

static GEN: AtomicU64 = AtomicU64::new(1);

fn next_gen() -> u64 {
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// An armed source.
struct Armed {
    auto: AutoAdvance,
    gen: u64,
    /// Server epoch ms of the next step.
    next_at: u64,
    /// Between the out and the in of a `retake`: kept armed although its layers are off air.
    retaking: bool,
    /// The layers that retake took out (screen, layer, out generation after the hide).
    retake_layers: Vec<(String, String, u64)>,
    abort: AbortHandle,
}

#[derive(Default)]
struct Auto {
    armed: HashMap<String, Armed>,
    /// Shuffled slideshows: the current cycle's order of row indexes.
    shuffles: HashMap<String, Vec<u32>>,
    /// xorshift state for shuffling (no rand crate).
    rng: u64,
    /// Ticket of the newest `rearm` applied (see [`Automation::rearm_ticket`]).
    rearm_applied: u64,
}

impl Auto {
    fn random(&mut self) -> u64 {
        if self.rng == 0 {
            self.rng = (fsx::now_epoch_ms() ^ 0x9E37_79B9_7F4A_7C15) | 1;
        }
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// A random permutation of `0..n` (Fisher-Yates).
    fn permutation(&mut self, n: usize) -> Vec<u32> {
        let mut v: Vec<u32> = (0..n as u32).collect();
        for i in (1..n).rev() {
            let j = (self.random() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
        v
    }

    /// The row after `index` in the shuffled order of an `n`-row source, and whether that step
    /// completes a cycle (see the module docs). `None` when a cycle without `loop` is over.
    fn shuffled_next(
        &mut self,
        id: &str,
        n: usize,
        index: u32,
        looped: bool,
    ) -> Option<(u32, bool)> {
        if n < 2 {
            return None;
        }
        let valid = self
            .shuffles
            .get(id)
            .is_some_and(|o| o.len() == n && o.contains(&index));
        if !valid {
            // A new cycle starting at the current row.
            let mut rest: Vec<u32> = self.permutation(n);
            rest.retain(|i| *i != index);
            let mut order = vec![index];
            order.extend(rest);
            self.shuffles.insert(id.to_string(), order);
        }
        let order = self.shuffles.get(id)?.clone();
        let pos = order.iter().position(|i| *i == index)?;
        if pos + 1 < n {
            return Some((order[pos + 1], !looped && pos + 2 == n));
        }
        if !looped {
            return None;
        }
        let mut next = self.permutation(n);
        if next[0] == index {
            // No repeat across the seam.
            let j = 1 + (self.random() % (n as u64 - 1)) as usize;
            next.swap(0, j);
        }
        let first = next[0];
        self.shuffles.insert(id.to_string(), next);
        Some((first, true))
    }

    /// Whether a shuffled cycle without `loop` has reached its last row.
    fn shuffle_done(&self, id: &str, n: usize, index: u32) -> bool {
        self.shuffles
            .get(id)
            .filter(|o| o.len() == n)
            .and_then(|o| o.iter().position(|i| *i == index))
            .is_some_and(|p| p + 1 >= n)
    }
}

/// The open project's automation state (see the module docs).
pub struct Automation {
    inner: Mutex<Auto>,
    /// Test override of every auto-advance interval (0 = none).
    every_override_ms: AtomicU64,
    retake_gap_ms: AtomicU64,
    /// Tickets of `rearm` passes, taken before each reads the state it decides on. A pass
    /// whose ticket is older than one already applied read staler state, so it is dropped:
    /// otherwise a rearm from a rotation step that read "automation on" just before Automation
    /// off could re-arm a source after the off pass disarmed it.
    rearm_ticket: AtomicU64,
}

impl Default for Automation {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Auto::default()),
            every_override_ms: AtomicU64::new(0),
            retake_gap_ms: AtomicU64::new(RETAKE_GAP.as_millis() as u64),
            rearm_ticket: AtomicU64::new(0),
        }
    }
}

/// What a rotation tick asks its task to do next.
enum Tick {
    /// Sleep this long until the next step.
    Next(Duration),
    /// `retake`: wait the gap, then finish with these layers (screen, layer, out generation).
    Retake(Vec<(String, String, u64)>, (u32, bool)),
    /// No longer armed.
    Stop,
}

impl Automation {
    /// Forgets the project (closed).
    pub(crate) fn forget(&self) {
        lock(&self.inner).shuffles.clear();
    }

    /// Disarms every source. Returns the ids of the sources that were armed.
    pub(crate) fn stop_all(&self) -> Vec<String> {
        let mut a = lock(&self.inner);
        let mut disarmed = Vec::with_capacity(a.armed.len());
        for (id, arm) in a.armed.drain() {
            arm.abort.abort();
            disarmed.push(id);
        }
        disarmed
    }

    /// Server epoch ms of the source's next automatic step while armed.
    pub fn next_at(&self, source_id: &str) -> Option<u64> {
        lock(&self.inner).armed.get(source_id).map(|a| a.next_at)
    }

    /// Ids of the armed sources (for tests and diagnostics).
    pub fn armed(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.inner).armed.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Number of automation tasks alive (armed sources).
    pub fn task_count(&self) -> usize {
        lock(&self.inner).armed.len()
    }

    /// Test timing: every auto-advance interval and the retake gap.
    #[doc(hidden)]
    pub fn set_test_timing(&self, every: Option<Duration>, retake_gap: Duration) {
        let ms = every.map_or(0, |d| d.as_millis() as u64);
        self.every_override_ms.store(ms, Ordering::Relaxed);
        self.retake_gap_ms
            .store(retake_gap.as_millis() as u64, Ordering::Relaxed);
    }

    /// How long the row `row` stays up (see `sources::row_duration_ms`), or the test override.
    fn every(&self, auto: &AutoAdvance, row: Option<&Fields>) -> Duration {
        match self.every_override_ms.load(Ordering::Relaxed) {
            0 => Duration::from_millis(u64::from(sources::row_duration_ms(auto, row))),
            ms => Duration::from_millis(ms),
        }
    }

    fn retake_gap(&self) -> Duration {
        Duration::from_millis(self.retake_gap_ms.load(Ordering::Relaxed))
    }
}

impl AppState {
    // ---- Timers ------------------------------------------------------------------------------

    /// One state change of a `timer` field (see the module docs). `invalid` if `key` is not a
    /// timer field of the layer's template or is bound to a source; `notFound` for an unknown
    /// screen or layer.
    pub fn set_timer(
        &self,
        screen_id: &str,
        layer_id: &str,
        key: &str,
        action: TimerAction,
    ) -> AppResult<LiveLayerView> {
        self.set_timer_at(screen_id, layer_id, key, action, fsx::now_epoch_ms())
    }

    /// [`AppState::set_timer`] at a given server time.
    pub fn set_timer_at(
        &self,
        screen_id: &str,
        layer_id: &str,
        key: &str,
        action: TimerAction,
        now: u64,
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
            return Err(AppError::Invalid("A media layer has no timer".to_string()));
        };
        let t = templates.get(template_id.as_str()).ok_or_else(|| {
            AppError::Invalid(
                "The layer's template is missing, so its timer can't be set".to_string(),
            )
        })?;
        let def = t
            .meta
            .fields
            .iter()
            .find(|f| f.key == key && f.kind == FieldType::Timer)
            .ok_or_else(|| AppError::Invalid(format!("\"{key}\" is not a timer field")))?;
        if bindings.contains_key(key) {
            return Err(AppError::Invalid(format!(
                "Field \"{key}\" is bound to a source: change the source instead"
            )));
        }
        let default = fieldvals::default_timer(def);
        let cur = values
            .get(key)
            .and_then(|v| fieldvals::timer_state(def, v).ok())
            .unwrap_or(default);
        let next = fieldvals::timer_next(cur, action, now, default);
        let json = fieldvals::timer_json(&next);
        if values.get(key) != Some(&json) {
            values.insert(key.to_string(), json);
            let force = [(layer_id.to_string(), vec![key.to_string()])];
            self.apply_screen_doc_forcing(&dir, &doc, &force)?;
            self.screens().mark_dirty(screen_id);
            self.schedule_content_flush();
        }
        Ok(self.live().layer_view(screen_id, layer_id)?)
    }

    // ---- Auto-advance ------------------------------------------------------------------------

    /// Turns auto-advance of a text source on (validated) or off; saved in the source file
    /// (`updatedAt`). Returns the summary (with `nextAt` when it is armed right away).
    pub fn set_auto_advance(
        &self,
        source_id: &str,
        auto: Option<AutoAdvance>,
    ) -> AppResult<SourceSummary> {
        let saved = {
            let (dir, _w) = self.screen_guard()?;
            let mut doc = self.loaded_source(source_id)?;
            if let Some(a) = &auto {
                sources::validate_auto(a)?;
            }
            let Some(table) = doc.meta.kind.table_mut() else {
                return Err(AppError::Invalid(
                    "Only text and API sources can auto-advance; a schedule follows the clock"
                        .into(),
                ));
            };
            *table.auto = auto;
            let saved = sources::save(&dir, doc, false)?;
            self.sources().put(saved.clone());
            // Bound lists carry `_duration`: on air at once, like a step.
            for doc in self.screens().source_users(source_id) {
                let force = screens::bound_layers(&doc, source_id);
                if let Err(e) = self.apply_screen_doc_forcing(&dir, &doc, &force) {
                    tracing::warn!(
                        "auto-advance of {source_id}: applying screen {}: {e}",
                        doc.id
                    );
                }
            }
            saved
        };
        self.rearm();
        Ok(self.source_summary(&saved))
    }

    /// Whether a layer bound to the source is on air.
    fn source_on_air(&self, source_id: &str) -> bool {
        self.screens().source_users(source_id).iter().any(|doc| {
            screens::bound_layers(doc, source_id)
                .iter()
                .any(|(layer, _)| self.live().is_on_air(&doc.id, layer))
        })
    }

    /// Re-evaluates which sources are armed (see the module docs). Cheap when no source has
    /// auto-advance. Never takes the screen write lock.
    pub(crate) fn rearm(&self) {
        let ticket = self
            .automation()
            .rearm_ticket
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        // Automation off: rotations hold (nothing armed); they resume from the current row.
        let autos = if self.project_dir().is_ok() && !self.flow_engine().automation_off() {
            self.sources().auto_sources()
        } else {
            Vec::new()
        };
        let auto = self.automation();
        let shuffle_over = |id: &str, a: &AutoAdvance, n: usize| {
            a.shuffle
                && !a.looped
                && self
                    .sources()
                    .doc(id)
                    .is_some_and(|d| lock(&auto.inner).shuffle_done(id, n, sources::position(&d).1))
        };
        let wanted: Vec<(String, AutoAdvance)> = autos
            .iter()
            .filter(|(id, a, steppable, n)| {
                *steppable && !shuffle_over(id, a, *n) && self.source_on_air(id)
            })
            .map(|(id, a, _, _)| (id.clone(), *a))
            .collect();
        let mut changed = Vec::new();
        let mut restore = Vec::new();
        {
            let mut a = lock(&auto.inner);
            if a.rearm_applied > ticket {
                // A pass that started later (so read fresher state) already applied.
                return;
            }
            a.rearm_applied = ticket;
            if wanted.is_empty() && a.armed.is_empty() {
                return;
            }
            a.armed.retain(|id, arm| {
                let is = |w: &String, s: &AutoAdvance| w == id && *s == arm.auto;
                // Mid-retake (its layers are out on purpose) it stays while its settings do.
                let keep = wanted.iter().any(|(w, s)| is(w, s))
                    || (arm.retaking && autos.iter().any(|(w, s, _, _)| is(w, s)));
                if !keep {
                    arm.abort.abort();
                    changed.push(id.clone());
                    // Turned off mid-retake: its layers come back (without a step).
                    restore.append(&mut arm.retake_layers);
                }
                keep
            });
            for (id, settings) in wanted {
                if a.armed.contains_key(&id) {
                    continue;
                }
                if let Some(arm) = self.spawn_rotation(&id, settings) {
                    a.armed.insert(id.clone(), arm);
                    changed.push(id);
                }
            }
        }
        for (s, l, g) in restore {
            self.show_if_untouched(&s, &l, g);
        }
        changed.sort();
        changed.dedup();
        for id in changed {
            self.emit_source_changed(&id);
        }
    }

    /// Shows a layer a retake took out, unless someone took it on or off air since (its out
    /// generation changed).
    fn show_if_untouched(&self, screen_id: &str, layer_id: &str, out_gen: u64) {
        let untouched = self
            .live()
            .with_layer(screen_id, layer_id, |x| !x.on_air && x.out_gen == out_gen)
            .unwrap_or(false);
        if untouched {
            let _ = self.live().show(screen_id, layer_id);
        }
    }

    /// Spawns the rotation task of an armed source. Called with the automation lock held; the
    /// task only does work after its first sleep.
    fn spawn_rotation(&self, id: &str, settings: AutoAdvance) -> Option<Armed> {
        let Some(handle) = self.live().runtime_handle() else {
            tracing::warn!("no async runtime: auto-advance of {id} not armed");
            return None;
        };
        let row = self
            .sources()
            .doc(id)
            .map(|d| sources::current_row(&d, &self.sources().clock().now()));
        let every = self.automation().every(&settings, row.as_ref());
        let gen = next_gen();
        let weak = self.downgrade();
        let source = id.to_string();
        let task = handle.spawn(rotation_task(weak, source, gen, every));
        Some(Armed {
            auto: settings,
            gen,
            next_at: fsx::now_epoch_ms() + every.as_millis() as u64,
            retaking: false,
            retake_layers: Vec::new(),
            abort: task.abort_handle(),
        })
    }

    /// The source's settings if it is still armed with generation `gen`.
    fn armed_settings(&self, id: &str, gen: u64) -> Option<AutoAdvance> {
        lock(&self.automation().inner)
            .armed
            .get(id)
            .filter(|a| a.gen == gen)
            .map(|a| a.auto)
    }

    fn update_armed(&self, id: &str, gen: u64, f: impl FnOnce(&mut Armed)) {
        if let Some(a) = lock(&self.automation().inner)
            .armed
            .get_mut(id)
            .filter(|a| a.gen == gen)
        {
            f(a);
        }
    }

    /// One automatic step (see the module docs). Runs on the blocking pool.
    fn rotation_tick(&self, id: &str, gen: u64) -> Tick {
        let (delay, wrapped_now) = {
            let Ok((dir, _w)) = self.screen_guard() else {
                return Tick::Stop;
            };
            let Some(settings) = self.armed_settings(id, gen) else {
                return Tick::Stop;
            };
            let Some(doc) = self.sources().doc(id) else {
                return Tick::Stop;
            };
            let Some((next, wrapped)) = self.next_row(id, &doc, &settings) else {
                return Tick::Stop;
            };
            let row = sources::current_row(&doc, &self.sources().clock().now());
            let every_ms = self.automation().every(&settings, Some(&row)).as_millis() as u64;
            if settings.transition == AutoTransition::Retake {
                let on_air: Vec<(String, String)> = self
                    .screens()
                    .source_users(id)
                    .iter()
                    .flat_map(|d| {
                        screens::bound_layers(d, id)
                            .into_iter()
                            .map(|(l, _)| (d.id.clone(), l))
                    })
                    .filter(|(s, l)| self.live().is_on_air(s, l))
                    .collect();
                if !on_air.is_empty() {
                    self.update_armed(id, gen, |a| {
                        a.retaking = true;
                        a.next_at = fsx::now_epoch_ms() + every_ms;
                    });
                    let mut out = Vec::with_capacity(on_air.len());
                    for (s, l) in on_air {
                        if self.live().hide(&s, &l).is_ok() {
                            if let Ok(g) = self.live().with_layer(&s, &l, |x| x.out_gen) {
                                out.push((s, l, g));
                            }
                        }
                    }
                    let kept = out.clone();
                    self.update_armed(id, gen, |a| a.retake_layers = kept);
                    return Tick::Retake(out, (next, wrapped));
                }
            }
            self.step_and_force(&dir, id, next);
            let delay = self.row_delay(id, &settings);
            self.update_armed(id, gen, |a| {
                a.next_at = fsx::now_epoch_ms() + delay.as_millis() as u64
            });
            (delay, wrapped)
        };
        self.rearm();
        self.emit_source_changed(id);
        if wrapped_now {
            self.rules_slideshow_wrapped(id);
        }
        Tick::Next(delay)
    }

    /// The row after the current one and whether that step completes a cycle (see the module
    /// docs); `None` when there is nothing to step to.
    fn next_row(&self, id: &str, doc: &SourceDoc, settings: &AutoAdvance) -> Option<(u32, bool)> {
        let (rows, index) = sources::position(doc);
        if settings.shuffle {
            return lock(&self.automation().inner).shuffled_next(id, rows, index, settings.looped);
        }
        if (index as usize) + 1 < rows {
            Some((index + 1, !settings.looped && (index as usize) + 2 == rows))
        } else if settings.looped && rows > 1 {
            Some((0, true))
        } else {
            None
        }
    }

    /// How long the source's current row stays up.
    fn row_delay(&self, id: &str, settings: &AutoAdvance) -> Duration {
        let row = self
            .sources()
            .doc(id)
            .map(|d| sources::current_row(&d, &self.sources().clock().now()));
        self.automation().every(settings, row.as_ref())
    }

    /// The second half of a `retake`: step, then bring the layers back in. Runs on the blocking
    /// pool.
    fn retake_finish(
        &self,
        id: &str,
        gen: u64,
        layers: Vec<(String, String, u64)>,
        step: (u32, bool),
    ) -> Tick {
        let mut wrapped_now = false;
        let delay = {
            let Ok((dir, _w)) = self.screen_guard() else {
                return Tick::Stop;
            };
            let Some(settings) = self.armed_settings(id, gen) else {
                return Tick::Stop;
            };
            let Some(doc) = self.sources().doc(id) else {
                return Tick::Stop;
            };
            // The step decided before the gap, unless the rows changed meanwhile.
            let next = if (step.0 as usize) < sources::position(&doc).0 {
                Some(step)
            } else {
                self.next_row(id, &doc, &settings)
            };
            if let Some((next, wrapped)) = next {
                self.step_and_force(&dir, id, next);
                wrapped_now = wrapped;
            }
            // Only layers nobody took on or off air during the gap come back.
            for (s, l, g) in layers {
                self.show_if_untouched(&s, &l, g);
            }
            let delay = self.row_delay(id, &settings);
            self.update_armed(id, gen, |a| {
                a.retaking = false;
                a.retake_layers.clear();
                a.next_at = fsx::now_epoch_ms() + delay.as_millis() as u64;
            });
            delay
        };
        self.rearm();
        self.emit_source_changed(id);
        if wrapped_now {
            self.rules_slideshow_wrapped(id);
        }
        Tick::Next(delay)
    }

    /// Moves the source to row `index` (written coalesced) and re-applies its bound screens with
    /// the bound fields of on-air layers forced on air. Caller holds the screen write lock.
    fn step_and_force(&self, dir: &Path, id: &str, index: u32) {
        let changed = self
            .sources()
            .step(id, SourceStep::Index { index })
            .is_some_and(|(_, c)| c);
        if !changed {
            return;
        }
        self.schedule_content_flush();
        for doc in self.screens().source_users(id) {
            let force = screens::bound_layers(&doc, id);
            if let Err(e) = self.apply_screen_doc_forcing(dir, &doc, &force) {
                tracing::warn!("auto-advance of {id}: applying screen {}: {e}", doc.id);
            }
        }
        self.flow_source_row_changed(id);
    }

    pub(crate) fn emit_source_changed(&self, id: &str) {
        self.control_api_touch();
        let Some(doc) = self.sources().doc(id) else {
            return;
        };
        let summary = self.source_summary(&doc);
        match serde_json::to_value(&summary) {
            Ok(v) => self.events().emit(names::SOURCE_CHANGED, v),
            Err(e) => tracing::warn!("source-changed payload: {e}"),
        }
    }

    /// PANIC's part: every source disarmed (`source-changed` for each), every flow instance
    /// stopped.
    pub(crate) fn stop_automation(&self) {
        self.engine_panic();
        for id in self.automation().stop_all() {
            self.emit_source_changed(&id);
        }
    }
}

/// The rotation task of one armed source: sleep, step, repeat; ends when disarmed.
async fn rotation_task(weak: WeakState, id: String, gen: u64, first: Duration) {
    let mut deadline = tokio::time::Instant::now() + first;
    loop {
        tokio::time::sleep_until(deadline).await;
        let Some(st) = weak.upgrade() else { return };
        let gap = st.automation().retake_gap();
        let (s, i) = (st.clone(), id.clone());
        let tick = match tokio::task::spawn_blocking(move || s.rotation_tick(&i, gen)).await {
            Ok(t) => t,
            Err(_) => return,
        };
        drop(st);
        let every = match tick {
            Tick::Next(d) => d,
            Tick::Stop => return,
            Tick::Retake(layers, step) => {
                tokio::time::sleep(gap).await;
                let Some(st) = weak.upgrade() else { return };
                let i = id.clone();
                let done =
                    tokio::task::spawn_blocking(move || st.retake_finish(&i, gen, layers, step))
                        .await;
                match done {
                    // The next row's time runs from when it came in.
                    Ok(Tick::Next(d)) => {
                        deadline = tokio::time::Instant::now();
                        d
                    }
                    _ => return,
                }
            }
        };
        deadline += every;
        // Never try to catch up on missed steps.
        let now = tokio::time::Instant::now();
        if deadline < now {
            deadline = now + every;
        }
    }
}

#[cfg(test)]
mod tests;
