//! THE entry point for external controls: MIDI, OSC, the Control API and anything else
//! ([`AppState::run_control`]). It parses the registry command ids the UI binds (the same ids as
//! the command palette and key bindings) and calls the same `AppState` methods Control and Flows
//! use, so every live rule (Studio mode, Live edit, PANIC) applies exactly as for the operator.
//!
//! # Commands (act on [`ControlInput::Press`]; `Release` and `Level` do nothing)
//! | id | off Studio | Studio mode |
//! |---|---|---|
//! | `live.toggle:<s>/<l>` | TAKE when off air, CLEAR when on air | cue in / out of Preview (like Control's toggle) |
//! | `live.take:<s>/<l>` | TAKE (on air with staged changes: applies them) | cue in |
//! | `live.clear:<s>/<l>` | CLEAR (animated out) | cue out |
//! | `live.update:<s>/<l>` | on air: apply staged changes (the take path); off air: nothing | take the screen's Preview |
//! | `cue.go:<flowId>` | run the flow from its Cue trigger (works with Automation off) | same |
//! | `source.next:<id>` / `source.prev:<id>` | step the source by ±1 | same |
//! | `live.clearScreen:<s>` | every on-air layer of the screen out | same |
//! | `live.clearAll` / `live.panic` | Clear all / PANIC | same |
//! | `media.play|pause|stop:<s>/<l>` | media layer transport (Program) | same |
//!
//! # Values (faders / knobs)
//! `ControlTarget::Value` sets a layer's opacity (0–100 %), a `range` variable (min..max, snapped
//! to its step) or a `number` field (0–127, the CC value) from a level 0..=1 (`Press` = 1). It is
//! an operator edit: Studio mode → the Preview diff, `onTake` on air → staged, `immediate` → on
//! air; saved to the screen coalesced (content flusher). Values are paced per target: the first
//! value applies at once, then at most one apply per [`PACE`] (≤ 30/s) with the latest value
//! winning, driven by one sleeping tokio task that exists only while values keep arriving.
//!
//! `run_control` may block briefly (screen lock, a template read the first time): call it from a
//! thread that may block (MIDI callback thread, `spawn_blocking`). Unknown targets are errors for
//! the caller to log; nothing panics.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::core::live::{is_reserved_screen, LiveError};
use crate::core::model::{
    ControlTarget, MediaAction, ScreenLayerKind, SourceStep, StudioOp, ValueTarget, VariableKind,
};
use crate::core::protocol::FieldType;
use crate::core::state::AppState;
use crate::core::templates::is_id;
use crate::core::vars;
use crate::error::{AppError, AppResult};

/// What arrived from the control surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ControlInput {
    /// A button went down (note-on with velocity > 0, a CC crossing 64 upwards, an OSC trigger).
    Press,
    /// A button went up.
    Release,
    /// A fader / knob position, 0..=1.
    Level(f32),
}

/// A parsed command id (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlCommand<'a> {
    Toggle {
        screen: &'a str,
        layer: &'a str,
    },
    Take {
        screen: &'a str,
        layer: &'a str,
    },
    Clear {
        screen: &'a str,
        layer: &'a str,
    },
    Update {
        screen: &'a str,
        layer: &'a str,
    },
    CueGo {
        flow: &'a str,
    },
    SourceNext {
        source: &'a str,
    },
    SourcePrev {
        source: &'a str,
    },
    ClearScreen {
        screen: &'a str,
    },
    ClearAll,
    Panic,
    /// Media transport (play, pause, stop) on a media layer's Program playback.
    Media {
        screen: &'a str,
        layer: &'a str,
        action: MediaAction,
    },
}

impl ControlCommand<'_> {
    /// The layer a command acts on (`live.toggle|take|clear|update`), for LED feedback.
    pub fn layer(&self) -> Option<(&str, &str)> {
        match *self {
            Self::Toggle { screen, layer }
            | Self::Take { screen, layer }
            | Self::Clear { screen, layer }
            | Self::Update { screen, layer } => Some((screen, layer)),
            _ => None,
        }
    }
}

fn layer_ref(rest: &str) -> Option<(&str, &str)> {
    let (s, l) = rest.split_once('/')?;
    (is_id(s) && is_id(l)).then_some((s, l))
}

fn one_id(rest: &str) -> Option<&str> {
    is_id(rest).then_some(rest)
}

