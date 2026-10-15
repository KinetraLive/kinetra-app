// What cloud backup's state reads as (CLOUD_PROJECTS.md §2.5): quiet text, never an alarm. Pure.

import type { CloudAnswer, CloudQuestion, CloudView } from "$lib/types";

/** Bytes as GB (10⁹), e.g. "7.34 GB", "0.2 GB", "20 GB". */
export function gb(bytes: number): string {
  const v = bytes / 1e9;
  const digits = v >= 100 || Number.isInteger(v) ? 0 : v >= 10 ? 1 : 2;
  return `${Number(v.toFixed(digits))} GB`;
}

/** "Dec 30, 2026" from an RFC 3339 time. */
export function day(rfc3339: string | null | undefined): string {
  if (!rfc3339) return "";
  const d = new Date(rfc3339);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" });
}

export interface StateWords {
  text: string;
  /** "warning" = shown in the warning color; never a modal. */
  tone: "plain" | "warning";
}

/** The status line for `view`; null when there is nothing to say (ok, or not asked yet). */
export function stateWords(view: CloudView | null): StateWords | null {
  if (!view || view.access !== "available") return null;
  if (view.offline) return { text: "Cloud backup: offline, will retry.", tone: "plain" };
  const q = view.quota;
  switch (view.state) {
    case "warning":
      return { text: `Cloud storage is ${q?.percent ?? q?.warnPercent ?? 80}% full.`, tone: "warning" };
    case "full":
      return {
        text: "Cloud storage is full. Your project is saved on this computer; cloud backup is paused. Free space by deleting old versions or projects, or upgrade.",
        tone: "warning",
      };
    case "lapsed": {
      const until = day(q?.purgeAt);
      return {
        text: until
          ? `Your plan no longer includes cloud backup. Your backups can be downloaded until ${until}.`
          : "Your plan no longer includes cloud backup. Your backups can still be downloaded.",
        tone: "warning",
      };
    }
    default:
      return null;
  }
}

/** After a downgrade: "Versions older than 30 days are deleted from Nov 14, 2026." */
export function retentionWords(view: CloudView | null): string | null {
  const q = view?.quota;
  if (!q?.retentionDaysNext || !q.retentionFrom) return null;
  return `Versions older than ${q.retentionDaysNext} days are deleted from ${day(q.retentionFrom)}.`;
}

export interface Choice {
  answer: CloudAnswer;
  label: string;
  primary?: boolean;
}

/** "Oct 15, 2:02 PM" from an RFC 3339 time. */
export function when(rfc3339: string | null | undefined): string {
  if (!rfc3339) return "";
  const d = new Date(rfc3339);
  return Number.isNaN(d.getTime())
    ? ""
    : d.toLocaleString("en-US", { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
}

/** What a waiting question says and offers (never a modal: it sits with the project). */
export function questionWords(q: CloudQuestion): { text: string; choices: Choice[] } {
  if (q.kind === "inTrash") {
    const until = day(q.purgeAt);
    return {
      text: until
        ? `This project's cloud backup is in the trash until ${until}.`
        : "This project's cloud backup is in the trash.",
      choices: [
        { answer: "restore", label: "Restore", primary: true },
        { answer: "startNew", label: "Start a new cloud backup" },
        { answer: "turnOff", label: "Turn off backup" },
      ],
    };
  }
  const from = q.machine ?? "another computer";
  const at = when(q.at);
  return {
    text: at ? `This project is already backed up from ${from} (${at}).` : `This project is already backed up from ${from}.`,
    choices: [
      { answer: "continueHere", label: "Continue that backup here", primary: true },
      { answer: "separate", label: "Back up as a separate project" },
      { answer: "turnOff", label: "Don't back up here" },
    ],
  };
}
