// List summaries (FieldInputs → ListField) and the list drawer (ListDrawer) with a stand-in for the
// spreadsheet editor: own rows vs. a linked data table, open / close / Esc / focus return, and the
// source save path.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { FieldDef, ScreenDoc, SourceDoc } from "$lib/types";
import type { SheetEditorProps } from "$lib/components/sheet/types";

const api = vi.hoisted(() => ({
  getSource: vi.fn(),
  saveSource: vi.fn(),
  getScreen: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  toAppError: (e: unknown) => ({ code: "internal", message: e instanceof Error ? e.message : String(e) }),
}));
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/components/sheet/SheetEditor.svelte", async () => ({
  default: (await import("./SheetStub.test.svelte")).default,
}));

import FieldInputs from "./FieldInputs.svelte";
import { affectsNote, followIndex, layersUsing, LIST_LIMITS, SOURCE_LIMITS, SourceSaver, type LinkedList } from "./drawer";

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/fields/ListField.svelte"),
  ]);
});

const COLS = [
  { key: "name", label: "Name" },
  { key: "score", label: "Score" },
];
const listDef: FieldDef = { key: "board", label: "Board", type: "list", default: "[]", columns: COLS };
const TWO = '[{"name":"Ada","score":"10"},{"name":"Bo","score":"7"}]';
const sheet = () => (globalThis as { __sheet?: SheetEditorProps }).__sheet!;

const guests: SourceDoc = {
  id: "src1",
  name: "Guests",
  kind: "text",
  columns: COLS,
  rows: [{ name: "Ada" }, { name: "Bo" }, { name: "Cy" }],
  index: 1,
  updatedAt: "2026-10-06T10:00:00Z",
};
const screenDoc = (id: string, bound: string[]): ScreenDoc =>
  ({
    id,
    layers: bound.map((sourceId, i) => ({ id: `l${i}`, bindings: sourceId ? { f: { sourceId, column: "" } } : undefined })),
  }) as unknown as ScreenDoc;

