// Limits and defaults of the layout editor's additions to the screen file and the ui-state.
// Mirror core/model.rs (SCREEN_GUIDES_MAX, SCREEN_SAFE_AREAS_MAX, SAFE_AREA_NAME_MAX,
// SCREEN_GRID_SIZE_RANGE, SCREEN_CROP_OVERLAYS, ScreenEditorView::default); change both together.
// Kept out of $lib/types so routes that don't edit screens don't carry them.

import type { CropOverlay, ScreenEditorView } from "$lib/types";

/** Ruler guides per axis. */
export const SCREEN_GUIDES_MAX = 64;
/** Named safe areas per screen. */
export const SCREEN_SAFE_AREAS_MAX = 16;
export const SAFE_AREA_NAME_MAX = 40;
/** Grid sizes the editor accepts (screen px); clamped server-side. */
export const SCREEN_GRID_SIZE_RANGE = { min: 2, max: 512 } as const;
/**
 * The smallest side the editor resizes a layer to (screen px; the screen's own minimum side). An
 * editor limit only: the screen file accepts 1 px, and a layer already smaller keeps its size as
 * its floor.
 */
export const LAYER_SIZE_MIN = 16;
export const SCREEN_CROP_OVERLAYS: readonly CropOverlay[] = ["none", "9:16", "4:5", "1:1", "4:3"];

export const DEFAULT_SCREEN_EDITOR: ScreenEditorView = {
  snap: true,
  guides: true,
  grid: false,
  gridSize: 16,
  actionSafe: false,
  titleSafe: false,
  centerCross: false,
  customSafe: true,
  crop: "none",
};
