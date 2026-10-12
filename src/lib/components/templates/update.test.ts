// "Update to the latest look": the flow (confirm, backup + update, toast with Undo), the notice,
// the templates list card / side pane, and the layout editor's "Fit box to graphic" and hint.
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ToastHost from "$lib/components/ToastHost.svelte";
import "$lib/components/templates/list/matchers";
import { toasts } from "$lib/stores/toasts.svelte";
import { TEMPLATE_UPDATED } from "$lib/templates-common";
import type { ScreenLayer, TemplateDoc, TemplateSummary, TemplateUpdated } from "$lib/types";

const api = vi.hoisted(() => ({
  updateTemplateFromPreset: vi.fn(),
  undoTemplateUpdate: vi.fn(),
  listSources: vi.fn(async () => []),
}));
vi.mock("$lib/api", () => ({
  ...api,
  on: () => () => {},
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/template-update", () => api);
vi.mock("$lib/api/sources", () => api);
// LivePreview (the side pane) watches its visibility and size; toasts animate out.
class NoObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("IntersectionObserver", NoObserver);
vi.stubGlobal("ResizeObserver", NoObserver);
if (!("getAnimations" in Element.prototype)) {
  Object.defineProperty(Element.prototype, "getAnimations", { value: () => [], configurable: true });
}

import { updateLook, UNDO_MS } from "./update";
import UpdateNotice from "./UpdateNotice.svelte";
import TemplateCard from "./list/TemplateCard.svelte";
import PreviewPane from "./list/PreviewPane.svelte";
import Inspector from "../screens/editor/Inspector.svelte";
import MultiInspector from "../screens/editor/MultiInspector.svelte";
import { fitBox } from "../screens/editor/fitbox";

const DESIGN = { w: 1400, h: 260, x: 101, y: 737, sizing: "fit" as const };

function doc(patch: Partial<TemplateDoc> = {}): TemplateDoc {
  return {
    id: "t1",
    name: "Now & Next",
    description: "",
    preset: { id: "now-next", version: 2 },
    fields: [],
    variables: [],
    autoOutMs: null,
    updatedAt: "2026-10-07T10:00:00Z",
    defaultVariant: "classic",
    variants: [{ id: "classic", name: "Classic", vars: {}, html: "<div></div>", css: "" }],
    ...patch,
  };
}

function summary(patch: Partial<TemplateSummary> = {}): TemplateSummary {
  return {
    id: "t1",
    name: "Now & Next",
    description: "",
    preset: { id: "now-next", version: 1 },
    fieldCount: 0,
    variableCount: 0,
    variantCount: 1,
    updatedAt: "2026-10-07T10:00:00Z",
    presetUpdate: { from: 1, to: 2 },
    ...patch,
  };
}

const UPDATED: TemplateUpdated = {
  doc: doc({ design: DESIGN }),
  backup: { id: "20261007T100000Z-pre-update", createdAt: "", reason: "pre-update", files: 3, bytes: 100 },
};

beforeEach(() => {
  api.updateTemplateFromPreset.mockResolvedValue(UPDATED);
  api.undoTemplateUpdate.mockResolvedValue(doc());
});

afterEach(() => {
  cleanup();
  toasts.clear();
  document.body.replaceChildren();
  vi.clearAllMocks();
});

describe("update flow", () => {
  it("asks first, saying what is replaced, what is kept and that a backup is taken; Cancel does nothing", async () => {
    const pending = updateLook({ id: "t1", name: "Now & Next" });
    const dlg = await screen.findByRole("dialog");
    expect(dlg).toHaveTextContent("Update “Now & Next” to the latest look?");
    expect(dlg).toHaveTextContent("Your edits to the built-in looks' HTML and CSS are replaced.");
    expect(dlg).toHaveTextContent("Variants you added and field values are kept.");
    expect(dlg).toHaveTextContent("A backup is taken first.");
    await fireEvent.click(within(dlg).getByRole("button", { name: "Cancel" }));
    expect(await pending).toBeNull();
    expect(api.updateTemplateFromPreset).not.toHaveBeenCalled();
  });

  it("Update saves first, updates, announces it and toasts with Undo; Undo restores from the backup", async () => {
    render(ToastHost);
    const order: string[] = [];
    const before = vi.fn(async () => void order.push("before"));
    api.updateTemplateFromPreset.mockImplementation(async () => {
      order.push("update");
      return UPDATED;
    });
    const after = vi.fn();
    const heard = vi.fn();
    addEventListener(TEMPLATE_UPDATED, heard);
    const pending = updateLook({ id: "t1", name: "Now & Next" }, { before, after });
    await fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Update" }));
    expect(await pending).toEqual(UPDATED.doc);
    expect(order).toEqual(["before", "update"]);
    expect(api.updateTemplateFromPreset).toHaveBeenCalledWith("t1");
    expect(after).toHaveBeenCalledWith(UPDATED.doc);
    expect(heard).toHaveBeenCalledTimes(1);
    expect(toasts.list).toHaveLength(1);
    expect(toasts.list[0].message).toBe("“Now & Next” has the latest look");
    expect(toasts.list[0].action?.label).toBe("Undo");
    expect(UNDO_MS).toBe(10_000);

    await fireEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(api.undoTemplateUpdate).toHaveBeenCalledWith("t1", "20261007T100000Z-pre-update"));
    await waitFor(() => expect(after).toHaveBeenCalledTimes(2));
    expect(before).toHaveBeenCalledTimes(2);
    expect(heard).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(toasts.list.map((t) => t.message)).toEqual(["“Now & Next” has its previous look again"]));
    removeEventListener(TEMPLATE_UPDATED, heard);
  });

  it("a failure toasts the error and changes nothing", async () => {
    api.updateTemplateFromPreset.mockRejectedValue({ code: "invalid", message: "“Now & Next” already has the latest look" });
    const after = vi.fn();
    const pending = updateLook({ id: "t1", name: "Now & Next" }, { after });
    await fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Update" }));
    expect(await pending).toBeNull();
    expect(after).not.toHaveBeenCalled();
    expect(toasts.list[0]).toMatchObject({
      kind: "error",
      message: "Couldn't update “Now & Next”: “Now & Next” already has the latest look",
    });
  });
});

describe("notice", () => {
  it("one quiet line; Update opens the confirmation", async () => {
    // `target` is also a render option: pass the props explicitly.
    render(UpdateNotice, { props: { target: { id: "t1", name: "Now & Next" } } });
    expect(screen.getByText("A newer look is available")).toBeTruthy();
    const btn = screen.getByRole("button", { name: "Update “Now & Next” to the latest look" });
    expect(btn).toHaveTextContent("Update");
    await fireEvent.click(btn);
    expect(await screen.findByRole("dialog")).toHaveTextContent("Update “Now & Next” to the latest look?");
  });

  it("the templates list: the card says so, the side pane has the action", async () => {
    const props = { origin: "From Now & Next", selected: false, now: Date.now(), onselect: () => {}, onopen: () => {} };
    const { unmount } = render(TemplateCard, { ...props, template: summary() });
    expect(screen.getByText("Newer look available")).toBeTruthy();
    unmount();
    render(TemplateCard, { ...props, template: summary({ presetUpdate: null }) });
    expect(screen.queryByText("Newer look available")).toBeNull();
    cleanup();

    const pane = { origin: "From Now & Next", fields: [], src: null, onrename: async () => true, onduplicate: () => {}, ondelete: () => {} };
    render(PreviewPane, { ...pane, template: summary() });
    expect(await screen.findByText("A newer look is available")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Update “Now & Next” to the latest look" })).toBeTruthy();
    cleanup();
    render(PreviewPane, { ...pane, template: summary({ presetUpdate: null }) });
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText("A newer look is available")).toBeNull();
  });
});

// ---- Layout editor ------------------------------------------------------------------------------

const SCREEN = { width: 1920, height: 1080 };

function layer(patch: Partial<ScreenLayer> = {}): ScreenLayer {
  return {
    id: "l1",
    name: "Now & Next",
    kind: "template",
    templateId: "t1",
    rect: { x: 312, y: 385, w: 472, h: 121 },
    fields: {},
    vars: {},
    ...patch,
  } as ScreenLayer;
}

describe("fit box to graphic", () => {
  it("keeps the position (moved inside the screen); a full-screen layer gets the default position", () => {
    expect(fitBox({ x: 312, y: 385, w: 472, h: 121 }, DESIGN, SCREEN)).toEqual({ x: 312, y: 385, w: 1400, h: 260 });
    expect(fitBox({ x: 900, y: 1000, w: 100, h: 50 }, DESIGN, SCREEN)).toEqual({ x: 520, y: 820, w: 1400, h: 260 });
    expect(fitBox({ x: 0, y: 0, w: 1920, h: 1080 }, DESIGN, SCREEN)).toEqual({ x: 101, y: 737, w: 1400, h: 260 });
    // Larger than the screen: shrunk uniformly, like a new layer.
    expect(fitBox({ x: 0, y: 0, w: 640, h: 360 }, { w: 3840, h: 2160, sizing: "fit" }, SCREEN)).toEqual({ x: 0, y: 0, w: 1920, h: 1080 });
  });

  const inspector = (l: ScreenLayer, t: TemplateDoc, s: TemplateSummary, onfit = vi.fn(), onrect = vi.fn(), aspectLocked = false) =>
    render(Inspector, {
      aspectLocked,
      layer: l,
      template: t,
      missing: false,
      templates: [s],
      screen: SCREEN,
      onname: () => {},
      ontemplate: () => {},
      onrect,
      onfit,
      onfield: () => {},
      onvar: () => {},
      onresetvar: () => {},
    });

  it("Inspector: the button sets the design box (with Scale to fit); a fit layer shows its scale instead", async () => {
    const onfit = vi.fn();
    inspector(layer(), doc({ design: DESIGN }), summary({ presetUpdate: null }), onfit);
    await fireEvent.click(screen.getByRole("button", { name: "Fit box to graphic" }));
    expect(onfit).toHaveBeenCalledWith({ x: 312, y: 385, w: 1400, h: 260 });
    expect(screen.queryByText("Scale")).toBeNull();
    cleanup();
    inspector(layer({ rect: { x: 312, y: 385, w: 1400, h: 260 }, sizing: "fit" }), doc({ design: DESIGN }), summary({ presetUpdate: null }));
    expect(screen.queryByRole("button", { name: "Fit box to graphic" })).toBeNull();
    expect(document.querySelector("[data-scale]")!.textContent).toBe("100 %");
    expect(screen.getByRole("button", { name: "Scale to 100 %" })).toBeDisabled();
    cleanup();
    // No design: no button.
    inspector(layer(), doc(), summary({ preset: null, presetUpdate: null }));
    expect(screen.queryByRole("button", { name: "Fit box to graphic" })).toBeNull();
  });

  it("Inspector: a Scale to fit layer's scale (box / design, the smaller axis); 100 % keeps the centre", async () => {
    const onrect = vi.fn();
    // 896×166 of a 1400×260 design: 64 %.
    inspector(layer({ rect: { x: 100, y: 800, w: 896, h: 166 }, sizing: "fit" }), doc({ design: DESIGN }), summary({ presetUpdate: null }), vi.fn(), onrect);
    expect(document.querySelector("[data-scale]")!.textContent).toBe("64 %");
    expect(document.querySelector("[data-scale]")!.classList.contains("mono")).toBe(true);
    await fireEvent.click(screen.getByRole("button", { name: "Scale to 100 %" }));
    // Centre (548, 883) kept, moved inside the screen (y 753..1013).
    expect(onrect).toHaveBeenCalledWith({ x: 0, y: 753, w: 1400, h: 260 }, true);
    cleanup();
    // Typed W keeps the design's aspect (1400:260) while locked; it is a resize.
    inspector(layer({ rect: { x: 100, y: 800, w: 896, h: 166 }, sizing: "fit" }), doc({ design: DESIGN }), summary({ presetUpdate: null }), vi.fn(), onrect, true);
    const w = screen.getByLabelText("Width") as HTMLInputElement;
    w.value = "700";
    await fireEvent.change(w);
    expect(onrect).toHaveBeenLastCalledWith({ x: 100, y: 800, w: 700, h: 130 }, true);
    cleanup();
    // No design: the screen is the design (as the renderer does).
    inspector(layer({ rect: { x: 0, y: 0, w: 960, h: 540 }, sizing: "fit" }), doc(), summary({ preset: null, presetUpdate: null }));
    expect(document.querySelector("[data-scale]")!.textContent).toBe("50 %");
  });

  it("Inspector: an outdated template says it doesn't follow its box, with Update; a custom one doesn't", async () => {
    inspector(layer(), doc({ design: undefined }), summary());
    expect(await screen.findByText("This graphic doesn't follow its box.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Update to the latest look" })).toBeTruthy();
    cleanup();
    // Outdated but with a design box: the plain notice.
    inspector(layer(), doc({ design: DESIGN }), summary());
    expect(await screen.findByText("A newer look is available")).toBeTruthy();
    cleanup();
    // The user's own template without a design reflows fine: no hint.
    inspector(layer(), doc({ preset: null }), summary({ preset: null, presetUpdate: null }));
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText("This graphic doesn't follow its box.")).toBeNull();
    expect(screen.queryByRole("button", { name: /latest look/ })).toBeNull();
  });

  it("MultiInspector: fits each selected layer whose template has a design", async () => {
    const onfit = vi.fn();
    const layers = [layer(), layer({ id: "l2", templateId: "own", rect: { x: 0, y: 0, w: 1920, h: 1080 } })];
    render(MultiInspector, {
      layers,
      templates: new Map([
        ["t1", doc({ design: DESIGN })],
        ["own", doc({ id: "own", preset: null })],
      ]),
      screen: SCREEN,
      onfit,
      onmove: () => {},
      onsizing: () => {},
      onalign: () => {},
      ondistribute: () => {},
      onanchor: () => {},
    });
    await fireEvent.click(screen.getByRole("button", { name: "Fit box to graphic" }));
    expect(onfit).toHaveBeenCalledWith([{ id: "l1", rect: { x: 312, y: 385, w: 1400, h: 260 } }]);
  });
});
