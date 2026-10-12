import { describe, expect, it } from "vitest";
import type { FilesChanged, ScreenDoc as AnyScreenDoc, TemplateScreenLayer as ScreenLayer, TemplateDoc } from "$lib/types";
import { diskReaction } from "$lib/components/templates/editor/sync";
import {
  addLayer,
  bindField,
  bindingText,
  blockedReason,
  effectiveVars,
  hasBindings,
  listBindingLabel,
  moveLayer,
  designRect,
  newLayer,
  newLayerId,
  overrideErrors,
  panelToDoc,
  removeLayer,
  resizedToFit,
  setLayerSizing,
  shiftLayer,
  switchTemplate,
  unbindField,
} from "./model";
/** Test docs hold template layers only. */
type ScreenDoc = Omit<AnyScreenDoc, "layers"> & { layers: ScreenLayer[] };

function tpl(id: string, over: Partial<TemplateDoc> = {}): TemplateDoc {
  return {
    id,
    name: `T ${id}`,
    description: "",
    preset: null,
    fields: [
      { key: "name", label: "Name", type: "text", default: "Ada" },
      { key: "title", label: "Title", type: "text", default: "" },
    ],
    variables: [
      { key: "accent", label: "Accent", type: "color", default: "#22b8cf" },
      { key: "radius", label: "Radius", type: "range", min: 0, max: 24, step: 1, unit: "px", default: "4" },
    ],
    autoOutMs: null,
    updatedAt: "",
    defaultVariant: "default",
    variants: [{ id: "default", name: "Default", vars: {}, html: "", css: "" }],
    ...over,
  };
}

const layer = (id: string, over: Partial<ScreenLayer> = {}): ScreenLayer => ({
  id,
  name: id,
  kind: "template",
  templateId: "t1",
  rect: { x: 0, y: 0, w: 100, h: 100 },
  fields: {},
  vars: {},
  ...over,
});

const doc = (layers: ScreenLayer[]): ScreenDoc => ({
  id: "s1",
  name: "Main",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers,
  updatedAt: "",
});

const ids = (d: AnyScreenDoc) => d.layers.map((l) => l.id);

describe("new layers", () => {
  it("ids are 10 chars of [A-Za-z0-9_-]", () => {
    for (let i = 0; i < 50; i++) expect(newLayerId()).toMatch(/^[A-Za-z0-9_-]{10}$/);
    expect(newLayerId((a) => a.fill(63))).toBe("----------");
  });

  it("start full screen with the template's name and field defaults, no overrides", () => {
    const l = newLayer(tpl("t1"), { width: 1280, height: 720 }, "abc");
    expect(l).toEqual({
      id: "abc",
      name: "T t1",
      kind: "template",
      templateId: "t1",
      rect: { x: 0, y: 0, w: 1280, h: 720 },
      fields: { name: "Ada", title: "" },
      vars: {},
    });
  });

  it("from a template with a design box get that box and its sizing", () => {
    const design = { w: 640, h: 180, x: 96, y: 840, sizing: "fit" as const };
    const l = newLayer(tpl("t1", { design }), { width: 1920, height: 1080 }, "abc");
    expect(l.rect).toEqual({ x: 96, y: 840, w: 640, h: 180 });
    expect(l.sizing).toBe("fit");
    // A reflow design leaves the sizing absent (reflow until the user resizes the layer).
    expect("sizing" in newLayer(tpl("t1", { design: { ...design, sizing: "reflow" } }), { width: 1920, height: 1080 })).toBe(false);
    // No design (or null): full screen, no sizing key.
    expect("sizing" in newLayer(tpl("t1", { design: null }), { width: 1280, height: 720 })).toBe(false);
  });

  it("are added in front (the end), up to the limit", () => {
    const d = addLayer(doc([layer("a")]), layer("b"));
    expect(ids(d)).toEqual(["a", "b"]);
    const full = doc(Array.from({ length: 64 }, (_, i) => layer(`l${i}`)));
    expect(addLayer(full, layer("x"))).toBe(full);
  });
});

