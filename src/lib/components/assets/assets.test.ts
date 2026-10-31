import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import "../templates/list/matchers";
import type { AssetInfo, FieldDef, ServerStatus } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (docs/TESTING.md).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/fields/ImageChoose.svelte"),
  ]);
});

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<(p: unknown) => void>>() }));

const api = vi.hoisted(() => ({
  listAssets: vi.fn(),
  importAssets: vi.fn(),
  deleteAsset: vi.fn(),
  renameAsset: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
vi.mock("$lib/api/assets", () => api);
vi.mock("$lib/api/live", () => api);
const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

import FieldInputs from "$lib/components/fields/FieldInputs.svelte";
import { server } from "$lib/stores/server.svelte";
import { toasts } from "$lib/stores/toasts.svelte";
import { assetName, filterAssets, formatBytes, isWebImage, renameIssue, thumbUrl, webThumbUrl } from "./assets";
import { pickAsset } from "./pick";

// ---- Fixtures -----------------------------------------------------------------------------------

const running: ServerStatus = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null };

const asset = (name: string, bytes = 2048, usedBy: string[] = []): AssetInfo => ({
  name,
  url: `/assets/${encodeURIComponent(name)}`,
  bytes,
  modifiedAt: "2026-10-04T10:00:00Z",
  usedBy,
});

let files: AssetInfo[];

function emit(event: string, payload: unknown) {
  for (const h of events.handlers.get(event) ?? []) h(payload);
}

const tiles = () => screen.queryAllByRole("option");
const tile = (name: string) => tiles().find((t) => t.dataset.asset === name)!;

async function settle() {
  for (let i = 0; i < 5; i++) {
    await tick();
    await Promise.resolve();
  }
}

/** Opens the picker and waits for its list. */
async function open(current = ""): Promise<{ result: Promise<string | null> }> {
  const result = pickAsset({ current, label: "Logo" });
  // The picker is a lazy chunk: its first (cold) load can take over a second under a full run.
  await vi.waitFor(() => expect(screen.getByRole("listbox", { name: "Images" })).toBeTruthy(), { timeout: 5_000 });
  return { result };
}

beforeEach(() => {
  vi.clearAllMocks();
  events.handlers.clear();
  toasts.clear();
  server.status = null;
  files = [asset("sponsor-b.png", 1536 * 1024, ["s1", "g1"]), asset("logo.svg", 900), asset("Árvore.jpg", 40 * 1024)];
  api.on.mockImplementation((event: string, h: Handler) => {
    let set = events.handlers.get(event);
    if (!set) events.handlers.set(event, (set = new Set()));
    set.add(h);
    return () => set.delete(h);
  });
  api.serverStatus.mockResolvedValue(running);
  api.listAssets.mockImplementation(async () => [...files]);
  api.importAssets.mockImplementation(async (paths: string[]) => {
    const added = paths.map((p) => asset(p.split(/[\\/]/).pop()!));
    files.push(...added);
    return added;
  });
  api.deleteAsset.mockImplementation(async (name: string) => {
    files = files.filter((f) => f.name !== name);
  });
  api.renameAsset.mockImplementation(async (name: string, newName: string) => {
    const a = { ...files.find((f) => f.name === name)!, name: newName, url: `/assets/${newName}` };
    files = files.map((f) => (f.name === name ? a : f));
    return a;
  });
});

afterEach(async () => {
  // Close a picker a test left open.
  const d = document.querySelector("dialog");
  if (d) {
    const cancel = [...d.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Cancel");
    cancel?.click();
    await settle();
  }
  cleanup();
});

// ---- Pure helpers -------------------------------------------------------------------------------

describe("asset helpers", () => {
  it("names, thumbnails, sizes, search and rename checks", () => {
    expect(assetName("/assets/%C3%81rvore.jpg")).toBe("Árvore.jpg");
    expect(assetName("https://x.test/a.png")).toBe("https://x.test/a.png");
    expect(thumbUrl(running, "/assets/a.png")).toBe("http://127.0.0.1:7770/assets/a.png");
    expect(thumbUrl({ state: "stopped" }, "/assets/a.png")).toBeNull();
    expect(thumbUrl(running, "https://x.test/a.png")).toBeNull();
    expect(formatBytes(900)).toBe("900 B");
    expect(formatBytes(40 * 1024)).toBe("40 KB");
    expect(formatBytes(1536 * 1024)).toBe("1.5 MB");
    expect(filterAssets(files, "O").map((a) => a.name)).toEqual(["Árvore.jpg", "logo.svg", "sponsor-b.png"]);
    expect(filterAssets(files, "spon").map((a) => a.name)).toEqual(["sponsor-b.png"]);
    expect(renameIssue("brand.svg", "logo.svg", ["logo.svg", "brand.svg"])).toBe("Another image has this name.");
    expect(renameIssue("brand.png", "logo.svg", [])).toBe("Keep the .svg extension.");
    expect(renameIssue("a/b.svg", "logo.svg", [])).toMatch(/Use letters/);
    expect(renameIssue("  ", "logo.svg", [])).toBe("Enter a name.");
    expect(renameIssue("Brand logo.svg", "logo.svg", ["logo.svg"])).toBeNull();
  });
});

// ---- Picker -------------------------------------------------------------------------------------

describe("asset picker", () => {
  it("lists images by name with mono sizes and thumbnails from the running server", async () => {
    const { result } = await open();
    await settle();
    expect(screen.getByRole("heading", { name: "Choose image for Logo" })).toBeTruthy();
    expect(tiles().map((t) => t.dataset.asset)).toEqual(["Árvore.jpg", "logo.svg", "sponsor-b.png"]);
    const t = tile("sponsor-b.png");
    expect(t.querySelector(".meta.mono")?.textContent).toBe("1.5 MB");
    const img = t.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("http://127.0.0.1:7770/assets/sponsor-b.png");
    expect(img.getAttribute("loading")).toBe("lazy");
    expect(img.getAttribute("decoding")).toBe("async");
    expect(document.querySelectorAll("[placeholder]")).toHaveLength(0);
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await result).toBeNull();
    await settle();
    expect(document.querySelector("dialog")).toBeNull();
  });

  it("without a running server only the names show", async () => {
    api.serverStatus.mockResolvedValue({ state: "stopped" });
    const { result } = await open();
    await settle();
    expect(tile("logo.svg").querySelector("img")).toBeNull();
    expect(tile("logo.svg")).toHaveTextContent("logo.svg");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await result;
  });

  it("the current value starts selected; Choose returns the selected image's URL", async () => {
    const { result } = await open("/assets/logo.svg");
    await settle();
    expect(tile("logo.svg")).toHaveAttribute("aria-selected", "true");
    await fireEvent.click(tile("sponsor-b.png"));
    expect(tile("sponsor-b.png")).toHaveAttribute("aria-selected", "true");
    await fireEvent.click(screen.getByRole("button", { name: "Choose" }));
    expect(await result).toBe("/assets/sponsor-b.png");
  });

  it("keyboard: arrows move the selection, Enter chooses", async () => {
    const { result } = await open();
    await settle();
    const grid = screen.getByRole("listbox", { name: "Images" });
    expect(document.activeElement).toBe(tile("Árvore.jpg"));
    await fireEvent.keyDown(grid, { key: "ArrowRight" });
    await settle();
    expect(tile("logo.svg")).toHaveAttribute("aria-selected", "true");
    expect(document.activeElement).toBe(tile("logo.svg"));
    await fireEvent.keyDown(grid, { key: "End" });
    await fireEvent.keyDown(grid, { key: "Enter" });
    expect(await result).toBe("/assets/sponsor-b.png");
  });

  it("double-click chooses; None returns an empty value", async () => {
    let { result } = await open();
    await settle();
    await fireEvent.dblClick(tile("logo.svg"));
    expect(await result).toBe("/assets/logo.svg");
    await settle();
    ({ result } = await open("/assets/logo.svg"));
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "None" }));
    expect(await result).toBe("");
  });

  it("search filters by name and says when nothing matches", async () => {
    const { result } = await open();
    await settle();
    const search = screen.getByRole("searchbox", { name: "Search images" });
    await fireEvent.input(search, { target: { value: "LOGO" } });
    expect(tiles().map((t) => t.dataset.asset)).toEqual(["logo.svg"]);
    await fireEvent.input(search, { target: { value: "zzz" } });
    expect(screen.getByText(/No image name contains “zzz”/)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await result;
  });

  it("Import images…: the file dialog, importAssets, then the imported image is selected", async () => {
    dialog.open.mockResolvedValue(["C:\\Media\\new-logo.png", "C:\\Media\\other.png"]);
    const { result } = await open();
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Import images…" }));
    await vi.waitFor(() => expect(api.importAssets).toHaveBeenCalled());
    expect(dialog.open).toHaveBeenCalledWith({
      multiple: true,
      filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "svg", "gif", "webp"] }],
    });
    expect(api.importAssets).toHaveBeenCalledWith(["C:\\Media\\new-logo.png", "C:\\Media\\other.png"]);
    await vi.waitFor(() => expect(tile("new-logo.png")).toHaveAttribute("aria-selected", "true"));
    expect(toasts.list.at(-1)?.message).toBe("Imported 2 images.");
    await fireEvent.click(screen.getByRole("button", { name: "Choose" }));
    expect(await result).toBe("/assets/new-logo.png");
  });

  it("a cancelled file dialog imports nothing; a failed import shows a toast", async () => {
    dialog.open.mockResolvedValueOnce(null);
    const { result } = await open();
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Import images…" }));
    await settle();
    expect(api.importAssets).not.toHaveBeenCalled();
    dialog.open.mockResolvedValueOnce("C:\\big.png");
    api.importAssets.mockRejectedValueOnce({ code: "invalid", message: "big.png is over 20 MB" });
    await fireEvent.click(screen.getByRole("button", { name: "Import images…" }));
    await vi.waitFor(() => expect(toasts.list.at(-1)?.message).toBe("Couldn't import: big.png is over 20 MB"));
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await result;
  });

  it("delete asks first and says how many screens and sources use the image", async () => {
    const { result } = await open("/assets/sponsor-b.png");
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await settle();
    const confirm = screen.getByRole("heading", { name: "Delete image?" }).closest("dialog")!;
    expect(confirm.textContent).toMatch(/used by 2 screens and sources/);
    expect(api.deleteAsset).not.toHaveBeenCalled();
    await fireEvent.click(within(confirm as HTMLElement).getByRole("button", { name: "Delete" }));
    await settle();
    expect(api.deleteAsset).toHaveBeenCalledWith("sponsor-b.png");
    expect(tiles().map((t) => t.dataset.asset)).toEqual(["Árvore.jpg", "logo.svg"]);
    expect(tile("logo.svg")).toHaveAttribute("aria-selected", "true");
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await result;
  });

  it("rename: inline, checked, saved with Enter; Escape cancels only the rename", async () => {
    const { result } = await open("/assets/logo.svg");
    await settle();
    await fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    await settle();
    const input = screen.getByRole("textbox", { name: "New name for logo.svg" }) as HTMLInputElement;
    expect(document.activeElement).toBe(input);
    await fireEvent.input(input, { target: { value: "logo.png" } });
    expect(screen.getByText("Keep the .svg extension.")).toBeTruthy();
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(api.renameAsset).not.toHaveBeenCalled();
    await fireEvent.keyDown(input, { key: "Escape" });
    await settle();
    expect(screen.queryByRole("textbox", { name: /New name/ })).toBeNull();
    expect(document.querySelector("dialog")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    await settle();
    const again = screen.getByRole("textbox", { name: "New name for logo.svg" });
    await fireEvent.input(again, { target: { value: "brand.svg" } });
    await fireEvent.keyDown(again, { key: "Enter" });
    await settle();
    expect(api.renameAsset).toHaveBeenCalledWith("logo.svg", "brand.svg");
    expect(tile("brand.svg")).toHaveAttribute("aria-selected", "true");
    await fireEvent.click(screen.getByRole("button", { name: "Choose" }));
    expect(await result).toBe("/assets/brand.svg");
  });

  it("refreshes when an image changes on disk", async () => {
    const { result } = await open();
    await settle();
    files.push(asset("dropped-in.gif"));
    emit("project-files-changed", { kind: "source", id: "x", change: "changed", message: null });
    await settle();
    expect(tiles()).toHaveLength(3);
    emit("project-files-changed", { kind: "asset", id: "dropped-in.gif", change: "changed", message: null });
    await vi.waitFor(() => expect(tiles()).toHaveLength(4));
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await result;
  });

  it("an empty project offers the import", async () => {
    files = [];
    const result = pickAsset();
    await vi.waitFor(() => expect(screen.getByText(/No images yet/)).toBeTruthy());
    expect(screen.getAllByRole("button", { name: "Import images…" })).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Choose" })).toBeDisabled();
    await fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await result).toBeNull();
  });
});

