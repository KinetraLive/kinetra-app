import { describe, expect, it } from "vitest";
import type { SourceColumn } from "$lib/types";
import { History } from "./history";
import { clampCell, enterDown, isTypingKey, move, tab } from "./keys";
import { cellIssue, countText, rowsIssue, sizeText, valueBytes } from "./limits";
import { IGNORE, NEW_COLUMN, applyPaste, planPaste } from "./paste";
import {
  addColumn,
  insertOrder,
  keyIssue,
  moveOrder,
  nudgeOrder,
  permute,
  removeColumn,
  removeOrder,
  sameTable,
  setKey,
  setKind,
  setLabel,
  shiftColumn,
} from "./rows";
import { NONE, all, range, remap, selectOne, sorted, toggle } from "./selection";
import type { SheetLimits, SheetRow } from "./types";

const size = { rows: 3, cols: 2 };
const cols: SourceColumn[] = [
  { key: "name", label: "Name" },
  { key: "score", label: "Score" },
];
const LIST: SheetLimits = { rows: 100, bytes: 64 * 1024, cell: null };
const TABLE: SheetLimits = { rows: 1000, bytes: null, cell: 4096, columns: 16 };

describe("keyboard model", () => {
  it("Tab wraps along rows and stays at either end", () => {
    expect(tab({ row: 0, col: 0 }, false, size)).toEqual({ row: 0, col: 1 });
    expect(tab({ row: 0, col: 1 }, false, size)).toEqual({ row: 1, col: 0 });
    expect(tab({ row: 1, col: 0 }, true, size)).toEqual({ row: 0, col: 1 });
    expect(tab({ row: 2, col: 1 }, false, size)).toEqual({ row: 2, col: 1 });
    expect(tab({ row: 0, col: 0 }, true, size)).toEqual({ row: 0, col: 0 });
  });

  it("arrows clamp; Ctrl jumps to the edge; Home/End; pages", () => {
    expect(move({ row: 0, col: 0 }, "ArrowUp", size)).toEqual({ row: 0, col: 0 });
    expect(move({ row: 0, col: 0 }, "ArrowDown", size)).toEqual({ row: 1, col: 0 });
    expect(move({ row: 0, col: 0 }, "ArrowRight", size)).toEqual({ row: 0, col: 1 });
    expect(move({ row: 0, col: 1 }, "ArrowRight", size)).toEqual({ row: 0, col: 1 });
    expect(move({ row: 0, col: 0 }, "ArrowDown", size, true)).toEqual({ row: 2, col: 0 });
    expect(move({ row: 1, col: 1 }, "Home", size)).toEqual({ row: 1, col: 0 });
    expect(move({ row: 1, col: 0 }, "End", size, true)).toEqual({ row: 2, col: 1 });
    expect(move({ row: 0, col: 0 }, "PageDown", { rows: 100, cols: 1 }, false, 20)).toEqual({ row: 20, col: 0 });
    expect(move({ row: 5, col: 0 }, "PageUp", { rows: 100, cols: 1 }, false, 20)).toEqual({ row: 0, col: 0 });
  });

  it("Enter after an edit goes down; on the last row it adds a row", () => {
    expect(enterDown({ row: 0, col: 1 }, size)).toEqual({ cell: { row: 1, col: 1 }, addRow: false });
    expect(enterDown({ row: 2, col: 1 }, size)).toEqual({ cell: { row: 3, col: 1 }, addRow: true });
  });

  it("clamps and recognises typing keys", () => {
    expect(clampCell({ row: 9, col: 9 }, size)).toEqual({ row: 2, col: 1 });
    expect(clampCell({ row: 0, col: 0 }, { rows: 0, cols: 2 })).toBeNull();
    const k = (key: string, mods = {}) => ({ key, ctrlKey: false, metaKey: false, altKey: false, ...mods });
    expect(isTypingKey(k("a"))).toBe(true);
    expect(isTypingKey(k("Enter"))).toBe(false);
    expect(isTypingKey(k("a", { ctrlKey: true }))).toBe(false);
  });
});

describe("selection model", () => {
  it("click, Ctrl toggle, Shift range from the anchor, all", () => {
    let s = selectOne(2);
    expect(sorted(s)).toEqual([2]);
    s = toggle(s, 5);
    expect(sorted(s)).toEqual([2, 5]);
    expect(s.anchor).toBe(5);
    s = range(s, 3);
    expect(sorted(s)).toEqual([3, 4, 5]);
    s = toggle(s, 4);
    expect(sorted(s)).toEqual([3, 5]);
    expect(sorted(range(NONE, 1))).toEqual([1]);
    expect(sorted(all(3))).toEqual([0, 1, 2]);
  });

  it("follows rows through a change", () => {
    const s = toggle(selectOne(1), 3); // {1,3}, anchor 3
    // Row 0 deleted: old 1 → new 0, old 3 → new 2.
    const r = remap(s, [1, 2, 3]);
    expect(sorted(r)).toEqual([0, 2]);
    expect(r.anchor).toBe(2);
    // Row 1 deleted.
    expect(sorted(remap(s, [0, 2, 3]))).toEqual([2]);
  });
});

