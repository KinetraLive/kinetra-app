// Display helpers for the Preferences page.

import type { BackupReason, MoveProgress, ResetScope, SettingsSection } from "$lib/types";

export const REASON_LABELS: Record<BackupReason, string> = {
  open: "Opened",
  timer: "Automatic",
  manual: "Manual",
  "pre-import": "Before import",
  "pre-move": "Before move",
  "pre-restore": "Before restore",
  "pre-reset": "Before reset",
  "pre-migrate": "Before migration",
  "pre-update": "Before update",
};

export const SECTION_LABELS: Record<SettingsSection, string> = {
  prefs: "Preferences",
  project: "Project settings",
  uiState: "Control & layout",
  controls: "MIDI mappings",
  panel: "Button panel",
};

export const RESET_DONE: Record<ResetScope, string> = {
  appearance: "Appearance reset.",
  controlLayout: "Control layout reset.",
  preferences: "Preferences reset.",
  everything: "Everything was reset.",
};

const BYTE_UNITS = ["KB", "MB", "GB", "TB"];

/** "512 B", "1.2 MB", "3.4 GB". */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 1024) return `${Math.max(0, Math.round(n || 0))} B`;
  let v = n;
  let i = -1;
  do {
    v /= 1024;
    i++;
  } while (v >= 1024 && i < BYTE_UNITS.length - 1);
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${BYTE_UNITS[i]}`;
}

/** Two sizes in the larger one's unit: "1.2 of 3.4 GB". */
export function formatBytePair(done: number, total: number): string {
  const unit = formatBytes(total).split(" ")[1];
  const idx = unit === "B" ? -1 : BYTE_UNITS.indexOf(unit);
  const scale = 1024 ** (idx + 1);
  const fmt = (n: number) => {
    const v = n / scale;
    return idx < 0 ? String(Math.round(v)) : v < 10 ? v.toFixed(1) : String(Math.round(v));
  };
  return `${fmt(Math.min(done, total))} of ${fmt(total)} ${unit}`;
}

let dtf: Intl.DateTimeFormat | undefined;

/** "3 Oct 2026, 14:02". */
export function formatDateTime(iso: string): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  dtf ??= new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
  return dtf.format(t);
}

/** Compact rendering of a changed value in the import preview. */
export function formatValue(v: unknown): string {
  if (v === undefined) return "(none)";
  if (v === null) return "none";
  if (typeof v === "string") return v === "" ? '""' : v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  let s: string;
  try {
    s = JSON.stringify(v) ?? String(v);
  } catch {
    s = String(v);
  }
  return s.length > 80 ? `${s.slice(0, 79)}…` : s;
}

/** Status line for a move in progress; also the progress bar's accessible value text. */
export function describeMove(p: MoveProgress | null): string {
  if (!p) return "Starting…";
  switch (p.phase) {
    case "scanning":
      return "Scanning…";
    case "snapshot":
      return "Backing up…";
    case "copying":
      return `Copying ${p.filesDone} of ${p.filesTotal} files · ${formatBytePair(p.bytesDone, p.bytesTotal)}`;
    case "verifying":
      return "Verifying…";
    case "switching":
      return "Switching…";
    case "deleting":
      return "Deleting original…";
    case "done":
      return "Done";
  }
}

/** Fraction for the move progress bar, or undefined for an indeterminate bar. */
export function moveRatio(p: MoveProgress | null): number | undefined {
  if (!p) return undefined;
  if (p.phase === "done") return 1;
  if (p.phase !== "copying") return undefined;
  if (p.bytesTotal > 0) return p.bytesDone / p.bytesTotal;
  if (p.filesTotal > 0) return p.filesDone / p.filesTotal;
  return undefined;
}
