// Which live thumbnails get an iframe. Pure; used by LiveThumbs (./thumbs.svelte.ts).

/** At most this many live thumbnails (each a WebView2 renderer process) exist at once. */
export const MAX_LIVE_THUMBNAILS = 6;

/**
 * Which thumbnails get a live iframe: the first `cap` screens, in grid order, that are on screen.
 * Off-screen screens never count against the cap.
 */
export function pickLive(
  order: readonly string[],
  visible: ReadonlySet<string>,
  cap: number = MAX_LIVE_THUMBNAILS,
): Set<string> {
  const live = new Set<string>();
  for (const id of order) {
    if (live.size >= cap) break;
    if (visible.has(id)) live.add(id);
  }
  return live;
}
