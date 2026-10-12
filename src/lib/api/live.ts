// Output server status and live control of screens and layers.
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { LiveLayerView, LiveScreenView, ServerStatus } from "../types";
import { call } from "./index";

export const serverStatus = () => call<ServerStatus>("server_status");
/** The user's live screens (reserved screens excluded). */
export const listScreens = () => call<LiveScreenView[]>("list_screens");
export const liveShow = (screenId: string, layerId: string) =>
  call<LiveLayerView>("live_show", { screenId, layerId });
export const liveHide = (screenId: string, layerId: string) =>
  call<LiveLayerView>("live_hide", { screenId, layerId });

// ---- Control -----------------------------------------------------------------------------
/** Saved to the screen and applied by the live rules (on air + onTake waits for TAKE). */
export const setLayerFields = (screenId: string, layerId: string, fields: Record<string, string>) =>
  call<LiveLayerView>("set_layer_fields", { screenId, layerId, fields });
/** Every on-air layer of the screen plays out. */
export const liveClear = (screenId: string) => call<LiveScreenView>("live_clear", { screenId });
/** Every on-air layer of every screen plays out. */
export const liveClearAll = () => call<LiveScreenView[]>("live_clear_all");
/** PANIC: everything off air at once, no out-animation. */
export const livePanic = () => call<LiveScreenView[]>("live_panic");

// ---- Studio mode (Preview / Program) ----
/** Studio settings (saved in ui-state); turning it off with a non-empty Preview needs `exit`. */
export const setStudio = (
  settings: import("../types").StudioSettings,
  exit?: "take" | "discard" | "keepStaged" | null,
) => call<void>("set_studio", { settings, exit: exit ?? null });
/** The single entry point for takes and cues (see StudioOp). Returns the new Preview. */
export const studioOp = (screenId: string, op: import("../types").StudioOp) =>
  call<import("../types").PreviewView>("studio_op", { screenId, op });
export const previewView = (screenId: string) =>
  call<import("../types").PreviewView>("preview_view", { screenId });

// ---- Test poll data (core/polls.rs: the safeguard every take path goes through) ----
/** Whether test poll data may go to Program this session. */
export const testDataAllowed = () => call<boolean>("test_data_allowed");
/** "Allow test data on air" for this session (the app asks first). */
export const allowTestDataOnAir = (on: boolean) => call<void>("allow_test_data_on_air", { on });
