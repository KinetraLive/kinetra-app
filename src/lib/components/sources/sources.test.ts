import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import "../templates/list/matchers";
import type { FilesChanged, SourceDoc, SourceSummary } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/sources/SourceEditor.svelte"),
    import("$lib/components/sources/schedule/ScheduleEditor.svelte"),
    import("$lib/components/sources/new/NewSourceDialog.svelte"),
    import("$lib/components/sheet/SheetEditor.svelte"),
    import("$lib/components/ConfirmDialog.svelte"),
  ]);
});

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));

const api = vi.hoisted(() => ({
  listSources: vi.fn(),
  getSource: vi.fn(),
  createSource: vi.fn(),
  saveSource: vi.fn(),
  renameSource: vi.fn(),
  duplicateSource: vi.fn(),
  deleteSource: vi.fn(),
  stepSource: vi.fn(),
  listScreenDocs: vi.fn(),
  createSchedule: vi.fn(),
  getSchedule: vi.fn(),
  saveSchedule: vi.fn(),
  scheduleView: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
}));
const picker = vi.hoisted(() => ({ pickAsset: vi.fn() }));
vi.mock("$lib/components/assets/pick", () => picker);
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/recipes", () => api);

import Page from "../../../routes/(app)/sources/+page.svelte";
import NewSourceDialog from "./new/NewSourceDialog.svelte";
import SourceEditor from "./SourceEditor.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { server } from "$lib/stores/server.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

function guests(over: Partial<Extract<SourceDoc, { kind: "text" }>> = {}): SourceDoc {
  return {
    id: "g1",
    name: "Guests",
    kind: "text",
    columns: [
      { key: "name", label: "Name" },
      { key: "title", label: "Title" },
    ],
    rows: [
      { name: "Grace Hopper", title: "Rear Admiral" },
      { name: "Ada Lovelace", title: "Analyst" },
      { name: "Alan Turing", title: "Mathematician" },
    ],
    index: 0,
    updatedAt: "2026-10-01T10:00:00Z",
    ...over,
  };
}

function summary(doc: SourceDoc, usedBy: string[] = []): SourceSummary {
  return {
    id: doc.id,
    name: doc.name,
    kind: "text",
    rowCount: doc.rows.length,
    index: doc.index,
    current: doc.rows[doc.index] ?? {},
    columns: doc.columns,
    usedBy,
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
    updatedAt: doc.updatedAt,
  };
}

const agenda: SourceDoc = {
  ...guests({ id: "a2", name: "Agenda", columns: [{ key: "item", label: "Item" }], rows: [{ item: "Intro" }] }),
};

let docs: Map<string, SourceDoc>;

const schedDoc = () => ({
  id: "sc5",
  name: "Run of show",
  kind: "schedule" as const,
  items: [],
  ripple: true,
  nextOverride: null,
  labels: { now: "NOW", next: "UP NEXT", delayed: "DELAYED", previous: "PREVIOUS", cancelled: "CANCELLED", upcoming: "LATER" },
  updatedAt: "2026-10-04T10:00:00Z",
});

function emit(event: string, payload: unknown) {
  for (const h of events.handlers.get(event) ?? []) h(payload);
}

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

const lastSaved = (): SourceDoc => api.saveSource.mock.calls.at(-1)?.[0];
const grid = () => screen.getByRole("grid");
const gridCell = (r: number, c: number) =>
  document.querySelector<HTMLElement>(`[role="gridcell"][data-r="${r}"][data-c="${c}"]`)!;
const cellValues = (col = 0) =>
  [...document.querySelectorAll<HTMLElement>(`[role="gridcell"][data-c="${col}"]`)].map((e) => e.textContent?.trim() ?? "");
