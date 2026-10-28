import { describe, expect, it } from "vitest";
import type { PanelButton, PanelTargets } from "$lib/types";
import { available, layersFor, move, newAction, newButton, problem, retarget, settableFields } from "./model";

const targets: PanelTargets = {
  screens: [
    {
      id: "s1",
      name: "Main",
      layers: [
        {
          id: "l1",
          name: "Lower third",
          media: false,
          fields: [
            { key: "name", label: "Name", type: "text", default: "" },
            { key: "title", label: "Title", type: "text", default: "" },
            { key: "photo", label: "Photo", type: "image", default: "" },
          ],
          bound: ["title"],
          variants: [
            { id: "frame", label: "Frame" },
            { id: "minimal", label: "Minimal" },
          ],
        },
        {
          id: "l2",
          name: "Countdown",
          media: false,
          fields: [{ key: "clock", label: "Clock", type: "timer", default: "" }],
          bound: [],
          variants: [{ id: "default", label: "Default" }],
        },
        { id: "m1", name: "Intro video", media: true, fields: [], bound: [], variants: [] },
      ],
    },
  ],
  cues: [{ id: "c1", label: "Opening" }],
  sources: [{ id: "src", name: "Speakers", rows: [{ id: "r1", label: "Ingrid Vale" }] }],
};

describe("button panel model", () => {
  it("offers each kind only the targets it can use", () => {
    expect(layersFor(targets, "take").map((r) => r.layer.id)).toEqual(["l1", "l2", "m1"]);
    expect(layersFor(targets, "setFields").map((r) => r.layer.id)).toEqual(["l1"]);
    expect(settableFields(targets.screens[0]!.layers[0]!).map((f) => f.key)).toEqual(["name"]);
    expect(layersFor(targets, "setVariant").map((r) => r.layer.id)).toEqual(["l1"]);
    expect(layersFor(targets, "timer").map((r) => r.layer.id)).toEqual(["l2"]);
    const empty: PanelTargets = { screens: [], cues: [], sources: [] };
    expect(available(empty, "take")).toBe(false);
    expect(newButton(empty, [])).toBeNull();
  });

  it("new steps point at the first fitting target", () => {
    expect(newAction(targets, "timer")).toEqual({ kind: "timer", screen: "s1", layer: "l2", field: "clock", op: "start" });
    expect(newAction(targets, "selectRow")).toEqual({ kind: "selectRow", source: "src", row: "r1" });
    expect(newAction(targets, "setVariant")).toMatchObject({ layer: "l1", variant: "frame" });
  });

  it("a new button is a complete, gray take with the next free name", () => {
    const b = newButton(targets, [])!;
    expect(b).toMatchObject({ label: "Button 1", color: "grey", confirm: false });
    expect(b.actions).toEqual([{ kind: "take", screen: "s1", layer: "l1" }]);
    expect(b.id).toMatch(/^[a-z0-9]{10}$/);
    expect(problem(b)).toBeNull();
    expect(newButton(targets, [b])!.label).toBe("Button 2");
    const full = Array.from({ length: 48 }, () => b);
    expect(newButton(targets, full)).toBeNull();
  });

  it("names what keeps a button from saving", () => {
    const b: PanelButton = { id: "b", label: "Go", color: "red", confirm: false, actions: [{ kind: "runCue", flow: "c1" }] };
    expect(problem(b)).toBeNull();
    expect(problem({ ...b, label: "  " })).toMatch(/label/);
    expect(problem({ ...b, label: "x".repeat(41) })).toMatch(/40/);
    expect(problem({ ...b, actions: [] })).toMatch(/action/);
    expect(problem({ ...b, actions: [{ kind: "setFields", screen: "s1", layer: "l1", values: {} }] })).toMatch(/Action 1/);
  });

  it("retargeting keeps what still fits", () => {
    const a = { kind: "setFields" as const, screen: "s1", layer: "l1", values: { name: "Kari", gone: "x" } };
    expect(retarget(targets, a, "s1", "l1")).toEqual({ ...a, values: { name: "Kari" } });
    const v = { kind: "setVariant" as const, screen: "s1", layer: "l2", variant: "minimal" };
    expect(retarget(targets, v, "s1", "l1")).toMatchObject({ variant: "minimal" });
    expect(move([1, 2, 3], 0, 2)).toEqual([2, 3, 1]);
    expect(move([1, 2, 3], 0, -1)).toEqual([1, 2, 3]);
  });
});
