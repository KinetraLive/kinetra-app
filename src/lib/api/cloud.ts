// Cloud backup (src-tauri/src/commands/cloud.rs). The core decides access and everything else;
// event: `cloud-changed`.

import type { CloudAnswer, CloudProjectSummary, CloudVersionPage, CloudVersionView, CloudView } from "../types";
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
/** "Back up now": starts a backup of the open project; progress follows on `cloud-changed`. */
export const cloudBackupNow = () => call<CloudView>("cloud_backup_now");
/** "Dismiss" on the conflict notice: both versions stay; the server marks it resolved. */
export const cloudConflictDismiss = () => call<CloudView>("cloud_conflict_dismiss");
/** Whether to ask "Back up before quitting?" (changes not in the cloud, nothing on air). */
export const cloudExitShouldAsk = () => call<boolean>("cloud_exit_should_ask");
/** "Back up and quit": a backup with reason "close"; follow it on `cloud-changed`. */
export const cloudBackupBeforeExit = () => call<CloudView>("cloud_backup_before_exit");

// ---- History and the account's projects ----

/** A page of the open project's history, newest first. */
export const cloudVersions = (before: string | null = null) => call<CloudVersionPage>("cloud_versions", { before });
export const cloudVersionKeep = (id: string, kept: boolean) => call<CloudVersionView>("cloud_version_keep", { id, kept });
/** null or blank clears the label. */
export const cloudVersionLabel = (id: string, label: string | null) =>
  call<CloudVersionView>("cloud_version_label", { id, label });
export const cloudVersionDelete = (id: string) => call<void>("cloud_version_delete", { id });
/** The account's cloud projects (with the trash when `trashed`). */
export const cloudProjectsList = (trashed: boolean) => call<CloudProjectSummary[]>("cloud_projects_list", { trashed });
export const cloudProjectRename = (id: string, name: string) =>
  call<CloudProjectSummary>("cloud_project_rename", { id, name });
export const cloudProjectTrash = (id: string) => call<CloudProjectSummary>("cloud_project_trash", { id });
export const cloudProjectUntrash = (id: string) => call<CloudProjectSummary>("cloud_project_untrash", { id });
