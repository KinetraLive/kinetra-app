// Restoring cloud backups (src-tauri/src/commands/cloud.rs): where a restore goes, starting and
// stopping one. Progress and the result arrive on `cloud-changed` (`CloudView.restore`). Loaded
// by the restore panel only (a lazy chunk).

import type { CloudRestoreMode, CloudRestorePlan, CloudRestoreRequest, CloudView } from "../types";
import { call } from "./index";

/** Where a restore would go; Replace: whether the project is backed up or copied first. */
export const cloudRestorePlan = (mode: CloudRestoreMode, name: string, at: string | null) =>
  call<CloudRestorePlan>("cloud_restore_plan", { mode, name, at });
/** Starts a restore (returns at once). */
export const cloudRestoreStart = (request: CloudRestoreRequest) => call<CloudView>("cloud_restore_start", { request });
/** Stops a running restore (the project stays as it was). */
export const cloudRestoreCancel = () => call<void>("cloud_restore_cancel");
/** Forgets a finished restore's result. */
export const cloudRestoreDismiss = () => call<CloudView>("cloud_restore_dismiss");
