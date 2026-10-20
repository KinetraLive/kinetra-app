// Cloud backup's view (src-tauri/src/commands/cloud.rs): what the top bar's mirror needs, kept
// tiny because the top bar is in every page's shell. Actions are in ./cloud-backup.ts, the history
// in ./cloud-history.ts (both loaded only by lazy chunks). Event: `cloud-changed`.

import type { CloudView } from "../types";
import { call } from "./index";

export const cloudView = () => call<CloudView>("cloud_view");
/** Asks the server now ("Check now"). `onDemand` ("Check for backups") asks even without Pro. */
export const cloudRefresh = (onDemand = false) => call<CloudView>("cloud_refresh", { onDemand });
