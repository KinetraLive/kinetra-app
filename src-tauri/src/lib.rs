mod close;
mod commands;
pub mod core;
pub mod error;
#[cfg(windows)]
pub mod frames;
mod menu;
pub mod ndi;
mod outputs;
#[cfg(windows)]
pub mod preview;
pub mod server;
mod updates;
mod webview2;

use std::fs::{self, OpenOptions};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tauri::{AppHandle, Emitter, EventTarget, Manager, RunEvent, WindowEvent};

use crate::core::events::{self, names, EventSink};
use crate::core::state::AppState;
use crate::server::{OscManager, ServerManager};

/// The main window was destroyed (the app is exiting).
static MAIN_GONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The log is truncated at startup once it grows past this.
const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: &str, payload: Value) {
        // Cross-window events skip the window whose command caused them (see
        // `events::with_origin`). Listeners must target their own window for the filter to
        // apply: `on()` in src/lib/api/index.ts does.
        let origin = names::CROSS_WINDOW
            .contains(&event)
            .then(events::origin)
            .flatten();
        let result = match origin {
            Some(origin) => self
                .0
                .emit_filter(event, payload, |t| !is_window(t, &origin)),
            None => self.0.emit(event, payload),
        };
        if let Err(e) = result {
            tracing::warn!("emit {event} failed: {e}");
        }
    }
}

/// Whether an event target is the window/webview labelled `label`.
fn is_window(target: &EventTarget, label: &str) -> bool {
    match target {
        EventTarget::Window { label: l }
        | EventTarget::Webview { label: l }
        | EventTarget::WebviewWindow { label: l }
        | EventTarget::AnyLabel { label: l } => l == label,
        _ => false,
    }
}

/// Flushes every store and stops the services (MIDI LEDs off and ports closed, renderer sockets
/// closed, OSC stopped). On exit, and before an update's installer replaces the app.
pub(crate) fn shutdown_services(handle: &AppHandle) {
    if let Some(state) = handle.try_state::<AppState>() {
        state.flush_all();
        // Remote control: `end {quit}` so browsers see the show machine ended it (≤ ~1 s).
        state.remote_quit();
        // LEDs off, MIDI ports closed, the MIDI thread stopped.
        state.midi_close_all();
        // Output windows closed, the displays thread stopped.
        state.outputs_shutdown();
        state.cloud_stop();
        state.cloud_projects_stop();
    }
    // Closes renderer sockets and releases the port; bounded to about a second.
    if let Some(server) = handle.try_state::<ServerManager>() {
        server.shutdown();
    }
    if let Some(osc) = handle.try_state::<OscManager>() {
        osc.shutdown();
    }
}

