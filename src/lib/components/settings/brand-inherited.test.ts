// Style & brand when the style itself leaves a color transparent: the inherited value shows as
// such (checkerboard swatch, "transparent" placeholder), and the toggle overrides it.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import "$lib/components/templates/list/matchers";
import { project } from "$lib/stores/project.svelte";
import type { BrandKit, Project } from "$lib/types";

const api = vi.hoisted(() => ({
  on: vi.fn(() => () => {}),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  serverStatus: vi.fn(),
  listStyles: vi.fn(),
  listPresets: vi.fn(() => Promise.resolve([])),
  saveBrandKit: vi.fn(),
}));
vi.mock("$lib/api", () => ({ ...api, toAppError: (e: unknown) => e }));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/components/styles/apply", () => ({ applyToProject: vi.fn() }));
vi.mock("$lib/components/assets/pick", () => ({ pickAsset: vi.fn() }));
// The style's lower third with a transparent plate (the real look otherwise).
vi.mock("$lib/components/templates/gallery/presets", async (original) => {
  const real = await original<typeof import("$lib/components/templates/gallery/presets")>();
  return {
    ...real,
    loadPresetLook: async (...args: Parameters<typeof real.loadPresetLook>) => {
      const look = await real.loadPresetLook(...args);
      return look && { ...look, vars: { ...look.vars, plate: "transparent" } };
    },
  };
});

import StyleBrand from "./StyleBrand.svelte";

const PROJECT: Project = {
  path: "D:\\Shows\\News",
  name: "News",
  createdAt: "2026-01-01T00:00:00Z",
  settings: {
    port: 7770,
    lanAccess: false,
    defaultResolution: { width: 1920, height: 1080 },
    style: "frame",
    brand: {},
  },
};

beforeEach(() => {
  project.replace(PROJECT);
  api.serverStatus.mockResolvedValue({ state: "stopped" });
  api.listStyles.mockResolvedValue([{ id: "frame", name: "Frame", description: "" }]);
  api.saveBrandKit.mockImplementation(async (b: BrandKit) => b);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

it("an inherited transparent color shows the checkerboard; the toggle makes it the project's own", async () => {
  render(StyleBrand);
  const hex = screen.getByLabelText("Plate color") as HTMLInputElement;
  await waitFor(() => expect(hex).toHaveAttribute("placeholder", "transparent"), { timeout: 5000 });
  expect(hex.value).toBe("");
  const swatch = hex.closest(".settings-row")!.querySelector(".swatch")!;
  expect(swatch.classList.contains("checker")).toBe(true);
  expect(swatch.classList.contains("inherited")).toBe(true);
  // Not the project's own value: the toggle is off, the accent row has no checkerboard.
  const toggle = screen.getByRole("button", { name: "Transparent Plate color" });
  expect(toggle).toHaveAttribute("aria-pressed", "false");
  const accent = (screen.getByLabelText("Accent color") as HTMLInputElement).closest(".settings-row")!;
  expect(accent.querySelector(".swatch")!.classList.contains("checker")).toBe(false);

  await fireEvent.click(toggle);
  expect(api.saveBrandKit).toHaveBeenLastCalledWith({ plate: "transparent" });
  await waitFor(() => expect(toggle).toHaveAttribute("aria-pressed", "true"));
  expect(hex.value).toBe("transparent");
});
