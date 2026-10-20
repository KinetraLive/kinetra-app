// Cloud backup's history and the account's projects (src-tauri/src/commands/cloud.rs). Loaded by
// the History dialog and Preferences only.

import type { CloudProjectSummary, CloudVersionPage, CloudVersionView } from "../types";
import { call } from "./index";

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