const keys = () => [...document.querySelectorAll<HTMLInputElement>("input.key")].map((k) => k.value);
const rowHeader = (r: number) => document.querySelector<HTMLElement>(`[role="rowheader"][data-r="${r}"]`)!;
const editInput = () => document.querySelector<HTMLInputElement>("input.edit")!;
async function clickCell(r: number, c: number) {
  await fireEvent.pointerDown(gridCell(r, c));
  grid().focus();
}
async function startEdit(r: number, c: number) {
  await clickCell(r, c);
  await fireEvent.keyDown(grid(), { key: "F2" });
  await tick();
  return editInput();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.clear();
  docs = new Map([
    ["g1", guests()],
    ["a2", agenda],
  ]);
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.listSources.mockImplementation(async () =>
    [...docs.values()].map((d) => summary(d, d.id === "g1" ? ["m1", "v2"] : [])),
  );
  api.getSource.mockImplementation(async (id: string) => {
    const d = docs.get(id);
    if (!d) throw { code: "notFound", message: "Source not found" };
    return d;
  });
  api.saveSource.mockImplementation(async (d: SourceDoc) => ({ ...d, updatedAt: "2026-10-04T10:00:00Z" }));
  api.stepSource.mockImplementation(async (id: string, step: { to: "index"; index: number }) =>
    summary({ ...docs.get(id)!, index: step.index }, ["m1", "v2"]),
  );
  api.createSource.mockImplementation(async (name: string) => {
    const d = guests({ id: "n3", name, columns: [{ key: "text", label: "Text" }], rows: [] });
    docs.set("n3", d);
    return d;
  });
  api.duplicateSource.mockImplementation(async (_id: string, name: string) => {
    const d = guests({ id: "c4", name });
    docs.set("c4", d);
    return d;
  });
  api.deleteSource.mockImplementation(async (id: string) => {
    docs.delete(id);
  });
  server.status = null;
  api.serverStatus.mockResolvedValue({ state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null });
  api.scheduleView.mockResolvedValue({ sourceId: "", items: [], nowId: null, nextId: null, computedAt: 0, canUndo: false });
  picker.pickAsset.mockResolvedValue("/assets/sponsor-b.png");
  api.createSchedule.mockImplementation(async (name: string) => ({ ...schedDoc(), name }));
  api.listScreenDocs.mockResolvedValue([
    { id: "m1", name: "Main program" },
    { id: "v2", name: "Vertical" },
  ]);
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

async function mountEditor(over: Partial<{ usedBy: string[] }> = {}) {
  const props = {
    id: "g1",
    usedBy: over.usedBy ?? [],
    onsaved: vi.fn(),
    onduplicated: vi.fn(),
    ondeleted: vi.fn(),
  };
  const r = render(SourceEditor, props);
  await vi.waitFor(() => expect(gridCell(0, 0)).toBeTruthy());
  return { ...r, props };
}

// ---- Page ---------------------------------------------------------------------------------------

describe("sources page", () => {
  it("lists sources by name with kind, row count (body text) and usage; selects the first", async () => {
    render(Page);
    await vi.waitFor(() => expect(screen.getByRole("listbox", { name: "Sources" })).toBeTruthy());
    const options = screen.getAllByRole("option");
    expect(options.map((o) => o.querySelector(".name")?.textContent)).toEqual(["Agenda", "Guests"]);
    expect(options[1]).toHaveTextContent("Data table · 3 rows · Used by 2 screens");
    expect(options[0]).toHaveTextContent("Not used");
    expect(within(options[1]).getByText("3 rows").classList.contains("mono")).toBe(false);
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    await vi.waitFor(() => expect(api.getSource).toHaveBeenCalledWith("a2"));
  });

  it("arrow keys move the selection and open that source in the editor", async () => {
    render(Page);
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    await fireEvent.keyDown(screen.getAllByRole("option")[0], { key: "ArrowDown" });
    await tick();
    expect(screen.getAllByRole("option")[1]).toHaveAttribute("aria-selected", "true");
    expect(document.activeElement).toBe(screen.getAllByRole("option")[1]);
    await vi.waitFor(() => expect((screen.getByLabelText("Source name") as HTMLInputElement).value).toBe("Guests"));
    await fireEvent.keyDown(screen.getAllByRole("option")[1], { key: "Home" });
    await tick();
    expect(screen.getAllByRole("option")[0]).toHaveAttribute("aria-selected", "true");
  });

  it("empty: a bracket-framed Create your first source opens the dialog", async () => {
    docs.clear();
    render(Page);
    const create = await vi.waitFor(() => screen.getByRole("button", { name: /Create your first source/ }));
    expect(create).toHaveAttribute("data-selected");
    expect(create.className).toContain("vf");
    await fireEvent.click(create);
    await vi.waitFor(() => expect(screen.getByRole("heading", { name: "New source" })).toBeTruthy());
  });

  it("New source: creates with the default name and selects the new source", async () => {
    render(Page);
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    await fireEvent.click(screen.getByRole("button", { name: "New source" }));
    const name = await vi.waitFor(() => screen.getByLabelText("Name") as HTMLInputElement);
    expect(name.value).toBe("Source 3");
    await fireEvent.input(name, { target: { value: "Speakers" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await advance(0);
    expect(api.createSource).toHaveBeenCalledWith("Speakers");
    await vi.waitFor(() =>
      expect(screen.getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true")).toHaveTextContent(
        "Speakers",
      ),
    );
  });

  it("a source changed on disk refreshes the list", async () => {
    render(Page);
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    docs.set("z9", guests({ id: "z9", name: "Zebra" }));
    emit("project-files-changed", { kind: "source", id: "z9", change: "changed", message: null });
    await vi.waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(3));
  });

  it("no input has a placeholder", async () => {
    render(Page);
    await vi.waitFor(() => expect(gridCell(0, 0)).toBeTruthy());
    expect(document.querySelectorAll("[placeholder]")).toHaveLength(0);
  });
});

describe("new source dialog", () => {
  it("validates the name and shows a failed create inline", async () => {
    api.createSource.mockRejectedValueOnce({ code: "invalid", message: "Name taken" });
    const onclose = vi.fn();
    render(NewSourceDialog, { open: true, onclose, names: ["Source 1"] });
    const name = screen.getByLabelText("Name") as HTMLInputElement;
    await tick();
    expect(name.value).toBe("Source 2");
    await fireEvent.input(name, { target: { value: "  " } });
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();
    await fireEvent.input(name, { target: { value: "Guests" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await advance(0);
    expect(screen.getByRole("alert")).toHaveTextContent("Name taken");
    expect(onclose).not.toHaveBeenCalled();
  });

  it("selecting text and releasing on the backdrop keeps it open; a backdrop press closes it", async () => {
    const onclose = vi.fn();
    render(NewSourceDialog, { open: true, onclose, names: [] });
    const name = screen.getByLabelText("Name");
    const dialog = screen.getByRole("dialog");
    // Down in the Name field, dragged out, up on the backdrop: the click lands on the dialog.
    await fireEvent.pointerDown(name);
    await fireEvent.click(dialog);
    expect(onclose).not.toHaveBeenCalled();
    await fireEvent.pointerDown(dialog);
    await fireEvent.click(dialog);
    expect(onclose).toHaveBeenCalledTimes(1);
  });
});

// ---- Editor -------------------------------------------------------------------------------------

describe("source editor", () => {
  it("renders the data table in the sheet: editable column keys, cells, the current row marked", async () => {
    await mountEditor();
    expect(screen.getByText("Data table")).toBeTruthy();
    expect(keys()).toEqual(["name", "title"]);
    expect(document.querySelector("input.key")?.classList.contains("mono")).toBe(true);
    expect(cellValues(0)).toEqual(["Grace Hopper", "Ada Lovelace", "Alan Turing"]);
    const rows = document.querySelectorAll('[role="row"].row');
    expect(rows[0].classList.contains("current")).toBe(true);
    expect(rows[0]).toHaveTextContent("current row");
    expect(screen.getByText(/^Current row/)).toHaveTextContent("Current row 1");
    expect(screen.getByText("3 / 1000 rows")).toBeTruthy();
  });

  it("a burst of cell edits saves once, 500 ms after the last", async () => {
    const { props } = await mountEditor();
    const input = await startEdit(0, 0);
    for (const v of ["G", "Gr", "Gra"]) await fireEvent.input(input, { target: { value: v } });
    await advance(499);
    expect(api.saveSource).not.toHaveBeenCalled();
    await advance(1);
    expect(api.saveSource).toHaveBeenCalledTimes(1);
    expect(lastSaved().rows[0]).toEqual({ name: "Gra", title: "Rear Admiral" });
    expect(props.onsaved).toHaveBeenCalledWith(expect.objectContaining({ id: "g1", rowCount: 3 }));
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Saved");
  });

  it("columns: add (key follows its label), rename, edit a key, reorder, remove", async () => {
    await mountEditor();
    await fireEvent.click(screen.getByRole("button", { name: "Add column" }));
    await tick();
    expect(document.activeElement).toBe(screen.getByLabelText("Column 3 label"));
    await fireEvent.focus(screen.getByLabelText("Column 3 label"));
    await fireEvent.input(screen.getByLabelText("Column 3 label"), { target: { value: "Country code" } });
    expect(keys()).toEqual(["name", "title", "country_code"]);
    await fireEvent.click(screen.getAllByRole("button", { name: "Move column left" })[2]);
    expect(keys()).toEqual(["name", "country_code", "title"]);
    // A loaded column keeps its key when relabelled; the key can be edited on its own.
    await fireEvent.input(screen.getByLabelText("Column 1 label"), { target: { value: "Guest" } });
    expect(keys()[0]).toBe("name");
    const key = screen.getByLabelText("Column 1 key") as HTMLInputElement;
    await fireEvent.change(key, { target: { value: "title" } });
    expect(screen.getByText(/Another column has this key/)).toBeTruthy();
    expect(key.value).toBe("name");
    await fireEvent.change(key, { target: { value: "guest" } });
    await fireEvent.click(screen.getByRole("button", { name: "Remove column Title" }));
    await advance(500);
    expect(api.saveSource).toHaveBeenCalledTimes(1);
    expect(lastSaved().columns).toEqual([
      { key: "guest", label: "Guest" },
      { key: "country_code", label: "Country code" },
    ]);
    expect(lastSaved().rows[0]).toEqual({ guest: "Grace Hopper" });
  });

  it("keys loaded from disk are locked while screens use the source", async () => {
    await mountEditor({ usedBy: ["m1"] });
    const key = screen.getByLabelText("Column 1 key") as HTMLInputElement;
    expect(key.readOnly).toBe(true);
    expect(key.getAttribute("title")).toMatch(/screens use this column/);
    await fireEvent.input(screen.getByLabelText("Column 1 label"), { target: { value: "Guest" } });
    expect(keys()[0]).toBe("name");
  });

  it("validation errors are marked and block the save", async () => {
    await mountEditor();
    await fireEvent.input(screen.getByLabelText("Column 2 label"), { target: { value: "" } });
    expect(screen.getByLabelText("Column 2 label")).toHaveAttribute("aria-invalid", "true");
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Not saved: Column 2: Enter a label.");
    await fireEvent.input(screen.getByLabelText("Source name"), { target: { value: " " } });
    expect(screen.getAllByRole("status")[0]).toHaveTextContent("Not saved: Name: Enter a name.");
    await advance(2000);
    expect(api.saveSource).not.toHaveBeenCalled();
    await fireEvent.input(screen.getByLabelText("Source name"), { target: { value: "Guests" } });
    await fireEvent.input(screen.getByLabelText("Column 2 label"), { target: { value: "Role" } });
    await advance(500);
    expect(api.saveSource).toHaveBeenCalledTimes(1);
  });

  it("rows: add, Alt+↓ moves the current row (it follows), Delete removes the selection with Undo", async () => {
    await mountEditor();
    await fireEvent.click(screen.getByRole("button", { name: "Add row" }));
    await tick();
    expect(cellValues(0)).toHaveLength(4);
    await fireEvent.keyDown(grid(), { key: "L" });
    await fireEvent.input(editInput(), { target: { value: "Linus" } });
    await fireEvent.keyDown(editInput(), { key: "Escape" });
    expect(cellValues(0)[3]).toBe("");
    await fireEvent.keyDown(grid(), { key: "L" });
    await fireEvent.input(editInput(), { target: { value: "Linus" } });
    await fireEvent.keyDown(editInput(), { key: "Tab" });
    expect(cellValues(0)[3]).toBe("Linus");
    // Alt+ArrowDown on Grace's row (the current one) moves it down; the current row follows.
    await clickCell(0, 1);
    await fireEvent.keyDown(grid(), { key: "ArrowDown", altKey: true });
    expect(cellValues(0)).toEqual(["Ada Lovelace", "Grace Hopper", "Alan Turing", "Linus"]);
    expect(document.querySelectorAll('[role="row"].row')[1].classList.contains("current")).toBe(true);
    // Select rows 3–4 with row-number clicks and delete them.
    await fireEvent.pointerDown(rowHeader(2));
    await fireEvent.pointerDown(rowHeader(3), { shiftKey: true });
    expect(screen.getByText(/^\d+ selected$/)).toHaveTextContent("2 selected");
    await fireEvent.keyDown(grid(), { key: "Delete" });
    expect(cellValues(0)).toEqual(["Ada Lovelace", "Grace Hopper"]);
    const toast = toasts.list.at(-1)!;
    expect(toast.message).toBe("Deleted 2 rows.");
    await advance(500);
    expect(lastSaved().rows.map((r) => r.name)).toEqual(["Ada Lovelace", "Grace Hopper"]);
    expect(lastSaved().index).toBe(1);
    toast.action!.run();
    await tick();
    expect(cellValues(0)).toEqual(["Ada Lovelace", "Grace Hopper", "Alan Turing", "Linus"]);
  });

  it("Make current steps the source to the active row", async () => {
    const { props } = await mountEditor();
    await clickCell(2, 0);
    await fireEvent.click(screen.getByRole("button", { name: "Make row 3 current" }));
    await advance(0);
    expect(api.stepSource).toHaveBeenCalledWith("g1", { to: "index", index: 2 });
    expect(document.querySelectorAll('[role="row"].row')[2].classList.contains("current")).toBe(true);
    expect(props.onsaved).toHaveBeenCalledWith(expect.objectContaining({ index: 2 }));
    expect(screen.getByRole("button", { name: "Make row 3 current" })).toBeDisabled();
  });

  it("Make current rolls back when the step fails", async () => {
    api.stepSource.mockRejectedValueOnce({ code: "invalid", message: "No such row" });
    await mountEditor();
    await clickCell(1, 0);
    await fireEvent.click(screen.getByRole("button", { name: "Make row 2 current" }));
    await advance(0);
    expect(document.querySelectorAll('[role="row"].row')[0].classList.contains("current")).toBe(true);
    expect(toasts.list.at(-1)).toMatchObject({ kind: "error", message: "No such row" });
  });

  it("pasting rows opens the mapping preview; Apply appends them and adds the new column", async () => {
    await mountEditor();
    grid().focus();
    const e = new Event("paste", { bubbles: true, cancelable: true }) as ClipboardEvent;
    Object.defineProperty(e, "clipboardData", {
      value: { getData: () => "Name\tTitle\tCity\r\nLinus\tEngineer\tHelsinki\r\nMargaret\tDirector\tBoston\r\n" },
    });
    grid().dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    const dialog = await vi.waitFor(() => screen.getByRole("dialog", { name: "Paste 2 rows" }));
    expect((within(dialog).getByLabelText("First line names the columns") as HTMLInputElement).checked).toBe(true);
    expect((within(dialog).getByLabelText(/Target for pasted column 3/) as HTMLSelectElement).value).toBe("-2");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Apply" }));
    await tick();
    expect(cellValues(0)).toEqual(["Grace Hopper", "Ada Lovelace", "Alan Turing", "Linus", "Margaret"]);
    expect(cellValues(2)).toContain("Helsinki");
    await advance(500);
    expect(lastSaved().columns.map((c) => c.key)).toEqual(["name", "title", "city"]);
  });

  it("Delete asks first, naming the screens that use it", async () => {
    const { props } = await mountEditor({ usedBy: ["m1", "v2"] });
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await advance(0);
    const dialog = await vi.waitFor(() => screen.getByRole("dialog"));
    expect(dialog).toHaveTextContent("“Main program”, “Vertical”");
    expect(api.deleteSource).not.toHaveBeenCalled();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await advance(0);
    expect(api.deleteSource).toHaveBeenCalledWith("g1");
    expect(props.ondeleted).toHaveBeenCalled();
  });

  it("Duplicate saves pending edits first", async () => {
    const { props } = await mountEditor();
    await fireEvent.input(await startEdit(0, 0), { target: { value: "Grace" } });
    await fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));
    await advance(0);
    expect(api.saveSource).toHaveBeenCalledTimes(1);
    expect(api.duplicateSource).toHaveBeenCalledWith("g1", "Guests copy");
    expect(props.onduplicated).toHaveBeenCalledWith(expect.objectContaining({ id: "c4" }));
  });

  it("unmount flushes a pending save", async () => {
    const { unmount } = await mountEditor();
    await fireEvent.input(await startEdit(0, 0), { target: { value: "Grace" } });
    unmount();
    expect(api.saveSource).toHaveBeenCalledTimes(1);
  });
});

describe("source editor: changed on disk", () => {
  const changed = (over: Partial<FilesChanged> = {}): FilesChanged => ({
    kind: "source",
    id: "g1",
    change: "changed",
    message: null,
    ...over,
  });

  it("without unsaved edits: reloads silently (the sheet takes the new rows)", async () => {
    await mountEditor();
    docs.set("g1", guests({ rows: [{ name: "Only" }] }));
    emit("project-files-changed", changed());
    await advance(0);
    expect(cellValues(0)).toEqual(["Only"]);
    expect(toasts.list.at(-1)?.message).toMatch(/changed on disk and was reloaded/);
  });

  it("with unsaved edits: a banner; Keep mine saves over the file", async () => {
    await mountEditor();
    await fireEvent.input(await startEdit(0, 0), { target: { value: "Mine" } });
    emit("project-files-changed", changed());
    await tick();
    const banner = screen.getByRole("alert");
    expect(banner).toHaveTextContent("This source was edited outside Kinetra");
    await advance(2000);
    expect(api.saveSource).not.toHaveBeenCalled();
    await fireEvent.click(within(banner).getByRole("button", { name: "Keep mine" }));
    await advance(0);
    expect(lastSaved().rows[0].name).toBe("Mine");
  });

  it("removed: a banner whose Close leaves; nothing is saved", async () => {
    const { props } = await mountEditor();
    await fireEvent.input(await startEdit(0, 0), { target: { value: "Mine" } });
    emit("project-files-changed", changed({ change: "removed" }));
    await tick();
    await fireEvent.click(within(screen.getByRole("alert")).getByRole("button", { name: "Close" }));
    expect(props.ondeleted).toHaveBeenCalled();
    await advance(2000);
    expect(api.saveSource).not.toHaveBeenCalled();
  });

  it("other sources are ignored", async () => {
    await mountEditor();
    emit("project-files-changed", changed({ id: "a2" }));
    await advance(0);
    expect(api.getSource).toHaveBeenCalledTimes(1);
  });
});

describe("source editor: image columns", () => {
  it("a column toggles between Text and Image; image cells show a thumbnail and the file name", async () => {
    docs.set("g1", guests({ rows: [{ name: "/assets/logo%20one.png", title: "A" }, { name: "", title: "B" }, { title: "C" }] }));
    await mountEditor();
    const toggle = screen.getAllByRole("button", { name: "Image column" })[0];
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    await fireEvent.click(toggle);
    expect(screen.getAllByRole("button", { name: "Image column" })[0]).toHaveAttribute("aria-pressed", "true");
    const first = gridCell(0, 0);
    expect(first.querySelector(".text")?.textContent).toBe("logo one.png");
    await vi.waitFor(() =>
      expect(first.querySelector("img")?.getAttribute("src")).toBe("http://127.0.0.1:7770/assets/logo%20one.png"),
    );
    expect(first.querySelector("img")?.getAttribute("loading")).toBe("lazy");
    expect(gridCell(1, 0)).toHaveTextContent("No image");
    await advance(500);
    expect(lastSaved().columns[0]).toEqual({ key: "name", label: "Name", kind: "image" });
    await fireEvent.click(screen.getAllByRole("button", { name: "Image column" })[0]);
    await advance(500);
    expect(lastSaved().columns[0]).toEqual({ key: "name", label: "Name" });
  });

  it("Enter on an image cell opens the asset picker; the choice is saved, None clears it", async () => {
    docs.set(
      "g1",
      guests({
        columns: [
          { key: "logo", label: "Logo", kind: "image" },
          { key: "name", label: "Name" },
        ],
        rows: [{ logo: "/assets/old.png" }],
      }),
    );
    await mountEditor();
    await clickCell(0, 0);
    await fireEvent.keyDown(grid(), { key: "Enter" });
    await advance(0);
    expect(picker.pickAsset).toHaveBeenCalledWith({ current: "/assets/old.png", label: "Logo, row 1" });
    await advance(500);
    expect(lastSaved().rows[0].logo).toBe("/assets/sponsor-b.png");
    picker.pickAsset.mockResolvedValueOnce("");
    await fireEvent.dblClick(gridCell(0, 0));
    await advance(500);
    expect(lastSaved().rows[0].logo).toBe("");
    picker.pickAsset.mockResolvedValueOnce(null);
    const before = api.saveSource.mock.calls.length;
    await fireEvent.keyDown(grid(), { key: "Enter" });
    await advance(500);
    expect(api.saveSource.mock.calls.length).toBe(before);
  });
});

describe("sources page: schedules", () => {
  it("New source offers Data table, Schedule, CSV link, API or Live data; Schedule calls createSchedule", async () => {
    const onclose = vi.fn();
    const oncreated = vi.fn();
    render(NewSourceDialog, { open: true, onclose, oncreated, names: [] });
    await tick();
    const group = screen.getByRole("radiogroup", { name: "Kind" });
    const radios = within(group).getAllByRole("radio") as HTMLInputElement[];
    expect(radios.map((r) => r.value)).toEqual(["text", "schedule", "csv", "api", "socket"]);
    expect(radios[0].checked).toBe(true);
    await fireEvent.click(radios[1]);
    await fireEvent.input(screen.getByLabelText("Name"), { target: { value: "Day one" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create" }));
    await advance(0);
    expect(api.createSchedule).toHaveBeenCalledWith("Day one");
    expect(api.createSource).not.toHaveBeenCalled();
    expect(oncreated).toHaveBeenCalledWith(expect.objectContaining({ id: "sc5", kind: "schedule" }));
  });

  it("a schedule lists as Schedule with its item count and opens the schedule editor", async () => {
    api.listSources.mockImplementation(async () => [
      { ...summary(guests()), id: "sc5", name: "Run of show", kind: "schedule", rowCount: 2 },
    ]);
    api.getSchedule.mockResolvedValue(schedDoc());
    render(Page);
    const option = await vi.waitFor(() => screen.getByRole("option"));
    expect(option).toHaveTextContent("Schedule · 2 items · Not used");
    await vi.waitFor(() => expect(api.getSchedule).toHaveBeenCalledWith("sc5"));
    expect(await screen.findByRole("toolbar", { name: "Schedule" })).toBeTruthy();
    expect(api.getSource).not.toHaveBeenCalled();
  });
});
