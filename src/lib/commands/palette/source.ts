// What the palette reads from the shell's command system (one place, so the UI stays decoupled):
// the registry (commands, providers, context, Recent, running), the binding table (keys) and the
// project item providers (registered and fed while the palette is open).

import type { Command, CommandContext } from "../registry.svelte";
import { commands, providerItems } from "../registry.svelte";

import { keysFor as boundKeys, LAYER_COMMAND } from "../bindings.svelte";

export { currentContext, recentIds, runCommand, runFollowUp, getCommand } from "../registry.svelte";
export { watchItems } from "../providers/project.svelte";
export { t } from "$lib/i18n";

const LAYER_ROW = /^live\.(take|clear|cue):(.+)$/;

/** The keys shown on a row: a Control layer row (Take / Clear / Cue) shows its layer key, which
 * belongs to the hidden "live.toggle:<screen>/<layer>" command that does exactly that action. */
export function keysFor(id: string): readonly string[] {
  const m = LAYER_ROW.exec(id);
  return boundKeys(m ? LAYER_COMMAND + m[2] : id);
}

/** Measurement seam (e2e/ui/palette.measure.ts only; the app never sets it): extra synthetic
 * entries, read like a provider's items. */
type Probe = { __KINETRA_PALETTE_EXTRA__?: readonly Command[] };

/** Registered commands plus the providers' items for `ctx` (reactive: re-read when either changes). */
export function paletteCommands(ctx: CommandContext): readonly Command[] {
  let items: readonly Command[] = [];
  try {
    items = providerItems(ctx);
  } catch {
    items = []; // a broken provider never breaks the palette
  }
  const extra = (globalThis as Probe).__KINETRA_PALETTE_EXTRA__;
  if (Array.isArray(extra)) items = [...items, ...extra];
  const list = commands();
  return items.length === 0 ? list : [...list, ...items];
}
