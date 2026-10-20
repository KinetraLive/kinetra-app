// Cloud backup's actions (src-tauri/src/commands/cloud.rs): the project's switch, questions,
// Back up now, the conflict, quitting. Loaded by lazy chunks only (chip, Preferences, exit).

import type { CloudAnswer, CloudView } from "../types";
import { call } from "./index";

/** Backup on or off for the open project. */
export const cloudSetBackup = (on: boolean) => call<CloudView>("cloud_set_backup", { on });
/** The one-time "Backing up to the cloud" notice was shown. */
export const cloudNoticeSeen = () => call<void>("cloud_notice_seen");
/** Answers the open project's question (in the trash / another computer's backup / large first). */
export const cloudAnswer = (answer: CloudAnswer) => call<CloudView>("cloud_answer", { answer });
/** "Back up now": starts a backup of the open project; progress follows on `cloud-changed`. */
export const cloudBackupNow = () => call<CloudView>("cloud_backup_now");
/** "Dismiss" on the conflict notice: both versions stay; the server marks it resolved. */
export const cloudConflictDismiss = () => call<CloudView>("cloud_conflict_dismiss");
/** Whether to ask "Back up before quitting?" (changes not in the cloud, nothing on air). */
export const cloudExitShouldAsk = () => call<boolean>("cloud_exit_should_ask");
/** "Back up and quit": a backup with reason "close"; follow it on `cloud-changed`. */
export const cloudBackupBeforeExit = () => call<CloudView>("cloud_backup_before_exit");
