// Outputs: the few helpers the Output menu (the menu chunk) and Control's outputs strip need, kept
// apart from ./model so those chunks don't carry the Preferences logic. ./model re-exports them.

import type { OutputConfig, OutputStatus } from "$lib/types";

/** The Output menu's item text: "Output 1 — DELL U2720Q", "Kinetra Main — NDI®". */
export function menuLabel(o: OutputConfig): string {
  if (o.kind === "ndi") return `${o.name} — NDI®`;
  return o.display.name ? `${o.name} — ${o.display.name}` : o.name;
}

/** Whether an output counts as on air (Exit asks, updates wait): a display output while its window
 * shows; an NDI output only while a receiver is connected (as the core decides). */
export function isOnAir(s: OutputStatus | undefined): boolean {
  if (!s || s.state !== "showing") return false;
  return s.ndi ? s.ndi.receivers > 0 : true;
}

/** Control's warnings: "Output 2: display disconnected", "Output 2: screen not found",
 * "Kinetra Main: NDI® Runtime not installed", "Kinetra Main: dropping frames". */
export function warningText(o: OutputConfig, s: OutputStatus | undefined): string | null {
  if (!o.enabled || !s) return null;
  if (s.state === "displayMissing") return `${o.name}: display disconnected`;
  if (s.state === "screenMissing") return `${o.name}: screen not found`;
  if (s.state === "ndiMissing") return `${o.name}: NDI® Runtime not installed`;
  if (s.state === "failed") return `${o.name}: ${(s.message || "couldn't open").replace(/^\p{Lu}/u, (c) => c.toLowerCase())}`;
  if (s.state === "showing" && s.ndi?.dropping) return `${o.name}: dropping frames`;
  return null;
}

/** "2 outputs are showing." for the Exit confirmation. */
export const showingText = (n: number): string => (n === 1 ? "1 output is showing." : `${n} outputs are showing.`);

/** The screen a key output shows: its fill's. */
export function screenOf(configs: readonly OutputConfig[], o: OutputConfig): string | null {
  if (o.mode === "key") return configs.find((f) => f.id === o.keyOf)?.screenId ?? null;
  return o.screenId;
}

/** Replaces one output (by id) with `patch` applied. */
export function patchOutput(configs: readonly OutputConfig[], id: string, patch: Partial<OutputConfig>): OutputConfig[] {
  return configs.map((c) => (c.id === id ? { ...c, ...patch } : c));
}
