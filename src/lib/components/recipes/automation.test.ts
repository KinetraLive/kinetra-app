import { describe, expect, it } from "vitest";
import type { FlowInfo, LiveScreenView, NodeKind, RecipeInstance, SourceSummary } from "$lib/types";
import { addedAt, aggregate, automationOf, brokenOf, qualifiers, sourceKindOf, summaryText, type AutomationInput } from "./automation";

const screen = (id: string, name: string, layers: string[]): LiveScreenView => ({
  screen: { id, name, width: 1920, height: 1080, background: "transparent" },
  layers: layers.map((l) => ({ id: l, name: l, onAir: false, pending: false, fields: {}, fieldDefs: [] })),
});

const source = (id: string, name: string, kind: "text" | "schedule" = "text"): SourceSummary => ({
  id,
  name,
  kind,
  rowCount: 1,
  index: 0,
  current: {},
  columns: [],
  usedBy: [],
  updatedAt: "",
  auto: null,
  nextAt: null,
  nowId: null,
  nextId: null,
});

function flow(id: string, name: string, kinds: NodeKind[], patch: Partial<FlowInfo> = {}): FlowInfo {
  return {
    id,
    name,
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: kinds.map((k, i) => ({ ...k, id: `${id}${i}`, position: { x: 0, y: 0 } }) as FlowInfo["nodes"][number]),
    edges: [],
    lastFiredAt: null,
    updatedAt: "",
    warnings: [],
    running: false,
    readOnly: false,
    ...patch,
  };
}

const cue: NodeKind = { type: "trigger.cue", config: { label: null } };
const take = (screenId: string, layerId = "l1"): NodeKind => ({ type: "action.take", config: { screenId, layerId, forMs: null } });
const wrap: NodeKind = { type: "trigger.slideshowWrap", config: { sourceId: "sp" } };

const recipe = (id: string, patch: Partial<RecipeInstance> = {}): RecipeInstance => ({
  id,
  recipeId: "sponsor-slideshow",
  name: "Sponsor slideshow",
  panel: "slideshow",
  sourceId: "sp",
  screenId: "s1",
  layerIds: ["l1"],
  templateIds: [],
  flowIds: [],
  createdAt: "2026-10-04T13:05:00",
  ...patch,
});

const base = (patch: Partial<AutomationInput>): AutomationInput => ({
  screenId: "s1",
  recipes: [],
  flows: [],
  sources: new Map([["sp", source("sp", "Sponsors")], ["sp2", source("sp2", "Partners")]]),
  screens: new Map([["s1", screen("s1", "Main", ["l1"])], ["s2", screen("s2", "Side", ["l1"])]]),
  ready: true,
  ...patch,
});

describe("summaryText", () => {
  it("rules always; armed and warnings only when there are any", () => {
    expect(summaryText(2, 1, 1)).toBe("2 rules · 1 armed · 1 warning");
    expect(summaryText(1, 0, 0)).toBe("1 rule");
    expect(summaryText(0, 0, 3)).toBe("0 rules · 3 warnings");
  });
});