describe("undo stack", () => {
  it("undo / redo; a new change clears redo; capped", () => {
    const h = new History<number>(3);
    expect(h.undo(0)).toBeNull();
    h.push(1);
    h.push(2);
    expect(h.undo(3)).toBe(2);
    expect(h.canRedo).toBe(true);
    expect(h.redo(2)).toBe(3);
    expect(h.undo(3)).toBe(2);
    h.push(9);
    expect(h.canRedo).toBe(false);
    h.push(10);
    h.push(11);
    h.push(12);
    expect(h.size).toBe(3);
    expect(h.undo(0)).toBe(12);
    expect(h.pop()).toBe(11);
    h.clear();
    expect(h.canUndo).toBe(false);
  });
});

describe("limits", () => {
  const rows = (n: number): SheetRow[] => Array.from({ length: n }, (_, i) => ({ name: `R${i}` }));

  it("refuses more rows than the limit, never a shrink", () => {
    expect(rowsIssue(rows(100), cols, LIST, 99)).toBeNull();
    expect(rowsIssue(rows(101), cols, LIST, 100)).toMatch(/Up to 100 rows: this would make 101 \(1 row too many\)/);
    expect(rowsIssue(rows(120), cols, LIST, 130)).toBeNull();
  });

  it("refuses a value over the byte limit and a cell over its limit", () => {
    const big = [{ name: "x".repeat(70 * 1024) }];
    expect(rowsIssue(big, cols, LIST, 0)).toMatch(/Up to 64 KB of data/);
    expect(cellIssue("é".repeat(3000), TABLE)).toMatch(/up to 4 KB/);
    expect(cellIssue("short", TABLE)).toBeNull();
    expect(rowsIssue([{ name: "y".repeat(5000) }], cols, TABLE, 0)).toMatch(/4 KB/);
  });

  it("counts against the limit", () => {
    expect(valueBytes([{ name: "a", score: "" }], cols)).toBe(JSON.stringify([{ name: "a" }]).length);
    expect(countText(rows(3), cols, TABLE)).toBe("3 / 1000 rows");
    expect(countText(rows(3), cols, LIST)).toMatch(/^3 \/ 100 rows · \d+ B \/ 64 KB$/);
  });

  it("writes sizes in bytes, then KB with one decimal under 10 KB", () => {
    expect(sizeText(0)).toBe("0 B");
    expect(sizeText(180)).toBe("180 B");
    expect(sizeText(1024)).toBe("1.0 KB");
    expect(sizeText(2458)).toBe("2.4 KB");
    expect(sizeText(10 * 1024)).toBe("10 KB");
    expect(sizeText(37_900)).toBe("37 KB");
  });
});

describe("reorder and row orders", () => {
  it("moves a block before a row, keeping its order", () => {
    expect(moveOrder(5, [1, 2], 4)).toEqual([0, 3, 1, 2, 4]);
    expect(moveOrder(5, [3], 0)).toEqual([3, 0, 1, 2, 4]);
    expect(moveOrder(5, [0, 4], 5)).toEqual([1, 2, 3, 0, 4]);
    expect(moveOrder(3, [1], 1)).toEqual([0, 1, 2]);
  });

  it("nudges up and down; nothing at the ends", () => {
    expect(nudgeOrder(4, [1], -1)).toEqual([1, 0, 2, 3]);
    expect(nudgeOrder(4, [1, 2], 1)).toEqual([0, 3, 1, 2]);
    expect(nudgeOrder(4, [0], -1)).toBeNull();
    expect(nudgeOrder(4, [3], 1)).toBeNull();
  });

  it("insert and remove orders; permute fills new rows", () => {
    expect(insertOrder(2, 1)).toEqual([0, -1, 1]);
    expect(removeOrder(4, [1, 3])).toEqual([0, 2]);
    expect(permute(["a", "b"], [1, -1, 0], () => "new")).toEqual(["b", "new", "a"]);
  });
});

describe("columns", () => {
  const t = { columns: cols, rows: [{ name: "Ada", score: "3" }] };

  it("label (key follows when asked), key, kind, add, remove, shift", () => {
    const relabeled = setLabel(t, 0, "Full name", true);
    expect(relabeled.columns[0]).toEqual({ key: "full_name", label: "Full name" });
    expect(relabeled.rows[0]).toEqual({ full_name: "Ada", score: "3" });
    expect(setLabel(t, 0, "Who", false).columns[0]).toEqual({ key: "name", label: "Who" });

    expect(keyIssue(t, 0, "score")).toMatch(/Another column/);
    expect(keyIssue(t, 0, "9x")).toMatch(/starts with a letter/);
    expect(setKey(t, 0, "9x")).toBe(t);
    expect(setKey(t, 0, "who").rows[0]).toEqual({ who: "Ada", score: "3" });

    expect(setKind(t, 1, "image").columns[1].kind).toBe("image");
    expect(setKind(setKind(t, 1, "image"), 1, "text").columns[1]).toEqual({ key: "score", label: "Score" });
    expect(addColumn(t).columns[2]).toEqual({ key: "column_3", label: "Column 3" });
    expect(removeColumn(t, 0)).toEqual({ columns: [cols[1]], rows: [{ score: "3" }] });
    expect(shiftColumn(t, 0, 1).columns.map((c) => c.key)).toEqual(["score", "name"]);
  });

  it("sameTable compares data, not identity", () => {
    expect(sameTable(t, { columns: [...cols], rows: [{ name: "Ada", score: "3" }] })).toBe(true);
    expect(sameTable(t, { columns: cols, rows: [{ name: "Ada", score: "4" }] })).toBe(false);
  });
});

