// What cloud backup's state reads as (CLOUD_PROJECTS.md §2.5): quiet text, never an alarm. Pure.

import type { CloudAnswer, CloudQuestion, CloudRunView, CloudView } from "$lib/types";
import { restoreWords } from "./restore/words";

/** A size in decimal units (10³, CLOUD_PROJECTS.md §3.2): "7.34 GB", "13.1 MB", "20 KB", "0 MB". */
export function size(bytes: number): string {
  if (bytes <= 0) return "0 MB";
  const [div, unit] = bytes >= 1e9 ? [1e9, "GB"] : bytes >= 1e6 ? [1e6, "MB"] : [1e3, "KB"];
  const v = bytes / div;
  const digits = v >= 100 || Number.isInteger(v) ? 0 : v >= 10 ? 1 : 2;
  return `${Math.max(Number(v.toFixed(digits)), unit === "KB" ? 1 : 0)} ${unit}`;
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
  if (q.kind === "firstBackup") {
    return {
      text: `This project's first cloud backup is ${size(q.bytes)}.`,
      choices: [
        { answer: "backUp", label: "Back up", primary: true },
        { answer: "turnOff", label: "Don't back up this project" },
      ],
    };
  }
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

/** What a running backup is doing. */
export function runWords(run: CloudRunView): string {
  if (run.paused) return "Paused while on air";
  const pct = run.totalBytes > 0 ? ` ${Math.floor((100 * run.doneBytes) / run.totalBytes)}%` : "";
  switch (run.phase) {
    case "scanning":
      return "Looking for changes…";
    case "hashing":
      return `Checking files…${pct}`;
    case "uploading":
      return run.limited ? `Backing up slowly while on air…${pct}` : `Backing up…${pct}`;
    case "committing":
      return "Saving the version…";
  }
}

/** "Last backup Oct 15, 2:02 PM", or null before the first. */
export function lastWords(at: string | null): string | null {
  const w = when(at);
  return w ? `Last backup ${w}` : null;
}

/** §9: "This project was also backed up from OTHER-PC at 2:02 PM. Both versions are kept." */
export function conflictWords(c: { machine: string | null; at: string | null }): string {
  const t = c.at ? new Date(c.at) : null;
  const time =
    t && !Number.isNaN(t.getTime()) ? ` at ${t.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" })}` : "";
  return `This project was also backed up from ${c.machine ?? "another computer"}${time}. Both versions are kept.`;
}

export interface ChipWords {
  /** Short, for the top bar. */
  text: string;
  /** The whole sentence (tooltip and accessible name). */
  label: string;
  tone: "plain" | "warning";
  /** The one-time "Backing up to the cloud" notice. */
  notice?: boolean;
}

/** The top bar chip for the open project, or null when cloud backup isn't on for it. */
export function chipWords(view: CloudView | null): ChipWords | null {
  const p = view?.project;
  if (!view || view.access !== "available" || !p || !p.backup) return null;
  const say = (text: string, tone: ChipWords["tone"] = "plain", label = text): ChipWords => ({ text, label, tone });
  if (p.question) return say("Cloud backup needs an answer", "warning", questionWords(p.question).text);
  const r = view.restore;
  if (r && !r.result) return say(restoreWords(r), r.paused ? "warning" : "plain");
  if (p.run) return say(runWords(p.run), p.run.paused ? "warning" : "plain");
  if (!p.noticeSeen) return { ...say("Backing up to the cloud"), notice: true };
  if (view.offline) return say("Cloud backup: offline", "warning", "Cloud backup: offline, will retry.");
  if (p.error) return say("Last backup didn't finish", "warning", p.error);
  const state = stateWords(view);
  if (state) return say(view.state === "warning" ? "Cloud storage almost full" : "Cloud backup paused", state.tone, state.text);
  if (p.conflict) return say("Backed up on two computers", "plain", conflictWords(p.conflict));
  const last = lastWords(p.lastBackupAt);
  return last ? say(last) : say("Not backed up yet");
}
