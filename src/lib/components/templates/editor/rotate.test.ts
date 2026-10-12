import { describe, expect, it } from "vitest";
import type { FieldDef, VariableDef } from "$lib/types";
import { markRow, rotateMs, rotatingLists, usesCurrent, ROTATE_DEFAULT_S } from "./rotate";

const list = (key: string): FieldDef =>
  ({ key, label: key, type: "list", columns: [{ key: "name", label: "Name" }], default: "[]" }) as FieldDef;

describe("editor preview slide rotation", () => {
  it("applies to CSS that styles the current row", () => {
    expect(usesCurrent('.ss__slide[data-current="true"]{opacity:1}')).toBe(true);
    expect(usesCurrent(".a[ data-current]{}")).toBe(true);
    expect(usesCurrent(".a[data-currently]{} .b{}")).toBe(false);
    expect(usesCurrent(".a{opacity:1}")).toBe(false);
  });

  it("rotates lists with two rows or more", () => {
    const fields = [list("one"), list("two"), { key: "t", label: "T", type: "text", default: "" } as FieldDef];
    const values = { one: '[{"name":"A"}]', two: '[{"name":"A"},{"name":"B"}]', t: "x" };
    expect(rotatingLists(fields, values)).toEqual(["two"]);
    expect(rotatingLists(fields, { ...values, two: "not json" })).toEqual([]);
  });

  it("moves the _current mark to one row, wrapping around", () => {
    const v = JSON.stringify([{ name: "A", _current: "true" }, { name: "B" }, { name: "C" }]);
    expect(JSON.parse(markRow(v, 1))).toEqual([{ name: "A" }, { name: "B", _current: "true" }, { name: "C" }]);
    expect(JSON.parse(markRow(v, 5))).toEqual([{ name: "A" }, { name: "B" }, { name: "C", _current: "true" }]);
    expect(JSON.parse(markRow(v, 3))).toEqual([{ name: "A", _current: "true" }, { name: "B" }, { name: "C" }]);
    expect(markRow("[]", 2)).toBe("[]");
  });

  it("paces by the interval variable (seconds), else the default", () => {
    const interval = (d: string) => [{ key: "interval", label: "I", type: "range", min: 1, max: 120, step: 1, unit: "s", default: d }] as VariableDef[];
    expect(rotateMs(interval("6"))).toBe(6000);
    expect(rotateMs(interval("0.2"))).toBe(1000);
    expect(rotateMs(interval("999"))).toBe(120_000);
    expect(rotateMs([])).toBe(ROTATE_DEFAULT_S * 1000);
    expect(rotateMs(interval("x"))).toBe(ROTATE_DEFAULT_S * 1000);
  });
});
