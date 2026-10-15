import { describe, expect, it } from "vitest";
import type { CloudView } from "$lib/types";
import { gb, retentionWords, stateWords } from "./words";

const quota = {
  plan: "pro_monthly",
  usedBytes: 7_340_032_000,
  pendingBytes: 0,
  limitBytes: 20e9,
  percent: 83,
  warnPercent: 80,
  projectsUsed: 12,
  projectsLimit: 100,
  maxFileBytes: 5e9,
  retentionDays: 30,
  keptPerProject: 50,
  uploadTodayUsed: 1.2e9,
  uploadTodayLimit: 40e9,
  uploadTodayResetsAt: null,
  purgeAt: null,
  retentionDaysNext: null,
  retentionFrom: null,
};

const view = (patch: Partial<CloudView>): CloudView => ({
  access: "available",
  state: "ok",
  offline: false,
  checkedAt: 1,
  quota,
  settings: { onAir: "pause", limitKbps: 2000, intervalS: 300, newProjects: true },
  ...patch,
});

describe("cloud words", () => {
  it("formats sizes in GB", () => {
    expect(gb(7_340_032_000)).toBe("7.34 GB");
    expect(gb(20e9)).toBe("20 GB");
    expect(gb(12.5e9)).toBe("12.5 GB");
    expect(gb(0)).toBe("0 GB");
  });

  it("says the contract's quiet texts, and nothing when all is well", () => {
    expect(stateWords(view({}))).toBeNull();
    expect(stateWords(view({ access: "free" }))).toBeNull();
    expect(stateWords(view({ state: "warning" }))).toEqual({ text: "Cloud storage is 83% full.", tone: "warning" });
    expect(stateWords(view({ state: "full" }))?.text).toMatch(/^Cloud storage is full\. Your project is saved on this computer/);
    expect(stateWords(view({ state: "lapsed", quota: { ...quota, purgeAt: "2026-12-30T12:00:00Z" } }))?.text).toBe(
      "Your plan no longer includes cloud backup. Your backups can be downloaded until Dec 30, 2026.",
    );
    // Offline wins: the last state may be stale.
    expect(stateWords(view({ state: "full", offline: true }))).toEqual({ text: "Cloud backup: offline, will retry.", tone: "plain" });
  });

  it("names the date a shorter history starts", () => {
    expect(retentionWords(view({}))).toBeNull();
    expect(retentionWords(view({ quota: { ...quota, retentionDaysNext: 30, retentionFrom: "2026-11-14T12:00:00Z" } }))).toBe(
      "Versions older than 30 days are deleted from Nov 14, 2026.",
    );
  });
});
