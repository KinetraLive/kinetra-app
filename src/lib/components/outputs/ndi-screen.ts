// "Send {screen} over NDI®" / "Stop sending {screen} over NDI®" (the palette's screen items):
// turns the screen's NDI output on (creating one with the region's frame rate when there is none)
// or off, through the prefs store like Preferences › Outputs. Loaded with the palette only.

import { toAppError } from "$lib/api";
import { ndiInfo } from "$lib/api/outputs";
import { showPreferences } from "$lib/components/settings/open";
import { prefs } from "$lib/stores/prefs.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { NDI_OUTPUTS_MAX } from "$lib/types";
import { ndiOutputOf, withScreenNdi } from "./model";

/** Whether this screen is being sent over NDI (its NDI output is on). */
export function sendingOverNdi(screenId: string): boolean {
  return ndiOutputOf(prefs.value?.outputs ?? [], screenId)?.enabled ?? false;
}

export async function setScreenNdi(screen: { id: string; name: string }, on: boolean): Promise<void> {
  const p = prefs.value;
  if (!p) return;
  try {
    const info = await ndiInfo();
    const next = withScreenNdi(p.outputs ?? [], screen, on, info.defaultRate);
    if (!next) {
      toasts.push({ kind: "error", message: `No room for another output (${NDI_OUTPUTS_MAX} NDI® outputs at most).` });
      return;
    }
    await prefs.update({ ...p, outputs: next });
    if (on && !info.installed) {
      toasts.push({
        kind: "info",
        message: "Install the free NDI® Runtime to send over NDI®.",
        action: { label: "Outputs", run: () => void showPreferences("outputs") },
      });
    }
  } catch (e) {
    toasts.push({ kind: "error", message: `Couldn't change the NDI® output: ${toAppError(e).message}` });
  }
}
