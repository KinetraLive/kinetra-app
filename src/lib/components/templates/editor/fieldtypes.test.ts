import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { SOURCE_COLUMNS_MAX, type FieldDef } from "$lib/types";
import FieldsForm from "./FieldsForm.svelte";
import { listDefault, newColumn, relabelColumn } from "./columns";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { clockFormatIssue, NUMBER_WORDS, previewDefault, sampleNumber, typeIssues, withFieldType } from "./fieldTypes";
import { issueWith, validate } from "./model";

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/templates/editor/ListColumns.svelte"),
    import("$lib/components/templates/editor/TimerDefault.svelte"),
  ]);
});

afterEach(cleanup);

const text: FieldDef = { key: "name", label: "Name", type: "text", default: "Ada" };
const list = (columns = [{ key: "name", label: "Name" }], dflt = "[]"): FieldDef => ({
  key: "board",
  label: "Board",
  type: "list",
  default: dflt,
  columns,
});
const issuesOf = (fields: FieldDef[]) => validate({ fields, variables: [], htmlLength: 0, cssLength: 0, fieldIssues: typeIssues });

describe("field types: model", () => {
  it("withFieldType sets up and drops type-specific settings", () => {
    expect(withFieldType(text, "list")).toEqual({
      key: "name",
      label: "Name",
      type: "list",
      default: "[]",
      columns: [{ key: "name", label: "Name" }],
    });
    expect(withFieldType(text, "clock")).toMatchObject({ type: "clock", format: "HH:mm:ss" });
    expect(JSON.parse(withFieldType(text, "timer").default)).toEqual({ run: false, at: 0, base: 0, dir: "up", end: "stop" });
    const back = withFieldType(withFieldType(text, "list"), "text");
    expect(back).toEqual({ key: "name", label: "Name", type: "text", default: "" });
    expect("columns" in back).toBe(false);
    expect(withFieldType(text, "number")).toEqual({ ...text, type: "number" });
  });

  it("columns: new keys are unique; a key follows its label until edited by hand", () => {
    const cols = [{ key: "column_1", label: "Column 1" }];
    expect(newColumn(cols)).toEqual({ key: "column_2", label: "Column 2" });
    const renamed = relabelColumn(cols, 0, "Team score");
    expect(renamed[0]).toEqual({ key: "team_score", label: "Team score" });
    const hand = [{ key: "pts", label: "Team score" }];
    expect(relabelColumn(hand, 0, "Points")[0]).toEqual({ key: "pts", label: "Points" });
    const clash = [{ key: "name", label: "Name" }, { key: "x", label: "X" }];
    expect(relabelColumn(clash, 1, "Name")[1].key).toBe("name_2");
  });

  it("list validation: 1–16 columns, valid unique keys, labels", () => {
    expect(issuesOf([list()])).toEqual([]);
    expect(issuesOf([list([])]).map((i) => i.prop)).toContain("columns");
    const many = Array.from({ length: SOURCE_COLUMNS_MAX + 1 }, (_, i) => ({ key: `c${i}`, label: `C${i}` }));
    expect(issuesOf([list(many)]).find((i) => i.prop === "columns")?.message).toMatch(/16 columns/);
    const dup = issuesOf([list([{ key: "a", label: "A" }, { key: "a", label: "B" }])]);
    expect(issueWith(dup, 0, "column")).toMatch(/Column 2: key “a” is already used/);
    expect(issueWith(issuesOf([list([{ key: "9x", label: "A" }])]), 0, "column")).toMatch(/Column 1/);
    expect(issueWith(issuesOf([list([{ key: "a", label: " " }])]), 0, "column")).toMatch(/enter a label/);
  });

  it("clock formats: only HH mm ss with : . or spaces, up to 32", () => {
    expect(clockFormatIssue("HH:mm:ss")).toBeNull();
    expect(clockFormatIssue("HH.mm")).toBeNull();
    expect(clockFormatIssue("mm ss")).toBeNull();
    expect(clockFormatIssue("")).not.toBeNull();
    expect(clockFormatIssue("HH:mm a")).not.toBeNull();
    expect(clockFormatIssue("hh:mm")).not.toBeNull();
    expect(clockFormatIssue(": :")).not.toBeNull();
    expect(clockFormatIssue("HH:".repeat(11))).toMatch(/32/);
    const f: FieldDef = { key: "t", label: "T", type: "clock", default: "", format: "YYYY" };
    expect(issuesOf([f]).map((i) => i.prop)).toEqual(["format"]);
  });

  it("timer defaults must be a valid paused state", () => {
    const ok: FieldDef = { key: "t", label: "T", type: "timer", default: '{"run":false,"at":0,"base":60000,"dir":"down","end":"hide"}' };
    expect(issuesOf([ok])).toEqual([]);
    expect(issuesOf([{ ...ok, default: "nope" }]).map((i) => i.prop)).toEqual(["default"]);
    expect(issuesOf([{ ...ok, default: '{"base":-1,"dir":"up","end":"stop"}' }])).toHaveLength(1);
  });

  it("preview defaults per type, and list defaults keep only known columns", () => {
    // A default shows as it is; an empty text field shows a placeholder.
    expect(previewDefault(text)).toBe("Ada");
    expect(previewDefault({ ...text, default: "" })).toBe("[Name]");
    expect(previewDefault({ ...text, type: "multiline", label: "Title", default: "" })).toBe("[Title]");
    expect(previewDefault({ ...text, type: "number", label: "Score" })).toBe("2");
    expect(JSON.parse(previewDefault(list([{ key: "name", label: "Name" }, { key: "score", label: "Score" }])))).toEqual([
      { name: "Name", score: "Score" },
    ]);
    expect(previewDefault(list(undefined, '[{"name":"Ada"}]'))).toBe('[{"name":"Ada"}]');
    expect(previewDefault({ key: "c", label: "C", type: "clock", default: "", format: "HH:mm" })).toBe("HH:mm");
    expect(listDefault(list(undefined, '[{"name":"A","gone":"x"}]'), [{ key: "name", label: "Name" }])).toBe('[{"name":"A"}]');
  });
});