/// Parses a registry command id the core understands; `None` for anything else.
pub fn parse_command(id: &str) -> Option<ControlCommand<'_>> {
    match id {
        "live.clearAll" => return Some(ControlCommand::ClearAll),
        "live.panic" => return Some(ControlCommand::Panic),
        _ => {}
    }
    let (head, rest) = id.split_once(':')?;
    Some(match head {
        "live.toggle" => {
            layer_ref(rest).map(|(screen, layer)| ControlCommand::Toggle { screen, layer })?
        }
        "live.take" => {
            layer_ref(rest).map(|(screen, layer)| ControlCommand::Take { screen, layer })?
        }
        "live.clear" => {
            layer_ref(rest).map(|(screen, layer)| ControlCommand::Clear { screen, layer })?
        }
        "live.update" => {
            layer_ref(rest).map(|(screen, layer)| ControlCommand::Update { screen, layer })?
        }
        "cue.go" => ControlCommand::CueGo {
            flow: one_id(rest)?,
        },
        "source.next" => ControlCommand::SourceNext {
            source: one_id(rest)?,
        },
        "source.prev" => ControlCommand::SourcePrev {
            source: one_id(rest)?,
        },
        "live.clearScreen" => ControlCommand::ClearScreen {
            screen: one_id(rest)?,
        },
        "media.play" | "media.pause" | "media.stop" => {
            let (screen, layer) = layer_ref(rest)?;
            let action = match head {
                "media.play" => MediaAction::Play,
                "media.pause" => MediaAction::Pause,
                _ => MediaAction::Stop,
            };
            ControlCommand::Media {
                screen,
                layer,
                action,
            }
        }
        _ => return None,
    })
}

// ---- Pacing ---------------------------------------------------------------------------------

/// Shortest time between two applies of one value target (≤ 30 per second).
pub const PACE: Duration = Duration::from_millis(34);
/// Idle pacing entries are dropped past this many.
const PACE_ENTRIES_MAX: usize = 256;

type PaceKey = (String, String, ValueTarget);

struct Pace {
    /// No apply before this.
    next_at: Instant,
    /// The newest value not applied yet.
    latest: Option<f32>,
    /// A task is sleeping until `next_at` to apply `latest`.
    sleeping: bool,
}

/// Value pacing state, one entry per target that received values recently.
#[derive(Default)]
pub struct Pacer {
    map: Mutex<HashMap<PaceKey, Pace>>,
    /// Paced applies so far (tests).
    applies: std::sync::atomic::AtomicU64,
}

enum Paced {
    Now,
    Later { spawn: bool },
}

impl Pacer {
    fn offer(&self, key: &PaceKey, v: f32) -> Paced {
        let now = Instant::now();
        let mut m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        if m.len() > PACE_ENTRIES_MAX {
            m.retain(|_, p| p.sleeping || p.next_at > now);
        }
        let p = m.entry(key.clone()).or_insert(Pace {
            next_at: now,
            latest: None,
            sleeping: false,
        });
        if !p.sleeping && now >= p.next_at {
            p.next_at = now + PACE;
            p.latest = None;
            return Paced::Now;
        }
        p.latest = Some(v);
        let spawn = !p.sleeping;
        p.sleeping = true;
        Paced::Later { spawn }
    }

    /// When the sleeping task should wake next.
    fn wake_at(&self, key: &PaceKey) -> Option<Instant> {
        let m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        m.get(key).map(|p| p.next_at)
    }

    /// The value to apply now (and the next slot taken), or `None`: the task ends.
    fn due(&self, key: &PaceKey) -> Option<f32> {
        let mut m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        let p = m.get_mut(key)?;
        match p.latest.take() {
            Some(v) => {
                p.next_at = Instant::now() + PACE;
                Some(v)
            }
            None => {
                m.remove(key);
                None
            }
        }
    }

    fn forget(&self, key: &PaceKey) {
        let mut m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        m.remove(key);
    }

    /// Value applies so far (tests and diagnostics).
    pub fn applies(&self) -> u64 {
        self.applies.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether a pacing task is sleeping (tests: 0 tasks at rest).
    pub fn sleeping(&self) -> usize {
        let m = self.map.lock().unwrap_or_else(PoisonError::into_inner);
        m.values().filter(|p| p.sleeping).count()
    }
}

/// A level 0..=1 as a fader value of `0..=max` (rounded).
fn scaled(level: f32, max: f64) -> f64 {
    (f64::from(level) * max).round()
}

impl AppState {
    /// Runs one control input on a target (see the module docs). `NoProject` without a project,
    /// `notFound` for an unknown screen / layer / flow / source, `invalid` for an unknown command
    /// id, a value target of the wrong type or a level outside 0..=1.
    pub fn run_control(&self, target: &ControlTarget, input: ControlInput) -> AppResult<()> {
        self.project_dir()?;
        match target {
            ControlTarget::Command { command_id } => {
                let cmd = parse_command(command_id)
                    .ok_or_else(|| AppError::Invalid(format!("Unknown action: {command_id}")))?;
                match input {
                    ControlInput::Press => self.run_command(cmd),
                    ControlInput::Release | ControlInput::Level(_) => Ok(()),
                }
            }
            ControlTarget::Value {
                screen_id,
                layer_id,
                target,
            } => {
                let level = match input {
                    ControlInput::Press => 1.0,
                    ControlInput::Release => return Ok(()),
                    ControlInput::Level(v) if (0.0..=1.0).contains(&v) => v,
                    ControlInput::Level(v) => {
                        return Err(AppError::Invalid(format!("Level out of range: {v}")))
                    }
                };
                self.paced_value(screen_id, layer_id, target, level)
            }
        }
    }

