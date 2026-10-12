// Settings export/import file (Settings > Settings file).
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { ImportPreview, SettingsSection } from "../types";
import { call } from "./index";

export const exportSettings = (path: string) => call<void>("export_settings", { path });
export const previewSettingsImport = (path: string) =>
  call<ImportPreview>("preview_settings_import", { path });
export const importSettings = (path: string, sections: SettingsSection[]) =>
  call<void>("import_settings", { path, sections });
