// NDI® links and the trademark line (NDI's SDK licensing asks for a link to ndi.video near every
// place NDI is chosen, and this notice near its first mention). Opened in the system browser.

import { toAppError } from "$lib/api";
import { toasts } from "$lib/stores/toasts.svelte";

export const NDI_URL = "https://ndi.video";
export const NDI_TRADEMARK = "NDI® is a registered trademark of Vizrt NDI AB.";

/** Opens `url` in the browser (a link's click: the webview never navigates). */
export async function openExternal(e: Event, url: string): Promise<void> {
  e.preventDefault();
  try {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } catch (err) {
    toasts.push({ kind: "error", message: `Couldn't open the link: ${toAppError(err).message}` });
  }
}