describe("FieldsForm: list, clock and timer fields", () => {
  function setup(fields: FieldDef[]) {
    const onchange = vi.fn();
    const r = render(FieldsForm, { fields, otherKeys: [], issues: issuesOf(fields), onchange });
    return { ...r, onchange, last: () => onchange.mock.calls.at(-1)![0] as FieldDef[] };
  }

  it("the type select offers List, Clock and Timer and switching sets them up", async () => {
    const { last } = setup([text]);
    const select = screen.getByRole("combobox", { name: "Field 1 type" }) as HTMLSelectElement;
    expect([...select.options].map((o) => o.textContent)).toEqual(
      expect.arrayContaining(["List", "Clock", "Timer"]),
    );
    await fireEvent.change(select, { target: { value: "list" } });
    expect(last()[0]).toMatchObject({ type: "list", columns: [{ key: "name", label: "Name" }], default: "[]" });
  });

  it("list columns: add, relabel (key follows), reorder and remove", async () => {
    const f = list([
      { key: "name", label: "Name" },
      { key: "score", label: "Score" },
    ]);
    const { last } = setup([f]);
    const group = await screen.findByRole("group", { name: "Columns of Board" });
    await fireEvent.click(within(group).getByRole("button", { name: "Add column" }));
    expect(last()[0].columns).toEqual([...f.columns!, { key: "column_3", label: "Column 3" }]);
    await fireEvent.input(within(group).getByRole("textbox", { name: "Board column 2 label" }), { target: { value: "Points" } });
    expect(last()[0].columns![1]).toEqual({ key: "points", label: "Points" });
    await fireEvent.click(within(group).getByRole("button", { name: "Move column 2 up" }));
    expect(last()[0].columns!.map((c) => c.key)).toEqual(["score", "name"]);
    await fireEvent.click(within(group).getByRole("button", { name: "Delete column 1" }));
    expect(last()[0].columns).toEqual([{ key: "score", label: "Score" }]);
  });

  it("the last column can't be deleted; column issues show under the columns", async () => {
    setup([list([{ key: "a", label: "A" }, { key: "a", label: "B" }])]);
    expect(await screen.findByText(/Column 2: key “a” is already used/)).toBeTruthy();
    cleanup();
    setup([list()]);
    expect((await screen.findByRole("button", { name: "Delete column 1" })).hasAttribute("disabled")).toBe(true);
  });

  it("clock fields edit their format, with the error shown", async () => {
    const f: FieldDef = { key: "t", label: "Time", type: "clock", default: "HH:mm", format: "HH:mm:x" };
    const { last } = setup([f]);
    const input = screen.getByRole("textbox", { name: "Field 1 clock format" });
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByText(/Use HH, mm and ss/)).toBeTruthy();
    await fireEvent.input(input, { target: { value: "HH:mm" } });
    expect(last()[0]).toMatchObject({ format: "HH:mm", default: "HH:mm" });
  });

  it("timer fields set direction, start and at-zero as the default state", async () => {
    const f: FieldDef = { key: "t", label: "Clock", type: "timer", default: '{"run":false,"at":0,"base":0,"dir":"up","end":"stop"}' };
    const { last, rerender } = setup([f]);
    await fireEvent.change(await screen.findByRole("combobox", { name: "Clock direction" }), { target: { value: "down" } });
    const down = last()[0];
    expect(JSON.parse(down.default)).toMatchObject({ dir: "down", run: false });
    await rerender({ fields: [down], otherKeys: [], issues: [], onchange: vi.fn() });
    const start = await screen.findByRole("textbox", { name: "Clock start time (mm:ss)" });
    expect((start as HTMLInputElement).value).toBe("00:00");
    expect(screen.getByRole("combobox", { name: "Clock at zero" })).toBeTruthy();
    await fireEvent.input(start, { target: { value: "x" } });
    expect(start.getAttribute("aria-invalid")).toBe("true");
  });

  it("timer start time emits the base in ms", async () => {
    const f: FieldDef = { key: "t", label: "Clock", type: "timer", default: '{"run":false,"at":0,"base":0,"dir":"down","end":"stop"}' };
    const { last } = setup([f]);
    await fireEvent.input(await screen.findByRole("textbox", { name: "Clock start time (mm:ss)" }), { target: { value: "2:00" } });
    expect(JSON.parse(last()[0].default)).toMatchObject({ base: 120_000, dir: "down" });
    await fireEvent.change(screen.getByRole("combobox", { name: "Clock at zero" }), { target: { value: "hide" } });
    expect(JSON.parse(last()[0].default)).toMatchObject({ end: "hide" });
  });
});

