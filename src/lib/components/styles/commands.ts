// The lazy half of "Apply style…" (./command.svelte.ts): lists the styles some preset ships. Loaded
// the first time the command's pick step opens, so it costs nothing on the shell or any route.

import { listPresets, listStyles } from "$lib/api/templates";
import type { StyleInfo } from "$lib/types";
import { offered } from "./styles";

/** The styles the palette offers (see `offered`); a failing preset list counts as none. */
export async function offeredStyles(): Promise<StyleInfo[]> {
  const [list, presets] = await Promise.all([listStyles(), listPresets().catch(() => [])]);
  return offered(list, presets);
}
