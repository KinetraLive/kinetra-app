//! `#[tauri::command]` handlers. Thin: validate, call into `core`, map errors.
//!
//! Rule: a command that touches the disk is `async` and does the work inside
//! `tauri::async_runtime::spawn_blocking`, so it never runs on the main (UI event loop) thread.
//! Commands that only read in-memory state may be sync.

pub mod ai;
pub mod app;
pub mod assets;
pub mod automation;
pub mod backup;
pub mod cloud;
pub mod control;
pub mod control_api;
pub mod flows;
pub mod license;
pub mod live;
pub mod midi;
pub mod move_project;
pub mod outputs;
pub mod polls;
pub mod prefs;
pub mod project;
pub mod recents;
pub mod recipes;
pub mod remote;
pub mod reset;
pub mod screens;
pub mod settings_io;
pub mod sources;
pub mod styles;
pub mod templates;
