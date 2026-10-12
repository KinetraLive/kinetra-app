// Saving project settings from the Preferences page. Writes are serialized so each one starts
// from the project the previous one returned (two quick edits never overwrite each other).

import { toAppError } from "$lib/api";
import { updateProjectSettings } from "$lib/api/app";
import { project } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ProjectSettings } from "$lib/types";

let chain: Promise<unknown> = Promise.resolve();

/** Resolves true when saved; on failure toasts the error and resolves false. */
export function saveProjectSettings(patch: Partial<ProjectSettings>): Promise<boolean> {
  const run = async () => {
    const current = project.current;
    if (!current) return false;
    try {
      project.replace(await updateProjectSettings({ ...current.settings, ...patch }));
      return true;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the setting: ${toAppError(e).message}` });
      return false;
    }
  };
  const next = chain.then(run);
  chain = next;
  return next;
}
