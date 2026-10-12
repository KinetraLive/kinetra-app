// How a MIDI source reads in the UI: "Note 36 · Ch 1 · APC40". Tiny on purpose: the app shell's
// conflict toast and Control's MIDI chips use it.

import type { MidiSource } from "$lib/types";

/** "Note 36 · Ch 1 · APC40" ("Any channel" / "Any device" for null); without the device when
 * `device` is false ("Note 36 · Ch 1"). */
export function sourceText(s: MidiSource, device = true): string {
  const parts = [`${s.kind === "cc" ? "CC" : "Note"} ${s.number}`, s.channel === null ? "Any channel" : `Ch ${s.channel}`];
  if (device) parts.push(s.device ?? "Any device");
  return parts.join(" · ");
}

/** The conflict toast: "Note 36 · Ch 1 runs nothing: it is assigned twice". */
export function conflictText(s: MidiSource, count: number): string {
  const times = count === 2 ? "twice" : `${count} times`;
  return `${sourceText(s, false)} runs nothing: it is assigned ${times}`;
}