    fn run_command(&self, cmd: ControlCommand<'_>) -> AppResult<()> {
        let studio = self.live().is_studio();
        match cmd {
            ControlCommand::Toggle { screen, layer } => {
                if studio {
                    let shown = self.preview_shows(screen, layer)?;
                    self.studio_cue(screen, layer, !shown)
                } else if self.live().layer_view(screen, layer)?.on_air {
                    self.live()
                        .hide(screen, layer)
                        .map(drop)
                        .map_err(Into::into)
                } else {
                    self.live()
                        .show(screen, layer)
                        .map(drop)
                        .map_err(Into::into)
                }
            }
            ControlCommand::Take { screen, layer } => {
                if studio {
                    self.studio_cue(screen, layer, true)
                } else {
                    self.live()
                        .show(screen, layer)
                        .map(drop)
                        .map_err(Into::into)
                }
            }
            ControlCommand::Clear { screen, layer } => {
                if studio {
                    self.studio_cue(screen, layer, false)
                } else {
                    self.live()
                        .hide(screen, layer)
                        .map(drop)
                        .map_err(Into::into)
                }
            }
            ControlCommand::Update { screen, layer } => {
                if studio {
                    // Staged content reaches Program only with a take of the Preview.
                    self.live().layer_view(screen, layer)?;
                    self.apply_studio_op(screen, StudioOp::Take { transition: None })
                        .map(drop)
                } else if self.live().layer_view(screen, layer)?.on_air {
                    // `show` on an on-air layer applies its staged content without replaying it.
                    self.live()
                        .show(screen, layer)
                        .map(drop)
                        .map_err(Into::into)
                } else {
                    Ok(())
                }
            }
            ControlCommand::CueGo { flow } => self.run_flow(flow, None),
            ControlCommand::SourceNext { source } => self
                .step_source(source, SourceStep::By { delta: 1 })
                .map(drop),
            ControlCommand::SourcePrev { source } => self
                .step_source(source, SourceStep::By { delta: -1 })
                .map(drop),
            ControlCommand::ClearScreen { screen } => {
                if is_reserved_screen(screen) {
                    return Err(AppError::NotFound(format!("Screen not found: {screen}")));
                }
                self.live_clear(screen).map(drop)
            }
            ControlCommand::ClearAll => self.live_clear_all().map(drop),
            ControlCommand::Panic => self.live_panic().map(drop),
            ControlCommand::Media {
                screen,
                layer,
                action,
            } => self
                .live()
                .media_control(screen, layer, action, None)
                .map(drop)
                .map_err(Into::into),
        }
    }

    /// Whether the layer shows in the screen's Preview (Studio mode).
    fn preview_shows(&self, screen: &str, layer: &str) -> AppResult<bool> {
        if is_reserved_screen(screen) {
            return Err(AppError::NotFound(format!("Screen not found: {screen}")));
        }
        let pv = self.live().preview_view(screen)?;
        pv.layers
            .iter()
            .find(|l| l.id == layer)
            .map(|l| l.on_air)
            .ok_or_else(|| LiveError::UnknownLayer(layer.to_string()).into())
    }

    /// Cues a layer in (`visible`) or out of Preview, back to "as Program" when that is what
    /// Program shows (like Control's cue toggle).
    fn studio_cue(&self, screen: &str, layer: &str, visible: bool) -> AppResult<()> {
        if is_reserved_screen(screen) {
            return Err(AppError::NotFound(format!("Screen not found: {screen}")));
        }
        let program = self.live().layer_view(screen, layer)?.on_air;
        let want = (visible != program).then_some(visible);
        self.apply_studio_op(
            screen,
            StudioOp::Cue {
                layer_id: layer.to_string(),
                visible: want,
            },
        )
        .map(drop)
    }

