//! MIDI devices, MIDI Learn and LED feedback (see `core::midi`). Device settings are written
//! through the prefs store (`update_prefs` with `Prefs.midi`); a change applies at once. The
//! mappings are `get_controls` / `save_controls` (`commands::control`).
//!
//! Port work (enumerating, opening) talks to the MIDI driver, so these run on the blocking pool.

use tauri::State;

use crate::core::model::MidiDevice;
use crate::core::state::AppState;
use crate::error::AppResult;

use super::project::blocking;

/// The MIDI devices on this machine (inputs, then outputs that aren't inputs). `active` is false
/// only for an input that couldn't be opened (in use by another app).
#[tauri::command]
pub async fn list_midi_devices(state: State<'_, AppState>) -> AppResult<Vec<MidiDevice>> {
    let state = state.inner().clone();
    blocking(move || Ok(state.midi_devices())).await
}

/// Opens (`true`) or closes (`false`) a learn session; counted, so every open needs a close.
/// While one is open, every note-on / CC is sent as `midi-learn` and nothing runs.
#[tauri::command]
pub async fn midi_learn(state: State<'_, AppState>, on: bool) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || {
        state.midi_learn(on);
        Ok(())
    })
    .await
}

/// The Preferences device list is shown (`true`) or hidden (`false`); counted. While shown, the
/// inputs are open (so "In use by another app" is known) and hot-plug updates `midi-devices`.
#[tauri::command]
pub async fn watch_midi_devices(state: State<'_, AppState>, on: bool) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || {
        state.watch_midi_devices(on);
        Ok(())
    })
    .await
}

/// Lights every mapped pad of `device` red, then green, then shows the real state again
/// (≈ 0.8 s). `invalid` without LED feedback or mapped pads; `notFound` when the feedback
/// output isn't connected.
#[tauri::command]
pub async fn test_midi_feedback(state: State<'_, AppState>, device: String) -> AppResult<()> {
    let state = state.inner().clone();
    blocking(move || state.test_midi_feedback(&device)).await
}
