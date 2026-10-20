import { describe, expect, it } from "vitest";
import type { CloudRestoreView } from "$lib/types";
import { damagedList, joinPath, leaf, restoreWords, resultWords } from "./words";

const base: CloudRestoreView = {
  mode: "newFolder",
  versionId: "v1",
  folder: "C:\\Shows\\gala-2026-10-15-1402",
  phase: "downloading",
  doneBytes: 50,
  totalBytes: 200,
  paused: false,
  limited: false,
  result: null,
};

describe("restore words", () => {
  it("folder names", () => {
    expect(leaf("C:\\Shows\\gala-old\\")).toBe("gala-old");
    expect(leaf("/Users/a/gala")).toBe("gala");
    expect(joinPath("C:\\Shows\\", "gala")).toBe("C:\\Shows\\gala");
    expect(joinPath("/Users/a", "gala")).toBe("/Users/a/gala");
  });

  it("progress", () => {
    expect(restoreWords(base)).toBe("Restoring… 25%");
    expect(restoreWords({ ...base, limited: true })).toBe("Restoring slowly while on air… 25%");
    expect(restoreWords({ ...base, paused: true })).toBe("Restore paused while on air");
    expect(restoreWords({ ...base, phase: "backingUp", totalBytes: 0 })).toBe("Backing up the project first…");
    expect(restoreWords({ ...base, phase: "replacing" })).toBe("Replacing the project's files…");
  });

  it("results", () => {
    const ok = { ok: true, error: null, damaged: [], damagedCount: 0, copyFolder: null, reused: 0 };
    expect(resultWords(base)).toBeNull();
    expect(resultWords({ ...base, result: ok })).toEqual({ text: "Restored into gala-2026-10-15-1402.", tone: "plain" });
    expect(resultWords({ ...base, mode: "replace", result: ok })?.text).toBe("The project now matches this version.");
    const damaged = { ...ok, damaged: ["a.png", "b.png"], damagedCount: 7 };
    expect(resultWords({ ...base, result: damaged })).toEqual({
      text: "Restored into gala-2026-10-15-1402. 7 files couldn't be restored intact.",
      tone: "warning",
    });
    expect(damagedList({ ...base, result: damaged }, 1)).toEqual({ files: ["a.png"], more: 6 });
    const failed = { ...ok, ok: false, error: "Nothing was replaced: 1 file couldn't be restored intact." };
    expect(resultWords({ ...base, mode: "replace", result: failed })).toEqual({ text: failed.error, tone: "warning" });
  });
});
