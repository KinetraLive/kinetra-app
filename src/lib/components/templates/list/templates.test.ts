import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "./matchers";
import type { PresetInfo, ServerStatus, TemplateDoc, TemplateSummary } from "$lib/types";
import Page from "../../../../routes/(app)/templates/+page.svelte";
import { countsLabel, moveIndex, originLabel, sortTemplates, validName } from "./format";

// ---- Mocks --------------------------------------------------------------------------------------

const api = vi.hoisted(() => ({
  serverStatus: vi.fn(),
  on: vi.fn(),
  listTemplates: vi.fn(),
  getTemplate: vi.fn(),
  createTemplate: vi.fn(),
  renameTemplate: vi.fn(),
  duplicateTemplate: vi.fn(),
  deleteTemplate: vi.fn(),
  listPresets: vi.fn(),
  listStyles: vi.fn(() => Promise.resolve([])),
  setDefaultTemplateVariant: vi.fn(),
  previewTemplate: vi.fn(),
  previewShow: vi.fn(),
  previewHide: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  ...api,
  toAppError: (e: unknown) =>
    e && typeof e === "object" && "code" in e
      ? e
      : { code: "internal", message: e instanceof Error ? e.message : String(e) },
}));
// Domain modules share the same mock functions (the split only changes where they are imported from).
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/live", () => api);
// The lazy "Generate template" lock asks for the license view; it never answers here.
vi.mock("$lib/api/license", () => ({ licenseView: () => new Promise(() => {}) }));

const nav = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => ({ goto: nav.goto, afterNavigate: vi.fn() }));
vi.mock("$app/state", () => ({ page: { url: new URL("http://localhost/templates") } }));

