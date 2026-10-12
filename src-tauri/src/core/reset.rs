//! Scoped resets from Preferences. Project scopes take a `pre-reset` snapshot before changing
//! anything, and every reset is flushed to disk before returning.

use crate::core::model::{
    BackupReason, ControlViewState, Prefs, ProjectSettings, ResetScope, Theme, UiState,
};
use crate::core::state::AppState;
use crate::error::AppResult;

pub fn reset(state: &AppState, scope: ResetScope) -> AppResult<()> {
    state.ensure_not_moving()?;
    let project_open = state.current_project().is_some();
    match scope {
        ResetScope::Appearance => {
            // Theme lives in the app config dir, not the project: no snapshot.
            let prefs = Prefs {
                theme: Theme::System,
                zoom: None,
                ..state.prefs()
            };
            state.set_prefs(prefs);
        }
        ResetScope::ControlLayout => {
            state.ui_state()?;
            state.create_snapshot(BackupReason::PreReset)?;
            let ui = UiState {
                control: ControlViewState::default(),
                ..state.ui_state()?
            };
            state.set_ui_state(ui)?;
        }
        ResetScope::Preferences => {
            if project_open {
                state.create_snapshot(BackupReason::PreReset)?;
            }
            state.set_prefs(defaults_keeping_mode(state));
            if project_open {
                state.set_ui_state(UiState::default())?;
            }
        }
        ResetScope::Everything => {
            if project_open {
                state.create_snapshot(BackupReason::PreReset)?;
            }
            state.set_prefs(defaults_keeping_mode(state));
            state.clear_recents();
            if project_open {
                state.set_project_settings(ProjectSettings::default())?;
                state.set_ui_state(UiState::default())?;
            }
            state.flush_all();
            state.close_project();
            return Ok(());
        }
    }
    state.flush_all();
    Ok(())
}

/// Default prefs, keeping Simple / Advanced: a reset never hides or reveals features.
fn defaults_keeping_mode(state: &AppState) -> Prefs {
    Prefs {
        ui_mode: state.prefs().ui_mode,
        ..Prefs::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::backup;
    use crate::core::events::NullSink;
    use crate::core::model::{BackupPrefs, ControlView, ProjectFile};
    use crate::core::persist;
    use crate::error::AppError;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Duration;

    fn setup(tmp: &Path) -> (AppState, PathBuf) {
        let st = AppState::load_with_debounce(
            tmp.join("config"),
            Arc::new(NullSink),
            Duration::from_secs(60),
        )
        .unwrap();
        let p = st.create_project(tmp, "Show").unwrap();
        st.set_prefs(Prefs {
            theme: Theme::Dark,
            backup: BackupPrefs {
                retention: 50,
                ..Default::default()
            },
            ..Default::default()
        });
        st.set_project_settings(ProjectSettings {
            port: 9000,
            ..Default::default()
        })
        .unwrap();
        let mut ui = UiState {
            sidebar_pinned: true,
            ..Default::default()
        };
        ui.control.view = ControlView::Single;
        st.set_ui_state(ui).unwrap();
        (st, PathBuf::from(p.path))
    }

    fn pre_reset_count(dir: &Path) -> usize {
        backup::list(dir)
            .unwrap()
            .iter()
            .filter(|b| b.reason == BackupReason::PreReset)
            .count()
    }

    #[test]
    fn appearance_resets_theme_only() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, dir) = setup(tmp.path());
        reset(&st, ResetScope::Appearance).unwrap();
        assert_eq!(st.prefs().theme, Theme::System);
        assert_eq!(st.prefs().backup.retention, 50);
        assert!(st.ui_state().unwrap().sidebar_pinned);
        assert_eq!(pre_reset_count(&dir), 0);
        let saved: Prefs = persist::load_strict(&tmp.path().join("config")).unwrap();
        assert_eq!(saved.theme, Theme::System);
    }

    #[test]
    fn resets_keep_the_mode_and_appearance_resets_zoom() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, _dir) = setup(tmp.path());
        st.set_prefs(Prefs {
            ui_mode: crate::core::model::UiMode::Advanced,
            zoom: Some(1.5),
            ..st.prefs()
        });
        reset(&st, ResetScope::Appearance).unwrap();
        assert_eq!(st.prefs().zoom, None);
        assert_eq!(st.prefs().ui_mode, crate::core::model::UiMode::Advanced);
        reset(&st, ResetScope::Preferences).unwrap();
        assert_eq!(st.prefs().ui_mode, crate::core::model::UiMode::Advanced);
        reset(&st, ResetScope::Everything).unwrap();
        assert_eq!(st.prefs().ui_mode, crate::core::model::UiMode::Advanced);
    }

    #[test]
    fn control_layout_resets_control_only() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, dir) = setup(tmp.path());
        reset(&st, ResetScope::ControlLayout).unwrap();
        let ui = st.ui_state().unwrap();
        assert_eq!(ui.control, ControlViewState::default());
        assert!(ui.sidebar_pinned);
        assert_eq!(st.prefs().theme, Theme::Dark);
        assert_eq!(pre_reset_count(&dir), 1);
        // The snapshot holds the state before the reset.
        let saved: UiState = persist::load_strict(&dir).unwrap();
        assert_eq!(saved.control, ControlViewState::default());

        st.close_project();
        assert!(matches!(
            reset(&st, ResetScope::ControlLayout),
            Err(AppError::NoProject)
        ));
    }

    #[test]
    fn preferences_resets_prefs_and_ui() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, dir) = setup(tmp.path());
        reset(&st, ResetScope::Preferences).unwrap();
        assert_eq!(st.prefs(), Prefs::default());
        assert_eq!(st.ui_state().unwrap(), UiState::default());
        assert_eq!(st.current_project().unwrap().settings.port, 9000);
        assert_eq!(pre_reset_count(&dir), 1);
        assert!(st.backup_worker_running());

        // Without a project: prefs only, no snapshot needed.
        st.set_prefs(Prefs {
            theme: Theme::Light,
            ..Default::default()
        });
        st.close_project();
        reset(&st, ResetScope::Preferences).unwrap();
        assert_eq!(st.prefs(), Prefs::default());
        assert_eq!(pre_reset_count(&dir), 1);
    }

    #[test]
    fn everything_resets_clears_recents_and_closes() {
        let tmp = tempfile::tempdir().unwrap();
        let (st, dir) = setup(tmp.path());
        reset(&st, ResetScope::Everything).unwrap();
        assert!(st.current_project().is_none());
        assert!(st.recents().is_empty());
        assert_eq!(st.prefs(), Prefs::default());
        assert!(!st.backup_worker_running());
        assert_eq!(pre_reset_count(&dir), 1);
        let file: ProjectFile = persist::load_strict(&dir).unwrap();
        assert_eq!(file.settings, ProjectSettings::default());
        let ui: UiState = persist::load_strict(&dir).unwrap();
        assert_eq!(ui, UiState::default());
        let saved: Prefs = persist::load_strict(&tmp.path().join("config")).unwrap();
        assert_eq!(saved, Prefs::default());
    }
}
