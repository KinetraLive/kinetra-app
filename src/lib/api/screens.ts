// Screen CRUD and the layout editor's live drag.
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type {
  LiveLayerView,
  NewScreen,
  Rect,
  ScreenDoc,
  ScreenState,
  ScreenSummary,
} from "../types";
import { call } from "./index";

/** Newest first by `updatedAt`. */
export const listScreenDocs = () => call<ScreenSummary[]>("list_screen_docs");
export const getScreen = (id: string) => call<ScreenState>("get_screen", { id });
export const createScreen = (screen: NewScreen) => call<ScreenState>("create_screen", { screen });
/** Validates, writes, and applies the live rules (on-air layers follow `liveEdit`). */
export const saveScreen = (doc: ScreenDoc) => call<ScreenState>("save_screen", { doc });
export const renameScreen = (id: string, name: string) =>
  call<ScreenSummary>("rename_screen", { id, name });
export const duplicateScreen = (id: string, name: string) =>
  call<ScreenState>("duplicate_screen", { id, name });
export const deleteScreen = (id: string) => call<void>("delete_screen", { id });
/** While dragging: in memory + broadcast only, no disk write (save after the drop). */
export const setLayerRect = (screenId: string, layerId: string, rect: Rect) =>
  call<LiveLayerView>("set_layer_rect", { screenId, layerId, rect });
/**
 * Shows another variant of a layer's template. Saved to the screen; on air it follows the live
 * edit setting (onTake: staged; immediate: swapped with `transition`).
 */
export const setLayerVariant = (
  screenId: string,
  layerId: string,
  variant: string,
  transition: import("../types").VariantTransition,
) => call<ScreenState>("set_layer_variant", { screenId, layerId, variant, transition });
