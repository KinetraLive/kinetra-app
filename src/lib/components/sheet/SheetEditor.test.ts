import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import "../templates/list/matchers";
import type { SourceColumn } from "$lib/types";
import type { SheetEditorProps, SheetLimits, SheetRow } from "./types";

const api = vi.hoisted(() => ({ serverStatus: vi.fn(), on: vi.fn(() => () => {}) }));
vi.mock("$lib/api", () => ({ ...api, toAppError: (e: unknown) => ({ code: "internal", message: String(e) }) }));
vi.mock("$lib/api/live", () => api);
const picker = vi.hoisted(() => ({ pickAsset: vi.fn() }));
vi.mock("$lib/components/assets/pick", () => picker);

import SheetEditor from "./SheetEditor.svelte";
import { toasts } from "$lib/stores/toasts.svelte";

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/sheet/PasteDialog.svelte"),
  ]);
});

const COLS: SourceColumn[] = [
  { key: "name", label: "Name" },
  { key: "score", label: "Score" },
];
const LIST: SheetLimits = { rows: 100, bytes: 64 * 1024, cell: null };
const people = (): SheetRow[] => [
  { name: "Ada", score: "3" },
  { name: "Grace", score: "5" },
  { name: "Alan", score: "4" },
];

function mount(over: Partial<SheetEditorProps> = {}) {
  const onchange = vi.fn();
  const onclose = vi.fn();
  const props: SheetEditorProps = { title: "Board", columns: COLS, rows: people(), limits: LIST, onchange, onclose, ...over };
  const r = render(SheetEditor, props);
  return { ...r, onchange: (over.onchange as typeof onchange) ?? onchange, onclose };
}

const grid = () => screen.getByRole("grid");
const cell = (r: number, c: number) => document.querySelector<HTMLElement>(`[role="gridcell"][data-r="${r}"][data-c="${c}"]`)!;
const col = (c: number) => [...document.querySelectorAll(`[role="gridcell"][data-c="${c}"]`)].map((e) => e.textContent?.trim());
const edit = () => document.querySelector<HTMLInputElement>("input.edit");
const activeCell = () => {
  const id = grid().getAttribute("aria-activedescendant");
  const el = id ? document.getElementById(id) : null;
  return el ? [Number(el.dataset.r), Number(el.dataset.c)] : null;
};
const key = async (k: string, mods: Partial<KeyboardEventInit> = {}) => {
  await fireEvent.keyDown(edit() ?? grid(), { key: k, ...mods });
  await tick();
};
const lastRows = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls.at(-1)?.[0] as SheetRow[];

beforeEach(() => {
  vi.clearAllMocks();
  toasts.clear();
  api.serverStatus.mockResolvedValue({ state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null });
});
afterEach(() => cleanup());

describe("sheet editor: grid", () => {
  it("shows every column with a header, row numbers, the count against the limit; grid semantics", () => {
    mount({ note: "Changes affect 2 layers." });
    expect(screen.getByRole("heading", { name: "Board" })).toBeTruthy();
    expect(screen.getByText("Changes affect 2 layers.")).toBeTruthy();
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent?.trim())).toEqual(["#", "Name name", "Score score"]);
    expect(grid()).toHaveAttribute("aria-rowcount", "4");
    expect(grid()).toHaveAttribute("aria-colcount", "3");
    expect(col(0)).toEqual(["Ada", "Grace", "Alan"]);
    expect(screen.getByText(/^3 \/ 100 rows/)).toBeTruthy();
    expect(activeCell()).toEqual([0, 0]);
  });

  it("virtualises past 60 rows: a window of rows, aria-rowcount for all of them", () => {
    const rows = Array.from({ length: 1000 }, (_, i) => ({ name: `R${i}`, score: String(i) }));
    mount({ rows, limits: { rows: 1000, bytes: null, cell: 4096 } });
    expect(grid()).toHaveAttribute("aria-rowcount", "1001");
    const rendered = document.querySelectorAll('[role="row"].row').length;
    expect(rendered).toBeGreaterThan(20);
    expect(rendered).toBeLessThan(100);
    expect(document.querySelector('[role="row"].row')).toHaveAttribute("aria-rowindex", "2");
  });

  it("takes new rows from the host (a reload), but not its own echo", async () => {
    const { rerender, onchange } = mount();
    grid().focus();
    await key("x");
    await key("Enter");
    const echoed = lastRows(onchange);
    await rerender({ rows: echoed });
    expect(screen.getByRole("button", { name: "Undo" })).not.toBeDisabled();
    await rerender({ rows: [{ name: "Only" }] });
    expect(col(0)).toEqual(["Only"]);
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled();
  });
});

