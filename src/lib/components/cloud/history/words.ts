// How the history reads (pure): days, a version's facts and marks, a cloud project's line.

import type { CloudProjectSummary, CloudVersionView } from "$lib/types";
import { day, size } from "../words";

const at = (iso: string) => new Date(iso);

/** "Today", "Yesterday" or "Oct 15, 2026" for `iso`, relative to `now`. */
export function dayLabel(iso: string, now = new Date()): string {
  const d = at(iso);
  if (Number.isNaN(d.getTime())) return "";
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (diff === 0) return "Today";
  if (diff === 1) return "Yesterday";
  return day(iso);
}

/** "2:02 PM". */
export function timeLabel(iso: string): string {
  const d = at(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
}

/** Versions grouped by day, in order (newest first). */
export function byDay(versions: CloudVersionView[], now = new Date()): { day: string; versions: CloudVersionView[] }[] {
  const out: { day: string; versions: CloudVersionView[] }[] = [];
  for (const v of versions) {
    const d = dayLabel(v.createdAt, now);
    const last = out.at(-1);
    if (last && last.day === d) last.versions.push(v);
    else out.push({ day: d, versions: [v] });
  }
  return out;
}

/** "312 files · 1.83 GB", plus " · 4.2 MB new" when it stored anything for the first time. */
export function versionFacts(v: CloudVersionView): string {
  const files = v.fileCount === 1 ? "1 file" : `${v.fileCount.toLocaleString("en-US")} files`;
  const parts = [files, size(v.logicalBytes)];
  if (v.newBytes > 0) parts.push(`${size(v.newBytes)} new`);
  return parts.join(" · ");
}

/** The marks a version shows, most important first. */
export function versionBadges(v: CloudVersionView): string[] {
  const b: string[] = [];
  if (v.isHead) b.push("Newest");
  if (v.kept) b.push("Kept");
  if (v.conflict) b.push("Also backed up elsewhere");
  if (v.damaged) b.push("Damaged");
  return b;
}

/** "Deleted on Nov 14, 2026" for a version the plan's history length will remove. */
export function expiresWords(v: CloudVersionView): string | null {
  return v.expiresAt ? `Deleted on ${day(v.expiresAt)}` : null;
}

/** A cloud project's second line: "Last backup Oct 15, 2026 from STUDIO-PC · 41 versions". */
export function projectLine(p: CloudProjectSummary): string {
  if (p.trashedAt) return p.purgeAt ? `In the trash · deleted on ${day(p.purgeAt)}` : "In the trash";
  const parts: string[] = [];
  if (p.lastAt) parts.push(`Last backup ${day(p.lastAt)}${p.lastMachine ? ` from ${p.lastMachine}` : ""}`);
  else parts.push("Not backed up yet");
  parts.push(p.versions === 1 ? "1 version" : `${p.versions} versions`);
  return parts.join(" · ");
}
