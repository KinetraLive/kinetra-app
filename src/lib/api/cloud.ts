// Cloud backup (src-tauri/src/commands/cloud.rs). The core decides access and everything else;
// event: `cloud-changed`.

import type { CloudAnswer, CloudView } from "../types";
import { call } from "./index";

export const cloudView = () => call<CloudView>("cloud_view");
/** Asks the server now ("Check now"). `onDemand` ("Check for backups") asks even without Pro. */
export const cloudRefresh = (onDemand = false) => call<CloudView>("cloud_refresh", { onDemand });
/** Backup on or off for the open project. */
export const cloudSetBackup = (on: boolean) => call<CloudView>("cloud_set_backup", { on });
/** The one-time "Backing up to the cloud" notice was shown. */
export const cloudNoticeSeen = () => call<void>("cloud_notice_seen");
/** Answers the open project's question (in the trash / another computer's backup). */
export const cloudAnswer = (answer: CloudAnswer) => call<CloudView>("cloud_answer", { answer });