    fn paced_value(
        &self,
        screen: &str,
        layer: &str,
        target: &ValueTarget,
        level: f32,
    ) -> AppResult<()> {
        if is_reserved_screen(screen) {
            return Err(AppError::NotFound(format!("Screen not found: {screen}")));
        }
        // Unknown screens and layers are errors at once, also for a value that would wait.
        self.live().with_layer(screen, layer, |_| ())?;
        let key: PaceKey = (screen.to_string(), layer.to_string(), target.clone());
        let pacer = &self.control_runtime().pacer;
        match pacer.offer(&key, level) {
            Paced::Now => self.apply_control_value(screen, layer, target, level),
            Paced::Later { spawn: false } => Ok(()),
            Paced::Later { spawn: true } => {
                let Some(rt) = self.live().runtime_handle() else {
                    // No runtime (some tests): apply now, unpaced.
                    pacer.forget(&key);
                    return self.apply_control_value(screen, layer, target, level);
                };
                let weak = self.downgrade();
                rt.spawn(async move {
                    loop {
                        let Some(at) = weak
                            .upgrade()
                            .and_then(|st| st.control_runtime().pacer.wake_at(&key))
                        else {
                            return;
                        };
                        tokio::time::sleep_until(at.into()).await;
                        let Some(st) = weak.upgrade() else { return };
                        let Some(v) = st.control_runtime().pacer.due(&key) else {
                            return;
                        };
                        let k = key.clone();
                        let r = tokio::task::spawn_blocking(move || {
                            st.apply_control_value(&k.0, &k.1, &k.2, v)
                        })
                        .await;
                        match r {
                            Ok(Err(e)) => tracing::debug!("control value {:?}: {e}", key.2),
                            Err(e) => tracing::warn!("control value task: {e}"),
                            Ok(Ok(())) => {}
                        }
                    }
                });
                Ok(())
            }
        }
    }

    /// Sets one layer value from a level 0..=1 as an operator edit (see the module docs). No-op
    /// when the value doesn't change.
    pub(crate) fn apply_control_value(
        &self,
        screen_id: &str,
        layer_id: &str,
        target: &ValueTarget,
        level: f32,
    ) -> AppResult<()> {
        self.control_runtime()
            .pacer
            .applies
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (dir, _w) = self.screen_guard()?;
        let not_found = || AppError::NotFound(format!("Screen not found: {screen_id}"));
        let mut doc = self.screens().doc(screen_id).ok_or_else(not_found)?;
        let templates = self.screens().templates_for(&dir, &doc);
        let layer = doc
            .meta
            .layers
            .iter_mut()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))?;
        let (template_id, fields, overrides, bindings, opacity) = match &mut layer.kind {
            ScreenLayerKind::Template {
                template_id,
                fields,
                vars,
                bindings,
                opacity,
                ..
            } => (
                Some(template_id),
                Some(fields),
                Some(vars),
                Some(bindings),
                opacity,
            ),
            ScreenLayerKind::Media { opacity, .. } => (None, None, None, None, opacity),
        };
        let template = || -> AppResult<_> {
            template_id
                .as_ref()
                .and_then(|id| templates.get(id.as_str()))
                .ok_or_else(|| AppError::Invalid("The layer's template is missing".to_string()))
        };
        let changed = match target {
            ValueTarget::Opacity => {
                let o = scaled(level, 100.0) as u8;
                let next = (o < 100).then_some(o);
                let changed = *opacity != next;
                *opacity = next;
                changed
            }
            ValueTarget::Var { key } => {
                let t = template()?;
                let def = t
                    .meta
                    .variables
                    .iter()
                    .find(|d| &d.key == key)
                    .ok_or_else(|| AppError::NotFound(format!("Variable not found: {key}")))?;
                let VariableKind::Range { min, max, step, .. } = &def.kind else {
                    return Err(AppError::Invalid(format!(
                        "\u{201c}{}\u{201d} isn't a range, so a fader can't set it",
                        def.label
                    )));
                };
                let (min, max) = (min.min(*max), min.max(*max));
                let mut v = min + f64::from(level) * (max - min);
                if *step > 0.0 {
                    v = min + ((v - min) / step).round() * step;
                }
                let v = vars::format_number(v.clamp(min, max));
                let Some(overrides) = overrides else {
                    return Ok(());
                };
                let changed = overrides.get(key) != Some(&v);
                overrides.insert(key.clone(), v);
                changed
            }
            ValueTarget::Field { key } => {
                let t = template()?;
                let def = t
                    .meta
                    .fields
                    .iter()
                    .find(|f| &f.key == key)
                    .ok_or_else(|| LiveError::UnknownField(key.clone()))?;
                if def.kind != FieldType::Number {
                    return Err(AppError::Invalid(format!(
                        "\u{201c}{}\u{201d} isn't a number, so a fader can't set it",
                        def.label
                    )));
                }
                let (Some(fields), Some(bindings)) = (fields, bindings) else {
                    return Ok(());
                };
                if bindings.contains_key(key) {
                    return Err(AppError::Invalid(format!(
                        "Field \"{key}\" is bound to a source: change the source instead"
                    )));
                }
                let v = format!("{}", scaled(level, 127.0) as u32);
                let changed = fields.get(key) != Some(&v);
                fields.insert(key.clone(), v);
                changed
            }
        };
        if !changed {
            return Ok(());
        }
        self.apply_screen_doc(&dir, &doc)?;
        self.screens().mark_dirty(screen_id);
        self.schedule_content_flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
