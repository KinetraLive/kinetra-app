import { describe, expect, it } from "vitest";
import type { CloudView } from "$lib/types";
import { size, questionWords, retentionWords, stateWords } from "./words";

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
  project: null,
  ...patch,
});

describe("cloud words", () => {
  it("formats sizes in decimal units", () => {
    expect(size(7_340_032_000)).toBe("7.34 GB");
    expect(size(20e9)).toBe("20 GB");
    expect(size(12.5e9)).toBe("12.5 GB");
    expect(size(0)).toBe("0 MB");
    // Small amounts stay readable (a dev account has 200 MB).
    expect(size(200e6)).toBe("200 MB");
    expect(size(13_100_000 + 20_000)).toBe("13.1 MB");
    expect(size(20_000)).toBe("20 KB");
    expect(size(120)).toBe("1 KB");
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

describe("cloud questions", () => {
  it("offers the trash choices with the date", () => {
    const w = questionWords({ kind: "inTrash", purgeAt: "2026-10-22T10:00:00Z" });
    expect(w.text).toBe("This project's cloud backup is in the trash until Oct 22, 2026.");
    expect(w.choices.map((c) => c.answer)).toEqual(["restore", "startNew", "turnOff"]);
  });

  it("names the other computer", () => {
    const w = questionWords({ kind: "otherComputer", machine: "STUDIO-PC", at: null });
    expect(w.text).toBe("This project is already backed up from STUDIO-PC.");
    expect(w.choices.map((c) => c.label)).toEqual(["Continue that backup here", "Back up as a separate project", "Don't back up here"]);
    expect(questionWords({ kind: "otherComputer", machine: null, at: null }).text).toBe(
      "This project is already backed up from another computer.",
    );
  });
});