// Every preview box is "in the viewport" as soon as it is observed.
class FakeIO {
  #cb: IntersectionObserverCallback;
  constructor(cb: IntersectionObserverCallback) {
    this.#cb = cb;
  }
  observe(target: Element) {
    this.#cb([{ target, isIntersecting: true } as unknown as IntersectionObserverEntry], this as never);
  }
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
class FakeRO {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal("IntersectionObserver", FakeIO);
vi.stubGlobal("ResizeObserver", FakeRO);

// ---- Fixtures -----------------------------------------------------------------------------------

const running: ServerStatus = {
  state: "running",
  port: 7770,
  url: "http://127.0.0.1:7770",
  lanUrl: null,
};

const presets: PresetInfo[] = [
  { id: "classic-bar", name: "Classic bar", description: "A solid bar.", version: 1 },
];

function summary(id: string, name: string, extra: Partial<TemplateSummary> = {}): TemplateSummary {
  return {
    id,
    name,
    description: "",
    preset: null,
    fieldCount: 2,
    variableCount: 1,
    variantCount: 1,
    updatedAt: "2026-10-01T10:00:00Z",
    presetUpdate: null,
    ...extra,
  };
}

function doc(id: string, name: string): TemplateDoc {
  return {
    id,
    name,
    description: "",
    preset: null,
    fields: [
      { key: "name", label: "Name", type: "text", default: "" },
      { key: "title", label: "Title", type: "multiline", default: "" },
    ],
    variables: [],
    autoOutMs: null,
    updatedAt: "2026-10-01T10:00:00Z",
    defaultVariant: "default",
    variants: [{ id: "default", name: "Default", vars: {}, html: "<div></div>", css: "" }],
  };
}

const alpha = summary("a1", "Alpha", {
  preset: { id: "classic-bar", version: 1 },
  description: "Lower third for guests",
  variantCount: 3,
});
const beta = summary("b2", "Beta", { fieldCount: 1, variableCount: 0 });

beforeEach(() => {
  api.serverStatus.mockResolvedValue(running);
  api.on.mockReturnValue(() => {});
  api.listTemplates.mockResolvedValue([beta, alpha]);
  api.getTemplate.mockImplementation(async (id: string) => doc(id, id));
  api.listPresets.mockResolvedValue(presets);
  api.previewTemplate.mockResolvedValue({});
  api.previewShow.mockResolvedValue({});
  api.previewHide.mockResolvedValue({});
  api.renameTemplate.mockResolvedValue(alpha);
  api.duplicateTemplate.mockResolvedValue(doc("c3", "Alpha copy"));
  api.deleteTemplate.mockResolvedValue(undefined);
  api.createTemplate.mockResolvedValue(doc("n1", "New"));
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const option = (name: string) => screen.getByRole("option", { name: new RegExp(`^${name}`) });

/** The page remembers the last selection across mounts; start each test from Alpha. */
async function selectAlpha() {
  await waitFor(() => expect(screen.getAllByRole("option").length).toBeGreaterThan(0));
  await fireEvent.click(option("Alpha"));
}

// ---- Helpers ------------------------------------------------------------------------------------

describe("templates list format", () => {
  it("labels origin and counts", () => {
    expect(originLabel(null, presets)).toBe("Blank");
    expect(originLabel({ id: "classic-bar", version: 1 }, presets)).toBe("From Classic bar");
    expect(originLabel({ id: "gone", version: 1 }, presets)).toBe("From gone");
    expect(countsLabel({ fieldCount: 1, variableCount: 2 })).toBe("1 field · 2 variables");
  });

  it("sorts by name and validates names", () => {
    expect(sortTemplates([beta, alpha]).map((t) => t.id)).toEqual(["a1", "b2"]);
    expect(validName("  ")).toBe(false);
    expect(validName("x".repeat(100))).toBe(true);
    expect(validName("x".repeat(101))).toBe(false);
  });

  it("moves through a grid with the arrow keys, clamped", () => {
    expect(moveIndex("ArrowRight", 0, 5, 3)).toBe(1);
    expect(moveIndex("ArrowDown", 1, 5, 3)).toBe(4);
    expect(moveIndex("ArrowDown", 4, 5, 3)).toBe(4);
    expect(moveIndex("ArrowLeft", 0, 5, 3)).toBe(0);
    expect(moveIndex("End", 0, 5, 3)).toBe(4);
    expect(moveIndex("a", 0, 5, 3)).toBeNull();
  });
});

// ---- Page ---------------------------------------------------------------------------------------

describe("/templates", () => {
  it("renders a card per template, sorted, with origin, counts and the first one selected", async () => {
    render(Page);
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    const [first, second] = screen.getAllByRole("option");
    expect(first).toHaveTextContent("Alpha");
    expect(first).toHaveTextContent("From Classic bar");
    expect(first).toHaveTextContent("2 fields · 1 variable");
    expect(first).toHaveTextContent("2 fields · 1 variable · 3 variants");
    expect(within(first).getByText("2 fields · 1 variable").classList.contains("mono")).toBe(false);
    expect(second).toHaveTextContent("Blank");
    expect(second).not.toHaveTextContent("variant");
    expect(first).toHaveAttribute("aria-selected", "true");
    expect(api.listTemplates).toHaveBeenCalledTimes(1);
  });

  it("selecting a card previews it (debounced) and shows it; leaving hides it", async () => {
    const { unmount, container } = render(Page);
    await waitFor(() => expect(api.previewShow).toHaveBeenCalledTimes(1));
    expect(api.previewTemplate).toHaveBeenLastCalledWith("a1");

    await fireEvent.click(option("Beta"));
    expect(option("Beta")).toHaveAttribute("aria-selected", "true");
    expect(option("Alpha")).toHaveAttribute("aria-selected", "false");
    // Debounced: nothing yet.
    expect(api.previewTemplate).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(api.previewTemplate).toHaveBeenLastCalledWith("b2"));
    await waitFor(() => expect(api.previewShow).toHaveBeenCalledTimes(2));

    // One preview iframe, of the backend preview screen.
    const frames = container.querySelectorAll("iframe");
    expect(frames).toHaveLength(1);
    expect(frames[0].getAttribute("src")).toBe("http://127.0.0.1:7770/screen/__preview?mute=1");

    // Pane details for the selection: fields (label + type).
    const pane = screen.getByRole("complementary", { name: "Selected template" });
    await waitFor(() => expect(within(pane).getByText("multiline")).toBeInTheDocument());

    unmount();
    expect(api.previewHide).toHaveBeenCalledTimes(1);
  });

  it("debounces quick selection changes into one preview", async () => {
    render(Page);
    await waitFor(() => expect(api.previewShow).toHaveBeenCalledTimes(1));
    await fireEvent.click(option("Beta"));
    await fireEvent.click(option("Alpha"));
    await fireEvent.click(option("Beta"));
    await waitFor(() => expect(api.previewShow).toHaveBeenCalledTimes(2));
    expect(api.previewTemplate).toHaveBeenCalledTimes(2);
    expect(api.previewTemplate).toHaveBeenLastCalledWith("b2");
  });

  it("arrow keys move the selection; double-click opens the editor", async () => {
    render(Page);
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    await fireEvent.keyDown(option("Alpha"), { key: "ArrowRight" });
    expect(option("Beta")).toHaveAttribute("aria-selected", "true");
    await fireEvent.dblClick(option("Beta"));
    expect(nav.goto).toHaveBeenCalledWith("/templates/b2");
  });

  it("the empty state opens the gallery", async () => {
    api.listTemplates.mockResolvedValue([]);
    render(Page);
    const tile = await screen.findByRole("button", { name: /Create your first template/ });
    expect(screen.queryByRole("complementary")).toBeNull();
    await fireEvent.click(tile);
    // The gallery is a lazy chunk: under a full parallel run its first load can take over 1 s.
    expect(await screen.findByRole("heading", { name: "New template" }, { timeout: 5000 })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("option", { name: /^Classic bar/ })).toBeInTheDocument(), { timeout: 5000 });
  });

  it("New template opens the gallery", async () => {
    render(Page);
    await fireEvent.click(screen.getByRole("button", { name: "New template" }));
    expect(await screen.findByRole("dialog", {}, { timeout: 5000 })).toBeInTheDocument();
  });

  it("Delete asks for confirmation; deleteTemplate runs only after confirm", async () => {
    render(Page);
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(2));
    await fireEvent.keyDown(option("Alpha"), { key: "Delete" });
    const dlg = await screen.findByRole("dialog");
    expect(within(dlg).getByText("Delete template?")).toBeInTheDocument();
    expect(api.deleteTemplate).not.toHaveBeenCalled();

    // Cancel: nothing deleted.
    await fireEvent.click(within(dlg).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.deleteTemplate).not.toHaveBeenCalled();

    // Pane Delete → confirm.
    api.listTemplates.mockResolvedValue([beta]);
    const pane = screen.getByRole("complementary", { name: "Selected template" });
    await fireEvent.click(within(pane).getByRole("button", { name: "Delete" }));
    const dlg2 = await screen.findByRole("dialog");
    await fireEvent.click(within(dlg2).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(api.deleteTemplate).toHaveBeenCalledWith("a1"));
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(1));
    expect(api.listTemplates).toHaveBeenCalledTimes(2);
    expect(option("Beta")).toHaveAttribute("aria-selected", "true");
  });

