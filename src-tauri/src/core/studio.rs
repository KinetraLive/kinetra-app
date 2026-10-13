//! Studio mode (Preview / Program) on `AppState`: the settings (`ui-state.json`,
//! `control.studio`), THE entry point for takes and cues ([`AppState::apply_studio_op`]) and the
//! persisted Preview cues.
//!
//! The Preview is a diff on top of Program, kept per screen in the live state
//! (`core::live::ScreenPreview`); `LiveHub::studio_locked` applies every change to it. In Studio
//! mode every content edit (Control field and variant edits, saves, drags, watcher reloads,
//! operator source steps) goes to the diff through that same function, while automation (flows,
//! timers, auto-advance, recipes) keeps going straight to Program: the diff stays on top
//! (rebased; its values win) and the layer is marked `programChanged`.

use crate::core::live::{is_reserved_screen, PreviewView};
use crate::core::live_hub::StudioAction;
use crate::core::model::{StudioExit, StudioOp, StudioSettings};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

impl AppState {
    /// The open project's Studio settings (off when none are stored or no project is open).
    pub fn studio_settings(&self) -> StudioSettings {
        self.ui_state()
            .ok()
            .and_then(|u| u.control.studio)
            .unwrap_or_default()
    }

    /// Stores the Studio settings (`ms` clamped into 100..=5000) and turns Studio mode on or off.
    /// Off while a Preview has changes needs `exit` (`take`: take every Preview with the
    /// default transition; `discard`; `keepStaged`): `invalid` "Preview has changes" otherwise,
    /// and nothing changes.
    pub fn set_studio(&self, settings: StudioSettings, exit: Option<StudioExit>) -> AppResult<()> {
        self.project_dir()?;
        let settings = settings.clamped();
        self.live()
            .set_studio(settings.enabled, exit, settings.transition)?;
        self.update_ui_state(|ui| ui.control.studio = Some(settings))?;
        self.save_studio_cues();
        self.emit_ui_state_changed();
        Ok(())
    }

    /// THE studio entry point for takes, cues, the T-bar and clearing a Preview, from every path
    /// (Control and hotkeys through the `studio_op` command, the palette, Flows' Take preview,
    /// later REST). Layer locks will be enforced here (and in `LiveHub::studio_locked`, which
    /// also carries the Preview edits). `invalid` when Studio mode is off; `notFound` for an
    /// unknown screen or layer. Returns the screen's Preview afterwards.
    pub fn apply_studio_op(&self, screen_id: &str, op: StudioOp) -> AppResult<PreviewView> {
        self.project_dir()?;
        if is_reserved_screen(screen_id) {
            return Err(AppError::NotFound(format!("Screen not found: {screen_id}")));
        }
        let settings = self.studio_settings();
        if !settings.enabled || !self.live().is_studio() {
            return Err(AppError::Invalid("Studio mode is off".into()));
        }
        let after = settings.preview_after_take;
        let view = match &op {
            StudioOp::Cue { layer_id, visible } => self.live().studio_op(
                screen_id,
                StudioAction::Cue {
                    layer_id,
                    visible: *visible,
                },
            ),
            StudioOp::Take { transition } => self.live().studio_op(
                screen_id,
                StudioAction::Take {
                    transition: transition.unwrap_or(settings.transition).clamped(),
                    after,
                },
            ),
            // The T-bar is a manual crossfade: its transition setting is not used.
            StudioOp::TbarBegin { .. } => self.live().studio_op(screen_id, StudioAction::TbarBegin),
            StudioOp::TbarSet { p } => self
                .live()
                .studio_op(screen_id, StudioAction::TbarSet { p: *p }),
            StudioOp::TbarEnd { commit } => self.live().studio_op(
                screen_id,
                StudioAction::TbarEnd {
                    commit: *commit,
                    after,
                },
            ),
            StudioOp::Clear {} => self.live().studio_op(screen_id, StudioAction::Clear),
        }?;
        Ok(view)
    }

    /// A screen's Preview (Program + diff).
    pub fn preview_view(&self, screen_id: &str) -> AppResult<PreviewView> {
        self.project_dir()?;
        if is_reserved_screen(screen_id) {
            return Err(AppError::NotFound(format!("Screen not found: {screen_id}")));
        }
        Ok(self.live().preview_view(screen_id)?)
    }

    /// Saves the hub's Preview cues into `ui-state.json` when they differ from the stored ones
    /// (the store flusher coalesces the writes). Called by the hub's cues hook.
    pub(crate) fn save_studio_cues(&self) {
        let cues = self.live().studio_cues();
        let _ = self.update_ui_state(|ui| {
            if ui.control.studio_preview != cues {
                ui.control.studio_preview = cues;
            }
        });
    }
}

#[cfg(test)]
mod tests;
