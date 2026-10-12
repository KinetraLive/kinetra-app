//! The main window's close button (X, Alt+F4, the taskbar's Close window) goes through the same
//! question as File › Exit: with a project open, the close is prevented and `close-requested` is
//! sent to the main window, whose menu sync runs the `app.exit` command (it asks first while
//! anything is on air or outputs are showing, else exits at once). The page answers with
//! `close_request_ack` as soon as it gets the event.
//!
//! Safety net: a page that never answers (crashed or hung) can't keep Kinetra open: a second
//! close request within [`UNANSWERED_FOR`] of an unanswered one exits anyway (stores flushed
//! first, like `exit_app`). The picker (no project open) closes at once.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tauri::{Emitter, Manager, State, Window};

use crate::commands::app::MAIN_WINDOW;
use crate::core::events::names;
use crate::core::state::AppState;

/// How long an unanswered close request lets a second one exit at once.
pub const UNANSWERED_FOR: Duration = Duration::from_secs(5);

/// What a close request of the main window does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Close the window (no project: the picker); its destruction exits Kinetra.
    Close,
    /// Prevent it and ask the page (`close-requested`).
    Ask,
    /// Prevent it, flush and exit: the page didn't answer the last request.
    Exit,
}

/// The close requests sent to the page and not answered yet (pure; the time is passed in).
#[derive(Debug, Default)]
pub struct CloseGuard {
    /// When the oldest unanswered request was sent to the page.
    asked_at: Mutex<Option<Instant>>,
}

impl CloseGuard {
    /// Decides a close request at `now`.
    pub fn request(&self, now: Instant, project_open: bool) -> CloseAction {
        let mut asked = self.asked_at.lock().unwrap_or_else(PoisonError::into_inner);
        if !project_open {
            *asked = None;
            return CloseAction::Close;
        }
        match *asked {
            Some(t) if now.saturating_duration_since(t) < UNANSWERED_FOR => CloseAction::Exit,
            // None, or an old one the page never answered: ask again (a hung page then needs
            // the second click within the window).
            _ => {
                *asked = Some(now);
                CloseAction::Ask
            }
        }
    }

    /// The page got the request (it asks the operator or exits itself).
    pub fn answered(&self) {
        *self.asked_at.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The main window's `CloseRequested`: returns whether to prevent the close.
pub fn on_main_close_requested(window: &Window) -> bool {
    let app = window.app_handle();
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let Some(guard) = app.try_state::<CloseGuard>() else {
        return false;
    };
    let open = state.project_dir().is_ok();
    match guard.request(Instant::now(), open) {
        CloseAction::Close => false,
        CloseAction::Ask => {
            if let Err(e) = app.emit_to(MAIN_WINDOW, names::CLOSE_REQUESTED, ()) {
                // No page to ask: close like the picker.
                tracing::warn!("close-requested not sent: {e}");
                return false;
            }
            true
        }
        CloseAction::Exit => {
            tracing::warn!("the main window didn't answer a close request: exiting");
            state.flush_all();
            app.exit(0);
            true
        }
    }
}

/// The main window got `close-requested` (see the module docs).
#[tauri::command]
pub fn close_request_ack(guard: State<'_, CloseGuard>) {
    guard.answered();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picker_closes_at_once() {
        let g = CloseGuard::default();
        let t = Instant::now();
        assert_eq!(g.request(t, false), CloseAction::Close);
        assert_eq!(g.request(t, false), CloseAction::Close);
    }

    #[test]
    fn with_a_project_the_page_is_asked_each_time_it_answers() {
        let g = CloseGuard::default();
        let t = Instant::now();
        assert_eq!(g.request(t, true), CloseAction::Ask);
        g.answered();
        // The operator canceled, then clicked X again: asked again, not exited.
        assert_eq!(
            g.request(t + Duration::from_secs(1), true),
            CloseAction::Ask
        );
        g.answered();
        assert_eq!(
            g.request(t + Duration::from_millis(1100), true),
            CloseAction::Ask
        );
    }

    #[test]
    fn a_second_request_while_unanswered_exits() {
        let g = CloseGuard::default();
        let t = Instant::now();
        assert_eq!(g.request(t, true), CloseAction::Ask);
        assert_eq!(
            g.request(t + Duration::from_millis(4900), true),
            CloseAction::Exit
        );
    }

    #[test]
    fn an_old_unanswered_request_asks_again() {
        let g = CloseGuard::default();
        let t = Instant::now();
        assert_eq!(g.request(t, true), CloseAction::Ask);
        let later = t + UNANSWERED_FOR;
        assert_eq!(g.request(later, true), CloseAction::Ask);
        // The window restarts from the new request.
        assert_eq!(
            g.request(later + Duration::from_secs(4), true),
            CloseAction::Exit
        );
    }

    #[test]
    fn closing_the_project_resets_it() {
        let g = CloseGuard::default();
        let t = Instant::now();
        assert_eq!(g.request(t, true), CloseAction::Ask);
        // The page closed the project instead (back to the picker): the X closes at once.
        assert_eq!(
            g.request(t + Duration::from_secs(1), false),
            CloseAction::Close
        );
        assert_eq!(
            g.request(t + Duration::from_secs(2), true),
            CloseAction::Ask
        );
    }
}
