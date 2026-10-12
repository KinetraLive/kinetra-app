import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("$lib/api", () => ({
  getPrefs: vi.fn(),
  setPrefs: vi.fn(),
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));

import { resolveTheme, theme, THEME_STORAGE_KEY } from "./theme.svelte";
import { prefs } from "./prefs.svelte";
import type { Theme } from "$lib/types";

describe("resolveTheme", () => {
  const cases: [Theme, boolean, "light" | "dark"][] = [
    ["system", false, "light"],
    ["system", true, "dark"],
    ["light", false, "light"],
    ["light", true, "light"],
    ["dark", false, "dark"],
    ["dark", true, "dark"],
  ];
  it.each(cases)("pref=%s systemDark=%s -> %s", (pref, systemDark, expected) => {
    expect(resolveTheme(pref, systemDark)).toBe(expected);
  });
});

describe("theme store", () => {
  beforeEach(() => {
    prefs.set(null);
    theme.set("system");
    localStorage.clear();
  });

  it("set() applies data-theme and mirrors to localStorage", () => {
    theme.set("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    expect(theme.pref).toBe("dark");

    theme.set("system");
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("system");
    expect(theme.resolved).toBe("light"); // jsdom has no matchMedia -> not dark
  });

  it("toggle() flips the resolved theme to an explicit choice", () => {
    theme.toggle();
    expect(theme.pref).toBe("dark");
    theme.toggle();
    expect(theme.pref).toBe("light");
  });

  it("prefs take precedence over the mirror once loaded", () => {
    theme.set("dark");
    prefs.set({ theme: "light", backup: { enabled: true, intervalMinutes: 10, retention: 5 } });
    expect(theme.pref).toBe("light");
  });
});
