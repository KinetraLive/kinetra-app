// App-level commands that only some views need: app info (About), project settings (Preferences),
// clearing the recent projects, the native menu, window zoom / full screen, exit and updates. Kept
// out of the core api module (it is in the picker's initial bundle).

import type { AppInfo, MenuSpec, Project, ProjectSettings, UpdateCheck } from "../types";
import { call } from "./index";

export const appInfo = () => call<AppInfo>("app_info");
export const updateProjectSettings = (settings: ProjectSettings) =>
  call<Project>("update_project_settings", { settings });
/** Empties the recent projects list (File › Open recent › Clear recent). */
export const clearRecents = () => call<void>("clear_recents");

/** Replaces the native menu (Rust diffs it; send only when the spec changed). */
export const setAppMenu = (spec: MenuSpec) => call<void>("set_app_menu", { spec });
/** Sets and stores `prefs.zoom` (clamped 0.5–2) and applies it to every app window. */
export const setZoom = (zoom: number) => call<number>("set_zoom", { zoom });
/** Toggles full screen on the main window; returns the new state. */
export const toggleFullscreen = () => call<boolean>("toggle_fullscreen");
/** Flushes and exits the app (the UI confirms first when anything is on air). */
export const exitApp = () => call<void>("exit_app");
/** Tells the backend the page got `close-requested` (else a second close exits without asking). */
export const closeRequestAck = () => call<void>("close_request_ack");
/** Checks this build's channel feed. Fails with code "updateCheck" on any network/feed error. */
export const checkForUpdate = () => call<UpdateCheck>("check_for_update");
/** Downloads and installs the update found by the last check, then restarts. Fails with code
 * "onAir" while anything is on air (nothing is downloaded then). */
export const installUpdate = () => call<void>("install_update");
