// Small display helpers for the project picker (and the relative times elsewhere in the app).

import type { AppError } from "$lib/types";

/** Parent folder of `path`, or null for a root / bare name. */
export function parentDir(path: string): string | null {
  const trimmed = path.replace(/[\\/]+$/, "");
  const i = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  if (i <= 0) return null;
  const head = trimmed.slice(0, i);
  // "C:" alone is a drive, keep its separator.
  return /^[A-Za-z]:$/.test(head) ? head + trimmed[i] : head;
}

/** Splits a path into [head incl. trailing separator, last segment] for middle truncation. */
export function splitPath(path: string): [string, string] {
  const i = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return i < 0 ? ["", path] : [path.slice(0, i + 1), path.slice(i + 1)];
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 24 * 3600],
  ["month", 30 * 24 * 3600],
  ["week", 7 * 24 * 3600],
  ["day", 24 * 3600],
  ["hour", 3600],
  ["minute", 60],
];

let rtf: Intl.RelativeTimeFormat | undefined;

/** "2 hours ago", "yesterday", "just now". */
export function relativeTime(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const secs = Math.round((t - now) / 1000);
  if (Math.abs(secs) < 60) return "just now";
  rtf ??= new Intl.RelativeTimeFormat("en", { numeric: "auto" });
  for (const [unit, size] of UNITS) {
    if (Math.abs(secs) >= size) return rtf.format(Math.round(secs / size), unit);
  }
  return rtf.format(Math.round(secs / 60), "minute");
}

/** Friendly copy for a failed project command. Null means "not an error" (user cancelled). */
export function describeError(e: AppError): string | null {
  switch (e.code) {
    case "cancelled":
      return null;
    case "notAProject":
      return "This folder isn't a Kinetra project.";
    case "notEmpty":
      return "A folder with that name already exists there and isn't empty. Try another name or location.";
    case "notFound":
      return "That folder doesn't exist anymore.";
    case "io":
      return `Couldn't read or write the folder. ${e.message}`;
    case "busy":
      return "Something else is still running. Try again in a moment.";
    // newerVersion's message already says the project is from a newer version; invalid's message
    // says what is wrong with the input.
    default:
      return e.message;
  }
}
