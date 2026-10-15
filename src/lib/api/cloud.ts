// Cloud backup (src-tauri/src/commands/cloud.rs). The core decides access and everything else;
// event: `cloud-changed`.

import type { CloudView } from "../types";
import { call } from "./index";

export const cloudView = () => call<CloudView>("cloud_view");
/** Asks the server now ("Check now"). `onDemand` ("Check for backups") asks even without Pro. */
export const cloudRefresh = (onDemand = false) => call<CloudView>("cloud_refresh", { onDemand });
