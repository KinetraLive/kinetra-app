// Display outputs: the few helpers the Output menu (the menu chunk) and Control's outputs strip
// need, kept apart from ./model so those chunks don't carry the Preferences logic. ./model
// re-exports them.

import type { OutputConfig, OutputStatus } from "$lib/types";

/** The Output menu's item text: "Output 1 — DELL U2720Q". */
export const menuLabel = (o: OutputConfig): string => (o.display.name ? `${o.name} — ${o.display.name}` : o.name);

/** Control's warnings: "Output 2: display disconnected", "Output 2: screen not found". */
export function warningText(o: OutputConfig, s: OutputStatus | undefined): string | null {
  if (!o.enabled || !s) return null;
  if (s.state === "displayMissing") return `${o.name}: display disconnected`;
  if (s.state === "screenMissing") return `${o.name}: screen not found`;
  if (s.state === "failed") return `${o.name}: ${(s.message || "couldn't open").replace(/^\p{Lu}/u, (c) => c.toLowerCase())}`;
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
