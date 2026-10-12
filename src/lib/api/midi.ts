// MIDI devices, MIDI Learn and LED feedback, plus the MIDI / OSC control events.
// Split out of the core `$lib/api` so only the pieces that use them load them. The mappings
// themselves (`controls.json`) are `$lib/api/controls`; OSC settings go through
// `updateProjectSettings` ($lib/api/app); device prefs through the prefs store (`Prefs.midi`).

import type { ControlsDoc, MidiDevice, MidiLearned, MidiSource, OscStatus } from "../types";
import { call, on } from "./index";

/** The MIDI devices on this machine (inputs and outputs, by port name). */
export const listMidiDevices = () => call<MidiDevice[]>("list_midi_devices");
/** Opens (true) or closes (false) a learn session; counted, so every open needs a close. While
 *  open, every incoming message is sent as `midi-learn` and runs nothing. */
export const midiLearn = (on: boolean) => call<void>("midi_learn", { on });
/** The device list is shown (true) or hidden (false); counted. While shown, inputs are open (so
 *  "In use by another app" is known) and hot-plug updates arrive as `midi-devices`. */
export const watchMidiDevices = (on: boolean) => call<void>("watch_midi_devices", { on });
/** Lights a short test pattern on a device's feedback output. */
export const testMidiFeedback = (device: string) => call<void>("test_midi_feedback", { device });

/** Device list changes (hot-plug included). */
export const onMidiDevices = (h: (d: MidiDevice[]) => void) => on("midi-devices", h);
/** A captured message (only while a learn session is open). */
export const onMidiLearn = (h: (l: MidiLearned) => void) => on("midi-learn", h);
/** The project's mappings after a save or reload. */
export const onControlsChanged = (h: (d: ControlsDoc) => void) => on("controls-changed", h);
/** An input that matched several mappings (nothing ran). */
export const onControlConflict = (h: (c: { source: MidiSource; mappingIds: string[] }) => void) =>
  on("control-conflict", h);

/** The OSC listener: off, listening, or an error (port in use). */
export const oscStatus = () => call<OscStatus>("osc_status");
/** OSC listener changes. */
export const onOscStatus = (h: (s: OscStatus) => void) => on("osc-status", h);
