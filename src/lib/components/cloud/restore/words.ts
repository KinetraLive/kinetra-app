// How a restore reads (pure): progress, the result, folder names.

import type { CloudRestoreView } from "$lib/types";

/** The last part of a folder path ("C:\Shows\gala-old" → "gala-old"). */
export function leaf(path: string): string {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? path;
}

/** `parent` + the separator it uses + `name`. */
export function joinPath(parent: string, name: string): string {
  const sep = parent.includes("\\") ? "\\" : "/";
  return parent.replace(/[\\/]+$/, "") + sep + name;
}

/** What a running restore is doing. */
export function restoreWords(r: CloudRestoreView): string {
  if (r.paused) return "Restore paused while on air";
  const pct = r.totalBytes > 0 ? ` ${Math.floor((100 * r.doneBytes) / r.totalBytes)}%` : "";
  switch (r.phase) {
    case "preparing":
      return "Getting ready to restore…";
    case "backingUp":
      return "Backing up the project first…";
    case "copying":
      return `Copying the project first…${pct}`;
    case "downloading":
      return r.limited ? `Restoring slowly while on air…${pct}` : `Restoring…${pct}`;
    case "replacing":
      return "Replacing the project's files…";
  }
}

export interface ResultWords {
  text: string;
  tone: "plain" | "warning";
}

/** How a finished restore ended, or null while it runs. */
export function resultWords(r: CloudRestoreView): ResultWords | null {
  const res = r.result;
  if (!res) return null;
  if (!res.ok) return { text: res.error ?? "The restore didn't finish.", tone: "warning" };
  const done = r.mode === "replace" ? "The project now matches this version." : `Restored into ${leaf(r.folder)}.`;
  const n = res.damagedCount;
  const damaged = n === 0 ? "" : n === 1 ? " 1 file couldn't be restored intact." : ` ${n} files couldn't be restored intact.`;
  return { text: done + damaged, tone: n > 0 ? "warning" : "plain" };
}

/** The damaged files to list, and how many more there are. */
export function damagedList(r: CloudRestoreView, show = 5): { files: string[]; more: number } {
  const all = r.result?.damaged ?? [];
  const files = all.slice(0, show);
  return { files, more: Math.max(0, (r.result?.damagedCount ?? 0) - files.length) };
}
