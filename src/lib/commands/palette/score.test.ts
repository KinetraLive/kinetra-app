import { describe, expect, it } from "vitest";
import { TIER, boundaries, fold, score } from "./score";

const s = (q: string, t: string) => score(q, t)?.score ?? null;
const tier = (q: string, t: string) => Math.floor((s(q, t) ?? -TIER) / TIER);

describe("score", () => {
  it("matches subsequences only", () => {
    expect(score("", "Toggle grid")).toEqual({ score: 0, ranges: [] });
    expect(s("tg", "Toggle grid")).not.toBeNull();
    expect(s("xyz", "Toggle grid")).toBeNull();
    expect(s("gridd", "Toggle grid")).toBeNull();
    expect(s("toggle grid and more", "Toggle grid")).toBeNull();
  });

  it("is case-insensitive and ignores surrounding / repeated spaces", () => {
    expect(score("  CLEAR   screen ", "Clear Screen 1")?.ranges).toEqual([[0, 12]]);
  });

  it("tiers: substring > word starts > consecutive > scattered", () => {
    expect(tier("screen", "Clear Screen 1")).toBe(3);
    expect(tier("cs", "Clear Screen 1")).toBe(2);
    expect(tier("clsc", "Clear Screen 1")).toBe(2);
    expect(tier("lesc", "Clear Screen 1")).toBe(1);
    expect(tier("lr", "Clear Screen 1")).toBe(0);
    expect(s("screen", "Clear Screen")!).toBeGreaterThan(s("scrn", "Clear Screen")!);
    expect(s("cs", "Clear Screen")!).toBeGreaterThan(s("cs", "Picks")!);
  });

  it("prefers a substring at a word start, then an earlier one", () => {
    expect(score("grid", "Hide grid")?.ranges).toEqual([[5, 9]]);
    expect(s("grid", "Grid size")!).toBeGreaterThan(s("grid", "Hide grid")!);
    expect(s("grid", "Hide grid")!).toBeGreaterThan(s("grid", "Ingrid")!);
    // "on" occurs mid-word first ("Toggle cONtrols"), but the word-start "On air" wins.
    expect(score("on", "Controls on air")?.ranges).toEqual([[9, 11]]);
  });

  it("word starts include camelCase, kebab, snake, digits and paths", () => {
    expect(tier("lt", "lowerThird")).toBe(2);
    expect(tier("lt", "lower-third")).toBe(2);
    expect(tier("lt", "lower_third")).toBe(2);
    expect(tier("s2", "Screen2")).toBe(2);
    expect(tier("st", "screens/title")).toBe(2);
    expect(Array.from(boundaries("aB-c d1"))).toEqual([9, 8, 0, 9, 0, 9, 8]);
    expect(tier("sg", "Screen—Guest")).toBe(2);
    expect(tier("sg", "Screen · Guest")).toBe(2);
    expect(tier("ag", "#a!g")).toBe(0);
  });

  it("ranges merge adjacent characters", () => {
    expect(score("clsc", "Clear Screen")?.ranges).toEqual([
      [0, 2],
      [6, 8],
    ]);
    expect(score("go keynote", "Go to Opening keynote")?.ranges).toEqual([
      [0, 2],
      [14, 21],
    ]);
  });

  it("shorter text wins ties", () => {
    expect(s("take", "Take")!).toBeGreaterThan(s("take", "Take all")!);
    expect(s("tk", "Take")!).toBeGreaterThan(s("tk", "Take all")!);
  });

  it("is accent-insensitive but keeps æ ø å distinct", () => {
    expect(s("cafe", "Café")).not.toBeNull();
    expect(s("café", "Cafe")).not.toBeNull();
    expect(s("uber", "Über")).not.toBeNull();
    expect(s("blabaer", "Blåbær")).toBeNull();
    expect(s("blabar", "Blåbær")).toBeNull();
    expect(s("blåbær", "BLÅBÆR")).not.toBeNull();
    expect(s("ost", "Østfold")).toBeNull();
    expect(s("øst", "Østfold")).not.toBeNull();
    expect(fold("Åsa Ærø é")).toBe("åsa ærø e");
    expect(fold("Åsa").length).toBe(3);
  });

  it("ranges point into the original text", () => {
    expect(score("ae", "Café Ærø")?.ranges).toEqual([[1, 2], [3, 4]]);
  });

  it("long texts and queries don't throw", () => {
    const long = "word ".repeat(200);
    expect(s("wwww", long)).not.toBeNull();
    expect(s("x".repeat(100), long)).toBeNull();
  });
});
