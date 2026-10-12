// Media: transport for media layers and imports with progress. The library itself is the assets
// list (`$lib/api/assets`: `AssetInfo.media` carries the probe).

import { call } from "./index";

export type MediaAction = "play" | "pause" | "stop" | "restart" | "seek";

/** Transport on a media layer's Program playback (never content, never saved). `seek` needs
 * `ms` (clamped to in..out). `play` on a stopped layer starts at the in point. */
export const mediaControl = (screenId: string, layerId: string, action: MediaAction, ms?: number) =>
  call<void>("media_control", { screenId, layerId, action, ms: ms ?? null });

/** Starts importing files (copied into assets/; progress as `asset-import` events). Returns the
 * job id; the list of imported names arrives with the events (`done`). */
export const importMedia = (paths: string[]) => call<string>("import_media", { paths });
/** Cancels an import job (partial copies are removed). */
export const cancelImport = (job: string) => call<void>("cancel_import", { job });