fn init_logging(log_dir: &Path) {
    let file = fs::create_dir_all(log_dir).and_then(|_| {
        let path = log_dir.join("kinetra.log");
        let too_big = fs::metadata(&path).is_ok_and(|m| m.len() > LOG_MAX_BYTES);
        OpenOptions::new()
            .create(true)
            .append(!too_big)
            .write(true)
            .truncate(too_big)
            .open(path)
    });
    let builder =
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(if cfg!(debug_assertions) {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            });
    // In dev, log to stderr; in release (no console), log to the file.
    let result = match file {
        Ok(f) if !cfg!(debug_assertions) => builder.with_writer(Mutex::new(f)).try_init(),
        Ok(_) | Err(_) => builder.with_writer(std::io::stderr).try_init(),
    };
    if let Err(e) = result {
        eprintln!("logging init failed: {e}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // Checks are manual (commands in updates.rs); the plugin's own JS commands are unused.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(menu::MenuState::default())
        .manage(updates::PendingUpdate::default())
        .manage(close::CloseGuard::default())
        .on_menu_event(menu::on_menu_event)
        .on_window_event(|window, event| match event {
            // Output and Identify windows close only when Kinetra closes them (Alt+F4 does
            // nothing); Kinetra destroys them, which skips this.
            WindowEvent::CloseRequested { api, .. } if outputs::is_output_label(window.label()) => {
                api.prevent_close();
            }
            // The X asks like File › Exit while a project is open (see close.rs).
            WindowEvent::CloseRequested { api, .. }
                if window.label() == commands::app::MAIN_WINDOW
                    && close::on_main_close_requested(window) =>
            {
                api.prevent_close();
            }
            // Output windows would keep the app running without its main window.
            WindowEvent::Destroyed
                if window.label() == commands::app::MAIN_WINDOW
                    && !MAIN_GONE.swap(true, std::sync::atomic::Ordering::SeqCst) =>
            {
                if let Some(state) = window.app_handle().try_state::<AppState>() {
                    state.flush_all();
                }
                window.app_handle().exit(0);
            }
            _ => {}
        })
        .setup(|app| {
            // A debug dev profile (`core::devprofile`) keeps its own folders.
            let profile = core::devprofile::active();
            init_logging(&core::devprofile::dir(app.path().app_log_dir()?, profile));
            let config_dir = core::devprofile::dir(app.path().app_config_dir()?, profile);
            if profile.is_none() {
                core::fsx::migrate_legacy_config(&config_dir);
            }
            let sink: Arc<dyn EventSink> = Arc::new(TauriSink(app.handle().clone()));
            let state = AppState::load(config_dir, sink.clone())?;
            // Auto-out timers run on Tauri's runtime (live commands may be sync, off-runtime).
            state
                .live()
                .set_runtime(tauri::async_runtime::handle().inner().clone());
            // The output server follows the open project (none is open yet: it stays stopped).
            let server = ServerManager::new(sink, state.live().clone());
            // The Control API rides on it (/api/*; off until switched on in Preferences).
            server.set_extra_routes(server::control::routes(&state));
            state.add_project_observer(Arc::new(server.clone()));
            // Display outputs: reconciles on Tauri's blocking pool; windows follow the server
            // port. No window and no displays thread until an output is on.
            state.set_outputs_runtime(tauri::async_runtime::handle().inner().clone());
            {
                let weak = state.downgrade();
                server.on_status(Arc::new(move |s| {
                    if let Some(st) = weak.upgrade() {
                        st.outputs_set_server(s);
                    }
                }));
            }
            #[cfg(windows)]
            {
                let main_hwnd = app
                    .get_webview_window(commands::app::MAIN_WINDOW)
                    .and_then(|w| w.hwnd().ok())
                    .map_or(0, |h| h.0 as isize);
                state.use_system_displays(main_hwnd);
            }
            state.set_output_host(Some(outputs::OutputWindows::new(app.handle().clone())));
            // NDI outputs (Windows): nothing loads or runs until one is on.
            #[cfg(windows)]
            {
                let data = core::devprofile::dir(app.path().app_local_data_dir()?, profile)
                    .join("ndi-capture");
                let weak = state.downgrade();
                let notify: ndi::sender::OnChange = Arc::new(move || {
                    if let Some(st) = weak.upgrade() {
                        st.outputs_ndi_changed();
                    }
                });
                let weak = state.downgrade();
                let restart: ndi::sender::OnChange = Arc::new(move || {
                    if let Some(st) = weak.upgrade() {
                        st.outputs_host_changed();
                    }
                });
                // Capture pages share the app's WebView2 browser process (its folder, wry's
                // arguments and language), else their own `ndi-capture` folder.
                let app_env = ndi::host::AppEnvironment {
                    data_dir: app.path().app_local_data_dir()?,
                    browser_args: ndi::host::APP_BROWSER_ARGS.to_string(),
                    language: frames::capture::system_language(),
                };
                state.set_ndi_host(Some(ndi::host::NdiOutputs::new(
                    data.clone(),
                    Some(app_env.clone()),
                    notify,
                    restart,
                )));
                // Remote preview pictures (remote control, protocol 2): the same capture
                // environments; nothing runs until a collaborator watches.
                state.set_preview_host(Some(preview::PreviewCaptures::new(data, Some(app_env))));
            }
            app.manage(server);
            // OSC control: no task or socket until a project enables it.
            let osc = OscManager::new(&state, tauri::async_runtime::handle().inner().clone());
            state.add_project_observer(Arc::new(osc.clone()));
            app.manage(osc);
            // MIDI control: no port or thread until a project uses MIDI.
            state.use_system_midi();
            // Licensing: the stored token is verified and evaluated locally (no network), then
            // one background task refreshes (keys, revocations, beta, token).
            state.cloud_init();
            state.cloud_start();
            if let Some(main) = app.get_webview_window(commands::app::MAIN_WINDOW) {
                if state.prefs().zoom.is_some() {
                    commands::app::apply_zoom(&main, state.prefs().zoom);
                }
                // Preferences › Outputs lists audio device labels (`enumerateDevices()`): the
                // app's origin (main and Preferences windows) needs the stored microphone
                // permission state. Nothing is captured.
                if let Some(origin) = webview2::app_origin(app.config(), commands::app::MAIN_WINDOW)
                {
                    webview2::allow_audio_devices(&main, &origin, true);
                }
            }
            app.manage(state);

            commands::app::show_if_never_shown(app.handle().clone(), commands::app::MAIN_WINDOW);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::license::license_view,
            commands::license::sign_in_start,
            commands::license::sign_in_cancel,
            commands::license::sign_out,
            commands::license::license_activate,
            commands::license::license_deactivate,
            commands::license::license_move_here,
            commands::license::license_redeem,
            commands::license::license_list,
            commands::license::license_check_now,
            commands::license::cloud_links,
            commands::ai::ai_limits,
            commands::ai::ai_quota,
            commands::ai::ai_generate,
            commands::ai::ai_tweak,
            commands::ai::ai_cancel,
            commands::ai::ai_jobs,
            commands::app::open_preferences,
            commands::app::set_zoom,
            commands::app::toggle_fullscreen,
            commands::app::exit_app,
            close::close_request_ack,
            menu::set_app_menu,
            updates::check_for_update,
            updates::install_update,
            commands::project::inspect_folder,
            commands::project::create_project,
            commands::project::init_project,
            commands::project::open_project,
            commands::project::close_project,
            commands::project::current_project,
            commands::project::update_project_settings,
            commands::recents::list_recents,
            commands::recents::remove_recent,
            commands::recents::clear_recents,
            commands::prefs::get_prefs,
            commands::prefs::set_prefs,
            commands::prefs::get_ui_state,
            commands::prefs::set_ui_state,
            commands::backup::list_backups,
            commands::backup::create_backup,
            commands::backup::restore_backup,
            commands::backup::delete_backup,
            commands::settings_io::export_settings,
            commands::settings_io::preview_settings_import,
            commands::settings_io::import_settings,
            commands::move_project::move_project,
            commands::move_project::cancel_move,
            commands::reset::reset_settings,
            commands::live::server_status,
            commands::live::osc_status,
            commands::live::list_screens,
            commands::live::live_screen,
            commands::live::live_show,
            commands::live::live_hide,
            commands::live::live_set_fields,
            commands::live::media_control,
            commands::live::set_studio,
            commands::live::studio_op,
            commands::live::preview_view,
            commands::screens::list_screen_docs,
            commands::screens::get_screen,
            commands::screens::create_screen,
            commands::screens::save_screen,
            commands::screens::rename_screen,
            commands::screens::duplicate_screen,
            commands::screens::delete_screen,
            commands::screens::set_layer_rect,
            commands::screens::set_layer_variant,
            commands::sources::list_sources,
            commands::sources::get_source,
            commands::sources::create_source,
            commands::sources::create_schedule,
            commands::sources::save_source,
            commands::sources::rename_source,
            commands::sources::duplicate_source,
            commands::sources::delete_source,
            commands::sources::step_source,
            commands::sources::create_api_source,
            commands::sources::test_api_source,
            commands::sources::set_source_secret,
            commands::sources::clear_source_secret,
            commands::sources::retry_api_source,
            commands::sources::set_api_paused,
            commands::sources::watch_api_source,
            commands::cloud::cloud_view,
            commands::cloud::cloud_refresh,
            commands::cloud::cloud_set_backup,
            commands::cloud::cloud_notice_seen,
            commands::cloud::cloud_answer,
            commands::cloud::cloud_backup_now,
            commands::cloud::cloud_conflict_dismiss,
            commands::cloud::cloud_exit_should_ask,
            commands::cloud::cloud_backup_before_exit,
            commands::cloud::cloud_versions,
            commands::cloud::cloud_version_keep,
            commands::cloud::cloud_version_label,
            commands::cloud::cloud_version_delete,
            commands::cloud::cloud_projects_list,
            commands::cloud::cloud_project_rename,
            commands::cloud::cloud_project_trash,
            commands::cloud::cloud_project_untrash,
            commands::cloud::cloud_restore_plan,
            commands::cloud::cloud_restore_start,
            commands::cloud::cloud_restore_cancel,
            commands::cloud::cloud_restore_dismiss,
            commands::polls::polls_quota,
            commands::polls::list_polls,
            commands::polls::create_poll,
            commands::polls::update_poll,
            commands::polls::get_poll,
            commands::polls::open_poll,
            commands::polls::close_poll,
            commands::polls::reset_poll,
            commands::polls::delete_poll,
            commands::polls::watch_poll,
            commands::polls::create_poll_source,
            commands::polls::poll_source_for,
            commands::polls::poll_qr,
            commands::polls::add_poll_to_screen,
            commands::assets::list_assets,
            commands::assets::import_assets,
            commands::assets::delete_asset,
            commands::assets::rename_asset,
            commands::assets::import_media,
            commands::assets::cancel_import,
            commands::recipes::list_recipes,
            commands::recipes::list_recipe_instances,
            commands::recipes::install_recipe,
            commands::recipes::remove_recipe,
            commands::recipes::relink_recipe_source,
            commands::recipes::schedule_view,
            commands::recipes::schedule_action,
            commands::automation::set_timer,
            commands::automation::set_auto_advance,
            commands::flows::list_flows,
            commands::flows::get_flow,
            commands::flows::create_flow,
            commands::flows::save_flow,
            commands::flows::save_flow_draft,
            commands::flows::deploy_flow,
            commands::flows::discard_flow_draft,
            commands::flows::rename_flow,
            commands::flows::duplicate_flow,
            commands::flows::delete_flow,
            commands::flows::set_flow_armed,
            commands::flows::run_flow,
            commands::flows::stop_flow,
            commands::flows::check_flow,
            commands::flows::subscribe_flow_debug,
            commands::flows::unsubscribe_flow_debug,
            commands::flows::list_flow_templates,
            commands::flows::add_flow_template,
            commands::control::set_layer_fields,
            commands::control::live_clear,
            commands::control::live_clear_all,
            commands::control::live_panic,
            commands::control::get_controls,
            commands::control::save_controls,
            commands::control::get_panel,
            commands::control::save_panel,
            commands::control::panel_targets,
            commands::midi::list_midi_devices,
            commands::midi::midi_learn,
            commands::midi::watch_midi_devices,
            commands::midi::test_midi_feedback,
            commands::outputs::list_displays,
            commands::outputs::watch_displays,
            commands::outputs::list_outputs,
            commands::outputs::output_test_pattern,
            commands::outputs::identify_displays,
            commands::outputs::all_outputs_off,
            commands::outputs::ndi_info,
            commands::control_api::control_api_view,
            commands::control_api::control_api_set,
            commands::control_api::control_api_token,
            commands::control_api::control_api_regenerate_token,
            commands::control_api::control_api_disconnect,
            commands::remote::remote_view,
            commands::remote::remote_set,
            commands::remote::remote_start_again,
            commands::remote::remote_end,
            commands::remote::remote_lock,
            commands::remote::remote_takeover,
            commands::remote::remote_new_show,
            commands::remote::remote_activity,
            commands::remote::cue_position,
            commands::remote::remote_test,
            commands::remote::remote_collaborators,
            commands::remote::remote_invite,
            commands::remote::remote_collaborator_set,
            commands::remote::remote_collaborator_remove,
            commands::remote::remote_diag_watch,
            commands::remote::remote_export_diagnostics,
            commands::templates::list_templates,
            commands::templates::get_template,
            commands::templates::create_template,
            commands::templates::save_template,
            commands::templates::rename_template,
            commands::templates::duplicate_template,
            commands::templates::delete_template,
            commands::templates::save_template_variant,
            commands::templates::create_template_variant,
            commands::templates::rename_template_variant,
            commands::templates::delete_template_variant,
            commands::templates::set_default_template_variant,
            commands::templates::update_template_from_preset,
            commands::templates::undo_template_update,
            commands::templates::list_presets,
            commands::styles::list_styles,
            commands::styles::apply_style,
            commands::styles::undo_apply_style,
            commands::styles::save_brand_kit,
            commands::templates::preview_template,
            commands::templates::preview_draft,
            commands::templates::preview_show,
            commands::templates::preview_hide,
        ])
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(e) => {
            // Startup failure: there is no UI to report to yet.
            eprintln!("error while building tauri application: {e}");
            std::process::exit(1);
        }
    };

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            shutdown_services(handle);
        }
    });
}