describe("z-order", () => {
  const d = doc([layer("a"), layer("b"), layer("c")]);

  it("shift +1 moves towards the front, -1 towards the back; ends are no-ops", () => {
    expect(ids(shiftLayer(d, "a", 1))).toEqual(["b", "a", "c"]);
    expect(ids(shiftLayer(d, "c", -1))).toEqual(["a", "c", "b"]);
    expect(shiftLayer(d, "c", 1)).toBe(d);
    expect(shiftLayer(d, "a", -1)).toBe(d);
  });

  it("drag reorder by panel rows (top = front)", () => {
    // Panel: c, b, a. Drag "a" (row 2) to the top (row 0).
    const from = d.layers.findIndex((l) => l.id === "a");
    expect(ids(moveLayer(d, from, panelToDoc(3, 0)))).toEqual(["b", "c", "a"]);
    expect(panelToDoc(3, 2)).toBe(0);
  });

  it("removes a layer", () => {
    expect(ids(removeLayer(d, "b"))).toEqual(["a", "c"]);
  });
});

describe("template switch", () => {
  it("keeps matching field values and overrides, defaults the rest, follows the name", () => {
    const old = tpl("t1");
    const next = tpl("t2", {
      fields: [
        { key: "name", label: "Name", type: "text", default: "x" },
        { key: "role", label: "Role", type: "text", default: "Host" },
      ],
      variables: [{ key: "accent", label: "Accent", type: "color", default: "#000000" }],
    });
    const l = layer("a", {
      name: "T t1",
      fields: { name: "Grace", title: "Admiral" },
      vars: { accent: "#ff0000", radius: "8" },
    });
    expect(switchTemplate(l, old, next)).toMatchObject({
      templateId: "t2",
      name: "T t2",
      fields: { name: "Grace", role: "Host" },
      vars: { accent: "#ff0000" },
    });
    // A custom name stays.
    expect(switchTemplate({ ...l, name: "Guest" }, old, next).name).toBe("Guest");
  });
});

describe("variables", () => {
  const t = tpl("t1");
  it("effective values: overrides over the template defaults", () => {
    expect(effectiveVars(t.variables, { radius: "10" })).toEqual({ accent: "#22b8cf", radius: "10" });
  });

  it("errors only for invalid overrides", () => {
    expect(overrideErrors(t.variables, {})).toEqual([null, null]);
    expect(overrideErrors(t.variables, { accent: "#zz", radius: "99" })).toEqual([
      "Enter a color like #1a1a1a.",
      "Between 0 and 24.",
    ]);
  });

  it("blocks saving on an invalid override, background or empty name", () => {
    const templates = new Map([["t1", t]]);
    expect(blockedReason(doc([layer("a")]), templates)).toBeNull();
    expect(blockedReason(doc([layer("a", { vars: { accent: "red" } })]), templates)).toBe(
      "a: Accent: Enter a color like #1a1a1a.",
    );
    expect(blockedReason({ ...doc([]), background: "#12" }, templates)).toMatch(/^Background/);
    expect(blockedReason({ ...doc([]), background: "#112233" }, templates)).toBeNull();
    expect(blockedReason(doc([layer("a", { name: " " })]), templates)).toMatch(/name/);
  });
});

describe("disk reaction", () => {
  const ev = (over: Partial<FilesChanged> = {}): FilesChanged => ({
    kind: "screen",
    id: "s1",
    change: "changed",
    message: null,
    ...over,
  });

  it("ignores other files, reloads when clean, asks when dirty", () => {
    expect(diskReaction(ev({ id: "s2" }), "screen", "s1", false)).toBe("ignore");
    expect(diskReaction(ev({ kind: "template" }), "screen", "s1", false)).toBe("ignore");
    expect(diskReaction(ev(), "screen", "s1", false)).toBe("reload");
    expect(diskReaction(ev(), "screen", "s1", true)).toBe("conflict");
    expect(diskReaction(ev({ change: "removed" }), "screen", "s1", true)).toBe("removed");
    expect(diskReaction(ev({ change: "invalid", message: "bad" }), "screen", "s1", false)).toBe("invalid");
  });
});

