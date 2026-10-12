import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "$lib/components/templates/list/matchers";
import { project } from "$lib/stores/project.svelte";
import type { BrandKit, Project } from "$lib/types";
import StyleBrand from "./StyleBrand.svelte";
import { clean, colorIssue, pickerHex, withValue } from "./brand";

const api = vi.hoisted(() => ({
  on: vi.fn(() => () => {}),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  serverStatus: vi.fn(),
  listStyles: vi.fn(),
  listPresets: vi.fn(() => Promise.resolve([])),
  saveBrandKit: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/templates", () => api);

const apply = vi.hoisted(() => ({ applyToProject: vi.fn() }));
vi.mock("$lib/components/styles/apply", () => apply);
const pick = vi.hoisted(() => ({ pickAsset: vi.fn() }));
vi.mock("$lib/components/assets/pick", () => pick);

const PROJECT: Project = {
  path: "D:\\Shows\\News",
  name: "News",
  createdAt: "2026-01-01T00:00:00Z",
  settings: {
    port: 7770,
    lanAccess: false,
    defaultResolution: { width: 1920, height: 1080 },
    style: "frame",
    brand: { accent: "#22b8cf" },
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
  vi.useRealTimers();
  vi.clearAllMocks();
});

const input = (label: string) => screen.getByLabelText(label) as HTMLInputElement | HTMLSelectElement;

async function mountForm(options = 2) {
  render(StyleBrand);
  await waitFor(() => expect(input("Style").querySelectorAll("option")).toHaveLength(options));
}

/** The settings row holding the control labelled `label`. */
const rowOf = (label: string) => input(label).closest(".settings-row") as HTMLElement;

/** The preview's layer host (the shadow root the lower third is compiled into). */
function previewLayer(): HTMLElement | null {
  const host = screen.getByRole("img", { name: "Lower third preview" }).querySelector(".thumb");
  return host?.shadowRoot?.querySelector<HTMLElement>(".layer") ?? null;
}
const previewVar = (key: string) => previewLayer()?.style.getPropertyValue(`--${key}`) ?? "";

describe("brand helpers", () => {
  it("validates hex colors; unset is fine", () => {
    expect(colorIssue("")).toBeNull();
    expect(colorIssue("#12abEF")).toBeNull();
    expect(colorIssue("#12abef80")).toBeNull();
    expect(colorIssue("#12ab")).toMatch(/Enter a color like/);
    expect(colorIssue("red")).toMatch(/Enter a color like/);
    expect(pickerHex("#12ABEF80")).toBe("#12abef");
    expect(pickerHex("")).toBe("#000000");
  });

  it("accepts transparent in any case", () => {
    expect(colorIssue("transparent")).toBeNull();
    expect(colorIssue("Transparent")).toBeNull();
    expect(colorIssue("transparentx")).toMatch(/Enter a color like/);
    expect(colorIssue(" transparent")).toMatch(/Enter a color like/);
    expect(pickerHex("transparent")).toBe("#000000");
  });

  it("sets and unsets keys, keeping only values", () => {
    expect(withValue({ accent: "#111111" }, "plate", "#00000080")).toEqual({ accent: "#111111", plate: "#00000080" });
    expect(withValue({ accent: "#111111", logo: "/assets/a.png" }, "accent", null)).toEqual({ logo: "/assets/a.png" });
    expect(clean({ accent: null, text: "", fontBody: "IBM Plex Sans" })).toEqual({ fontBody: "IBM Plex Sans" });
  });
});

describe("Style & brand", () => {
  it("the Style select shows the project's style; choosing one is Apply to project", async () => {
    await mountForm();
    const select = input("Style") as HTMLSelectElement;
    expect(select.value).toBe("frame");
    expect([...select.options].map((o) => o.textContent)).toEqual(["Frame", "Classic"]);
    apply.applyToProject.mockResolvedValueOnce(null); // cancelled
    await fireEvent.change(select, { target: { value: "classic" } });
    await waitFor(() => expect(apply.applyToProject).toHaveBeenCalledWith(expect.objectContaining({ id: "classic" })));
    await waitFor(() => expect((input("Style") as HTMLSelectElement).value).toBe("frame"));
  });

  it("a typed color is checked, then saved after a pause; the project follows", async () => {
    await mountForm();
    vi.useFakeTimers();
    const hex = input("Secondary color") as HTMLInputElement;
    await fireEvent.input(hex, { target: { value: "#12" } });
    expect(screen.getByRole("alert")).toHaveTextContent("Enter a color like");
    expect(hex).toHaveAttribute("aria-invalid", "true");
    await vi.advanceTimersByTimeAsync(1000);
    expect(api.saveBrandKit).not.toHaveBeenCalled();

    await fireEvent.input(hex, { target: { value: "#123456" } });
    expect(screen.queryByRole("alert")).toBeNull();
    await vi.advanceTimersByTimeAsync(399);
    expect(api.saveBrandKit).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(api.saveBrandKit).toHaveBeenCalledWith({ accent: "#22b8cf", secondary: "#123456" });
    vi.useRealTimers();
    await waitFor(() => expect(project.current?.settings.brand).toEqual({ accent: "#22b8cf", secondary: "#123456" }));
  });

  it("a rejected save rolls back and shows the reason under the row", async () => {
    await mountForm();
    api.saveBrandKit.mockRejectedValueOnce({ code: "invalid", message: "Plate must be a colour" });
    const swatch = screen.getByLabelText("Plate color picker") as HTMLInputElement;
    await fireEvent.change(swatch, { target: { value: "#ff0000" } });
    expect(api.saveBrandKit).toHaveBeenCalledWith({ accent: "#22b8cf", plate: "#ff0000" });
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Plate must be a colour"));
    expect((input("Plate color") as HTMLInputElement).value).toBe("");
    expect(project.current?.settings.brand).toEqual({ accent: "#22b8cf" });
  });

  it("Reset unsets a value (the style's own shows again)", async () => {
    await mountForm();
    expect((input("Accent color") as HTMLInputElement).value).toBe("#22b8cf");
    await fireEvent.click(screen.getByRole("button", { name: "Reset Accent to the style" }));
    expect(api.saveBrandKit).toHaveBeenCalledWith({});
    await waitFor(() => expect((input("Accent color") as HTMLInputElement).value).toBe(""));
    expect(screen.queryByRole("button", { name: "Reset Accent to the style" })).toBeNull();
    // The style's own value shows again: dimmed hex (placeholder) and swatch, with the style's name.
    // The inherited look loads lazily (a dynamic import), so allow for a slow worker under load.
    await waitFor(
      () => {
        expect(input("Accent color")).toHaveAttribute("placeholder", "#33d6ff");
        expect((screen.getByLabelText("Accent color picker") as HTMLInputElement).value).toBe("#33d6ff");
        expect(rowOf("Accent color").querySelector(".swatch")?.classList.contains("inherited")).toBe(true);
        expect(rowOf("Accent color")).toHaveTextContent("Frame");
      },
      { timeout: 5000 },
    );
  });

  it("fonts and the logo save on change", async () => {
    await mountForm();
    await fireEvent.change(input("Heading font"), { target: { value: "Space Grotesk" } });
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "#22b8cf", fontHeading: "Space Grotesk" });
    pick.pickAsset.mockResolvedValueOnce("/assets/logo.png");
    await fireEvent.click(screen.getByRole("button", { name: "Choose image for Logo" }));
    await waitFor(() =>
      expect(api.saveBrandKit).toHaveBeenLastCalledWith({
        accent: "#22b8cf",
        fontHeading: "Space Grotesk",
        logo: "/assets/logo.png",
      }),
    );
    await waitFor(() => expect(screen.getByText("/assets/logo.png")).toBeInTheDocument());
  });
});

describe("inherited values", () => {
  it("an unset color shows the style's value dimmed with the style's name; a set one its own + Reset", async () => {
    await mountForm();
    await waitFor(() => expect(input("Secondary color")).toHaveAttribute("placeholder", "#b3bac5"));
    const secondary = rowOf("Secondary color");
    expect((input("Secondary color") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Secondary color picker") as HTMLInputElement).value).toBe("#b3bac5");
    expect(secondary.querySelector(".swatch")?.classList.contains("inherited")).toBe(true);
    expect(secondary.querySelector(".tag")).toHaveTextContent("Frame");
    expect(screen.queryByRole("button", { name: "Reset Secondary to the style" })).toBeNull();

    const accent = rowOf("Accent color");
    expect(accent.querySelector(".swatch")?.classList.contains("inherited")).toBe(false);
    expect(accent.querySelector(".tag")).toBeNull();
    expect(screen.getByRole("button", { name: "Reset Accent to the style" })).toBeInTheDocument();
    // Fonts: the unset option names the style's font and the style.
    expect((input("Heading font") as HTMLSelectElement).options[0].textContent).toBe("Space Grotesk · Frame");
  });

  it("picking a color on an inherited swatch overrides it (the style's alpha is kept)", async () => {
    await mountForm();
    await waitFor(() => expect(input("Plate color")).toHaveAttribute("placeholder", "#0c0d11d9"));
    await fireEvent.change(screen.getByLabelText("Plate color picker"), { target: { value: "#ff0000" } });
    expect(api.saveBrandKit).toHaveBeenCalledWith({ accent: "#22b8cf", plate: "#ff0000d9" });
    await waitFor(() => expect((input("Plate color") as HTMLInputElement).value).toBe("#ff0000d9"));
    expect(rowOf("Plate color").querySelector(".swatch")?.classList.contains("inherited")).toBe(false);
    expect(screen.getByRole("button", { name: "Reset Plate to the style" })).toBeInTheDocument();
  });

  it("follows the project's style", async () => {
    api.listStyles.mockResolvedValue([
      { id: "frame", name: "Frame", description: "" },
      { id: "inline", name: "Inline", description: "" },
    ]);
    await mountForm(3);
    await waitFor(() => expect(input("Text color")).toHaveAttribute("placeholder", "#f5f5f2"));
    project.replace({ ...PROJECT, settings: { ...PROJECT.settings, style: "inline" } });
    await waitFor(() => expect(input("Text color")).toHaveAttribute("placeholder", "#12141a"));
    expect(rowOf("Text color").querySelector(".tag")).toHaveTextContent("Inline");
  });
});

describe("preview", () => {
  it("draws the lower third in the style with the brand kit over it", async () => {
    await mountForm();
    await waitFor(() => expect(previewVar("accent")).toBe("#22b8cf"));
    expect(previewVar("secondary")).toBe("#b3bac5");
    expect(previewLayer()?.shadowRoot?.textContent).toContain("Alex Morgan");
  });

  it("redraws shortly after a value changes, not on every keystroke", async () => {
    await mountForm();
    await waitFor(() => expect(previewVar("secondary")).toBe("#b3bac5"));
    await fireEvent.input(input("Secondary color"), { target: { value: "#123456" } });
    expect(previewVar("secondary")).toBe("#b3bac5"); // debounced
    await waitFor(() => expect(previewVar("secondary")).toBe("#123456"));
    await waitFor(() => expect(api.saveBrandKit).toHaveBeenCalled());
    // Half-typed values never reach it.
    await fireEvent.input(input("Secondary color"), { target: { value: "#12" } });
    await new Promise((r) => setTimeout(r, 250));
    expect(previewVar("secondary")).toBe("#123456");
  });
});

describe("transparent", () => {
  const toggle = (label: string) => screen.getByRole("button", { name: `Transparent ${label} color` });
  const swatchOf = (label: string) => rowOf(`${label} color`).querySelector(".swatch")!;

  it("the toggle saves transparent and shows it; off restores the last own color", async () => {
    await mountForm();
    expect(toggle("Accent")).toHaveAttribute("aria-pressed", "false");
    await fireEvent.click(toggle("Accent"));
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "transparent" });
    await waitFor(() => expect((input("Accent color") as HTMLInputElement).value).toBe("transparent"));
    expect(toggle("Accent")).toHaveAttribute("aria-pressed", "true");
    expect(swatchOf("Accent").classList.contains("checker")).toBe(true);
    await waitFor(() => expect(previewVar("accent")).toBe("transparent"));

    await fireEvent.click(toggle("Accent"));
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "#22b8cf" });
    await waitFor(() => expect(toggle("Accent")).toHaveAttribute("aria-pressed", "false"));
    expect(swatchOf("Accent").classList.contains("checker")).toBe(false);
  });

  it("off without an own color goes back to the style's", async () => {
    await mountForm();
    await fireEvent.click(toggle("Plate"));
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "#22b8cf", plate: "transparent" });
    await waitFor(() => expect(toggle("Plate")).toHaveAttribute("aria-pressed", "true"));
    await fireEvent.click(toggle("Plate"));
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "#22b8cf" });
  });

  it("picking or typing a color turns it off; typed Transparent is stored lowercase", async () => {
    await mountForm();
    await fireEvent.click(toggle("Accent"));
    await waitFor(() => expect(toggle("Accent")).toHaveAttribute("aria-pressed", "true"));
    await fireEvent.change(screen.getByLabelText("Accent color picker"), { target: { value: "#ff0000" } });
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "#ff0000" });
    await waitFor(() => expect(toggle("Accent")).toHaveAttribute("aria-pressed", "false"));

    vi.useFakeTimers();
    await fireEvent.input(input("Secondary color"), { target: { value: "Transparent" } });
    expect(screen.queryByRole("alert")).toBeNull();
    await vi.advanceTimersByTimeAsync(400);
    expect(api.saveBrandKit).toHaveBeenLastCalledWith({ accent: "#ff0000", secondary: "transparent" });
  });
});
