// Images in the project's assets/ folder (served at /assets/<name>).
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type { AssetInfo } from "../types";
import { call } from "./index";

export const listAssets = () => call<AssetInfo[]>("list_assets");
/** Copies the files (absolute paths from the dialog) into assets/. */
export const importAssets = (paths: string[]) => call<AssetInfo[]>("import_assets", { paths });
export const deleteAsset = (name: string) => call<void>("delete_asset", { name });
export const renameAsset = (name: string, newName: string) =>
  call<AssetInfo>("rename_asset", { name, newName });