describe("source bindings", () => {
  const g = { sourceId: "g1", column: "name" };

  it("bind adds or replaces a field's binding; other layers are untouched", () => {
    const d = doc([layer("a"), layer("b")]);
    expect(hasBindings(d)).toBe(false);
    const bound = bindField(d, "a", "name", g);
    expect(bound.layers[0].bindings).toEqual({ name: g });
    expect(bound.layers[1]).toBe(d.layers[1]);
    expect(hasBindings(bound)).toBe(true);
    const again = bindField(bound, "a", "name", { sourceId: "g2", column: "title" });
    expect(again.layers[0].bindings).toEqual({ name: { sourceId: "g2", column: "title" } });
    const two = bindField(again, "a", "title", g);
    expect(Object.keys(two.layers[0].bindings!)).toEqual(["name", "title"]);
  });

  it("unbind removes the key, and `bindings` when it was the last", () => {
    const d = bindField(bindField(doc([layer("a")]), "a", "name", g), "a", "title", g);
    const one = unbindField(d, "a", "name");
    expect(one.layers[0].bindings).toEqual({ title: g });
    const none = unbindField(one, "a", "title");
    expect("bindings" in none.layers[0]).toBe(false);
    expect(unbindField(none, "a", "title")).toBe(none);
    // The layer's own value stays as the fallback.
    expect(none.layers[0].fields).toEqual(d.layers[0].fields);
  });

  it("a template switch drops bindings for keys the new template doesn't have", () => {
    const next = tpl("t2", { fields: [{ key: "name", label: "Name", type: "text", default: "" }] });
    const l = layer("a", { bindings: { name: g, title: { sourceId: "g1", column: "title" } } });
    expect(switchTemplate(l, tpl("t1"), next).bindings).toEqual({ name: g });
    const none = tpl("t3", { fields: [{ key: "headline", label: "Headline", type: "text", default: "" }] });
    expect("bindings" in switchTemplate(l, tpl("t1"), none)).toBe(false);
  });
});

// ---- List bindings ------------------------------------------------------------------------------


describe("list bindings", () => {
  const doc = (): ScreenDoc => ({
    id: "s",
    name: "S",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      {
        id: "l",
        name: "L",
        kind: "template",
        templateId: "t",
        rect: { x: 0, y: 0, w: 10, h: 10 },
        fields: {},
        vars: {},
      },
    ],
    updatedAt: "",
  });
  const guests = {
    name: "Guests",
    columns: [
      { key: "name", label: "Name" },
      { key: "score", label: "Score" },
    ],
  };

  it("keeps sort and a clamped limit for a whole-source binding only", () => {
    const sort = { column: "score", numeric: true, desc: true };
    const list = bindField(doc(), "l", "board", { sourceId: "g", column: "", sort, limit: 500 });
    expect(list.layers[0].bindings).toEqual({ board: { sourceId: "g", column: "", sort, limit: 100 } });
    const zero = bindField(doc(), "l", "board", { sourceId: "g", column: "", limit: 0 });
    expect(zero.layers[0].bindings?.board.limit).toBe(1);
    const col = bindField(doc(), "l", "name", { sourceId: "g", column: "name", sort, limit: 3 });
    expect(col.layers[0].bindings).toEqual({ name: { sourceId: "g", column: "name" } });
  });

  it("labels a list binding", () => {
    expect(listBindingLabel(guests, { sourceId: "g", column: "" })).toBe("Guests · all rows");
    expect(
      listBindingLabel(guests, { sourceId: "g", column: "", sort: { column: "score", numeric: true, desc: true }, limit: 10 }),
    ).toBe("Guests · all rows, sorted by Score ↓, top 10");
    expect(listBindingLabel(guests, { sourceId: "g", column: "", sort: { column: "gone", numeric: false, desc: false } })).toBe(
      "Guests · all rows, sorted by gone (missing) ↑",
    );
    expect(listBindingLabel(null, { sourceId: "g", column: "" })).toBe("Missing source · all rows");
    expect(bindingText(guests, { sourceId: "g", column: "name" })).toBe("Guests · Name");
  });
});