describe("sheet editor: keyboard", () => {
  it("arrows move; Tab wraps along rows; Tab at the very end leaves the grid", async () => {
    mount();
    grid().focus();
    await key("ArrowDown");
    expect(activeCell()).toEqual([1, 0]);
    await key("ArrowRight");
    await key("ArrowRight");
    expect(activeCell()).toEqual([1, 1]);
    await key("Tab");
    expect(activeCell()).toEqual([2, 0]);
    await key("Tab", { shiftKey: true });
    expect(activeCell()).toEqual([1, 1]);
    await key("End", { ctrlKey: true });
    expect(activeCell()).toEqual([2, 1]);
    const tab = new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true });
    grid().dispatchEvent(tab);
    expect(tab.defaultPrevented).toBe(false);
  });

  it("Enter edits, Enter commits and moves down; Enter on the last row adds a row", async () => {
    const { onchange } = mount();
    grid().focus();
    await key("ArrowDown", { ctrlKey: true });
    await key("Enter");
    expect(edit()?.value).toBe("Alan");
    await fireEvent.input(edit()!, { target: { value: "Alan T" } });
    expect(lastRows(onchange)[2].name).toBe("Alan T");
    await key("Enter");
    expect(edit()).toBeNull();
    expect(col(0)).toEqual(["Ada", "Grace", "Alan T", ""]);
    expect(activeCell()).toEqual([3, 0]);
    expect(lastRows(onchange)).toHaveLength(4);
    expect(onchange.mock.calls.at(-1)?.[2]).toEqual([0, 1, 2, -1]);
  });

  it("typing starts an edit with that character; Esc cancels it; Esc again closes", async () => {
    const { onchange, onclose } = mount();
    grid().focus();
    await key("Z");
    expect(edit()?.value).toBe("Z");
    expect(lastRows(onchange)[0].name).toBe("Z");
    await key("Escape");
    expect(edit()).toBeNull();
    expect(col(0)[0]).toBe("Ada");
    expect(lastRows(onchange)[0].name).toBe("Ada");
    expect(onclose).not.toHaveBeenCalled();
    await key("Escape");
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it("undo / redo with Ctrl+Z / Ctrl+Shift+Z; one step per edit", async () => {
    mount();
    grid().focus();
    await key("Enter");
    for (const v of ["A", "Ad", "Ada L"]) await fireEvent.input(edit()!, { target: { value: v } });
    await key("Enter");
    await key("Delete"); // clears Grace's name
    expect(col(0)).toEqual(["Ada L", "", "Alan"]);
    await key("z", { ctrlKey: true });
    expect(col(0)).toEqual(["Ada L", "Grace", "Alan"]);
    await key("z", { ctrlKey: true });
    expect(col(0)).toEqual(["Ada", "Grace", "Alan"]);
    await key("z", { ctrlKey: true, shiftKey: true });
    expect(col(0)).toEqual(["Ada L", "Grace", "Alan"]);
    await key("y", { ctrlKey: true });
    expect(col(0)).toEqual(["Ada L", "", "Alan"]);
  });

  it("Alt+↑/↓ move the selected rows; origin tells the host where rows came from", async () => {
    const { onchange } = mount();
    grid().focus();
    await key("ArrowDown", { altKey: true });
    expect(col(0)).toEqual(["Grace", "Ada", "Alan"]);
    expect(onchange.mock.calls.at(-1)?.[2]).toEqual([1, 0, 2]);
    expect(activeCell()).toEqual([1, 0]);
    await key("ArrowUp", { altKey: true });
    expect(col(0)).toEqual(["Ada", "Grace", "Alan"]);
  });
});

describe("sheet editor: selection and delete", () => {
  it("click, Shift range and Ctrl toggle on row numbers; Ctrl+A; Delete removes them with an Undo toast", async () => {
    const { onchange } = mount();
    const head = (r: number) => document.querySelector<HTMLElement>(`[role="rowheader"][data-r="${r}"]`)!;
    await fireEvent.pointerDown(head(0));
    await fireEvent.pointerDown(head(2), { shiftKey: true });
    expect(document.querySelectorAll('[role="row"][aria-selected="true"]')).toHaveLength(3);
    await fireEvent.pointerDown(head(1), { ctrlKey: true });
    expect(document.querySelectorAll('[role="row"][aria-selected="true"]')).toHaveLength(2);
    grid().focus();
    await key("Delete");
    expect(col(0)).toEqual(["Grace"]);
    expect(onchange.mock.calls.at(-1)?.[2]).toEqual([1]);
    const toast = toasts.list.at(-1)!;
    expect(toast.message).toBe("Deleted 2 rows.");
    toast.action!.run();
    await tick();
    expect(col(0)).toEqual(["Ada", "Grace", "Alan"]);
    await key("a", { ctrlKey: true });
    expect(screen.getByText(/^\d+ selected$/)).toHaveTextContent("3 selected");
  });

  it("a Shift-click on a cell selects the row range from the active row", async () => {
    mount();
    await fireEvent.pointerDown(cell(0, 1));
    await fireEvent.pointerDown(cell(2, 1), { shiftKey: true });
    expect(document.querySelectorAll('[role="row"][aria-selected="true"]')).toHaveLength(3);
    expect(activeCell()).toEqual([2, 1]);
  });
});

