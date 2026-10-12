import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Prefs } from "$lib/types";

const api = vi.hoisted(() => ({
  getPrefs: vi.fn(),
  setPrefs: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) => ({ code: "internal", message: String(e) }),
}));

import ThemeToggle from "./ThemeToggle.svelte";
import { prefs } from "$lib/stores/prefs.svelte";
import { theme } from "$lib/stores/theme.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

const basePrefs: Prefs = {
  theme: "light",
  backup: { enabled: true, intervalMinutes: 15, retention: 10 },
};

describe("ThemeToggle", () => {
  beforeEach(() => {
    prefs.set(null);
    theme.set("system");
    localStorage.clear();
    api.setPrefs.mockReset();
  });
  afterEach(() => {
    cleanup();
    toasts.clear();
  });

  it("works without startViewTransition (jsdom)", () => {
    expect((document as { startViewTransition?: unknown }).startViewTransition).toBeUndefined();
  });

  it("flips data-theme and aria-pressed on click", async () => {
    render(ThemeToggle);
    const btn = screen.getByRole("button");
    expect(btn.getAttribute("aria-pressed")).toBe("false");
    expect(btn.getAttribute("aria-label")).toBe("Switch to dark theme");

    await fireEvent.click(btn);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(btn.getAttribute("aria-pressed")).toBe("true");
    expect(btn.getAttribute("aria-label")).toBe("Switch to light theme");

    await fireEvent.click(btn);
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(btn.getAttribute("aria-pressed")).toBe("false");
  });

  it("persists through prefs when loaded", async () => {
    api.setPrefs.mockImplementation(async (p: Prefs) => p);
    prefs.set(basePrefs);
    render(ThemeToggle);
    await fireEvent.click(screen.getByRole("button"));
    expect(api.setPrefs).toHaveBeenCalledWith({ ...basePrefs, theme: "dark" });
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("follows the rollback when saving fails", async () => {
    api.setPrefs.mockRejectedValue({ code: "io", message: "disk full" });
    prefs.set(basePrefs);
    render(ThemeToggle);
    const btn = screen.getByRole("button");

    await fireEvent.click(btn);
    await vi.waitFor(() => expect(toasts.list.length).toBe(1));
    await tick();
    expect(prefs.value?.theme).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(btn.getAttribute("aria-pressed")).toBe("false");
    expect(toasts.list[0].kind).toBe("error");
  });
});
