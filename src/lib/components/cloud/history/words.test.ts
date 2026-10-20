import { describe, expect, it } from "vitest";
import type { CloudProjectSummary, CloudVersionView } from "$lib/types";
import { byDay, dayLabel, expiresWords, projectLine, versionBadges, versionFacts } from "./words";

const v = (patch: Partial<CloudVersionView>): CloudVersionView => ({
  id: "v",
  createdAt: "2026-10-20T12:02:00Z",
  machine: "STUDIO-PC",
  label: null,
  kept: false,
  isHead: false,
  damaged: false,
  conflict: false,
  fileCount: 312,
  logicalBytes: 1.83e9,
  newBytes: 0,
  reason: "auto",
  expiresAt: null,
  ...patch,
});

describe("history words", () => {
  const now = new Date("2026-10-20T18:00:00");

  it("names days relative to today", () => {
    expect(dayLabel("2026-10-20T08:00:00", now)).toBe("Today");
    expect(dayLabel("2026-10-19T23:00:00", now)).toBe("Yesterday");
    expect(dayLabel("2026-10-15T12:00:00", now)).toBe("Oct 15, 2026");
  });

  it("groups versions by day in order", () => {
    const groups = byDay(
      [
        v({ id: "a", createdAt: "2026-10-20T10:00:00" }),
        v({ id: "b", createdAt: "2026-10-20T09:00:00" }),
        v({ id: "c", createdAt: "2026-10-18T09:00:00" }),
      ],
      now,
    );
    expect(groups.map((g) => [g.day, g.versions.map((x) => x.id)])).toEqual([
      ["Today", ["a", "b"]],
      ["Oct 18, 2026", ["c"]],
    ]);
  });

  it("says what a version holds and what marks it", () => {
    expect(versionFacts(v({}))).toBe("312 files · 1.83 GB");
    expect(versionFacts(v({ fileCount: 1, logicalBytes: 5000, newBytes: 5000 }))).toBe("1 file · 5 KB · 5 KB new");
    expect(versionBadges(v({ isHead: true, kept: true, damaged: true }))).toEqual(["Newest", "Kept", "Damaged"]);
    expect(expiresWords(v({ expiresAt: "2026-11-14T12:00:00Z" }))).toBe("Deleted on Nov 14, 2026");
    expect(expiresWords(v({}))).toBeNull();
  });

  it("describes a cloud project in one line", () => {
    const p: CloudProjectSummary = {
      id: "p",
      name: "Gala",
      versions: 41,
      kept: 3,
      conflict: false,
      lastAt: "2026-10-15T12:00:00Z",
      lastMachine: "STUDIO-PC",
      trashedAt: null,
      purgeAt: null,
      open: false,
      here: true,
    };
    expect(projectLine(p)).toBe("Last backup Oct 15, 2026 from STUDIO-PC · 41 versions");
    expect(projectLine({ ...p, trashedAt: "x", purgeAt: "2026-10-27T12:00:00Z" })).toBe("In the trash · deleted on Oct 27, 2026");
    expect(projectLine({ ...p, lastAt: null, versions: 1 })).toBe("Not backed up yet · 1 version");
  });
});
