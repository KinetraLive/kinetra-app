import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ToastHost from "$lib/components/ToastHost.svelte";
import "$lib/components/templates/list/matchers";
import { currentContext, provideContext, registerCommands, runFollowUp, titleOf } from "$lib/commands/registry.svelte";
import { project, uiState } from "$lib/stores/project.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import type { ApplyStyleResult, LiveScreenView, Project, UiState } from "$lib/types";
import { applyToLayer, applyToProject, onAirAffected, summary } from "./apply";
import { applyStyleCommand, offeredStyles } from "./command.svelte";
import { CLASSIC, offered, styleHook, withClassic } from "./styles";

const api = vi.hoisted(() => ({
  currentProject: vi.fn(),
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  listScreens: vi.fn(),
  getScreen: vi.fn(),
  listPresets: vi.fn(),
  listTemplates: vi.fn(),
  getTemplate: vi.fn(),
  listStyles: vi.fn(),
  applyStyle: vi.fn(),
  undoApplyStyle: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/templates", () => api);

const FRAME = { id: "frame", name: "Frame" };
const PROJECT: Project = {
  path: "D:\\Shows\\News",
  name: "News",
  createdAt: "2026-01-01T00:00:00Z",
  settings: { port: 7770, lanAccess: false, defaultResolution: { width: 1920, height: 1080 }, style: null },
};
const RESULT: ApplyStyleResult = { switched: 12, unchanged: 3, unchangedTemplates: 2, added: 4, undo: "tok1" };

const layer = (id: string, onAir: boolean, variant = "classic") => ({
  id,
  name: id,
  onAir,
  fields: {},
  fieldDefs: [],
  pending: false,
  variant,
});
const view = (id: string, layers: ReturnType<typeof layer>[]): LiveScreenView => ({
  screen: { id, name: id, width: 1920, height: 1080, background: "transparent" },
  layers,
});

/** Screen s1: "lt" (library lower third, on air), "custom" (blank template, on air), "off" (library, off air). */
function scenario(onAir = true) {
  api.listScreens.mockResolvedValue([view("s1", [layer("lt", onAir), layer("custom", true, "default"), layer("off", false)])]);
  api.getScreen.mockResolvedValue({
    doc: {
      layers: [
        { id: "lt", templateId: "tLt" },
        { id: "custom", templateId: "tCustom" },
        { id: "off", templateId: "tLt" },
      ],
    },
  });
  api.listPresets.mockResolvedValue([{ id: "lower-third", name: "Lower third", description: "", version: 1, variants: [FRAME] }]);
  api.listTemplates.mockResolvedValue([
    { id: "tLt", preset: { id: "lower-third", version: 1 }, variantCount: 1 },
    { id: "tCustom", preset: null, variantCount: 1 },
  ]);
}

function setLiveEdit(liveEdit: "onTake" | "immediate") {
  uiState.set({ control: { liveEdit } } as unknown as UiState);
}

// jsdom has no Web Animations; the toast host's transitions only need the call to exist.
if (!("getAnimations" in Element.prototype)) {
  Object.defineProperty(Element.prototype, "getAnimations", { value: () => [], configurable: true });
}

beforeEach(() => {
  project.replace(PROJECT);
  setLiveEdit("immediate");
  api.currentProject.mockResolvedValue({ ...PROJECT, settings: { ...PROJECT.settings, style: "frame" } });
  api.applyStyle.mockResolvedValue(RESULT);
  api.undoApplyStyle.mockResolvedValue(undefined);
  scenario();
});

afterEach(() => {
  cleanup();
  toasts.clear();
  document.body.replaceChildren();
  vi.clearAllMocks();
});

const confirmDialog = () => screen.queryByRole("dialog");

describe("summary", () => {
  it("says what switched, what couldn't and what was added", () => {
    expect(summary(RESULT, "Frame")).toBe("Frame applied · 12 switched · 2 custom templates unchanged · 4 variants added");
    expect(summary({ switched: 1, unchanged: 1, unchangedTemplates: 1, added: 0, undo: "t" }, "Frame")).toBe(
      "Frame applied · 1 switched · 1 custom template unchanged",
    );
    expect(summary({ switched: 0, unchanged: 0, unchangedTemplates: 0, added: 0, undo: null }, "Frame")).toBe(
      "Frame applied · 0 switched",
    );
    expect(summary({ switched: 1, unchanged: 0, unchangedTemplates: 0, added: 1, undo: "t" }, "Frame", "Guest")).toBe(
      "Frame applied to Guest",
    );
  });
});

describe("apply to project", () => {
  it("counts only on-air layers that would switch (a library template, or one that has the look)", async () => {
    expect(await onAirAffected("frame")).toBe(1);
    api.listTemplates.mockResolvedValue([
      { id: "tLt", preset: { id: "lower-third", version: 1 }, variantCount: 1 },
      { id: "tCustom", preset: null, variantCount: 2 },
    ]);
    api.getTemplate.mockResolvedValue({ variants: [{ id: "default" }, { id: "frame" }] });
    expect(await onAirAffected("frame")).toBe(2);
    scenario(false);
    api.listScreens.mockResolvedValue([view("s1", [layer("lt", false)])]);
    expect(await onAirAffected("frame")).toBe(0);
    expect(api.listPresets).not.toHaveBeenCalledTimes(3); // nothing on air: no further lookups
  });

  it("nothing on air: applies at once, toasts the summary with Undo, refreshes the project", async () => {
    scenario(false);
    api.listScreens.mockResolvedValue([view("s1", [layer("lt", false)])]);
    const r = await applyToProject(FRAME);
    expect(r).toEqual(RESULT);
    expect(confirmDialog()).toBeNull();
    expect(api.applyStyle).toHaveBeenCalledWith("frame", { kind: "project" }, "crossfade");
    expect(toasts.list).toHaveLength(1);
    expect(toasts.list[0].message).toBe("Frame applied · 12 switched · 2 custom templates unchanged · 4 variants added");
    expect(toasts.list[0].action?.label).toBe("Undo");
    await waitFor(() => expect(project.current?.settings.style).toBe("frame"));
  });

  it("layers on air: asks first, saying what Live edit will do; Cancel applies nothing", async () => {
    setLiveEdit("onTake");
    const pending = applyToProject(FRAME);
    const dlg = await screen.findByRole("dialog");
    expect(dlg).toHaveTextContent("Apply Frame to the project?");
    expect(dlg).toHaveTextContent("1 layer on air will switch. They change on the next take.");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await pending).toBeNull();
    expect(api.applyStyle).not.toHaveBeenCalled();
    await waitFor(() => expect(confirmDialog()).toBeNull());
  });

  it("layers on air: Apply in the dialog applies", async () => {
    const pending = applyToProject(FRAME);
    const dlg = await screen.findByRole("dialog");
    expect(dlg).toHaveTextContent("They crossfade to the new look now.");
    await fireEvent.click(screen.getByRole("button", { name: "Apply" }));
    expect(await pending).toEqual(RESULT);
    expect(api.applyStyle).toHaveBeenCalledTimes(1);
  });

  it("Undo in the toast restores the previous looks in one step", async () => {
    render(ToastHost);
    scenario(false);
    await applyToProject(FRAME);
    await fireEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(api.undoApplyStyle).toHaveBeenCalledWith("tok1"));
    await waitFor(() => expect(toasts.list.map((t) => t.message)).toEqual(["Frame undone"]));
    // The Ctrl+Z listener went with the toast.
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true }));
    expect(api.undoApplyStyle).toHaveBeenCalledTimes(1);
  });

  it("Ctrl+Z while the toast is up undoes too (not while typing, and only once)", async () => {
    scenario(false);
    await applyToProject(FRAME);
    const input = document.createElement("input");
    document.body.append(input);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true }));
    expect(api.undoApplyStyle).not.toHaveBeenCalled();

    const e = new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true, cancelable: true });
    document.body.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    await waitFor(() => expect(api.undoApplyStyle).toHaveBeenCalledWith("tok1"));
    expect(toasts.list.some((t) => t.action)).toBe(false);
    document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "z", ctrlKey: true, bubbles: true }));
    expect(api.undoApplyStyle).toHaveBeenCalledTimes(1);
  });

  it("nothing switched: no Undo; a failure toasts the error", async () => {
    scenario(false);
    api.applyStyle.mockResolvedValueOnce({ switched: 0, unchanged: 2, unchangedTemplates: 1, added: 0, undo: null });
    await applyToProject(FRAME);
    expect(toasts.list[0].action).toBeUndefined();
    toasts.clear();
    api.applyStyle.mockRejectedValueOnce({ code: "notFound", message: "No such style" });
    expect(await applyToProject(FRAME)).toBeNull();
    expect(toasts.list.map((t) => [t.kind, t.message])).toEqual([["error", "Couldn't apply Frame: No such style"]]);
  });

  it("hooks: a page flushes before and reloads after an apply and an undo", async () => {
    scenario(false);
    const order: string[] = [];
    const off = styleHook(
      async () => void order.push("before"),
      () => void order.push("after"),
    );
    api.applyStyle.mockImplementationOnce(async () => {
      order.push("apply");
      return RESULT;
    });
    await applyToProject(FRAME);
    expect(order).toEqual(["before", "apply", "after"]);
    toasts.list[0].action!.run();
    await waitFor(() => expect(order).toEqual(["before", "apply", "after", "before", "after"]));
    off();
    await applyToProject(FRAME);
    expect(order).toHaveLength(5);
  });
});