describe("sheet editor: limits", () => {
  it("refuses a row past the limit with a message; Add row is disabled at the limit", async () => {
    const rows = Array.from({ length: 40 }, (_, i) => ({ name: `R${i}` }));
    const { onchange } = mount({ rows, limits: { rows: 40, bytes: null, cell: null } });
    expect(screen.getByRole("button", { name: "Add row" })).toBeDisabled();
    grid().focus();
    await key("End", { ctrlKey: true });
    await key("Home");
    await key("Enter");
    await key("Enter");
    expect(screen.getByText(/Up to 40 rows: this would make 41 \(1 row too many\)/)).toBeTruthy();
    expect(onchange.mock.calls.every((c) => (c[0] as SheetRow[]).length <= 40)).toBe(true);
  });

  it("refuses a cell over the cell limit and keeps the last good value", async () => {
    const { onchange } = mount({ limits: { rows: 10, bytes: null, cell: 8 } });
    grid().focus();
    await key("Enter");
    await fireEvent.input(edit()!, { target: { value: "x".repeat(9) } });
    expect(screen.getByText(/A cell holds up to/)).toBeTruthy();
    expect(edit()!.value).toBe("Ada");
    expect(onchange).not.toHaveBeenCalled();
  });
});

describe("sheet editor: paste", () => {
  const pasteText = (text: string) => {
    const e = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(e, "clipboardData", { value: { getData: () => text } });
    grid().dispatchEvent(e);
    return e;
  };

  it("one value goes into the active cell", async () => {
    mount();
    grid().focus();
    await key("ArrowRight");
    expect(pasteText("9").defaultPrevented).toBe(true);
    await tick();
    expect(col(1)).toEqual(["9", "5", "4"]);
  });

  it("several cells open the mapping preview; Replace from the active row; Apply is one undo step", async () => {
    const { onchange } = mount();
    grid().focus();
    await key("ArrowDown");
    pasteText("Score\tName\n7\tKate\n8\tLin\n");
    const dialog = await vi.waitFor(() => screen.getByRole("dialog", { name: "Paste 2 rows" }));
    const preview = within(dialog).getByRole("table");
    expect(within(preview).getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["Score", "Name"]);
    expect(within(preview).getAllByRole("row")).toHaveLength(3);
    await fireEvent.click(within(dialog).getByLabelText(/Replace from row/));
    expect(within(dialog).getByRole("status")).toHaveTextContent("replaces 2 rows");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Apply" }));
    await tick();
    expect(col(0)).toEqual(["Ada", "Kate", "Lin"]);
    expect(col(1)).toEqual(["3", "7", "8"]);
    expect(onchange).toHaveBeenCalledTimes(1);
    await key("z", { ctrlKey: true });
    expect(col(0)).toEqual(["Ada", "Grace", "Alan"]);
  });

  it("a paste past the row limit can't be applied and says why", async () => {
    mount({ limits: { rows: 4, bytes: null, cell: null } });
    grid().focus();
    pasteText("a\t1\nb\t2\nc\t3\n");
    const dialog = await vi.waitFor(() => screen.getByRole("dialog"));
    expect(within(dialog).getByRole("status")).toHaveTextContent("Up to 4 rows: this would make 6 (2 rows too many).");
    expect(within(dialog).getByRole("button", { name: "Apply" })).toBeDisabled();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("sheet editor: image cells and host toolbar", () => {
  it("Enter on an image cell opens the asset picker; the host toolbar gets the active row", async () => {
    picker.pickAsset.mockResolvedValue("/assets/logo.png");
    const { onchange } = mount({
      columns: [{ key: "logo", label: "Logo", kind: "image" }],
      rows: [{ logo: "" }],
      currentRow: 0,
    });
    expect(cell(0, 0)).toHaveTextContent("No image");
    expect(document.querySelector('[role="row"].row')!.classList.contains("current")).toBe(true);
    grid().focus();
    await key("Enter");
    await vi.waitFor(() => expect(lastRows(onchange)?.[0].logo).toBe("/assets/logo.png"));
    expect(picker.pickAsset).toHaveBeenCalledWith({ current: "", label: "Logo, row 1" });
  });
});
