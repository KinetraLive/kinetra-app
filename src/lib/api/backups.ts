// Backup snapshots of the project config (Settings > Backups).
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { BackupInfo, Project } from "../types";
import { call } from "./index";

export const listBackups = () => call<BackupInfo[]>("list_backups");
export const createBackup = () => call<BackupInfo>("create_backup");
export const restoreBackup = (id: string) => call<Project>("restore_backup", { id });
export const deleteBackup = (id: string) => call<void>("delete_backup", { id });
