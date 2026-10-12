// The folder flows shared by the project picker and the top-bar project switcher.
// Plugins are imported on demand so they stay out of the initial bundle.

import { inspectFolder } from "$lib/api";
import { project } from "$lib/stores/project.svelte";
import type { Project } from "$lib/types";

/** Opens the native folder picker. Resolves to null when the user cancels. */
export async function pickFolder(title: string): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" ? picked : null;
}

export type OpenOutcome =
  | { status: "opened"; project: Project }
  /** The folder has unrelated files: ask before initializing (then call `project.init(path)`). */
  | { status: "needsConfirm"; path: string; entries: number };

/**
 * Inspects `path` without writing, then opens it (existing project), initializes it (empty
 * folder), or reports that the user must confirm (non-empty foreign folder).
 * Rejects with an `AppError`.
 */
export async function openFolder(path: string): Promise<OpenOutcome> {
  const kind = await inspectFolder(path);
  switch (kind.kind) {
    case "project":
      return { status: "opened", project: await project.open(path) };
    case "empty":
      return { status: "opened", project: await project.init(path) };
    case "nonEmptyForeign":
      return { status: "needsConfirm", path, entries: kind.entries };
  }
}

/** Shows `path` in Explorer. Rejects with whatever the opener plugin throws. */
export async function revealInExplorer(path: string): Promise<void> {
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

/** "12 items" / "1000+ items" for the non-empty folder warning. */
export function describeEntries(entries: number, cap = 1000): string {
  if (entries >= cap) return `${cap}+ items`;
  return entries === 1 ? "1 item" : `${entries} items`;
}