  it("rename saves the new name and refreshes", async () => {
    render(Page);
    await selectAlpha();
    const pane = await screen.findByRole("complementary", { name: "Selected template" });
    await fireEvent.click(within(pane).getByRole("button", { name: "Rename" }));
    const input = within(pane).getByLabelText("Template name");
    expect(input).not.toHaveAttribute("placeholder");
    await fireEvent.input(input, { target: { value: "Guests" } });
    await fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(api.renameTemplate).toHaveBeenCalledWith("a1", "Guests"));
    await waitFor(() => expect(api.listTemplates).toHaveBeenCalledTimes(2));
  });

  it("Escape cancels a rename without saving", async () => {
    render(Page);
    await selectAlpha();
    const pane = await screen.findByRole("complementary", { name: "Selected template" });
    await fireEvent.click(within(pane).getByRole("button", { name: "Rename" }));
    const input = within(pane).getByLabelText("Template name");
    await fireEvent.input(input, { target: { value: "Nope" } });
    await fireEvent.keyDown(input, { key: "Escape" });
    await fireEvent.blur(input);
    expect(within(pane).queryByLabelText("Template name")).toBeNull();
    expect(api.renameTemplate).not.toHaveBeenCalled();
  });

  it("duplicate calls the API, refreshes and selects the copy", async () => {
    api.listTemplates
      .mockResolvedValueOnce([beta, alpha])
      .mockResolvedValueOnce([beta, alpha, summary("c3", "Alpha copy")]);
    render(Page);
    await selectAlpha();
    const pane = await screen.findByRole("complementary", { name: "Selected template" });
    await fireEvent.click(within(pane).getByRole("button", { name: "Duplicate" }));
    await waitFor(() => expect(api.duplicateTemplate).toHaveBeenCalledWith("a1", "Alpha copy"));
    await waitFor(() => expect(screen.getAllByRole("option")).toHaveLength(3));
    expect(option("Alpha copy")).toHaveAttribute("aria-selected", "true");
  });

  it("an API error becomes a toast", async () => {
    const { toasts } = await import("$lib/stores/toasts.svelte");
    toasts.clear();
    api.renameTemplate.mockRejectedValue({ code: "invalid", message: "Name taken" });
    render(Page);
    await selectAlpha();
    const pane = await screen.findByRole("complementary", { name: "Selected template" });
    await fireEvent.click(within(pane).getByRole("button", { name: "Rename" }));
    const input = within(pane).getByLabelText("Template name");
    await fireEvent.input(input, { target: { value: "Beta" } });
    await fireEvent.submit(input.closest("form")!);
    await waitFor(() => expect(toasts.list.map((t) => t.message).join()).toContain("Name taken"));
    toasts.clear();
  });
});
