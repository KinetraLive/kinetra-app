// Preferences > MIDI & OSC > Devices, the pure part: which rows show, the feedback output choices,
// and the prefs edits. No Svelte, no I/O.

import type { MidiDevice, MidiFeedbackProfile, MidiPrefs } from "$lib/types";

export type DeviceStatus = "connected" | "busy" | "missing";

export interface DeviceRow {
  name: string;
  status: DeviceStatus;
  /** Listened to (a device not in prefs is on). */
  enabled: boolean;
}

export const PROFILE_OPTIONS: { value: MidiFeedbackProfile; label: string }[] = [
  { value: "generic", label: "Generic" },
  { value: "apc40", label: "APC40" },
  { value: "apcMini", label: "APC mini" },
  { value: "launchpad", label: "Launchpad" },
  { value: "xtouch", label: "X-Touch" },
];

/** Inputs present now (in the core's order), then remembered ones that aren't (by name). */
export function deviceRows(devices: readonly MidiDevice[], midi: MidiPrefs): DeviceRow[] {
  const enabled = (name: string) => midi.inputs[name]?.enabled ?? true;
  const rows: DeviceRow[] = devices
    .filter((d) => d.input)
    .map((d) => ({ name: d.name, enabled: enabled(d.name), status: d.active || !enabled(d.name) ? "connected" : "busy" }));
  const seen = new Set(rows.map((r) => r.name));
  const remembered = [...Object.keys(midi.inputs), ...Object.keys(midi.feedback)].filter((n) => !seen.has(n));
  for (const name of [...new Set(remembered)].sort((a, b) => a.localeCompare(b))) {
    rows.push({ name, enabled: enabled(name), status: "missing" });
  }
  return rows;
}

/**
 * LED feedback choices for an input: Off ("off"), the output with the input's own name ("": the
 * prefs' null) when there is one (or it is chosen), other outputs by name, and the saved output
 * when it is gone. `current`: the saved output (undefined = no lights).
 */
export function outputOptions(devices: readonly MidiDevice[], input: string, current?: string | null): { value: string; label: string }[] {
  const outputs = devices.filter((d) => d.output).map((d) => d.name);
  const out = [{ value: "off", label: "No lights" }];
  if (outputs.includes(input) || current === null) out.push({ value: "", label: input });
  for (const o of outputs) if (o !== input) out.push({ value: o, label: o });
  if (current != null && !outputs.includes(current)) out.push({ value: current, label: `${current} (not connected)` });
  return out;
}

export function withInput(midi: MidiPrefs, name: string, enabled: boolean): MidiPrefs {
  return { ...midi, inputs: { ...midi.inputs, [name]: { enabled } } };
}

/** Sets (or, with null, removes) a device's LED feedback. */
export function withFeedback(midi: MidiPrefs, name: string, fb: { output: string | null; profile: MidiFeedbackProfile } | null): MidiPrefs {
  const feedback = { ...midi.feedback };
  if (fb) feedback[name] = fb;
  else delete feedback[name];
  return { ...midi, feedback };
}