describe("design placement", () => {
  const d = (over: Partial<{ w: number; h: number; x: number | null; y: number | null }> = {}) => ({
    w: 640,
    h: 180,
    sizing: "fit" as const,
    ...over,
  });

  it("keeps the design size and the position on a 1920×1080 screen", () => {
    expect(designRect(d({ x: 96, y: 840 }), { width: 1920, height: 1080 })).toEqual({ x: 96, y: 840, w: 640, h: 180 });
  });

  it("scales the position only, by W / 1920 and H / 1080", () => {
    // 3840×2160: position doubles, size stays.
    expect(designRect(d({ x: 96, y: 840 }), { width: 3840, height: 2160 })).toEqual({ x: 192, y: 1680, w: 640, h: 180 });
    // 1280×720: 96 × 2/3 = 64, 840 × 2/3 = 560, clamped to 720 - 180 = 540.
    expect(designRect(d({ x: 96, y: 840 }), { width: 1280, height: 720 })).toEqual({ x: 64, y: 540, w: 640, h: 180 });
  });

  it("centres when x / y are absent", () => {
    expect(designRect(d(), { width: 1920, height: 1080 })).toEqual({ x: 640, y: 450, w: 640, h: 180 });
    // Odd remainder: floor.
    expect(designRect(d({ w: 641, h: 181 }), { width: 1920, height: 1080 })).toEqual({ x: 639, y: 449, w: 641, h: 181 });
    // One axis given, the other centred.
    expect(designRect(d({ x: 0 }), { width: 1920, height: 1080 })).toEqual({ x: 0, y: 450, w: 640, h: 180 });
  });

  it("shrinks a box larger than the screen uniformly to fit", () => {
    // 1920×1080 design on a 1280×720 screen: k = 2/3.
    expect(designRect(d({ w: 1920, h: 1080, x: 0, y: 0 }), { width: 1280, height: 720 })).toEqual({
      x: 0,
      y: 0,
      w: 1280,
      h: 720,
    });
    // Too tall for a portrait screen... and too wide: limited by the tighter side.
    expect(designRect(d({ w: 2000, h: 500 }), { width: 1080, height: 1920 })).toEqual({ x: 0, y: 825, w: 1080, h: 270 });
  });

  it("clamps the box inside the screen", () => {
    expect(designRect(d({ x: 1800, y: 1000 }), { width: 1920, height: 1080 })).toEqual({ x: 1280, y: 900, w: 640, h: 180 });
    expect(designRect(d({ x: -50, y: -10 }), { width: 1920, height: 1080 })).toEqual({ x: 0, y: 0, w: 640, h: 180 });
  });
});

describe("content sizing", () => {
  it("sets the sizing of the given layers only", () => {
    const base = doc([layer("a"), layer("b"), layer("c")]);
    const next = setLayerSizing(base, ["a", "c"], "fit");
    expect(next.layers.map((l) => l.sizing)).toEqual(["fit", undefined, "fit"]);
    expect(setLayerSizing(next, ["a"], "reflow").layers[0].sizing).toBe("reflow");
  });
});

describe("resizing scales", () => {
  const doc = (layers: Partial<ScreenLayer>[]): ScreenDoc => ({
    id: "s",
    name: "S",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: layers.map((l, i) => ({ id: `l${i}`, name: "L", kind: "template", templateId: "t", rect: { x: 0, y: 0, w: 10, h: 10 }, fields: {}, vars: {}, ...l }) as ScreenLayer),
    updatedAt: "",
  });

  it("a resized layer becomes Scale to fit unless the user chose Reflow", () => {
    const d = doc([{}, { sizing: "reflow" }, { sizing: "fit" }, {}]);
    const next = resizedToFit(d, ["l0", "l1", "l2"]);
    expect(next.layers.map((l) => l.sizing)).toEqual(["fit", "reflow", "fit", undefined]);
    // Nothing to switch: the same document (no edit).
    expect(resizedToFit(d, ["l1", "l2"])).toBe(d);
    expect(resizedToFit(next, ["l0"])).toBe(next);
  });
});
