import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../list/matchers";
import type { PresetInfo, StyleInfo } from "$lib/types";
import Gallery from "./Gallery.svelte";
import { loadGallery } from "./launcher.svelte";

const api = vi.hoisted(() => ({
  createTemplate: vi.fn(),
  listPresets: vi.fn(),
  listStyles: vi.fn(),
  getTemplate: vi.fn(),
  setDefaultTemplateVariant: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
// Domain modules share the same mock functions (the split only changes where they are imported from).
vi.mock("$lib/api/templates", () => api);

const nav = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => ({ goto: nav.goto }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/templates") } }));

// The live preview (its own chunk) and the apply flow are mocked: their own tests cover them.
const preview = vi.hoisted(() => ({ showPreview: vi.fn(), hidePreview: vi.fn() }));
vi.mock("./preview", () => preview);
const apply = vi.hoisted(() => ({ applyToProject: vi.fn(), applyToLayer: vi.fn() }));
vi.mock("$lib/components/styles/apply", () => apply);

const FRAME = { id: "frame", name: "Frame" };
const CLASSIC = { id: "classic", name: "Classic" };
const presets: PresetInfo[] = [
  {
    id: "lower-third",
    name: "Lower third",
    description: "Name and title.",
    version: 1,
    kind: "lowerThird",
    variants: [FRAME],
  },
  {
    id: "countdown",
    name: "Countdown",
    description: "A countdown.",
    version: 2,
    kind: "countdown",
    variants: [CLASSIC, FRAME],
  },
  { id: "leaderboard", name: "Leaderboard", description: "Ranked rows.", version: 1, kind: null },
  { id: "classic-bar", name: "Classic bar", description: "A solid bar with name and title.", version: 1, legacy: true },
  { id: "minimal-line", name: "Minimal line", description: "A thin line.", version: 1, legacy: true },
];
// "Minimal" is listed but no preset ships it yet: no chip.
const styles: StyleInfo[] = [
  { id: "minimal", name: "Minimal", description: "Quiet." },
  { id: "frame", name: "Frame", description: "Brackets." },
];

beforeEach(() => {
  api.listPresets.mockResolvedValue(presets);
  api.listStyles.mockResolvedValue(styles);
  api.createTemplate.mockResolvedValue({ id: "new1", defaultVariant: "classic" });
  api.setDefaultTemplateVariant.mockResolvedValue({});
  api.getTemplate.mockResolvedValue({ id: "t1", preset: { id: "lower-third", version: 1 } });
  preview.showPreview.mockResolvedValue(true);
  nav.goto.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const names = (list: HTMLElement) =>
  within(list)
    .queryAllByRole("option")
    .map((o) => o.querySelector(".name")?.textContent);

async function openGallery(layer?: { screenId: string; layerId: string; name: string; templateId: string }) {
  const onclose = vi.fn();
  render(Gallery, { open: true, onclose, layer });
  await waitFor(() => expect(screen.getAllByRole("option").length).toBeGreaterThan(0));
  await waitFor(() => expect(screen.getByRole("group", { name: "Style" })).toBeInTheDocument());
  return onclose;
}

const chip = (name: string) => within(screen.getByRole("group", { name: "Style" })).getByRole("button", { name });

describe("Gallery", () => {
  it("one card per library type with its styles, then More; Legacy is collapsed until opened", async () => {
    await openGallery();
    expect(screen.getByRole("heading", { name: "New template" })).toBeInTheDocument();
    const library = screen.getByRole("listbox", { name: "Start from" });
    expect(names(library)).toEqual(["Blank", "Lower third", "Countdown"]);
    const countdown = within(library).getByRole("option", { name: /^Countdown/ });
    expect(countdown.querySelector(".styles")).toHaveAttribute("aria-label", "Styles: Classic, Frame");
    // The card shows its default (first) look.
    expect(countdown.querySelector(".style.on")).toHaveTextContent("Classic");
    expect(names(screen.getByRole("listbox", { name: "More" }))).toEqual(["Leaderboard"]);

    const legacy = screen.getByRole("button", { name: /Legacy/ });
    expect(legacy).toHaveAttribute("aria-expanded", "false");
    expect(legacy).toHaveTextContent("2");
    expect(screen.queryByRole("listbox", { name: "Legacy" })).toBeNull();
    await fireEvent.click(legacy);
    expect(legacy).toHaveAttribute("aria-expanded", "true");
    expect(names(screen.getByRole("listbox", { name: "Legacy" }))).toEqual(["Classic bar", "Minimal line"]);
    await waitFor(() => expect(document.activeElement).toBe(within(library).getAllByRole("option")[0]));
  });

  it("the style filter: All, every style a preset ships and Classic; a style shows only the presets that ship it", async () => {
    await openGallery();
    const chips = within(screen.getByRole("group", { name: "Style" })).getAllByRole("button");
    expect(chips.map((c) => c.textContent?.trim())).toEqual(["All", "Frame", "Classic"]);
    expect(chip("All")).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: /to project/ })).toBeNull();

    await fireEvent.click(chip("Frame"));
    expect(chip("Frame")).toHaveAttribute("aria-pressed", "true");
    expect(names(screen.getByRole("listbox", { name: "Start from" }))).toEqual(["Lower third", "Countdown"]);
    expect(screen.queryByRole("listbox", { name: "More" })).toBeNull();
    expect(screen.queryByRole("button", { name: /Legacy/ })).toBeNull();
    // Cards show the chosen look.
    const countdown = screen.getByRole("option", { name: /^Countdown/ });
    expect(countdown.querySelector(".style.on")).toHaveTextContent("Frame");
    expect(screen.getByRole("button", { name: "Apply Frame to project" })).toBeInTheDocument();

    await fireEvent.click(chip("Classic"));
    expect(names(screen.getByRole("listbox", { name: "Start from" }))).toEqual(["Countdown"]);
    await fireEvent.click(chip("All"));
    expect(names(screen.getByRole("listbox", { name: "Start from" }))).toEqual(["Blank", "Lower third", "Countdown"]);
  });

  it("a card's thumbnail is its look's static image (else the preset's own)", async () => {
    await openGallery();
    await fireEvent.click(screen.getByRole("button", { name: /Legacy/ }));
    await waitFor(() =>
      expect(screen.getByRole("option", { name: /^Classic bar/ }).querySelector(".thumb img")?.getAttribute("src")).toMatch(
        /\/presets\/thumbs\/classic-bar\.png/,
      ),
    );
    expect(screen.getByRole("option", { name: /^Classic bar/ }).querySelector(".checker-dark")).not.toBeNull();
  });

  it("hover and focus play ONE live preview at a time, released when the pointer and focus leave", async () => {
    await openGallery();
    const lt = screen.getByRole("option", { name: /^Lower third/ });
    const cd = screen.getByRole("option", { name: /^Countdown/ });

    await fireEvent.pointerEnter(lt);
    await waitFor(() => expect(preview.showPreview).toHaveBeenCalledTimes(1));
    const first = lt.querySelector<HTMLElement>("[data-preview]")!;
    expect(preview.showPreview).toHaveBeenLastCalledWith(first, "lower-third", "frame");
    expect(document.querySelectorAll("[data-preview]")).toHaveLength(1);

    await fireEvent.pointerLeave(lt);
    await fireEvent.pointerEnter(cd);
    await waitFor(() => expect(preview.showPreview).toHaveBeenCalledTimes(2));
    expect(preview.hidePreview).toHaveBeenCalledWith(first);
    expect(preview.showPreview).toHaveBeenLastCalledWith(cd.querySelector("[data-preview]"), "countdown", "classic");
    expect(document.querySelectorAll("[data-preview]")).toHaveLength(1);
    expect(lt.querySelector("[data-preview]")).toBeNull();

    await fireEvent.pointerLeave(cd);
    expect(document.querySelectorAll("[data-preview]")).toHaveLength(0);
    await waitFor(() => expect(preview.hidePreview).toHaveBeenCalledTimes(2));

    // Keyboard focus previews too; a chosen style plays that look.
    await fireEvent.click(chip("Frame"));
    await fireEvent.focus(screen.getByRole("option", { name: /^Countdown/ }));
    await waitFor(() => expect(preview.showPreview).toHaveBeenLastCalledWith(expect.any(HTMLElement), "countdown", "frame"));
    // Blank has no preview.
    await fireEvent.click(chip("All"));
    await fireEvent.pointerEnter(screen.getByRole("option", { name: /^Blank/ }));
    expect(screen.getByRole("option", { name: /^Blank/ }).querySelector("[data-preview]")).toBeNull();
  });

  it("the name input has no placeholder and is filled from the chosen preset", async () => {
    await openGallery();
    const input = screen.getByLabelText("Name") as HTMLInputElement;
    expect(input).not.toHaveAttribute("placeholder");
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();

    await fireEvent.click(screen.getByRole("option", { name: /^Countdown/ }));
    expect(screen.getByRole("option", { name: /^Countdown/ })).toHaveAttribute("aria-selected", "true");
    expect(input.value).toBe("Countdown");
  });

  it("Create sends the preset source, closes and opens the editor", async () => {
    const onclose = await openGallery();
    await fireEvent.click(screen.getByRole("option", { name: /^Countdown/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(api.createTemplate).toHaveBeenCalledWith("Countdown", { kind: "preset", presetId: "countdown" }),
    );
    await waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/templates/new1"));
    expect(api.setDefaultTemplateVariant).not.toHaveBeenCalled();
    expect(onclose).toHaveBeenCalled();
  });

  it("Create with a style chosen makes that look the new template's default", async () => {
    await openGallery();
    await fireEvent.click(chip("Frame"));
    await fireEvent.click(screen.getByRole("option", { name: /^Countdown/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(api.setDefaultTemplateVariant).toHaveBeenCalledWith("new1", "frame"));
    await waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/templates/new1"));
  });

  it("Blank starts as Untitled template and sends the blank source", async () => {
    await openGallery();
    await fireEvent.keyDown(screen.getAllByRole("option")[0], { key: "Enter" });
    const input = screen.getByLabelText("Name") as HTMLInputElement;
    expect(input.value).toBe("Untitled template");
    expect(screen.getByRole("button", { name: "Create" })).not.toBeDisabled();
    await fireEvent.input(input, { target: { value: "  Score bug  " } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(api.createTemplate).toHaveBeenCalledWith("Score bug", { kind: "blank" }));
  });

  it("choosing a card focuses Name with the default selected, so Enter creates", async () => {
    await openGallery();
    await fireEvent.click(screen.getByRole("option", { name: /^Countdown/ }));
    const input = screen.getByLabelText("Name") as HTMLInputElement;
    await waitFor(() => expect(document.activeElement).toBe(input));
    expect(input.value).toBe("Countdown");
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, "Countdown".length]);
    // Enter in the field submits the form.
    await fireEvent.submit(input.form!);
    await waitFor(() =>
      expect(api.createTemplate).toHaveBeenCalledWith("Countdown", { kind: "preset", presetId: "countdown" }),
    );
    await waitFor(() => expect(nav.goto).toHaveBeenCalledWith("/templates/new1"));
  });

  it("a double-click creates at once with the default name, Blank too", async () => {
    await openGallery();
    await fireEvent.dblClick(screen.getByRole("option", { name: /^Countdown/ }));
    await waitFor(() =>
      expect(api.createTemplate).toHaveBeenCalledWith("Countdown", { kind: "preset", presetId: "countdown" }),
    );
    cleanup();
    vi.clearAllMocks();
    await openGallery();
    await fireEvent.dblClick(screen.getByRole("option", { name: /^Blank/ }));
    await waitFor(() => expect(api.createTemplate).toHaveBeenCalledWith("Untitled template", { kind: "blank" }));
  });

  it("arrow keys move the choice without leaving the cards", async () => {
    await openGallery();
    const library = screen.getByRole("listbox", { name: "Start from" });
    const opts = within(library).getAllByRole("option");
    await fireEvent.keyDown(opts[0], { key: " " });
    opts[0].focus();
    await fireEvent.keyDown(opts[0], { key: "ArrowRight" });
    await waitFor(() => expect(document.activeElement).toBe(opts[1]));
    expect(opts[1]).toHaveAttribute("aria-selected", "true");
    await new Promise((r) => setTimeout(r, 20));
    expect(document.activeElement).toBe(opts[1]);
  });

  it("arrow keys move within a section", async () => {
    await openGallery();
    const library = screen.getByRole("listbox", { name: "Start from" });
    const opts = within(library).getAllByRole("option");
    await fireEvent.keyDown(opts[0], { key: "ArrowRight" });
    await waitFor(() => expect(document.activeElement).toBe(opts[1]));
    await fireEvent.keyDown(opts[1], { key: "End" });
    await waitFor(() => expect(document.activeElement).toBe(opts[2]));
  });

  it("Esc (the dialog's cancel) and Cancel close it without creating", async () => {
    const onclose = await openGallery();
    await fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
    expect(onclose).toHaveBeenCalledTimes(1);
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onclose).toHaveBeenCalledTimes(2);
    expect(api.createTemplate).not.toHaveBeenCalled();
  });

  it("a failed create shows a toast and keeps the gallery open", async () => {
    const { toasts } = await import("$lib/stores/toasts.svelte");
    toasts.clear();
    api.createTemplate.mockRejectedValue({ code: "io", message: "Disk full" });
    const onclose = await openGallery();
    await fireEvent.click(screen.getByRole("option", { name: /^Countdown/ }));
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() => expect(toasts.list.map((t) => t.message).join()).toContain("Disk full"));
    expect(onclose).not.toHaveBeenCalled();
    expect(nav.goto).not.toHaveBeenCalled();
    toasts.clear();
  });

  it("Apply <style> to project runs the apply flow and closes; cancelled, it stays open", async () => {
    const onclose = await openGallery();
    await fireEvent.click(chip("Frame"));
    apply.applyToProject.mockResolvedValueOnce(null); // cancelled at the confirm
    await fireEvent.click(screen.getByRole("button", { name: "Apply Frame to project" }));
    await waitFor(() => expect(apply.applyToProject).toHaveBeenCalledWith(expect.objectContaining({ id: "frame", name: "Frame" })));
    expect(onclose).not.toHaveBeenCalled();
    apply.applyToProject.mockResolvedValueOnce({ switched: 3, unchanged: 0, unchangedTemplates: 0, added: 1, undo: "u1" });
    await fireEvent.click(screen.getByRole("button", { name: "Apply Frame to project" }));
    await waitFor(() => expect(onclose).toHaveBeenCalledTimes(1));
  });

  it("from a layer: its template's card, and Apply to this layer only / to project once a style is chosen", async () => {
    const layer = { screenId: "s1", layerId: "l1", name: "Lower third", templateId: "t1" };
    const onclose = await openGallery(layer);
    expect(screen.getByRole("heading", { name: "Styles for “Lower third”" })).toBeInTheDocument();
    await waitFor(() => expect(names(screen.getByRole("listbox", { name: "Template" }))).toEqual(["Lower third"]));
    expect(api.getTemplate).toHaveBeenCalledWith("t1");
    expect(screen.queryByLabelText("Name")).toBeNull();
    expect(screen.queryByRole("button", { name: /Legacy/ })).toBeNull();
    const only = screen.getByRole("button", { name: "Apply to this layer only" });
    expect(only).toBeDisabled();
    expect(screen.getByRole("button", { name: "Apply to project" })).toBeDisabled();

    await fireEvent.click(chip("Frame"));
    expect(only).not.toBeDisabled();
    apply.applyToLayer.mockResolvedValueOnce({ switched: 1, unchanged: 0, unchangedTemplates: 0, added: 1, undo: "u2" });
    await fireEvent.click(only);
    await waitFor(() =>
      expect(apply.applyToLayer).toHaveBeenCalledWith(expect.objectContaining({ id: "frame" }), {
        screenId: "s1",
        layerId: "l1",
        name: "Lower third",
      }),
    );
    await waitFor(() => expect(onclose).toHaveBeenCalled());
    expect(apply.applyToProject).not.toHaveBeenCalled();
  });

  it("is loadable as a lazy chunk", async () => {
    expect(await loadGallery()).toBe(Gallery);
  });
});
