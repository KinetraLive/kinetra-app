// Flow templates: the gallery's groups, search and keyboard, the slots' options and smart defaults,
// required-slot gating, what add_flow_template gets, /flows (New flow → From a template → the
// editor) and that only lazy imports reach the gallery chunk.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  FlowInfo,
  FlowSlot,
  FlowTemplateInfo,
  LiveLayerView,
  LiveScreenView,
  ScreenState,
  SourceSummary,
  TemplateSummary,
} from "$lib/types";

type Handler = (payload: unknown) => void;
const nav = vi.hoisted(() => ({ goto: vi.fn(), url: new URL("http://localhost/flows") }));
const api = vi.hoisted(() => ({
  listFlows: vi.fn(),
  getFlow: vi.fn(),
  createFlow: vi.fn(),
  renameFlow: vi.fn(),
  duplicateFlow: vi.fn(),
  deleteFlow: vi.fn(),
  setFlowArmed: vi.fn(),
  listFlowTemplates: vi.fn(),
  addFlowTemplate: vi.fn(),
  listSources: vi.fn(),
  listScreens: vi.fn(),
  getScreen: vi.fn(),
  listTemplates: vi.fn(),
  listRecipeInstances: vi.fn(),
  listRecipes: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  getUiState: vi.fn(),
  setUiState: vi.fn(),
  on: (_event: string, _h: Handler) => () => {},
  toAppError: (e: unknown) => (e && typeof e === "object" && "message" in e ? e : { code: "internal", message: String(e) }),
}));
vi.mock("$lib/api/flows", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$lib/api/recipes", () => api);
vi.mock("$app/navigation", () => ({ goto: nav.goto }));
vi.mock("$app/state", () => ({
  page: {
    get url() {
      return nav.url;
    },
  },
}));

import { toasts } from "$lib/stores/toasts.svelte";
import FlowsPage from "../../../../routes/(app)/flows/+page.svelte";
import FlowTemplateGallery from "./FlowTemplateGallery.svelte";
import {
  filledSlots,
  groupTemplates,
  missingSlots,
  slotDefault,
  slotDefaults,
  slotOptions,
  armHint,
  armNote,
  armingOf,
  type Targets,
} from "./slots";

// ---- Fixtures -----------------------------------------------------------------------------------

const slot = (key: string, label: string, kind: FlowSlot["kind"], template: string | null = null, optional = false): FlowSlot => ({
  key,
  label,
  kind,
  prefer: { template },
  optional,
});

const tpl = (id: string, name: string, category: FlowTemplateInfo["category"], chain: string, slots: FlowSlot[], flows = 1): FlowTemplateInfo => ({
  id,
  name,
  description: `${name} does its thing.`,
  category,
  chain,
  slots,
  flows,
});

const TEMPLATES: FlowTemplateInfo[] = [
  tpl("clean-all-log", "Clean all with a log", "blocks", "Cue → Clear all screens → Log", []),
  tpl("show-open", "Show open", "show", "Cue → Take → Delay → Clear layer → Take → Set timer", [
    slot("opener", "Opener layer", "layer", "fullscreen-title"),
    slot("bug", "Logo bug layer", "layer", "corner-bug"),
    slot("clock", "Show clock timer", "timer", null, true),
  ]),
  tpl("now-next-on-item", "Now & Next on new item", "schedule", "Schedule item status → Take", [
    slot("schedule", "Schedule", "scheduleSource"),
    slot("strap", "Now & Next layer", "layer", "now-next"),
  ]),
  tpl(
    "break-countdown",
    "Break with countdown",
    "show",
    "Cue → Sequence → Clear screen → Take",
    [slot("screen", "Break screen", "screen", "countdown"), slot("timer", "Countdown timer", "timer", "countdown")],
    2,
  ),
  tpl("sponsor-reel-between", "Sponsor reel between items", "sponsors", "Schedule item status → Pause/Resume source → Take", [
    slot("slides", "Sponsor slides", "textSource"),
    slot("then", "Then run", "flow", null, true),
    slot("title", "Title field", "field"),
  ]),
];

const layer = (id: string, name: string, fieldDefs: LiveLayerView["fieldDefs"] = []): LiveLayerView => ({
  id,
  name,
  onAir: false,
  pending: false,
  fields: {},
  fieldDefs,
});
const view = (id: string, name: string, layers: LiveLayerView[]): LiveScreenView => ({
  screen: { id, name, width: 1920, height: 1080, background: "transparent" },
  layers,
});

/** Layers bottom first (as the backend sends them): pickers list them top first. */
const SCREENS: LiveScreenView[] = [
  view("s1", "Main", [layer("guest", "Guest"), layer("bug", "LIVE bug")]),
  view("s2", "Stage", [
    layer("open", "Opener"),
    layer("cd", "Countdown", [
      { key: "title", label: "Title", type: "text", default: "" },
      { key: "left", label: "Time left", type: "timer", default: "" },
    ]),
  ]),
];

function source(id: string, name: string, kind: "text" | "schedule"): SourceSummary {
  return { id, name, kind, rowCount: 1, index: 0, current: {}, columns: [], usedBy: [], updatedAt: "", auto: null, nextAt: null } as unknown as SourceSummary;
}
const SOURCES = [source("guests", "Guests", "text"), source("run", "Run of show", "schedule"), source("sp", "Sponsors", "text")];

function flow(id: string, name: string): FlowInfo {
  return {
    id,
    name,
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: [{ id: `${id}t`, type: "trigger.cue", config: { label: null }, position: { x: 0, y: 0 } }],
    edges: [],
    draft: null,
    lastFiredAt: null,
    updatedAt: "2026-10-04T10:00:00Z",
    warnings: [],
    running: false,
    readOnly: false,
  };
}

const PRESETS = new Map([
  ["s1/bug", "corner-bug"],
  ["s2/open", "fullscreen-title"],
  ["s2/cd", "countdown"],
]);
const targets: Targets = { screens: SCREENS, sources: SOURCES, flows: [flow("f1", "Opener cue")], presets: PRESETS };

// The backend side of `loadTargets`: screen documents name the templates, templates their preset.
const TEMPLATE_SUMMARIES = [
  { id: "tbug", preset: { id: "corner-bug", version: 1 } },
  { id: "topen", preset: { id: "fullscreen-title", version: 1 } },
  { id: "tcd", preset: { id: "countdown", version: 1 } },
  { id: "tguest", preset: null },
] as unknown as TemplateSummary[];
const DOC_LAYERS: Record<string, [string, string][]> = {
  s1: [
    ["guest", "tguest"],
    ["bug", "tbug"],
  ],
  s2: [
    ["open", "topen"],
    ["cd", "tcd"],
  ],
};

const flush = async () => {
  for (let i = 0; i < 8; i++) {
    await Promise.resolve();
    await tick();
  }
};

beforeEach(() => {
  toasts.list = [];
  nav.url = new URL("http://localhost/flows");
  nav.goto.mockReset().mockResolvedValue(undefined);
  for (const f of Object.values(api)) f.mockReset();
  api.listFlowTemplates.mockResolvedValue(TEMPLATES);
  api.listScreens.mockResolvedValue(SCREENS);
  api.listSources.mockResolvedValue(SOURCES);
  api.listFlows.mockResolvedValue([flow("f1", "Opener cue")]);
  api.listTemplates.mockResolvedValue(TEMPLATE_SUMMARIES);
  api.getScreen.mockImplementation(async (id: string) => ({
    doc: { id, layers: DOC_LAYERS[id].map(([lid, templateId]) => ({ id: lid, kind: "template", templateId })) },
  }) as unknown as ScreenState);
  api.listRecipeInstances.mockResolvedValue([]);
});

afterEach(cleanup);

// ---- Pure logic ---------------------------------------------------------------------------------

describe("gallery groups and search", () => {
  it("groups in category order (gallery order within one), empty groups left out", () => {
    const g = groupTemplates(TEMPLATES, "");
    expect(g.map((x) => x.label)).toEqual(["Show running", "Event schedule", "Sponsors", "Building blocks"]);
    expect(g[0].items.map((t) => t.id)).toEqual(["show-open", "break-countdown"]);
  });

  it("search matches every word in name, description, chain or category", () => {
    expect(groupTemplates(TEMPLATES, "countdown").flatMap((g) => g.items.map((t) => t.id))).toEqual(["break-countdown"]);
    expect(groupTemplates(TEMPLATES, "pause take").flatMap((g) => g.items.map((t) => t.id))).toEqual(["sponsor-reel-between"]);
    expect(groupTemplates(TEMPLATES, "building").flatMap((g) => g.items.map((t) => t.id))).toEqual(["clean-all-log"]);
    expect(groupTemplates(TEMPLATES, "nothing like this")).toEqual([]);
  });

  it("arming: on-demand templates armed, mixed ones say which flow, every other one disarmed", () => {
    expect(armingOf("show-open")).toBe("armed");
    expect(armingOf("reusable-lower-third")).toBe("armed");
    expect(armingOf("break-countdown")).toBe("mixed");
    expect(armingOf("now-next-on-item")).toBe("disarmed");
    expect(armHint("show-open")).toBeNull();
    expect(armHint("manual-hold")).toBe("Partly disarmed");
    expect(armHint("doors-countdown")).toBe("Starts disarmed");
    expect(armNote({ id: "show-open", flows: 1 }, "Open")).toBeNull();
    expect(armNote({ id: "manual-hold", flows: 3 }, "Hold")).toBe(
      "“Hold · take” starts disarmed. “Hold · hold” and “Hold · release” are armed, but do nothing until “Hold · take” is armed.",
    );
    expect(armNote({ id: "now-next-on-item", flows: 1 }, "N")).toBe("It starts disarmed, so nothing goes on air until you arm it.");
  });
});

describe("slot options and smart defaults", () => {
  it("layers as Screen · Layer (top first), timers and fields of a layer, sources by kind, flows", () => {
    expect(slotOptions(slot("x", "X", "layer"), targets).map((o) => [o.value, o.label])).toEqual([
      ["s1/bug", "Main · LIVE bug"],
      ["s1/guest", "Main · Guest"],
      ["s2/cd", "Stage · Countdown"],
      ["s2/open", "Stage · Opener"],
    ]);
    expect(slotOptions(slot("x", "X", "timer"), targets).map((o) => [o.value, o.label])).toEqual([["s2/cd/left", "Stage · Countdown · Time left"]]);
    expect(slotOptions(slot("x", "X", "field"), targets).map((o) => o.value)).toEqual(["s2/cd/title", "s2/cd/left"]);
    expect(slotOptions(slot("x", "X", "textSource"), targets).map((o) => o.value)).toEqual(["guests", "sp"]);
    expect(slotOptions(slot("x", "X", "scheduleSource"), targets).map((o) => o.value)).toEqual(["run"]);
    expect(slotOptions(slot("x", "X", "flow"), targets).map((o) => o.label)).toEqual(["Opener cue"]);
    expect(slotOptions(slot("x", "X", "screen"), targets).map((o) => o.value)).toEqual(["s1", "s2"]);
  });

  it("prefers the slot's preset template, else the first choice; optional stays empty", () => {
    expect(slotDefault(slot("x", "X", "layer", "fullscreen-title"), targets)).toBe("s2/open");
    expect(slotDefault(slot("x", "X", "layer", "corner-bug"), targets)).toBe("s1/bug");
    expect(slotDefault(slot("x", "X", "layer", "ticker"), targets)).toBe("s1/bug");
    expect(slotDefault(slot("x", "X", "screen", "countdown"), targets)).toBe("s2");
    expect(slotDefault(slot("x", "X", "timer", "countdown"), targets)).toBe("s2/cd/left");
    expect(slotDefault(slot("x", "X", "layer", null, true), targets)).toBe("");
    expect(slotDefault(slot("x", "X", "layer", "countdown", true), targets)).toBe("s2/cd");
    expect(slotDefault(slot("x", "X", "flow"), { ...targets, flows: [] })).toBe("");
  });

  it("required slots gate; only filled slots are sent", () => {
    const slots = TEMPLATES[1].slots;
    const values = slotDefaults(slots, targets);
    expect(values).toEqual({ opener: "s2/open", bug: "s1/bug", clock: "" });
    expect(missingSlots(slots, values)).toEqual([]);
    expect(missingSlots(slots, { ...values, bug: "" }).map((s) => s.key)).toEqual(["bug"]);
    expect(filledSlots(slots, values)).toEqual({ opener: "s2/open", bug: "s1/bug" });
  });
});

// ---- The dialog ---------------------------------------------------------------------------------

async function openGallery(onadded = vi.fn(), onclose = vi.fn()) {
  render(FlowTemplateGallery, { onadded, onclose });
  const dialog = await screen.findByRole("dialog", { name: "New flow from a template" });
  await waitFor(() => expect(dialog.querySelectorAll("[data-flow-template]").length).toBe(TEMPLATES.length));
  return { dialog, onadded, onclose };
}

const card = (id: string) => document.querySelector<HTMLElement>(`[data-flow-template="${id}"]`)!;

describe("gallery dialog", () => {
  it("shows the groups with headings, chains and notes; the search filters", async () => {
    const { dialog } = await openGallery();
    const groups = within(dialog).getAllByRole("group");
    expect(groups.map((g) => g.querySelector("h3")?.textContent)).toEqual(["Show running", "Event schedule", "Sponsors", "Building blocks"]);
    expect(card("show-open").textContent).toContain("Cue → Take → Delay → Clear layer → Take → Set timer");
    expect(card("break-countdown").textContent).toContain("Adds 2 flows");
    expect(card("break-countdown").textContent).toContain("Partly disarmed");
    expect(card("now-next-on-item").textContent).toContain("Starts disarmed");
    expect(card("show-open").textContent).not.toContain("Starts disarmed");

    await fireEvent.input(within(dialog).getByRole("searchbox", { name: "Search templates" }), { target: { value: "sponsor" } });
    expect([...dialog.querySelectorAll<HTMLElement>("[data-flow-template]")].map((c) => c.dataset.flowTemplate)).toEqual(["sponsor-reel-between"]);
    await fireEvent.input(within(dialog).getByRole("searchbox", { name: "Search templates" }), { target: { value: "zzz" } });
    expect(dialog.textContent).toContain("No templates match “zzz”.");
    await fireEvent.click(within(dialog).getByRole("button", { name: "Clear the search" }));
    expect(dialog.querySelectorAll("[data-flow-template]").length).toBe(TEMPLATES.length);
  });

  it("keyboard: ArrowDown from the search, arrows and End move, Enter chooses", async () => {
    const { dialog } = await openGallery();
    const search = within(dialog).getByRole("searchbox", { name: "Search templates" });
    await waitFor(() => expect(document.activeElement).toBe(search));
    await fireEvent.keyDown(search, { key: "ArrowDown" });
    await flush();
    expect(document.activeElement).toBe(card("show-open"));
    expect(card("show-open").tabIndex).toBe(0);
    expect(card("break-countdown").tabIndex).toBe(-1);
    await fireEvent.keyDown(card("show-open"), { key: "ArrowRight" });
    await flush();
    expect(document.activeElement).toBe(card("break-countdown"));
    await fireEvent.keyDown(card("break-countdown"), { key: "End" });
    await flush();
    expect(document.activeElement).toBe(card("clean-all-log"));
    await fireEvent.keyDown(card("clean-all-log"), { key: "ArrowLeft" });
    await flush();
    expect(document.activeElement).toBe(card("sponsor-reel-between"));
    await fireEvent.keyDown(card("sponsor-reel-between"), { key: "Enter" });
    await flush();
    expect(screen.getByRole("dialog", { name: "Sponsor reel between items" })).toBeTruthy();
    expect(document.activeElement?.closest("[data-slot]")?.getAttribute("data-slot")).toBe("slides");

    // Back returns to the card.
    await fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await flush();
    expect(document.activeElement).toBe(card("sponsor-reel-between"));
  });

  it("the form: smart defaults from the layers' presets, required marks, the arm note", async () => {
    await openGallery();
    await fireEvent.click(card("show-open"));
    await flush();
    const form = screen.getByRole("dialog", { name: "Show open" });
    const select = (key: string) => form.querySelector<HTMLSelectElement>(`[data-slot="${key}"] select`)!;
    expect(select("opener").value).toBe("s2/open");
    expect(select("bug").value).toBe("s1/bug");
    expect(select("clock").value).toBe("");
    expect(select("opener").getAttribute("aria-required")).toBe("true");
    expect(form.querySelector('[data-slot="clock"]')?.textContent).toContain("Optional");
    expect([...select("bug").options].map((o) => o.text)).toEqual(["Choose…", "Main · LIVE bug", "Main · Guest", "Stage · Countdown", "Stage · Opener"]);
    expect(form.textContent).not.toContain("disarmed");
    expect((within(form).getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("Show open");
  });

  it("Add stays disabled with the reason until required slots are filled", async () => {
    api.listScreens.mockResolvedValue([]);
    await openGallery();
    await fireEvent.click(card("now-next-on-item"));
    await flush();
    const form = screen.getByRole("dialog", { name: "Now & Next on new item" });
    expect(form.textContent).toContain("It starts disarmed");
    const add = within(form).getByRole("button", { name: "Add" }) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    expect(form.textContent).toContain("Choose Now & Next layer to add it.");
    const strap = form.querySelector<HTMLSelectElement>('[data-slot="strap"] select')!;
    expect(strap.options[0].text).toBe("No layers in this project");
    // The schedule defaulted to the only schedule.
    expect(form.querySelector<HTMLSelectElement>('[data-slot="schedule"] select')!.value).toBe("run");

    // A name is required too.
    const name = within(form).getByRole("textbox", { name: "Name" });
    await fireEvent.input(name, { target: { value: "  " } });
    expect(form.textContent).toContain("Choose Now & Next layer to add it.");
  });

  it("Add sends the slot values in their formats, toasts, closes and hands the flows over", async () => {
    const created = [flow("n1", "Break · start"), flow("n2", "Break · end")];
    api.addFlowTemplate.mockResolvedValue(created);
    const { onadded, onclose } = await openGallery();
    await fireEvent.click(card("break-countdown"));
    await flush();
    const form = screen.getByRole("dialog", { name: "Break with countdown" });
    expect(form.textContent).toContain("Adds 2 flows");
    expect(form.textContent).toContain("“Break with countdown · start” is added armed.");
    await fireEvent.input(within(form).getByRole("textbox", { name: "Name" }), { target: { value: "Break" } });
    await fireEvent.click(within(form).getByRole("button", { name: "Add" }));
    await flush();
    expect(api.addFlowTemplate).toHaveBeenCalledWith({
      templateId: "break-countdown",
      name: "Break",
      slots: { screen: "s2", timer: "s2/cd/left" },
    });
    expect(toasts.list.at(-1)?.message).toBe("Added Break");
    expect(onclose).toHaveBeenCalled();
    expect(onadded).toHaveBeenCalledWith(created);
  });

  it("field and flow slots: values as screenId/layerId/key and the flow id; errors inline", async () => {
    api.addFlowTemplate.mockRejectedValue({ code: "invalid", message: "Sponsor slides is not a text source" });
    const { onclose } = await openGallery();
    await fireEvent.click(card("sponsor-reel-between"));
    await flush();
    const form = screen.getByRole("dialog", { name: "Sponsor reel between items" });
    const sel = (key: string) => form.querySelector<HTMLSelectElement>(`[data-slot="${key}"] select`)!;
    expect(sel("slides").value).toBe("guests");
    await fireEvent.change(sel("slides"), { target: { value: "sp" } });
    await fireEvent.change(sel("then"), { target: { value: "f1" } });
    await fireEvent.change(sel("title"), { target: { value: "s2/cd/title" } });
    await fireEvent.click(within(form).getByRole("button", { name: "Add" }));
    await flush();
    expect(api.addFlowTemplate).toHaveBeenCalledWith({
      templateId: "sponsor-reel-between",
      name: "Sponsor reel between items",
      slots: { slides: "sp", then: "f1", title: "s2/cd/title" },
    });
    expect(within(form).getByRole("alert").textContent).toBe("Sponsor slides is not a text source");
    expect(onclose).not.toHaveBeenCalled();
  });
});

// ---- /flows -------------------------------------------------------------------------------------

describe("/flows", () => {
  it("New flow → From a template → Add opens the first new flow in the editor", async () => {
    api.addFlowTemplate.mockResolvedValue([flow("n1", "Clean all with a log")]);
    render(FlowsPage);
    await fireEvent.click(await screen.findByRole("button", { name: "New flow" }));
    const choice = await screen.findByRole("dialog", { name: "New flow" });
    expect(within(choice).getByRole("button", { name: /^Blank/ }).getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(within(choice).getByRole("button", { name: /^From a template/ }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "New flow" })).toBeNull());
    const gallery = await screen.findByRole("dialog", { name: "New flow from a template" });
    await waitFor(() => expect(gallery.querySelectorAll("[data-flow-template]").length).toBe(TEMPLATES.length));
    await fireEvent.click(card("clean-all-log"));
    await flush();
    await fireEvent.click(within(screen.getByRole("dialog", { name: "Clean all with a log" })).getByRole("button", { name: "Add" }));
    await flush();
    expect(api.addFlowTemplate).toHaveBeenCalledWith({ templateId: "clean-all-log", name: "Clean all with a log", slots: {} });
    expect(nav.goto).toHaveBeenCalledWith("/flows/n1");
  });

  it("the empty state offers Start from a template", async () => {
    api.listFlows.mockResolvedValue([]);
    render(FlowsPage);
    await fireEvent.click(await screen.findByRole("button", { name: "Start from a template" }));
    expect(await screen.findByRole("dialog", { name: "New flow from a template" })).toBeTruthy();
  });
});

// ---- Chunks -------------------------------------------------------------------------------------

describe("the gallery is a lazy chunk", () => {
  it("nothing outside flows/templates imports it statically; /flows import()s it", () => {
    const root = join(process.cwd(), "src");
    const offenders: string[] = [];
    const walk = (dir: string) => {
      for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name) && !/flows[\\/]templates[\\/]/.test(p)) {
          if (/(from\s+|import\s+)["'][^"']*flows\/templates\//.test(readFileSync(p, "utf8").replace(/import\(["'][^"']*["']\)/g, "")))
            offenders.push(p);
        }
      }
    };
    walk(root);
    expect(offenders).toEqual([]);
    const page = readFileSync(join(root, "routes/(app)/flows/+page.svelte"), "utf8");
    expect(page).toContain('import("$lib/components/flows/templates/FlowTemplateGallery.svelte")');
  });
});
