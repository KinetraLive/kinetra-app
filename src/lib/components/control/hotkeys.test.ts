import { describe, expect, it } from "vitest";
import {
  assignHotkey,
  captureAction,
  comboOf,
  formatCombo,
  hotkeyKey,
  isCombo,
  dropFlowHotkey,
  flowHotkey,
  flowHotkeyKey,
  flowOfKey,
  ownerKey,
  ownerOf,
  parseCombo,
  removeHotkey,
  splitHotkeyKey,
  toggleDecision,
  type KeyLike,
} from "./hotkeys";

const key = (k: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  key: k,
  shiftKey: false,
  ctrlKey: false,
  altKey: false,
  metaKey: false,
  repeat: false,
  ...mods,
});

describe("combos", () => {
  it("parses and formats F1–F12 with optional Shift", () => {
    expect(parseCombo("F1")).toEqual({ shift: false, n: 1 });
    expect(parseCombo("Shift+F12")).toEqual({ shift: true, n: 12 });
    for (const bad of ["F0", "F13", "f1", "Ctrl+F1", "Shift+", "", "Shift+F1x", "F01"]) {
      expect(parseCombo(bad)).toBeNull();
      expect(isCombo(bad)).toBe(false);
    }
    expect(formatCombo({ shift: true, n: 3 })).toBe("Shift+F3");
    expect(formatCombo({ shift: false, n: 10 })).toBe("F10");
  });

  it("reads a keydown as a combo; Ctrl/Alt/Meta and other keys are none", () => {
    expect(comboOf(key("F5"))).toBe("F5");
    expect(comboOf(key("F5", { shiftKey: true }))).toBe("Shift+F5");
    expect(comboOf(key("F5", { ctrlKey: true }))).toBeNull();
    expect(comboOf(key("F5", { altKey: true }))).toBeNull();
    expect(comboOf(key("F5", { metaKey: true }))).toBeNull();
    expect(comboOf(key("a"))).toBeNull();
    expect(comboOf(key("F13"))).toBeNull();
  });

  it("keys are screen/layer and split back", () => {
    expect(hotkeyKey("s1", "l1")).toBe("s1/l1");
    expect(splitHotkeyKey("s1/l1")).toEqual({ screenId: "s1", layerId: "l1" });
    expect(splitHotkeyKey("nolayer")).toBeNull();
    expect(splitHotkeyKey("/l1")).toBeNull();
    expect(splitHotkeyKey("s1/")).toBeNull();
  });
});

describe("assigning", () => {
  it("a combo has one owner: assigning it moves it", () => {
    const map = { "s1/a": "F1", "s1/b": "F2" };
    expect(assignHotkey(map, "s2/c", "F1")).toEqual({ "s1/b": "F2", "s2/c": "F1" });
    // Re-assigning a layer replaces its own combo.
    expect(assignHotkey(map, "s1/a", "Shift+F1")).toEqual({ "s1/a": "Shift+F1", "s1/b": "F2" });
    // Not mutated.
    expect(map).toEqual({ "s1/a": "F1", "s1/b": "F2" });
    expect(removeHotkey(map, "s1/a")).toEqual({ "s1/b": "F2" });
    expect(ownerOf(map, "F2")).toEqual({ screenId: "s1", layerId: "b" });
    expect(ownerOf(map, "F3")).toBeNull();
  });
});

// Which command a key runs (layer, cue, PANIC; typing, repeats) is the registry's job now:
// src/lib/commands/commands.test.ts.
describe("decisions", () => {
  it("toggle takes an off-air layer and clears an on-air one", () => {
    expect(toggleDecision({ onAir: false })).toBe("take");
    expect(toggleDecision({ onAir: true })).toBe("clear");
  });

  it("capture: an F-key sets, Esc cancels, the rest is ignored", () => {
    expect(captureAction(key("F4", { shiftKey: true }))).toEqual({ kind: "set", combo: "Shift+F4" });
    expect(captureAction(key("Escape"))).toEqual({ kind: "cancel" });
    expect(captureAction(key("a"))).toBeNull();
    expect(captureAction(key("F4", { repeat: true }))).toBeNull();
    expect(captureAction(key("Escape", { shiftKey: true }))).toBeNull();
  });
});

describe("cue (flow) keys", () => {
  it("flow keys are flow/<id>; a leftover macro/<id> is the same flow; never a layer", () => {
    expect(flowHotkeyKey("f1")).toBe("flow/f1");
    expect(flowOfKey("flow/f1")).toBe("f1");
    expect(flowOfKey("macro/m1")).toBe("m1");
    expect(flowOfKey("flow/")).toBeNull();
    expect(flowOfKey("macro/")).toBeNull();
    expect(flowOfKey("s1/l1")).toBeNull();
    expect(splitHotkeyKey("flow/f1")).toBeNull();
    expect(splitHotkeyKey("macro/m1")).toBeNull();
  });

  it("one owner per combo, flows and layers alike", () => {
    const map = { "s1/a": "F1", "flow/f1": "F2", "macro/m1": "Shift+F2" };
    expect(ownerKey(map, "F2")).toBe("flow/f1");
    expect(ownerOf(map, "F2")).toBeNull();
    // Assigning a layer's combo to a flow steals it.
    expect(assignHotkey(map, "flow/f2", "F1")).toEqual({ "flow/f1": "F2", "macro/m1": "Shift+F2", "flow/f2": "F1" });
  });

  it("a flow's combo reads either spelling; setting or removing it replaces the legacy key", () => {
    const map = { "s1/a": "F1", "macro/m1": "F3" };
    expect(flowHotkey(map, "m1")).toBe("F3");
    expect(flowHotkey({ "flow/m1": "F4", "macro/m1": "F3" }, "m1")).toBe("F4");
    expect(flowHotkey(map, "m2")).toBeNull();
    expect(assignHotkey(map, "flow/m1", "F5")).toEqual({ "s1/a": "F1", "flow/m1": "F5" });
    expect(removeHotkey(map, "flow/m1")).toEqual({ "s1/a": "F1" });
    expect(dropFlowHotkey({ "flow/m1": "F4", "macro/m1": "F3", "s1/a": "F1" }, "m1")).toEqual({ "s1/a": "F1" });
  });
});