describe("paste: plan, mapping, apply", () => {
  const rows: SheetRow[] = [
    { name: "Ada", score: "1" },
    { name: "Grace", score: "2" },
  ];

  it("a header line names columns (by label or key) and maps them; the rest is Ignore", () => {
    const plan = planPaste("Score\tName\tTeam\n5\tKate\tRed\n", cols)!;
    expect(plan.header).toBe(true);
    expect(plan.names).toEqual(["Score", "Name", "Team"]);
    expect(plan.map).toEqual([1, 0, IGNORE]);
  });

  it("without a header columns map in order; extra columns are New column when editable", () => {
    expect(planPaste("a,b,c\nd,e,f", cols)!.map).toEqual([0, 1, IGNORE]);
    const editable = planPaste("a\tb\tc", cols, true, 16)!;
    expect(editable.header).toBe(false);
    expect(editable.map).toEqual([0, 1, NEW_COLUMN]);
    expect(planPaste("  \n", cols)).toBeNull();
  });

  it("appends, or replaces from a row down (adding rows past the end)", () => {
    const plan = planPaste("Name\tScore\nKate\t9\nAlan\t7\nLin\t4", cols)!;
    const appended = applyPaste(rows, cols, plan, { header: true, map: plan.map, mode: "append", at: 0 }, LIST);
    expect(appended.ok && appended.rows.map((r) => r.name)).toEqual(["Ada", "Grace", "Kate", "Alan", "Lin"]);
    expect(appended.ok && [appended.added, appended.replaced]).toEqual([3, 0]);
    expect(appended.ok && appended.order).toEqual([0, 1, -1, -1, -1]);

    const replaced = applyPaste(rows, cols, plan, { header: true, map: plan.map, mode: "replace", at: 1 }, LIST);
    expect(replaced.ok && replaced.rows).toEqual([
      { name: "Ada", score: "1" },
      { name: "Kate", score: "9" },
      { name: "Alan", score: "7" },
      { name: "Lin", score: "4" },
    ]);
    expect(replaced.ok && [replaced.added, replaced.replaced]).toEqual([2, 1]);

    // Only Score mapped: names stay.
    const scoreOnly = applyPaste(rows, cols, plan, { header: true, map: [IGNORE, 1], mode: "replace", at: 0 }, LIST);
    expect(scoreOnly.ok && scoreOnly.rows[0]).toEqual({ name: "Ada", score: "9" });
  });

  it("new columns take the header names; nothing mapped or over the limit is refused", () => {
    const plan = planPaste("Name\tTeam\nKate\tRed", cols, true)!;
    const r = applyPaste(rows, cols, plan, { header: true, map: plan.map, mode: "append", at: 0 }, TABLE);
    expect(r.ok && r.columns.at(-1)).toEqual({ key: "team", label: "Team" });
    expect(r.ok && r.rows.at(-1)).toEqual({ name: "Kate", team: "Red" });

    const none = applyPaste(rows, cols, plan, { header: true, map: [IGNORE, IGNORE], mode: "append", at: 0 }, TABLE);
    expect(none).toEqual({ ok: false, reason: "Map at least one column." });

    const many = planPaste(Array.from({ length: 120 }, (_, i) => `R${i}\t${i}`).join("\n"), cols)!;
    const over = applyPaste(rows, cols, many, { header: false, map: many.map, mode: "append", at: 0 }, LIST);
    expect(over.ok).toBe(false);
    expect(!over.ok && over.reason).toMatch(/Up to 100 rows: this would make 122 \(22 rows too many\)/);
  });

  it("500 rows: parse + plan + preview + apply in ≤ 50 ms", () => {
    const text = Array.from({ length: 500 }, (_, i) => `Name ${i}\t${i}\t"quoted, ${i}"\tTeam ${i % 7}`).join("\r\n");
    const t0 = performance.now();
    const plan = planPaste(text, cols, true, 16)!;
    const preview = plan.lines.slice(0, 5);
    const r = applyPaste([], cols, plan, { header: plan.header, map: plan.map, mode: "append", at: 0 }, TABLE);
    const ms = performance.now() - t0;
    expect(preview).toHaveLength(5);
    expect(r.ok && r.rows).toHaveLength(500);
    expect(ms).toBeLessThan(50);
  });
});
