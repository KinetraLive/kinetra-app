// Simple / Advanced mode (per machine, `prefs.uiMode`). Simple hides Flows, the template code
// editor, MIDI/OSC and JSON API sources; hidden features keep working in projects. Where something
// is hidden, show `AdvancedHint` instead of nothing. Tiny: the shell and the picker import it.
// Read at call time (a namespace import): tests that mock $lib/api must still load this store.
import * as api from "$lib/api";
import type { UiMode } from "$lib/types";
import { prefs } from "./prefs.svelte";
import { toasts } from "./toasts.svelte";

/** The current mode; "advanced" until prefs load (never hides things during startup). */
export function uiMode(): UiMode {
  return prefs.value?.uiMode ?? "advanced";
}

/** True when advanced features are shown. Reactive inside `$derived` / templates. */
export function isAdvanced(): boolean {
  return uiMode() === "advanced";
}

/** Switches the mode (optimistic; prefs.json is per machine). A failure rolls back and is a toast. */
export async function setUiMode(mode: UiMode): Promise<void> {
  const p = prefs.value;
  if (!p || p.uiMode === mode) return;
  try {
    await prefs.update({ ...p, uiMode: mode });
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't save the setting: ${api.toAppError(e).message}` });
  }
}