describe("automationOf", () => {
  it("scopes to the screen: installed here, or a rule aims here", () => {
    const here = recipe("a");
    const there = recipe("b", { screenId: "s2" });
    const aimed = recipe("c", { screenId: "s2", flowIds: ["f1"] });
    const v = automationOf(base({ recipes: [here, there, aimed], flows: [flow("f1", "Take main", [wrap, take("s1")])] }));
    expect(v.recipes.map((r) => r.recipe.id)).toEqual(["a", "c"]);
  });

  it("every rule once: under its automation, else Other rules when it aims here; cues never", () => {
    const r = recipe("a", { flowIds: ["f1"] });
    const flows = [
      flow("f1", "Owned by flowIds", [wrap, take("s2")]),
      flow("f2", "Owned by origin", [wrap, take("s1")], { origin: "a", armed: false }),
      flow("f3", "Own rule here", [wrap, take("s1")]),
      flow("f4", "Own rule elsewhere", [wrap, take("s2")]),
      flow("f5", "A cue", [cue, take("s1")]),
      flow("f6", "A cue of the automation", [cue, take("s1")], { origin: "a" }),
    ];
    const v = automationOf(base({ recipes: [r], flows }));
    expect(v.recipes[0].rules.map((f) => f.id)).toEqual(["f1", "f2"]);
    expect(v.recipes[0].armed).toBe(1);
    expect(v.other?.rules.map((f) => f.id)).toEqual(["f3"]);
    const ids = [...v.recipes.flatMap((x) => x.rules), ...(v.other?.rules ?? [])].map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("broken automations (source or layer gone) go to the banner, not the rows; their rules are hidden", () => {
    const ok = recipe("a");
    const noSource = recipe("b", { sourceId: "gone", flowIds: ["f1"] });
    const noLayer = recipe("c", { layerIds: ["l1", "deleted"] });
    const noScreen = recipe("d", { screenId: "s9" });
    const v = automationOf(base({ recipes: [ok, noSource, noLayer, noScreen], flows: [flow("f1", "Pause", [wrap, take("s1")])] }));
    expect(v.recipes.map((r) => r.recipe.id)).toEqual(["a"]);
    expect(v.broken.map((b) => b.recipe.id)).toEqual(["b", "c", "d"]);
    expect(v.broken[0]).toMatchObject({ sourceMissing: true, missingLayers: [] });
    expect(v.broken[1]).toMatchObject({ sourceMissing: false, missingLayers: ["deleted"] });
    // Its screen is gone: every layer is, and it shows on every screen.
    expect(v.broken[2]).toMatchObject({ screenMissing: true, missingLayers: ["l1"] });
    expect(v.other).toBeNull();
    // Another screen: only the one whose screen is gone.
    expect(automationOf(base({ screenId: "s2", recipes: [ok, noSource, noScreen] })).broken.map((b) => b.recipe.id)).toEqual(["d"]);
  });

  it("nothing counts as missing until sources and screens are loaded", () => {
    const v = automationOf(base({ ready: false, recipes: [recipe("b", { sourceId: "gone" })] }));
    expect(v.broken).toEqual([]);
    expect(v.recipes.length).toBe(1);
  });

  it("brokenOf: null when everything is there", () => {
    expect(brokenOf(recipe("a"), base({}).sources, base({}).screens)).toBeNull();
    expect(brokenOf(recipe("a", { sourceId: null }), base({}).sources, base({}).screens)).toBeNull();
  });
});

describe("warnings", () => {
  it("each message once per row, with the rules it affects, in order", () => {
    const m = "The layer it takes was deleted (screen “Main”)";
    const w = aggregate([
      flow("f1", "Pause", [], { warnings: [{ node: "a", message: m }, { node: "b", message: m }] }),
      flow("f2", "Resume", [], { warnings: [{ node: "a", message: "Source no longer exists" }, { node: "b", message: m }] }),
    ]);
    expect(w).toEqual([
      { message: m, rules: ["Pause", "Resume"] },
      { message: "Source no longer exists", rules: ["Resume"] },
    ]);
  });

  it("the row counts distinct messages", () => {
    const r = recipe("a", { flowIds: ["f1", "f2"] });
    const w = [{ node: "x", message: "Source no longer exists" }];
    const v = automationOf(base({ recipes: [r], flows: [flow("f1", "A", [wrap], { warnings: w }), flow("f2", "B", [wrap], { warnings: w })] }));
    expect(v.recipes[0].warnings.length).toBe(1);
  });
});

describe("qualifiers", () => {
  const { sources, screens } = base({});
  it("only for clashing names: the source, else the screen, else when it was added", () => {
    const q1 = qualifiers([recipe("a"), recipe("b", { sourceId: "sp2" }), recipe("c", { name: "Other" })], sources, screens);
    expect(q1.get("a")).toEqual({ text: "Sponsors (source)", mono: false });
    expect(q1.get("b")).toEqual({ text: "Partners (source)", mono: false });
    expect(q1.has("c")).toBe(false);

    const q2 = qualifiers([recipe("a"), recipe("b", { screenId: "s2" })], sources, screens);
    expect(q2.get("a")?.text).toBe("Main");
    expect(q2.get("b")?.text).toBe("Side");

    const q3 = qualifiers([recipe("a", { sourceId: "gone" }), recipe("b", { sourceId: "gone2", createdAt: "2026-10-05T09:30:00" })], sources, screens);
    expect(q3.get("a")).toEqual({ text: `added ${addedAt("2026-10-04T13:05:00")}`, mono: true });
    expect(q3.get("b")?.text).toBe("added 5 Oct 09:30");
  });
});

describe("sourceKindOf", () => {
  it("a slideshow runs on a text source; schedules and countdowns on a schedule", () => {
    expect(sourceKindOf("slideshow")).toBe("text");
    expect(sourceKindOf("schedule")).toBe("schedule");
    expect(sourceKindOf("countdown")).toBe("schedule");
  });
});