describe("apply to one layer", () => {
  it("applies to the layer (no confirm) and names it in the toast", async () => {
    const r = await applyToLayer(FRAME, { screenId: "s1", layerId: "lt", name: "Guest" });
    expect(r).toEqual(RESULT);
    expect(confirmDialog()).toBeNull();
    expect(api.applyStyle).toHaveBeenCalledWith("frame", { kind: "layer", screenId: "s1", layerId: "lt" }, "crossfade");
    expect(toasts.list[0].message).toBe("Frame applied to Guest");
    expect(toasts.list[0].action?.label).toBe("Undo");
  });

  it("a template without the look: says so, nothing to undo", async () => {
    api.applyStyle.mockResolvedValueOnce({ switched: 0, unchanged: 1, unchangedTemplates: 1, added: 0, undo: null });
    await applyToLayer(FRAME, { screenId: "s1", layerId: "custom", name: "Bug" });
    expect(toasts.list[0]).toMatchObject({ kind: "info", message: "Bug has no Frame look (custom template)" });
  });
});

describe("palette commands", () => {
  const pick = () => {
    const next = applyStyleCommand.next;
    if (next?.kind !== "pick") throw new Error("not a pick");
    return next;
  };

  it("one 'Apply style…' with a pick step: the styles some preset ships (loaded lazily); a pick runs the project apply", async () => {
    offeredStyles.list = null;
    api.listStyles.mockResolvedValue([
      { id: "minimal", name: "Minimal", description: "" },
      { id: "frame", name: "Frame", description: "" },
    ]);
    api.listPresets.mockResolvedValue([
      { id: "lower-third", name: "Lower third", description: "", version: 1, variants: [FRAME] },
      { id: "ticker", name: "Ticker", description: "", version: 1, variants: [{ id: "classic", name: "Classic" }, FRAME] },
    ]);
    expect(titleOf(applyStyleCommand)).toBe("Apply style…");
    const ctx = { ...currentContext(), project: true, route: "control" as const };
    // Empty until listed, then only the styles a preset ships.
    expect(pick().options(ctx)).toEqual([]);
    await waitFor(() => expect(pick().options(ctx).map((o) => o.title)).toEqual(["Frame", "Classic"]));
    expect(api.listStyles).toHaveBeenCalledTimes(1);

    scenario(false);
    const off = registerCommands([applyStyleCommand]);
    const drop = provideContext({ project: true, route: "control" });
    try {
      await runFollowUp("style.apply", "frame");
      expect(api.applyStyle).toHaveBeenCalledWith("frame", { kind: "project" }, "crossfade");
    } finally {
      off();
      drop();
    }
  });

  it("not offered in the template editor or without a project", () => {
    const ctx = currentContext();
    expect(applyStyleCommand.when?.({ ...ctx, project: true, route: "templateEditor" })).toBe(false);
    expect(applyStyleCommand.when?.({ ...ctx, project: false, route: "control" })).toBe(false);
    expect(applyStyleCommand.when?.({ ...ctx, project: true, route: "control" })).toBe(true);
  });

  it("offered: only styles some preset ships; every style while there are no presets", () => {
    const styles = [
      { id: "minimal", name: "Minimal", description: "" },
      { id: "frame", name: "Frame", description: "" },
    ];
    const presets = [{ id: "p", name: "P", description: "", version: 1, variants: [FRAME] }];
    expect(offered(styles, presets).map((s) => s.id)).toEqual(["frame"]);
    expect(offered(styles, []).map((s) => s.id)).toEqual(["minimal", "frame", "classic"]);
  });

  it("withClassic adds Classic only when missing", () => {
    expect(withClassic([]).map((s) => s.id)).toEqual(["classic"]);
    expect(withClassic([CLASSIC, { id: "frame", name: "Frame", description: "" }]).map((s) => s.id)).toEqual([
      "classic",
      "frame",
    ]);
  });
});
