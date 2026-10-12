//! Manual update checks with `tauri-plugin-updater` (Help › Check for updates, Preferences ›
//! About). Never at startup, never blocking the UI. The feed follows the build channel; the
//! public key is in tauri.conf.json (`plugins.updater.pubkey`).
//!
//! Errors reach the UI as `updateCheck` ("Couldn't check for updates. Try again later.") with the
//! real error at debug in the log; installing while anything is on air fails with `onAir`.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, State, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::app::build_info;
use crate::core::events::names;
use crate::core::live_hub::LiveHub;
use crate::core::model::{BuildChannel, UpdateCheck, UpdatePhase, UpdateProgress};
use crate::core::state::AppState;
use crate::error::{AppError, AppResult};

const STABLE_FEED: &str = "https://download.kinetra.live/updates/latest.json";
const CANARY_FEED: &str = "https://download.kinetra.live/updates/latest-canary.json";
/// The whole check (request + body).
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
/// The installer download.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// `update-progress` at most every 100 ms (≤ 10/s).
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// The update found by the last check, kept for `install_update`.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

impl PendingUpdate {
    fn set(&self, u: Option<Update>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = u;
    }
    fn get(&self) -> Option<Update> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The feed of a build channel.
pub fn feed_url(channel: BuildChannel) -> &'static str {
    match channel {
        BuildChannel::Stable => STABLE_FEED,
        BuildChannel::Canary => CANARY_FEED,
    }
}

/// Why an update can't install right now: anything on air in Program, or a display output
/// showing; `None` otherwise.
pub fn install_blocked(live: &LiveHub, outputs_showing: bool) -> Option<String> {
    install_error(live, outputs_showing).map(|e| e.to_string())
}

fn install_error(live: &LiveHub, outputs_showing: bool) -> Option<AppError> {
    if live.any_on_air() {
        Some(AppError::OnAir)
    } else if outputs_showing {
        Some(AppError::OutputsOn)
    } else {
        None
    }
}

fn check_failed(e: impl std::fmt::Display) -> AppError {
    tracing::debug!("update check failed: {e}");
    AppError::UpdateCheck
}

async fn check(app: &AppHandle) -> AppResult<Option<Update>> {
    let url = Url::parse(feed_url(build_info().0)).map_err(check_failed)?;
    let handle = app.clone();
    let updater = app
        .updater_builder()
        // Windows exits with std::process::exit right after this (no Exit event): flush and stop
        // everything first, then what the plugin's default hook does.
        .on_before_exit(move || {
            crate::shutdown_services(&handle);
            handle.cleanup_before_exit();
        })
        .endpoints(vec![url])
        .map_err(check_failed)?
        .timeout(CHECK_TIMEOUT)
        .build()
        .map_err(check_failed)?;
    // The request timeout covers the body too; this bounds anything else (DNS, TLS setup).
    match tokio::time::timeout(CHECK_TIMEOUT + Duration::from_secs(1), updater.check()).await {
        Ok(r) => r.map_err(check_failed),
        Err(_) => Err(check_failed("timed out")),
    }
}

/// Checks this build's channel feed.
#[tauri::command]
pub async fn check_for_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingUpdate>,
) -> AppResult<UpdateCheck> {
    let found = check(&app).await?;
    let result = match &found {
        Some(u) => UpdateCheck {
            available: true,
            version: Some(u.version.clone()),
            notes: u.body.clone().filter(|b| !b.trim().is_empty()),
            date: u.date.and_then(|d| {
                d.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            }),
            blocked: install_blocked(state.live(), state.any_output_showing()),
        },
        None => UpdateCheck {
            available: false,
            version: None,
            notes: None,
            date: None,
            blocked: None,
        },
    };
    tracing::info!(
        "update check: {}",
        result.version.as_deref().unwrap_or("up to date")
    );
    pending.set(found);
    Ok(result)
}

/// Downloads and installs the update of the last check (checking again when there was none),
/// then restarts. Refused with `onAir` while anything is on air, before and after the download.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingUpdate>,
) -> AppResult<()> {
    if let Some(e) = install_error(state.live(), state.any_output_showing()) {
        return Err(e);
    }
    let update = match pending.get() {
        Some(u) => u,
        None => match check(&app).await? {
            Some(u) => u,
            None => return Err(AppError::Invalid("Kinetra is up to date.".into())),
        },
    };
    let mut update = update;
    update.timeout = Some(DOWNLOAD_TIMEOUT);

    let mut downloaded: u64 = 0;
    let mut last: Option<Instant> = None;
    let progress = |downloaded: u64, total: Option<u64>, phase: UpdatePhase| {
        let p = UpdateProgress {
            downloaded,
            total,
            phase,
        };
        if let Err(e) = app.emit(names::UPDATE_PROGRESS, p) {
            tracing::debug!("update-progress not sent: {e}");
        }
    };
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                if last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
                    last = Some(Instant::now());
                    progress(downloaded, total, UpdatePhase::Download);
                }
            },
            || {},
        )
        .await
        .map_err(check_failed)?;
    // Something may have gone on air during the download.
    if let Some(e) = install_error(state.live(), state.any_output_showing()) {
        return Err(e);
    }
    progress(
        bytes.len() as u64,
        Some(bytes.len() as u64),
        UpdatePhase::Install,
    );
    tracing::info!("installing update {}", update.version);
    // Windows: the installer starts and the process exits right after the updater's
    // on_before_exit hook (lib.rs flushes everything there). macOS: restart below.
    let installed = tauri::async_runtime::spawn_blocking(move || update.install(bytes))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    installed.map_err(check_failed)?;
    // The Exit event (lib.rs) flushes and stops the services on the way out.
    app.restart()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feeds_follow_the_channel() {
        assert_eq!(feed_url(BuildChannel::Stable), STABLE_FEED);
        assert_eq!(feed_url(BuildChannel::Canary), CANARY_FEED);
        for f in [STABLE_FEED, CANARY_FEED] {
            assert!(Url::parse(f).unwrap().scheme() == "https");
        }
    }

    #[test]
    fn install_is_blocked_while_on_air() {
        let mut live = crate::core::live::LiveState::with_test_screen();
        let mut main = live.screens[0].clone();
        main.info.id = "main".into();
        live.screens.push(main);
        let layer = live.screens[1].layers[0].id.clone();
        let hub = LiveHub::new(live);
        assert_eq!(install_blocked(&hub, false), None);
        assert_eq!(
            install_blocked(&hub, true).as_deref(),
            Some("Turn off the outputs before installing.")
        );
        assert_eq!(
            serde_json::to_value(AppError::OutputsOn).unwrap()["code"],
            "onAir"
        );
        hub.show("main", &layer).unwrap();
        assert_eq!(
            install_blocked(&hub, true).as_deref(),
            Some("Take everything off air before installing.")
        );
        let err = serde_json::to_value(AppError::OnAir).unwrap();
        assert_eq!(err["code"], "onAir");
        let err = serde_json::to_value(AppError::UpdateCheck).unwrap();
        assert_eq!(err["code"], "updateCheck");
        assert_eq!(
            err["message"],
            "Couldn't check for updates. Try again later."
        );
    }
}
