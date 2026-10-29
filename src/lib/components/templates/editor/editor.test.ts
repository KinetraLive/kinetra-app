import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { LiveLayerView, ServerStatus, TemplateDoc, VariantDoc } from "$lib/types";

// ---- Mocks --------------------------------------------------------------------------------------

const api = vi.hoisted(() => ({
  getTemplate: vi.fn(),
  saveTemplate: vi.fn(),
  saveTemplateVariant: vi.fn(),
  createTemplateVariant: vi.fn(),
  renameTemplateVariant: vi.fn(),
  deleteTemplateVariant: vi.fn(),
  setDefaultTemplateVariant: vi.fn(),
  renameTemplate: vi.fn(),
  duplicateTemplate: vi.fn(),
  deleteTemplate: vi.fn(),
  previewTemplate: vi.fn(),
  previewDraft: vi.fn(),
  previewShow: vi.fn(),
  previewHide: vi.fn(),
  serverStatus: vi.fn(),
  on: vi.fn(),
  listTemplates: vi.fn(),
  setPrefs: vi.fn(async (p: unknown) => p),
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
// The lazy "Tweak with AI" lock asks for the license view; it never answers here.
vi.mock("$lib/api/license", () => ({ licenseView: () => new Promise(() => {}) }));

const nav = vi.hoisted(() => ({ goto: vi.fn() }));
vi.mock("$app/navigation", () => nav);
vi.mock("$app/state", () => ({
  page: { params: { id: "t1" }, url: new URL("http://localhost/templates/t1") },
}));

/** Fake CodeMirror: records each view; `set` simulates typing (new text + change callback). */
// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (CLAUDE.md › Testing policy).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/ConfirmDialog.svelte"),
    import("$lib/components/templates/UpdateNotice.svelte"),
    import("$lib/components/ai/AiEntry.svelte"),
    import("$lib/components/AdvancedHint.svelte"),
    import("$lib/components/templates/editor/FieldsForm.svelte"),
    import("$lib/components/templates/editor/VariablesForm.svelte"),
    import("$lib/components/templates/editor/VariantMenu.svelte"),
  ]);
});

interface FakeView {
  lang: string;
  value: string;
  label: string;
  destroyed: boolean;
  completions: () => ReadonlyArray<{ key: string; label: string }>;
  set(v: string): void;
}
const cm = vi.hoisted(() => ({ views: [] as FakeView[] }));
vi.mock("./codemirror", () => ({
  createEditor: (
    _parent: HTMLElement,
    o: {
      lang: string;
      doc: string;
      label: string;
      onChange: () => void;
      completions: () => ReadonlyArray<{ key: string; label: string }>;
    },
  ) => {
    const v: FakeView = {
      lang: o.lang,
      value: o.doc,
      label: o.label,
      destroyed: false,
      completions: o.completions,
      set(next) {
        this.value = next;
        o.onChange();
      },
    };
    cm.views.push(v);
    return {
      getValue: () => v.value,
      length: () => v.value.length,
      focus() {},
      destroy() {
        v.destroyed = true;
      },
    };
  },
}));

/** Thumbnails (the lazy renderer-compile chunk): recorded, not rendered. */
const thumbs = vi.hoisted(() => ({ renderThumb: vi.fn(), releaseThumb: vi.fn() }));
vi.mock("./thumbs", () => thumbs);

class FakeObserver {
  constructor(_cb: unknown) {}
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
}
vi.stubGlobal("IntersectionObserver", FakeObserver);
vi.stubGlobal("ResizeObserver", FakeObserver);

import { prefs } from "$lib/stores/prefs.svelte";
import TemplateEditor from "./TemplateEditor.svelte";
import { commands, titleOf } from "$lib/commands/registry.svelte";
import RoutePage from "../../../../routes/(app)/templates/[id]/+page.svelte";
import { server } from "$lib/stores/server.svelte";

// ---- Fixtures -----------------------------------------------------------------------------------

const running: ServerStatus = { state: "running", port: 7770, url: "http://127.0.0.1:7770", lanUrl: null };

function makeDoc(html = "<div>{{name}}</div>", more: VariantDoc[] = []): TemplateDoc {
  return {
    id: "t1",
    name: "Classic bar",
    description: "",
    preset: { id: "classic-bar", version: 1 },
    fields: [
      { key: "name", label: "Name", type: "text", default: "" },
      { key: "title", label: "Title", type: "text", default: "" },
    ],
    variables: [
      { key: "accent", label: "Accent", type: "color", default: "#22b8cf" },
      { key: "font", label: "Font", type: "font", default: "IBM Plex Sans" },
      { key: "radius", label: "Corner radius", type: "range", min: 0, max: 24, step: 1, unit: "px", default: "4" },
      {
        key: "align",
        label: "Position",
        type: "select",
        options: [
          { value: "left", label: "Left" },
          { value: "right", label: "Right" },
        ],
        default: "left",
      },
    ],
    autoOutMs: null,
    updatedAt: "2026-10-03T20:00:00Z",
    defaultVariant: "default",
    variants: [{ id: "default", name: "Default", vars: {}, html, css: ".bar { color: var(--accent); }" }, ...more],
  };
}

/** saveTemplateVariant calls as objects. */
function variantSaves(): { templateId: string; variantId: string; html: string; css: string; vars: Record<string, string> }[] {
  return api.saveTemplateVariant.mock.calls.map(([templateId, variantId, html, css, vars]) => ({
    templateId,
    variantId,
    html,
    css,
    vars,
  }));
}

const layer = (onAir: boolean): LiveLayerView => ({
  id: "template",
  name: "Classic bar",
  onAir,
  fields: {},
  fieldDefs: [],
  pending: false,
});

/** previewDraft calls whose draft has `key`. */
function draftsWith(key: string): Record<string, unknown>[] {
  return api.previewDraft.mock.calls
    .map((c) => c[0] as Record<string, unknown>)
    .filter((d) => key in d);
}

async function advance(ms: number) {
  await vi.advanceTimersByTimeAsync(ms);
  await tick();
}