describe("preview values of number-like fields", () => {
  const f = (key: string, label: string, type: FieldDef["type"], def = ""): FieldDef => ({ key, label, type, default: def });

  it("never preview a label in a number's slot (it overflowed the graphic)", () => {
    // Number fields: their default if numeric, else a number for their kind.
    expect(previewDefault(f("boost", "Player 1 boost", "number"))).toBe("45");
    expect(previewDefault(f("boost", "Player 1 boost", "number", "Player 1 boost"))).toBe("45");
    expect(previewDefault(f("n", "Anything", "number", "7"))).toBe("7");
    // Text fields named like numbers (models often type scores as text).
    expect(previewDefault(f("blue-score", "Blue series score", "text"))).toBe("3");
    expect(previewDefault(f("home-goals", "Home", "text"))).toBe("2");
    expect(previewDefault(f("pos", "Position", "text"))).toBe("1");
    expect(previewDefault(f("score", "Score", "text", "0"))).toBe("0");
    // Other text: its default, else the placeholder (as before).
    expect(previewDefault(f("team", "Blue team", "text"))).toBe("[Blue team]");
    expect(previewDefault(f("team", "Blue team", "text", "Rangers"))).toBe("Rangers");
    expect(sampleNumber("year", "Season")).toBe("2026");
  });

  it("uses the same number words as the core (src-tauri/src/core/fieldvals.rs)", () => {
    const rs = readFileSync(join(__dirname, "../../../../../src-tauri/src/core/fieldvals.rs"), "utf8");
    const block = /pub const NUMBER_WORDS: &\[&str\] = &\[([\s\S]*?)\];/.exec(rs)?.[1] ?? "";
    const core = [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(core.length).toBeGreaterThan(20);
    expect([...NUMBER_WORDS]).toEqual(core);
  });
});
