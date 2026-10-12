import { describe, expect, it, vi } from "vitest";
import type { SourceDoc, SourceSummary } from "$lib/types";
import { sortRows, unlinkCopy, unlinkedValue } from "./link";

const cols = [
  { key: "name", label: "Name" },
  { key: "score", label: "Score" },
];
const rows: Record<string, string>[] = [
  { name: "Ada", score: "3", team: "Red" },
  { name: "grace", score: "x" },
  { name: "Alan", score: "10" },
  { name: "Kate", score: "3" },
];

describe("link / unlink a list to a data table", () => {
  it("sorts like the server: numeric (numbers first, stable), text case-insensitive, descending", () => {
    expect(sortRows(rows, { column: "score", numeric: true, desc: true }).map((r) => r.name)).toEqual(["Alan", "Ada", "Kate", "grace"]);
    expect(sortRows(rows, { column: "score", numeric: true, desc: false }).map((r) => r.name)).toEqual(["Ada", "Kate", "Alan", "grace"]);
    expect(sortRows(rows, { column: "name", numeric: false, desc: false }).map((r) => r.name)).toEqual(["Ada", "Alan", "grace", "Kate"]);
    expect(sortRows(rows, undefined)).toEqual(rows);
  });

  it("the copy keeps the field's columns by key, sorted and limited like the link", () => {
    const c = unlinkCopy(rows, { sourceId: "t", column: "", sort: { column: "score", numeric: true, desc: true }, limit: 2 }, cols);
    expect(c).toEqual({ value: '[{"name":"Alan","score":"10"},{"name":"Ada","score":"3"}]', note: null });
  });

  it("more than the list holds: the first 100 rows, said in a note", () => {
    const many = Array.from({ length: 150 }, (_, i) => ({ name: `R${i}` }));
    const c = unlinkCopy(many, { sourceId: "t", column: "" }, cols);
    expect(JSON.parse(c.value)).toHaveLength(100);
    expect(c.note).toMatch(/first 100 of 150/);
  });

  it("loads data tables only; a schedule or a single-column binding has nothing to copy", async () => {
    const doc: SourceDoc = { id: "t", name: "T", kind: "text", columns: cols, rows, index: 0, updatedAt: "" };
    const load = vi.fn(async () => doc);
    const summaries = [{ id: "s", kind: "schedule" }] as SourceSummary[];
    expect(await unlinkedValue({ sourceId: "t", column: "name" }, cols, null, load)).toBeNull();
    expect(await unlinkedValue({ sourceId: "s", column: "" }, cols, summaries, load)).toBeNull();
    expect(load).not.toHaveBeenCalled();
    const c = await unlinkedValue({ sourceId: "t", column: "" }, cols, summaries, load);
    expect(load).toHaveBeenCalledWith("t");
    expect(JSON.parse(c!.value)).toHaveLength(4);
  });
});