async function mountEditor() {
  const r = render(TemplateEditor, { id: "t1" });
  await vi.waitFor(() => expect(cm.views.length).toBe(1));
  await vi.waitFor(() => expect(api.previewTemplate).toHaveBeenCalled());
  await tick();
  return r;
}

async function openTab(name: string) {
  const t = screen.getByRole("tab", { name: new RegExp(`^${name}`) });
  await fireEvent.click(t);
  await tick();
  // Fields and Variables load their form lazily on the first visit.
  const panel = document.getElementById(t.getAttribute("aria-controls") ?? "");
  if (panel && !panel.classList.contains("code"))
    await vi.waitFor(() => expect(panel.childElementCount).toBeGreaterThan(0));
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  cm.views.length = 0;
  server.status = null;
  api.getTemplate.mockImplementation(async () => makeDoc());
  api.saveTemplate.mockImplementation(async (doc: TemplateDoc) => ({
    ...doc,
    updatedAt: "2026-10-03T21:00:00Z",
  }));
  api.saveTemplateVariant.mockImplementation(async () => ({ ...makeDoc(), updatedAt: "2026-10-03T21:00:00Z" }));
  api.previewTemplate.mockResolvedValue(layer(false));
  api.previewDraft.mockResolvedValue(layer(false));
  api.previewShow.mockResolvedValue(layer(true));
  api.previewHide.mockResolvedValue(layer(false));
  api.serverStatus.mockResolvedValue(running);
  api.on.mockReturnValue(() => {});
  api.listTemplates.mockResolvedValue([]);
  api.renameTemplate.mockImplementation(async (id: string, name: string) => ({
    id,
    name,
    description: "",
    preset: null,
    fieldCount: 2,
    variableCount: 4,
    updatedAt: "2026-10-03T21:30:00Z",
  }));
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

// ---- Load / teardown ----------------------------------------------------------------------------

describe("load and teardown", () => {
  it("loads the template, previews the saved state and creates the HTML view", async () => {
    await mountEditor();
    expect(api.getTemplate).toHaveBeenCalledWith("t1");
    expect(api.previewTemplate).toHaveBeenCalledWith("t1");
    expect(cm.views[0]).toMatchObject({ lang: "html", value: "<div>{{name}}</div>" });
    expect((screen.getByLabelText("Template name") as HTMLInputElement).value).toBe("Classic bar");
    expect(screen.getByText("Saved")).toBeTruthy();
    // Preview text starts as each field's label.
    await vi.waitFor(() =>
      expect(draftsWith("fieldValues")[0]).toEqual({ fieldValues: { name: "[Name]", title: "[Title]" } }),
    );
    // HTML completions come from the field list.
    expect(cm.views[0].completions().map((c) => c.key)).toEqual(["name", "title"]);
  });

  it("the route page reads the id from the URL", async () => {
    render(RoutePage);
    await vi.waitFor(() => expect(api.getTemplate).toHaveBeenCalledWith("t1"));
  });

  it("unmount hides the preview, flushes a pending save and destroys the views", async () => {
    const { unmount } = await mountEditor();
    cm.views[0].set("<div>{{title}}</div>");
    await advance(100);
    expect(api.saveTemplate).not.toHaveBeenCalled();
    unmount();
    expect(api.previewHide).toHaveBeenCalledTimes(1);
    await advance(0);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    expect(variantSaves()[0]).toMatchObject({ templateId: "t1", variantId: "default", html: "<div>{{title}}</div>" });
    expect(api.saveTemplate).not.toHaveBeenCalled();
    expect(cm.views[0].destroyed).toBe(true);
    // Pending preview drafts are dropped, and no timers keep running.
    await advance(2000);
    expect(draftsWith("html")).toHaveLength(0);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
  });

  it("shows the error when the template can't be loaded", async () => {
    api.getTemplate.mockRejectedValueOnce({ code: "notFound", message: "Template not found" });
    render(TemplateEditor, { id: "t1" });
    await vi.waitFor(() => expect(screen.getByText(/Couldn't open this template/)).toBeTruthy());
    expect(api.previewTemplate).not.toHaveBeenCalled();
  });
});

// ---- Drafts and autosave ------------------------------------------------------------------------

describe("drafts and autosave", () => {
  it("CSS edits send one debounced draft and one save", async () => {
    await mountEditor();
    await openTab("CSS");
    await vi.waitFor(() => expect(cm.views.length).toBe(2));
    const css = cm.views[1];
    expect(css.lang).toBe("css");
    expect(css.completions().map((c) => c.key)).toEqual(["accent", "font", "radius", "align"]);

    css.set(".a{}");
    await advance(50);
    css.set(".ab{}");
    await advance(50);
    css.set(".abc{}");
    expect(screen.getByText("Saving…")).toBeTruthy();
    await advance(149);
    expect(draftsWith("css")).toHaveLength(0);
    await advance(1);
    expect(draftsWith("css")).toEqual([{ css: ".abc{}" }]);
    expect(api.saveTemplateVariant).not.toHaveBeenCalled();

    await advance(650);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    expect(variantSaves()[0]).toEqual({
      templateId: "t1",
      variantId: "default",
      css: ".abc{}",
      html: "<div>{{name}}</div>",
      vars: {},
    });
    // A look edit never rewrites the shared meta.
    expect(api.saveTemplate).not.toHaveBeenCalled();
    await advance(2000);
    expect(draftsWith("css")).toHaveLength(1);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Saved")).toBeTruthy();
  });

  it("HTML edits draft only the HTML", async () => {
    await mountEditor();
    cm.views[0].set("<p>{{name}}</p>");
    await advance(150);
    expect(draftsWith("html")).toEqual([{ html: "<p>{{name}}</p>" }]);
    expect(draftsWith("css")).toHaveLength(0);
    expect(draftsWith("fields")).toHaveLength(0);
  });

  it("keeps one save in flight and saves the latest doc afterwards", async () => {
    let release!: () => void;
    api.saveTemplateVariant.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = () => resolve({ ...makeDoc(), updatedAt: "2026-10-03T21:00:00Z" });
        }),
    );
    await mountEditor();
    cm.views[0].set("<p>1</p>");
    await advance(800);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    cm.views[0].set("<p>2</p>");
    await advance(800);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1); // still in flight
    release();
    await advance(0);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(2);
    expect(variantSaves()[1].html).toBe("<p>2</p>");
  });

  it("Ctrl+S and Cmd+S save immediately", async () => {
    await mountEditor();
    cm.views[0].set("<p>x</p>");
    await fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    await advance(0);
    expect(api.saveTemplate).toHaveBeenCalledTimes(1);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    await fireEvent.keyDown(window, { key: "s", metaKey: true });
    await advance(0);
    expect(api.saveTemplate).toHaveBeenCalledTimes(2);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(2);
  });

  it("a save carries the template's design box through unchanged", async () => {
    const design = { w: 640, h: 180, x: 96, y: 840, sizing: "fit" as const };
    api.getTemplate.mockImplementation(async () => ({ ...makeDoc(), design }));
    await mountEditor();
    await fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    await advance(0);
    expect(api.saveTemplate.mock.calls.at(-1)?.[0].design).toEqual(design);
  });

  it("a template without a design box saves without one", async () => {
    await mountEditor();
    await fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    await advance(0);
    expect(api.saveTemplate).toHaveBeenCalledTimes(1);
    expect("design" in api.saveTemplate.mock.calls[0][0]).toBe(false);
  });

  it("a failed save shows the reason and Retry saves again", async () => {
    api.saveTemplateVariant.mockRejectedValueOnce({ code: "io", message: "Disk full" });
    await mountEditor();
    cm.views[0].set("<p>x</p>");
    await advance(800);
    expect(screen.getByText("Not saved: Disk full")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await advance(0);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(2);
    expect(screen.getByText("Saved")).toBeTruthy();
  });

  it("a duplicate key blocks saving and shows why", async () => {
    await mountEditor();
    await openTab("Fields");
    const key2 = screen.getByLabelText("Field 2 key");
    await fireEvent.input(key2, { target: { value: "name" } });
    expect(screen.getByText("Key “name” is already used.")).toBeTruthy();
    expect(screen.getByText("Not saved: Fields: Key “name” is already used.")).toBeTruthy();
    expect(key2.getAttribute("aria-invalid")).toBe("true");
    await advance(2000);
    await fireEvent.keyDown(window, { key: "s", ctrlKey: true });
    await advance(0);
    expect(api.saveTemplate).not.toHaveBeenCalled();
    // Invalid fields never reach the preview either.
    expect(draftsWith("fields")).toHaveLength(0);

    // Fixing it saves.
    await fireEvent.input(key2, { target: { value: "subtitle" } });
    await advance(800);
    expect(api.saveTemplate).toHaveBeenCalledTimes(1);
    expect(api.saveTemplate.mock.calls[0][0].fields.map((f: { key: string }) => f.key)).toEqual([
      "name",
      "subtitle",
    ]);
  });

  it("a field key may not collide with a variable key", async () => {
    await mountEditor();
    await openTab("Fields");
    await fireEvent.input(screen.getByLabelText("Field 1 key"), { target: { value: "accent" } });
    // The later one (the variable) carries the message; the status explains it.
    expect(screen.getByText(/Not saved: Variables: Key “accent” is already used\./)).toBeTruthy();
  });
});

// ---- Variables ------------------------------------------------------------------------------------

describe("variables values form", () => {
  it("a colour change drafts variableValues and saves it as the variant's value", async () => {
    await mountEditor();
    await openTab("Variables");
    const picker = screen.getByLabelText("Accent color picker") as HTMLInputElement;
    expect(picker.value).toBe("#22b8cf");
    await fireEvent.input(picker, { target: { value: "#ff0000" } });
    expect((screen.getByLabelText("Accent") as HTMLInputElement).value).toBe("#ff0000");
    await advance(150);
    expect(draftsWith("variableValues")).toEqual([{ variableValues: { accent: "#ff0000" } }]);
    await advance(650);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    expect(variantSaves()[0].vars).toEqual({ accent: "#ff0000" });
    // The definition default is shared and unchanged.
    expect(api.saveTemplate).not.toHaveBeenCalled();
  });

  it("an invalid hex shows an error, isn't drafted and isn't saved", async () => {
    await mountEditor();
    await openTab("Variables");
    await fireEvent.input(screen.getByLabelText("Accent"), { target: { value: "#zzz" } });
    expect(screen.getByText("Enter a color like #1a1a1a.")).toBeTruthy();
    await advance(2000);
    expect(draftsWith("variableValues")).toHaveLength(0);
    expect(api.saveTemplate).not.toHaveBeenCalled();
    expect(api.saveTemplateVariant).not.toHaveBeenCalled();
  });

  it("slider, font and select controls draft only the changed values", async () => {
    await mountEditor();
    await openTab("Variables");
    await fireEvent.input(screen.getByLabelText("Corner radius slider"), { target: { value: "12" } });
    await fireEvent.change(screen.getByLabelText("Font"), { target: { value: "Oswald" } });
    await fireEvent.change(screen.getByLabelText("Position"), { target: { value: "right" } });
    await advance(150);
    expect(draftsWith("variableValues")).toEqual([
      { variableValues: { radius: "12", font: "Oswald", align: "right" } },
    ]);
    expect((screen.getByLabelText("Corner radius") as HTMLInputElement).value).toBe("12");
    await advance(650);
    expect(variantSaves()[0].vars).toEqual({ radius: "12", font: "Oswald", align: "right" });
  });

  it("definitions: add a variable, change its kind, delete it", async () => {
    await mountEditor();
    await openTab("Variables");
    const toggle = screen.getByRole("button", { name: /Edit variables/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    await fireEvent.click(screen.getByRole("button", { name: "Add variable" }));
    expect((screen.getByLabelText("Variable 5 key") as HTMLInputElement).value).toBe("var1");
    await fireEvent.change(screen.getByLabelText("Variable 5 kind"), { target: { value: "range" } });
    await advance(150);
    const sent = draftsWith("variables").at(-1)?.variables as { key: string; type: string }[];
    expect(sent.at(-1)).toMatchObject({ key: "var1", type: "range", min: 0, max: 100 });
    const deletes = screen.getAllByRole("button", { name: "Delete variable" });
    await fireEvent.click(deletes[deletes.length - 1]);
    await advance(800);
    expect(api.saveTemplate.mock.calls.at(-1)?.[0].variables).toHaveLength(4);
  });
});

// ---- Fields ---------------------------------------------------------------------------------------

describe("fields form", () => {
  const keys = () =>
    screen
      .getAllByLabelText(/^Field \d+ key$/)
      .map((el) => (el as HTMLInputElement).value);

  it("adds, moves and deletes fields", async () => {
    await mountEditor();
    await openTab("Fields");
    expect(keys()).toEqual(["name", "title"]);

    await fireEvent.click(screen.getByRole("button", { name: "Add field" }));
    expect(keys()).toEqual(["name", "title", "field1"]);

    const ups = screen.getAllByRole("button", { name: "Move up" });
    expect((ups[0] as HTMLButtonElement).disabled).toBe(true);
    await fireEvent.click(ups[2]);
    expect(keys()).toEqual(["name", "field1", "title"]);

    await fireEvent.click(screen.getAllByRole("button", { name: "Move down" })[0]);
    expect(keys()).toEqual(["field1", "name", "title"]);

    await fireEvent.click(screen.getAllByRole("button", { name: "Delete field" })[0]);
    expect(keys()).toEqual(["name", "title"]);

    await advance(150);
    expect(draftsWith("fields").at(-1)?.fields).toEqual(makeDoc().fields);
    await advance(650);
    expect(api.saveTemplate).toHaveBeenCalledTimes(1);
  });

  it("shows key validation messages", async () => {
    await mountEditor();
    await openTab("Fields");
    const key = screen.getByLabelText("Field 1 key");
    await fireEvent.input(key, { target: { value: "1bad" } });
    expect(screen.getByText("Use a–z, 0–9, - or _, starting with a letter (max 32).")).toBeTruthy();
    await fireEvent.input(key, { target: { value: "" } });
    expect(screen.getByText("Enter a key.")).toBeTruthy();
    await fireEvent.input(key, { target: { value: "Name" } });
    expect(screen.getByText("Use a–z, 0–9, - or _, starting with a letter (max 32).")).toBeTruthy();
    await fireEvent.input(key, { target: { value: "headline" } });
    expect(screen.queryByText(/Use a–z/)).toBeNull();
    expect(screen.getByText("Saving…")).toBeTruthy();
  });

  it("changing a field type updates the doc", async () => {
    await mountEditor();
    await openTab("Fields");
    await fireEvent.change(screen.getByLabelText("Field 2 type"), { target: { value: "multiline" } });
    await advance(800);
    expect(api.saveTemplate.mock.calls[0][0].fields[1].type).toBe("multiline");
  });
});

// ---- Preview --------------------------------------------------------------------------------------

describe("preview panel", () => {
  it("opens with the graphic shown (held, no auto-out); Play out / Play in toggle the tally", async () => {
    const { container } = await mountEditor();
    // Shown at once, so a picked image or typed value is visible without pressing Play in.
    await vi.waitFor(() => expect(api.previewShow).toHaveBeenCalledTimes(1));
    await advance(0);
    expect(draftsWith("autoOutMs")[0]).toEqual({ autoOutMs: null });
    expect(screen.getByText("On air")).toBeTruthy();
    expect(container.querySelector(".tally.program")?.classList.contains("pulse")).toBe(true);
    expect(container.querySelector(".preview[data-selected]")).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Play out" }));
    await advance(0);
    expect(api.previewHide).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Off air")).toBeTruthy();
    expect(container.querySelector(".tally.idle")).toBeTruthy();
    expect(container.querySelector(".preview[data-selected]")).toBeNull();

    const playIn = screen.getByRole("button", { name: "Play in" }) as HTMLButtonElement;
    expect(playIn.disabled).toBe(false);
    await fireEvent.click(playIn);
    await advance(0);
    expect(api.previewShow).toHaveBeenCalledTimes(2);
    expect(screen.getByText("On air")).toBeTruthy();
  });

  it("without a running server the preview shows the status and Play is disabled", async () => {
    api.serverStatus.mockResolvedValue({ state: "stopped" });
    await mountEditor();
    await vi.waitFor(() => expect(server.status).toEqual({ state: "stopped" }));
    await tick();
    expect(screen.getByText("The output server isn't running.")).toBeTruthy();
    expect((screen.getByRole("button", { name: "Play in" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "Play out" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("preview values start as placeholders and send fieldValues, never saved", async () => {
    await mountEditor();
    const aside = screen.getByRole("complementary", { name: "Preview" });
    const input = within(aside).getByLabelText("Name") as HTMLInputElement;
    expect(input.value).toBe("[Name]");
    api.previewDraft.mockClear();
    await fireEvent.input(input, { target: { value: "Ada" } });
    await advance(150);
    expect(draftsWith("fieldValues")).toEqual([{ fieldValues: { name: "Ada" } }]);
    await advance(2000);
    expect(api.saveTemplate).not.toHaveBeenCalled();
  });

  it("auto-out: the switch saves autoOutMs (null when off); only Play in arms it in the preview", async () => {
    await mountEditor();
    await vi.waitFor(() => expect(api.previewShow).toHaveBeenCalledTimes(1));
    const sw = screen.getByRole("switch", { name: "Play out automatically" });
    await fireEvent.click(sw);
    await advance(150);
    // The held preview keeps showing: the setting isn't sent to it...
    expect(draftsWith("autoOutMs")).toEqual([{ autoOutMs: null }]);
    // ...until Play in, which plays it with the real auto-out.
    await fireEvent.click(screen.getByRole("button", { name: "Play out" }));
    await advance(0);
    await fireEvent.click(screen.getByRole("button", { name: "Play in" }));
    await advance(0);
    expect(draftsWith("autoOutMs").at(-1)).toEqual({ autoOutMs: 5000 });
    const secs = screen.getByLabelText("Auto-out after seconds") as HTMLInputElement;
    secs.value = "12";
    await fireEvent.change(secs);
    await advance(800);
    expect(api.saveTemplate.mock.calls.at(-1)?.[0].autoOutMs).toBe(12000);
    await fireEvent.click(sw);
    await advance(800);
    expect(api.saveTemplate.mock.calls.at(-1)?.[0].autoOutMs).toBeNull();
  });
});

// ---- Header ---------------------------------------------------------------------------------------

describe("header", () => {
  it("renames on Enter and validates the length", async () => {
    await mountEditor();
    const input = screen.getByLabelText("Template name") as HTMLInputElement;
    await fireEvent.input(input, { target: { value: "   " } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByText("Enter a name.")).toBeTruthy();
    expect(api.renameTemplate).not.toHaveBeenCalled();
    await fireEvent.input(input, { target: { value: "x".repeat(101) } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(screen.getByText("Up to 100 characters.")).toBeTruthy();
    await fireEvent.input(input, { target: { value: "News strap" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await advance(0);
    expect(api.renameTemplate).toHaveBeenCalledWith("t1", "News strap");
    expect(screen.queryByText("Up to 100 characters.")).toBeNull();
  });

  it("delete asks first (lazy dialog), then deletes without saving and leaves", async () => {
    api.deleteTemplate.mockResolvedValue(undefined);
    await mountEditor();
    cm.views[0].set("<p>about to go</p>");
    await fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await vi.waitFor(() => expect(screen.getByText("Delete template?")).toBeTruthy());
    const dialog = screen.getByRole("dialog", { hidden: true });
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete", hidden: true }));
    await advance(0);
    expect(api.deleteTemplate).toHaveBeenCalledWith("t1");
    expect(nav.goto).toHaveBeenCalledWith("/templates");
    cleanup();
    expect(api.saveTemplate).not.toHaveBeenCalled();
  });

  it("duplicate saves first, then opens the copy", async () => {
    api.duplicateTemplate.mockResolvedValue({ ...makeDoc(), id: "t2", name: "Classic bar copy" });
    await mountEditor();
    cm.views[0].set("<p>v2</p>");
    await fireEvent.click(screen.getByRole("button", { name: "Duplicate" }));
    await advance(0);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    expect(api.duplicateTemplate).toHaveBeenCalledWith("t1", "Classic bar copy");
    expect(nav.goto).toHaveBeenCalledWith("/templates/t2");
  });
});

// ---- Tabs -----------------------------------------------------------------------------------------

describe("tabs", () => {
  it("is a tablist with arrow-key navigation", async () => {
    await mountEditor();
    const tabs = within(screen.getByRole("tablist", { name: "Template parts" })).getAllByRole("tab");
    expect(tabs.map((t) => t.textContent?.trim())).toEqual(["HTML", "CSS", "Fields", "Variables"]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    expect(tabs[0].classList.contains("vf")).toBe(false); // selected = accent + underline, no brackets
    await fireEvent.keyDown(tabs[0], { key: "ArrowRight" });
    expect(tabs[1].getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(tabs[1]);
    await fireEvent.keyDown(tabs[1], { key: "End" });
    expect(tabs[3].getAttribute("aria-selected")).toBe("true");
    await fireEvent.keyDown(tabs[3], { key: "ArrowRight" });
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    const panel = document.getElementById(tabs[0].getAttribute("aria-controls") ?? "");
    expect(panel?.getAttribute("role")).toBe("tabpanel");
    expect(panel?.hidden).toBe(false);
    // The CSS view was created on its first visit and is kept.
    await vi.waitFor(() => expect(cm.views.map((v) => v.lang)).toEqual(["html", "css"]));
  });
});

// ---- Simple mode -----------------------------------------------------------------------------------

describe("Simple mode", () => {
  afterEach(() => prefs.set(null));

  it("hides the HTML and CSS tabs behind the hint; Fields first; switching shows the code", async () => {
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" });
    render(TemplateEditor, { id: "t1" });
    await vi.waitFor(() => expect(api.previewTemplate).toHaveBeenCalled());
    await tick();
    const names = () => within(screen.getByRole("tablist", { name: "Template parts" })).getAllByRole("tab").map((t) => t.textContent?.trim());
    expect(names()).toEqual(["Fields", "Variables"]);
    expect(screen.getByRole("tab", { name: "Fields" }).getAttribute("aria-selected")).toBe("true");
    await vi.waitFor(() => expect(screen.getByText(/HTML and CSS: available in Advanced mode/)).toBeTruthy());
    expect(cm.views).toEqual([]);
    await fireEvent.click(screen.getByRole("button", { name: "Switch to Advanced" }));
    await vi.waitFor(() => expect(names()).toEqual(["HTML", "CSS", "Fields", "Variables"]));
    expect(screen.queryByText(/available in Advanced mode/)).toBeNull();
    await openTab("HTML");
    await vi.waitFor(() => expect(cm.views.map((v) => v.lang)).toEqual(["html"]));
    // Back to Simple while on HTML: Fields is shown and the code view goes.
    prefs.set({ theme: "dark", backup: { enabled: true, intervalMinutes: 15, retention: 10 }, uiMode: "simple" });
    await tick();
    expect(names()).toEqual(["Fields", "Variables"]);
    expect(screen.getByRole("tab", { name: "Fields" }).getAttribute("aria-selected")).toBe("true");
    expect(cm.views[0].destroyed).toBe(true);
  });
});

// ---- Static rules -----------------------------------------------------------------------------------

describe("static rules", () => {
  it("no input has a placeholder", async () => {
    await mountEditor();
    for (const name of ["CSS", "Fields", "Variables"]) await openTab(name);
    await fireEvent.click(screen.getByRole("button", { name: /Edit variables/ }));
    expect(document.querySelectorAll("[placeholder]")).toHaveLength(0);
  });

  it("only codemirror.ts imports @codemirror, and only dynamically", () => {
    const root = join(process.cwd(), "src");
    const files: string[] = [];
    const walk = (dir: string) => {
      for (const n of readdirSync(dir)) {
        const p = join(dir, n);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(ts|svelte|js)$/.test(n) && !n.endsWith(".test.ts")) files.push(p);
      }
    };
    walk(root);
    const offenders: string[] = [];
    const importers: string[] = [];
    for (const p of files) {
      const src = readFileSync(p, "utf8");
      if (!p.endsWith(join("editor", "codemirror.ts")) && /from\s+["']@codemirror|import\s+["']@codemirror|import\(\s*["']@codemirror/.test(src))
        offenders.push(p);
      // Value imports of the codemirror module (type-only imports are erased).
      if (/import\s+(?!type\b)[^;]*from\s+["'][^"']*\/codemirror["']/.test(src)) offenders.push(`${p} (static)`);
      if (!p.endsWith("codemirror.ts") && /import\(\s*["'][^"']*\/codemirror["']\s*\)/.test(src))
        importers.push(p);
    }
    expect(offenders).toEqual([]);
    expect(importers.map((p) => p.split(/[\\/]/).pop())).toEqual(["CodeEditor.svelte"]);
  });
});

// ---- Changed on disk ------------------------------------------------------------------------------

describe("changed on disk", () => {
  let emit: (payload: unknown) => void = () => {};

  beforeEach(() => {
    api.on.mockImplementation((event: string, h: (p: unknown) => void) => {
      if (event !== "project-files-changed") return () => {};
      emit = h;
      return () => {
        emit = () => {};
      };
    });
  });

  const changed = (change = "changed", id = "t1") => ({ kind: "template", id, change, message: null });

  it("without unsaved edits: reloads silently (new code views) with a toast", async () => {
    await mountEditor();
    api.getTemplate.mockResolvedValueOnce(makeDoc("<p>from disk</p>"));
    emit(changed());
    await advance(0);
    expect(api.getTemplate).toHaveBeenCalledTimes(2);
    await vi.waitFor(() => expect(cm.views.at(-1)?.value).toBe("<p>from disk</p>"));
    expect(cm.views[0].destroyed).toBe(true);
    expect(screen.queryByRole("alert")).toBeNull();
    expect(api.previewTemplate).toHaveBeenCalledTimes(2);
    await advance(2000);
    expect(api.saveTemplate).not.toHaveBeenCalled();
  });

  it("ignores other templates and screens", async () => {
    await mountEditor();
    emit(changed("changed", "t9"));
    emit({ kind: "screen", id: "t1", change: "changed", message: null });
    await advance(0);
    expect(api.getTemplate).toHaveBeenCalledTimes(1);
  });

  it("with unsaved edits: a banner; autosave waits; Keep mine saves over the file", async () => {
    await mountEditor();
    cm.views[0].set("<p>mine</p>");
    emit(changed());
    await tick();
    const banner = screen.getByRole("alert");
    expect(within(banner).getByText("Changed on disk.")).toBeTruthy();
    await advance(2000);
    expect(api.saveTemplate).not.toHaveBeenCalled();
    expect(api.saveTemplateVariant).not.toHaveBeenCalled();
    await fireEvent.click(within(banner).getByRole("button", { name: "Keep mine" }));
    await advance(0);
    expect(api.saveTemplate).toHaveBeenCalledTimes(1);
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
    expect(variantSaves()[0].html).toBe("<p>mine</p>");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("with unsaved edits: Reload replaces the working copy and drops the edits", async () => {
    await mountEditor();
    cm.views[0].set("<p>mine</p>");
    emit(changed());
    await tick();
    api.getTemplate.mockResolvedValueOnce(makeDoc("<p>theirs</p>"));
    await fireEvent.click(within(screen.getByRole("alert")).getByRole("button", { name: "Reload" }));
    await advance(0);
    await vi.waitFor(() => expect(cm.views.at(-1)?.value).toBe("<p>theirs</p>"));
    expect(screen.queryByRole("alert")).toBeNull();
    await advance(2000);
    expect(api.saveTemplate).not.toHaveBeenCalled();
  });

  it("removed: a banner; Close goes back to the list and nothing is saved", async () => {
    await mountEditor();
    cm.views[0].set("<p>mine</p>");
    emit(changed("removed"));
    await tick();
    const banner = screen.getByRole("alert");
    expect(within(banner).getByText("Deleted on disk.")).toBeTruthy();
    await fireEvent.click(within(banner).getByRole("button", { name: "Close" }));
    expect(nav.goto).toHaveBeenCalledWith("/templates");
    cleanup();
    await advance(2000);
    expect(api.saveTemplate).not.toHaveBeenCalled();
  });

  it("invalid: a toast with the message; the working copy stays", async () => {
    const { toasts } = await import("$lib/stores/toasts.svelte");
    toasts.clear();
    await mountEditor();
    emit({ kind: "template", id: "t1", change: "invalid", message: "template.json: bad key" });
    await tick();
    expect(toasts.list.at(-1)).toMatchObject({ kind: "error" });
    expect(toasts.list.at(-1)?.message).toMatch(/template\.json: bad key/);
    expect(api.getTemplate).toHaveBeenCalledTimes(1);
  });

  it("unsubscribes on unmount", async () => {
    const off = vi.fn();
    api.on.mockImplementation(() => off);
    const { unmount } = await mountEditor();
    unmount();
    expect(off).toHaveBeenCalled();
  });
});

// ---- Variants -------------------------------------------------------------------------------------

describe("variants", () => {
  const compact: VariantDoc = {
    id: "compact",
    name: "Compact",
    vars: { accent: "#ff0000" },
    html: "<b>{{name}} {{title}}</b>",
    css: ".c{}",
  };
  const sponsor: VariantDoc = { id: "sponsor", name: "Sponsor", vars: {}, html: "<i>{{title}}</i>", css: "" };

  beforeEach(() => {
    api.getTemplate.mockImplementation(async () => makeDoc("<div>{{name}}</div>", [compact, sponsor]));
  });

  const variantTabs = () => within(screen.getByRole("tablist", { name: "Variants" })).getAllByRole("tab");
  const variantTab = (name: string) => variantTabs().find((t) => t.textContent?.includes(name))!;

  async function openMenu() {
    await fireEvent.click(screen.getByRole("button", { name: /^Variant actions/ }));
    await vi.waitFor(() => expect(screen.getByRole("menu")).toBeTruthy());
    return screen.getByRole("menu");
  }

  it("shows one tab per variant with the default marked; arrows switch the editors and the preview", async () => {
    await mountEditor();
    const tabs = variantTabs();
    expect(tabs.map((t) => t.querySelector(".name")?.textContent)).toEqual(["Default", "Compact", "Sponsor"]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    expect(within(tabs[0]).getByText("Default", { selector: ".badge" })).toBeTruthy();
    expect(tabs[1].querySelector(".badge")).toBeNull();

    await fireEvent.keyDown(tabs[0], { key: "ArrowRight" });
    await tick();
    expect(variantTab("Compact").getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(variantTab("Compact"));
    // The HTML view is recreated with the variant's text; the old one is destroyed.
    await vi.waitFor(() => expect(cm.views.at(-1)).toMatchObject({ lang: "html", value: compact.html }));
    expect(cm.views[0].destroyed).toBe(true);
    expect(cm.views.filter((v) => !v.destroyed)).toHaveLength(1);
    expect(draftsWith("variant").at(-1)).toEqual({
      variant: "compact",
      html: compact.html,
      css: compact.css,
      variableValues: { accent: "#ff0000", font: "IBM Plex Sans", radius: "4", align: "left" },
    });

    await fireEvent.keyDown(variantTab("Compact"), { key: "End" });
    expect(variantTab("Sponsor").getAttribute("aria-selected")).toBe("true");
    await fireEvent.keyDown(variantTab("Sponsor"), { key: "ArrowRight" });
    expect(variantTab("Default").getAttribute("aria-selected")).toBe("true");
  });

  it("routes saves: a look edit saves that variant, a field edit saves the shared meta", async () => {
    await mountEditor();
    await fireEvent.click(variantTab("Compact"));
    await vi.waitFor(() => expect(cm.views.at(-1)?.value).toBe(compact.html));
    cm.views.at(-1)!.set("<b>{{name}}</b>");
    await advance(800);
    expect(variantSaves()).toEqual([
      { templateId: "t1", variantId: "compact", html: "<b>{{name}}</b>", css: ".c{}", vars: { accent: "#ff0000" } },
    ]);
    expect(api.saveTemplate).not.toHaveBeenCalled();

    await openTab("Fields");
    await fireEvent.click(screen.getByRole("button", { name: "Add field" }));
    await advance(800);
    expect(api.saveTemplate).toHaveBeenCalledTimes(1);
    const meta = api.saveTemplate.mock.calls[0][0] as TemplateDoc;
    expect(meta.fields.map((f) => f.key)).toEqual(["name", "title", "field1"]);
    expect(meta.defaultVariant).toBe("default");
    expect(api.saveTemplateVariant).toHaveBeenCalledTimes(1);
  });

  it("keeps a variant's unsaved text when switching away and back", async () => {
    await mountEditor();
    cm.views[0].set("<div>{{name}} edited</div>");
    await fireEvent.click(variantTab("Compact"));
    await vi.waitFor(() => expect(cm.views.at(-1)?.value).toBe(compact.html));
    await fireEvent.click(variantTab("Default"));
    await vi.waitFor(() => expect(cm.views.at(-1)?.value).toBe("<div>{{name}} edited</div>"));
    await advance(800);
    expect(variantSaves().map((s) => [s.variantId, s.html])).toEqual([["default", "<div>{{name}} edited</div>"]]);
  });

  it("variable values are the active variant's", async () => {
    await mountEditor();
    await fireEvent.click(variantTab("Compact"));
    await openTab("Variables");
    expect((screen.getByLabelText("Accent") as HTMLInputElement).value).toBe("#ff0000");
    await fireEvent.input(screen.getByLabelText("Corner radius slider"), { target: { value: "9" } });
    await advance(800);
    expect(variantSaves().at(-1)).toMatchObject({ variantId: "compact", vars: { accent: "#ff0000", radius: "9" } });
    await fireEvent.click(variantTab("Default"));
    await tick();
    expect((screen.getByLabelText("Accent") as HTMLInputElement).value).toBe("#22b8cf");
  });

  it("warns about fields a variant doesn't use, with a dot on its tab", async () => {
    await mountEditor();
    expect(screen.getByText(/Not used in Default:/).textContent).toMatch(/title/);
    expect(variantTab("Default").querySelector("[aria-label='Has unused fields']")).toBeTruthy();
    expect(variantTab("Compact").querySelector("[aria-label='Has unused fields']")).toBeNull();
    expect(variantTab("Sponsor").querySelector("[aria-label='Has unused fields']")).toBeTruthy();
    await fireEvent.click(variantTab("Compact"));
    await tick();
    expect(screen.queryByText(/Not used in/)).toBeNull();
  });

  it("the ⋯ menu is keyboard accessible and duplicates the active variant under a new name", async () => {
    api.createTemplateVariant.mockResolvedValue(
      makeDoc("<div>{{name}}</div>", [compact, sponsor, { ...compact, id: "compact-copy", name: "Compact copy" }]),
    );
    await mountEditor();
    await fireEvent.click(variantTab("Compact"));
    const menu = await openMenu();
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((i) => i.textContent?.trim())).toEqual([
      "Duplicate as variant…",
      "Rename…",
      "Set as default",
      "Delete…",
    ]);
    await tick();
    expect(document.activeElement).toBe(items[0]);
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(items[1]);
    await fireEvent.keyDown(menu, { key: "ArrowUp" });
    await fireEvent.click(items[0]);
    const dialog = await vi.waitFor(() => screen.getByRole("dialog", { hidden: true }));
    const input = within(dialog).getByLabelText("Name", { selector: "input" }) as HTMLInputElement;
    expect(input.value).toBe("Compact copy");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Duplicate", hidden: true }));
    await advance(0);
    expect(api.createTemplateVariant).toHaveBeenCalledWith("t1", "compact", "Compact copy");
    await vi.waitFor(() => expect(variantTab("Compact copy").getAttribute("aria-selected")).toBe("true"));
    expect(screen.queryByRole("dialog", { hidden: true })).toBeNull();
  });

  it("rename refuses a duplicate name, then renames", async () => {
    api.renameTemplateVariant.mockResolvedValue(
      makeDoc("<div>{{name}}</div>", [{ ...compact, name: "Tight" }, sponsor]),
    );
    await mountEditor();
    await fireEvent.click(variantTab("Compact"));
    await fireEvent.click(within(await openMenu()).getByRole("menuitem", { name: "Rename…" }));
    const dialog = await vi.waitFor(() => screen.getByRole("dialog", { hidden: true }));
    const input = within(dialog).getByLabelText("Name", { selector: "input" });
    await fireEvent.input(input, { target: { value: "sponsor" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(within(dialog).getByText("Another variant has this name.")).toBeTruthy();
    expect(api.renameTemplateVariant).not.toHaveBeenCalled();
    await fireEvent.input(input, { target: { value: "Tight" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    await advance(0);
    expect(api.renameTemplateVariant).toHaveBeenCalledWith("t1", "compact", "Tight");
    await vi.waitFor(() => expect(variantTab("Tight")).toBeTruthy());
  });

  it("set default moves the badge", async () => {
    api.setDefaultTemplateVariant.mockResolvedValue({ ...makeDoc("<div>{{name}}</div>", [compact, sponsor]), defaultVariant: "compact" });
    await mountEditor();
    await fireEvent.click(variantTab("Compact"));
    await fireEvent.click(within(await openMenu()).getByRole("menuitem", { name: "Set as default" }));
    await advance(0);
    expect(api.setDefaultTemplateVariant).toHaveBeenCalledWith("t1", "compact");
    await vi.waitFor(() => expect(variantTab("Compact").querySelector(".badge")).toBeTruthy());
    expect(variantTab("Default").querySelector(".badge")).toBeNull();
  });

  it("delete confirms, then removes the variant and shows the default", async () => {
    api.deleteTemplateVariant.mockResolvedValue(makeDoc("<div>{{name}}</div>", [sponsor]));
    await mountEditor();
    await fireEvent.click(variantTab("Compact"));
    await fireEvent.click(within(await openMenu()).getByRole("menuitem", { name: "Delete…" }));
    const dialog = await vi.waitFor(() => screen.getByRole("dialog", { hidden: true }));
    expect(within(dialog).getByText("Delete variant?")).toBeTruthy();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Delete", hidden: true }));
    await advance(0);
    expect(api.deleteTemplateVariant).toHaveBeenCalledWith("t1", "compact");
    await vi.waitFor(() => expect(variantTabs()).toHaveLength(2));
    expect(variantTab("Default").getAttribute("aria-selected")).toBe("true");
    await vi.waitFor(() => expect(cm.views.at(-1)?.value).toBe("<div>{{name}}</div>"));
  });

  it("the last variant can't be deleted", async () => {
    api.getTemplate.mockImplementation(async () => makeDoc());
    await mountEditor();
    const del = within(await openMenu()).getByRole("menuitem", { name: /Delete…/ }) as HTMLButtonElement;
    expect(del.disabled).toBe(true);
    expect(del.textContent).toMatch(/Last variant/);
    await fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    await tick();
    expect(screen.queryByRole("menu")).toBeNull();
    await tick();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: /^Variant actions/ }));
  });

  it("renders a thumbnail per variant (lazy, debounced) and releases them on unmount", async () => {
    const { unmount } = await mountEditor();
    await vi.waitFor(() => expect(thumbs.renderThumb).toHaveBeenCalledTimes(3));
    const rendered = thumbs.renderThumb.mock.calls.map((c) => (c[1] as VariantDoc).id);
    expect(rendered).toEqual(["default", "compact", "sponsor"]);
    // Nothing changed: no re-render.
    await advance(1000);
    expect(thumbs.renderThumb).toHaveBeenCalledTimes(3);
    // A saved edit re-renders only that variant.
    cm.views[0].set("<div>{{name}} {{title}}</div>");
    await advance(800 + 300);
    expect(thumbs.renderThumb).toHaveBeenCalledTimes(4);
    expect((thumbs.renderThumb.mock.calls[3][1] as VariantDoc).id).toBe("default");
    unmount();
    expect(thumbs.releaseThumb).toHaveBeenCalledTimes(3);
  });
});

describe("newer look", () => {
  it("an outdated preset template shows the notice and registers its palette command", async () => {
    api.listTemplates.mockResolvedValue([{ id: "t1", presetUpdate: { from: 1, to: 2 } }]);
    const { unmount } = await mountEditor();
    await vi.waitFor(() => expect(screen.getByText("A newer look is available")).toBeTruthy());
    expect(screen.getByRole("button", { name: "Update “Classic bar” to the latest look" })).toBeTruthy();
    const cmd = commands().find((c) => c.id === "template.update:t1");
    expect(cmd && titleOf(cmd)).toBe("Update Classic bar to the latest look");
    unmount();
    expect(commands().some((c) => c.id === "template.update:t1")).toBe(false);
  });

  it("up to date: no notice, no command", async () => {
    api.listTemplates.mockResolvedValue([{ id: "t1", presetUpdate: null }]);
    await mountEditor();
    await tick();
    expect(screen.queryByText("A newer look is available")).toBeNull();
    expect(commands().some((c) => c.id === "template.update:t1")).toBe(false);
  });
});