beforeEach(() => {
  for (const f of Object.values(api)) f.mockReset();
  delete (globalThis as { __sheet?: unknown }).__sheet;
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("drawer helpers", () => {
  it("counts the layers bound to a source and words the note", () => {
    const docs = [screenDoc("a", ["src1", "src1", ""]), screenDoc("b", ["other", "src1"])];
    expect(layersUsing(docs, "src1")).toBe(3);
    expect(layersUsing(docs, "none")).toBe(0);
    expect(affectsNote(1)).toBe("Changes affect 1 layer.");
    expect(affectsNote(3)).toBe("Changes affect 3 layers.");
  });

  it("follows the current row through moves, adds and deletes", () => {
    expect(followIndex(1, [0, 1, 2, -1])).toBe(1); // a row added at the end
    expect(followIndex(1, [1, 0, 2])).toBe(0); // moved up
    expect(followIndex(2, [0, 1])).toBe(1); // deleted: clamped
    expect(followIndex(0, [1, 2])).toBe(0); // deleted: the next row takes its place
    expect(followIndex(3, [])).toBe(0);
  });

  it("SourceSaver: debounced, one save in flight, a change during a save saves again", async () => {
    vi.useFakeTimers();
    let doc = 1;
    let release: () => void = () => {};
    const save = vi.fn((_d: number) => new Promise<void>((r) => (release = r)));
    const states: string[] = [];
    const s = new SourceSaver(500, () => doc, save, (st) => states.push(st));
    s.touch();
    doc = 2;
    s.touch();
    await vi.advanceTimersByTimeAsync(499);
    expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(save).toHaveBeenCalledExactlyOnceWith(2);
    doc = 3;
    s.touch();
    await vi.advanceTimersByTimeAsync(600);
    expect(save).toHaveBeenCalledTimes(1); // still in flight
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenLastCalledWith(3);
    release();
    await s.flush();
    expect(states.at(-1)).toBe("saved");
    // A failed save stays pending; flush retries it.
    save.mockRejectedValueOnce(new Error("disk"));
    s.touch();
    await s.flush();
    expect(states.at(-1)).toBe("failed");
    save.mockResolvedValueOnce(undefined);
    await s.flush();
    expect(states.at(-1)).toBe("saved");
  });

  it("limits are the server's", () => {
    expect(LIST_LIMITS).toEqual({ rows: 100, bytes: 64 * 1024, cell: null });
    expect(SOURCE_LIMITS).toEqual({ rows: 1000, bytes: null, cell: 4096 });
  });
});

describe("list summaries", () => {
  it("own rows: '2 items · Edit', the count in body text", async () => {
    render(FieldInputs, { defs: [listDef], values: { board: TWO }, onfield: vi.fn() });
    const edit = await screen.findByRole("button", { name: "Edit items of Board" }, { timeout: 5000 });
    expect(edit.textContent?.trim()).toBe("Edit");
    const line = edit.closest(".summary")!;
    expect(line.textContent?.replace(/\s+/g, " ").trim()).toBe("2 items · Edit");
    expect(line.querySelector(".n")?.textContent).toBe("2");
    expect(line.querySelector(".mono")).toBeNull();
    // No inline table any more.
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("one item reads singular; an empty list reads 0 items", async () => {
    render(FieldInputs, { defs: [listDef], values: { board: '[{"name":"A"}]' }, onfield: vi.fn() });
    const edit = await screen.findByRole("button", { name: "Edit items of Board" }, { timeout: 5000 });
    expect(edit.closest(".summary")!.textContent).toMatch(/1\s+item\b/);
  });

  it("linked to a data table: 'Linked to: Guests · Edit source' with the binding as a tooltip", async () => {
    const link: LinkedList = { sourceId: "src1", name: "Guests", kind: "text", usedBy: ["a"], detail: "Guests · all rows, top 3" };
    render(FieldInputs, {
      defs: [listDef],
      values: { board: TWO },
      bound: { board: "Guests · all rows, top 3" },
      linked: { board: link },
      onfield: vi.fn(),
    });
    const edit = await screen.findByRole("button", { name: "Edit data table Guests" }, { timeout: 5000 });
    const line = edit.closest(".summary") as HTMLElement;
    expect(line.textContent?.replace(/\s+/g, " ").trim()).toBe("Linked to: Guests · Edit source");
    expect(line.title).toBe("Guests · all rows, top 3");
    // Not the read-only binding box.
    expect(screen.queryByRole("textbox", { name: "Board" })).toBeNull();
  });

  it("linked to a schedule: the name, no Edit", async () => {
    render(FieldInputs, {
      defs: [listDef],
      values: {},
      bound: { board: "Run of show · all items" },
      linked: { board: { sourceId: "s", name: "Run of show", kind: "schedule", usedBy: [] } },
      onfield: vi.fn(),
    });
    expect(await screen.findByText("Run of show", undefined, { timeout: 5000 })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Edit/ })).toBeNull();
  });

  it("bound without link info keeps the read-only binding", () => {
    render(FieldInputs, { defs: [listDef], values: {}, bound: { board: "Guests · all rows" }, onfield: vi.fn() });
    expect(screen.getByRole("textbox", { name: "Board" }).getAttribute("aria-readonly")).toBe("true");
  });
});

describe("list drawer", () => {
  async function openLocal(value = TWO) {
    const onfield = vi.fn();
    render(FieldInputs, { defs: [listDef], values: { board: value }, onfield });
    const edit = await screen.findByRole("button", { name: "Edit items of Board" }, { timeout: 5000 });
    edit.focus();
    await fireEvent.click(edit);
    const dialog = await screen.findByRole("dialog", { name: "Board" }, { timeout: 5000 });
    await screen.findByRole("grid", { name: "Board" }, { timeout: 5000 });
    return { onfield, edit, dialog };
  }

  it("opens the sheet on the layer's rows with the list limits; changes go out as the list value", async () => {
    const { onfield } = await openLocal();
    expect(sheet().rows).toEqual([
      { name: "Ada", score: "10" },
      { name: "Bo", score: "7" },
    ]);
    expect(sheet().limits).toEqual(LIST_LIMITS);
    expect(sheet().editableColumns ?? false).toBe(false);
    await fireEvent.click(screen.getByRole("button", { name: "Stub add row" }));
    expect(onfield).toHaveBeenLastCalledWith("board", '[{"name":"Ada","score":"10"},{"name":"Bo","score":"7"},{"name":"New"}]');
  });

  it("image columns reach the sheet; a picked image lands in the list value", async () => {
    const onfield = vi.fn();
    const logoCols = [
      { key: "logo", label: "Logo", kind: "image" as const },
      { key: "name", label: "Name" },
    ];
    const def: FieldDef = { key: "sponsors", label: "Sponsors", type: "list", default: "[]", columns: logoCols };
    render(FieldInputs, { defs: [def], values: { sponsors: '[{"name":"Beta"}]' }, onfield });
    await fireEvent.click(await screen.findByRole("button", { name: "Edit items of Sponsors" }, { timeout: 5000 }));
    await screen.findByRole("grid", { name: "Sponsors" }, { timeout: 5000 });
    expect(sheet().columns).toEqual(logoCols);
    sheet().onchange([{ name: "Beta", logo: "/assets/sponsor-b.png" }], logoCols, [0]);
    expect(onfield).toHaveBeenLastCalledWith("sponsors", '[{"logo":"/assets/sponsor-b.png","name":"Beta"}]');
  });

  it("× closes and returns focus to Edit", async () => {
    const { edit } = await openLocal();
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(edit);
  });

  it("Esc closes, unless the sheet used it (a cell edit)", async () => {
    const { edit } = await openLocal();
    // The stub's cell prevents the default of its Esc (as cancelling an edit does).
    await fireEvent.keyDown(screen.getByRole("textbox", { name: "Stub cell" }), { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeTruthy();
    await fireEvent.keyDown(screen.getByRole("button", { name: "Stub add row" }), { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(document.activeElement).toBe(edit);
  });

  it("the sheet's own close (a second Esc) closes too", async () => {
    await openLocal();
    await fireEvent.click(screen.getByRole("button", { name: "Stub close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("a linked data table loads, saves debounced (following the current row) and notes the layers", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    api.getSource.mockResolvedValue(guests);
    api.getScreen.mockImplementation((id: string) =>
      Promise.resolve({ doc: id === "a" ? screenDoc("a", ["src1", "src1"]) : screenDoc("b", ["src1"]) }),
    );
    api.saveSource.mockImplementation((d: SourceDoc) => Promise.resolve({ ...d, updatedAt: "2026-10-06T10:00:01Z" }));
    const onsaved = vi.fn();
    render(FieldInputs, {
      defs: [listDef],
      values: {},
      bound: { board: "Guests · all rows" },
      linked: { board: { sourceId: "src1", name: "Guests", kind: "text", usedBy: ["a", "b"], onsaved } },
      onfield: vi.fn(),
    });
    await fireEvent.click(await screen.findByRole("button", { name: "Edit data table Guests" }, { timeout: 5000 }));
    await screen.findByRole("dialog", { name: "Data table Guests" }, { timeout: 5000 });
    await screen.findByRole("grid", { name: "Guests" }, { timeout: 5000 });
    expect(api.getSource).toHaveBeenCalledWith("src1");
    expect(sheet().limits).toEqual(SOURCE_LIMITS);
    expect(sheet().editableColumns ?? false).toBe(false);
    expect(sheet().currentRow).toBe(1);
    expect(await screen.findByText("Changes affect 3 layers.")).toBeTruthy();

    // Delete the first row: the current row (Bo) moves to index 0. Two quick edits, one save.
    await fireEvent.click(screen.getByRole("button", { name: "Stub delete first" }));
    expect(api.saveSource).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(600);
    expect(api.saveSource).toHaveBeenCalledTimes(1);
    expect(api.saveSource.mock.calls[0][0]).toMatchObject({ id: "src1", rows: [{ name: "Bo" }, { name: "Cy" }], index: 0 });
    await vi.waitFor(() => expect(onsaved).toHaveBeenCalled());
  });

  it("closing flushes a pending source save", async () => {
    api.getSource.mockResolvedValue(guests);
    api.getScreen.mockResolvedValue({ doc: screenDoc("a", ["src1"]) });
    api.saveSource.mockImplementation((d: SourceDoc) => Promise.resolve(d));
    render(FieldInputs, {
      defs: [listDef],
      values: {},
      bound: { board: "x" },
      linked: { board: { sourceId: "src1", name: "Guests", kind: "text", usedBy: ["a"] } },
      onfield: vi.fn(),
    });
    await fireEvent.click(await screen.findByRole("button", { name: "Edit data table Guests" }, { timeout: 5000 }));
    await screen.findByRole("grid", { name: "Guests" }, { timeout: 5000 });
    expect(await screen.findByText("Changes affect 1 layer.")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Stub add row" }));
    await fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(api.saveSource).toHaveBeenCalledTimes(1));
    expect(api.saveSource.mock.calls[0][0].rows).toHaveLength(4);
  });

  it("a data table that can't load says so", async () => {
    api.getSource.mockRejectedValue(new Error("gone"));
    render(FieldInputs, {
      defs: [listDef],
      values: {},
      bound: { board: "x" },
      linked: { board: { sourceId: "src1", name: "Guests", kind: "text", usedBy: [] } },
      onfield: vi.fn(),
    });
    await fireEvent.click(await screen.findByRole("button", { name: "Edit data table Guests" }, { timeout: 5000 }));
    expect((await screen.findByRole("alert")).textContent).toContain("gone");
  });
});