// ---- Image fields -------------------------------------------------------------------------------

describe("image fields", () => {
  const defs: FieldDef[] = [{ key: "logo", label: "Logo", type: "image", default: "" }];

  it("show a thumbnail, Choose… (the picker) and Clear; the URL input stays", async () => {
    const onfield = vi.fn();
    render(FieldInputs, { defs, values: { logo: "/assets/logo.svg" }, onfield });
    const input = screen.getByLabelText("Logo") as HTMLInputElement;
    expect(input.value).toBe("/assets/logo.svg");
    expect(input.className).toContain("mono");
    const choose = await screen.findByRole("button", { name: "Choose image for Logo" });
    await vi.waitFor(() =>
      expect(document.querySelector("img.thumb")?.getAttribute("src")).toBe("http://127.0.0.1:7770/assets/logo.svg"),
    );
    // A typed URL still works.
    await fireEvent.input(input, { target: { value: "https://cdn.test/a.png" } });
    expect(onfield).toHaveBeenLastCalledWith("logo", "https://cdn.test/a.png");

    await fireEvent.click(choose);
    await vi.waitFor(() => expect(screen.getByRole("listbox", { name: "Images" })).toBeTruthy());
    await settle();
    expect(tile("logo.svg")).toHaveAttribute("aria-selected", "true");
    await fireEvent.click(tile("sponsor-b.png"));
    await fireEvent.click(screen.getByRole("button", { name: "Choose" }));
    await vi.waitFor(() => expect(onfield).toHaveBeenLastCalledWith("logo", "/assets/sponsor-b.png"));

    await fireEvent.click(screen.getByRole("button", { name: "Clear Logo" }));
    expect(onfield).toHaveBeenLastCalledWith("logo", "");
  });

  it("an empty value has no thumbnail and no Clear; a bound image shows its binding", async () => {
    render(FieldInputs, { defs, values: { logo: "" }, onfield: vi.fn() });
    await screen.findByRole("button", { name: "Choose image for Logo" });
    expect(document.querySelector("img.thumb")).toBeNull();
    expect(screen.queryByRole("button", { name: "Clear Logo" })).toBeNull();
    cleanup();
    render(FieldInputs, { defs, values: { logo: "/assets/x.png" }, bound: { logo: "Sponsors · Logo" }, onfield: vi.fn() });
    await settle();
    expect(screen.queryByRole("button", { name: "Choose image for Logo" })).toBeNull();
    expect(screen.getByText("Sponsors · Logo")).toBeTruthy();
  });

  it("a web image shows Kinetra's copy, and says so when it can't be downloaded", async () => {
    vi.useFakeTimers({ toFake: ["setTimeout"] });
    try {
      render(FieldInputs, { defs, values: { logo: "https://cdn.test/a.png?w=2&h=3" }, onfield: vi.fn() });
      await screen.findByRole("button", { name: "Choose image for Logo" });
      const thumb = () => document.querySelector("img.thumb");
      await vi.waitFor(() =>
        expect(thumb()?.getAttribute("src")).toBe("http://127.0.0.1:7770/web-image?u=https%3A%2F%2Fcdn.test%2Fa.png%3Fw%3D2%26h%3D3"),
      );
      expect(screen.queryByRole("note")).toBeNull();
      // The layer may get the link a moment after the field: two more tries, then the note.
      for (const n of [1, 2]) {
        await fireEvent.error(thumb()!);
        await vi.advanceTimersByTimeAsync(1500);
        expect(thumb()?.getAttribute("src")).toContain(`&try=${n}`);
      }
      await fireEvent.error(thumb()!);
      expect(screen.getByRole("note").textContent).toBe("Couldn't download this image.");
      expect(thumb()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("links that never show on air still say so", async () => {
    render(FieldInputs, { defs, values: { logo: "//cdn.test/a.png" }, onfield: vi.fn() });
    await screen.findByRole("button", { name: "Choose image for Logo" });
    expect(screen.getByRole("note").textContent?.trim()).toBe("Start the link with https:// to show it on air.");
    expect(isWebImage("https://cdn.test/a.png")).toBe(true);
    expect(isWebImage("file:///C:/a.png")).toBe(false);
    expect(webThumbUrl({ state: "stopped" }, "https://cdn.test/a.png")).toBeNull();
  });
});

