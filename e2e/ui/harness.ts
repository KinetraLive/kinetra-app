// Shared by the UI screenshot harness (shots.spec.ts) and the UI measurements (*.measure.ts):
// output routing to the fixture server, the project's templates built from the real presets, the
// project screens mirrored into the fixture, and opening a page with the mock backend.

import { existsSync, mkdtempSync, readFileSync, readdirSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { BrowserContext, Page } from "@playwright/test";
import { expect, type Fixture } from "../fixture";
import type {
  AssetInfo,
  FlowEdge,
  FlowInfo,
  FlowSlot,
  FlowTemplateInfo,
  FlowNode,
  NodeKind,
  PresetInfo,
  RecipeInstance,
  ScheduleDoc,
  ScheduleItem,
  ScreenDoc,
  ScreenLayer,
  SourceDoc,
  StyleInfo,
  TemplateDoc,
} from "../../src/lib/types";
import { installMockTauri, type MockScenario } from "./mock-tauri";
import { logoPng, type Logo } from "./png";

/**
 * Routes the app's output URLs to the fixture. `onAir` screens mirror the fixture's test screen;
 * `direct` screens (project screens mirrored into the fixture) connect to the same id there.
 * Studio mode: /screen/<id>/preview is the renderer page connected to the fixture screen
 * "pv" + id.slice(2), where the mock backend mirrors that screen's Preview (mock-tauri.ts).
 */
export async function routeOutput(
  context: BrowserContext,
  fixture: Fixture,
  onAir: Set<string>,
  direct: Set<string> = new Set(),
) {
  await context.route("http://127.0.0.1:7770/**", async (route) => {
    const u = new URL(route.request().url());
    const pv = /^\/screen\/([^/]+)\/preview$/.exec(u.pathname);
    if (pv) {
      const page = await route.fetch({ url: `${fixture.base}/screen/pv${pv[1].slice(2)}` });
      const swap = `<script>(()=>{const W=WebSocket;window.WebSocket=function(u,p){return new W(String(u).replace(/screen=([^&]+)/,(m,id)=>"screen=pv"+decodeURIComponent(id).slice(2)).replace(/&bus=preview/,""),p)};window.WebSocket.prototype=W.prototype;Object.assign(window.WebSocket,{OPEN:1,CLOSED:3,CONNECTING:0,CLOSING:2})})()</script>`;
      await route.fulfill({ response: page, body: (await page.text()).replace("<head>", `<head>${swap}`) });
      return;
    }
    const response = await route.fetch({ url: `${fixture.base}${u.pathname}${u.search}` });
    await route.fulfill({ response });
  });
  await context.routeWebSocket(/^ws:\/\/127\.0\.0\.1:7770\/ws/, (ws) => {
    const id = new URL(ws.url()).searchParams.get("screen") ?? "";
    if (direct.has(id) || (id.startsWith("pv") && direct.size > 0)) {
      const up = new WebSocket(`${fixture.base.replace(/^http/, "ws")}/ws?screen=${id}`);
      up.onmessage = (e) => ws.send(String(e.data));
      ws.onClose(() => up.close());
      return;
    }
    // The test screen and on-air screens mirror the fixture's test screen.
    if (id !== "test" && id !== "__preview" && !onAir.has(id)) return;
    const upstream = new WebSocket(`${fixture.base.replace(/^http/, "ws")}/ws?screen=test`);
    upstream.onmessage = (e) => ws.send(String(e.data));
    ws.onClose(() => upstream.close());
  });
}

/** The project's templates: one per preset, read from presets/templates/<id>/, with the name a
 *  user would give it and staggered update times (newest first in this list). */
export const PROJECT_TEMPLATES: { id: string; name: string; preset: string; minutesAgo: number }[] = [
  { id: "t1guestbar", name: "Guest lower third", preset: "classic-bar", minutesAgo: 4 },
  { id: "t2breaking", name: "Breaking news strap", preset: "news-strap", minutesAgo: 47 },
  { id: "t3liveflag", name: "LIVE corner bug", preset: "corner-bug", minutesAgo: 60 * 5 },
  { id: "t4reporter", name: "Reporter name", preset: "minimal-line", minutesAgo: 60 * 26 },
  { id: "t5presentr", name: "Presenter intro", preset: "slide-in-accent", minutesAgo: 60 * 24 * 3 },
  { id: "t6opener01", name: "Show opener", preset: "fullscreen-title", minutesAgo: 60 * 24 * 12 },
  { id: "t7standing", name: "Standings", preset: "leaderboard", minutesAgo: 9 },
  { id: "t8matchbug", name: "Match bug", preset: "scoreboard", minutesAgo: 15 },
  { id: "t9newstick", name: "News ticker", preset: "ticker", minutesAgo: 60 * 2 },
  // The templates the Event schedule and Sponsor slideshow recipes installed.
  { id: "taboard001", name: "Main stage schedule", preset: "schedule-board", minutesAgo: 35 },
  { id: "tbnownext1", name: "Now & Next strap", preset: "now-next", minutesAgo: 35 },
  { id: "tcnextup01", name: "Next up countdown", preset: "next-up", minutesAgo: 35 },
  { id: "tdsponsor1", name: "Sponsor card", preset: "sponsor-slideshow", minutesAgo: 34 },
];

/** Style library index (presets/library.json, as presets.rs reads it); this fallback until the
 *  file exists: Frame, the ten library types and the four legacy lower thirds. */
function library(): { styles: StyleInfo[]; presets: { id: string; kind?: string | null; legacy?: boolean }[] } {
  const file = join(process.cwd(), "presets", "library.json");
  if (existsSync(file)) return JSON.parse(readFileSync(file, "utf8"));
  return {
    styles: [{ id: "frame", name: "Frame", description: "Viewfinder brackets on a dark glass plate, mono labels." }],
    presets: [
      { id: "lower-third", kind: "lowerThird" },
      { id: "fullscreen-title", kind: "title" },
      { id: "corner-bug", kind: "bug" },
      { id: "ticker", kind: "ticker" },
      { id: "now-next", kind: "nowNext" },
      { id: "sponsor-slideshow", kind: "slideshow" },
      { id: "countdown", kind: "countdown" },
      { id: "scoreboard", kind: "scoreboard" },
      { id: "classic-bar", legacy: true },
      { id: "minimal-line", legacy: true },
      { id: "slide-in-accent", legacy: true },
      { id: "news-strap", legacy: true },
    ],
  };
}

/** The built-in styles (`list_styles`). */
export const STYLES: StyleInfo[] = library().styles;

type PresetFile = Omit<TemplateDoc, "id" | "defaultVariant" | "variants"> & {
  defaultVariant?: string;
  variants?: { id: string; name: string; vars?: Record<string, string> }[];
};

/** A preset folder as a project template's looks: schema 2 (`variants/<id>/`) or one "Default". */
function presetLooks(id: string): { meta: PresetFile; defaultVariant: string; variants: TemplateDoc["variants"] } {
  const dir = join(process.cwd(), "presets", "templates", id);
  const meta = JSON.parse(readFileSync(join(dir, "template.json"), "utf8")) as PresetFile;
  const read = (...p: string[]) => (existsSync(join(dir, ...p)) ? readFileSync(join(dir, ...p), "utf8") : "");
  if (meta.variants?.length) {
    const variants = meta.variants
      .filter((v) => existsSync(join(dir, "variants", v.id, "template.html")))
      .map((v) => ({
        id: v.id,
        name: v.name,
        vars: v.vars ?? {},
        html: read("variants", v.id, "template.html"),
        css: read("variants", v.id, "template.css"),
      }));
    if (variants.length === 0) throw new Error(`${id}: no variant markup`);
    const dv = variants.find((v) => v.id === meta.defaultVariant)?.id ?? variants[0].id;
    return { meta, defaultVariant: dv, variants };
  }
  return {
    meta,
    defaultVariant: "default",
    variants: [{ id: "default", name: "Default", vars: {}, html: read("template.html"), css: read("template.css") }],
  };
}

/** Every built-in preset (`list_presets`): the library's order, then the older gallery order. */
export function loadPresets(): PresetInfo[] {
  const lib = library();
  const root = join(process.cwd(), "presets", "templates");
  const ids = readdirSync(root).filter((id) => id !== "test-lower-third" && existsSync(join(root, id, "template.json")));
  // As list_presets returns them (presets::ORDER after the library's own).
  const order = [
    ...lib.presets.map((p) => p.id),
    "classic-bar",
    "minimal-line",
    "slide-in-accent",
    "news-strap",
    "corner-bug",
    "fullscreen-title",
    "leaderboard",
    "scoreboard",
    "ticker",
    "schedule-board",
    "now-next",
    "next-up",
    "countdown",
    "sponsor-slideshow",
    "sponsor-wall",
  ];
  const rank = (id: string) => (order.includes(id) ? order.indexOf(id) : order.length);
  return ids
    .sort((a, b) => rank(a) - rank(b) || a.localeCompare(b))
    .flatMap((id): PresetInfo[] => {
      let looks: ReturnType<typeof presetLooks>;
      try {
        looks = presetLooks(id);
      } catch {
        return []; // a preset folder that is still being written
      }
      const entry = lib.presets.find((p) => p.id === id);
      const first = looks.variants.filter((v) => v.id === looks.defaultVariant);
      return [
        {
          id,
          name: looks.meta.name,
          description: looks.meta.description,
          version: looks.meta.preset?.version ?? 1,
          kind: entry?.kind ?? null,
          legacy: entry?.legacy ?? false,
          // Default first.
          variants: [...first, ...looks.variants.filter((v) => v.id !== looks.defaultVariant)].map((v) => ({
            id: v.id,
            name: v.name,
          })),
        },
      ];
    });
}

export function loadTemplates(): { templates: TemplateDoc[]; presets: PresetInfo[] } {
  const now = Date.now();
  const templates = PROJECT_TEMPLATES.map((t): TemplateDoc => {
    const { meta, defaultVariant, variants } = presetLooks(t.preset);
    const { variants: _v, defaultVariant: _d, ...shared } = meta;
    return {
      ...shared,
      id: t.id,
      name: t.name,
      updatedAt: new Date(now - t.minutesAgo * 60_000).toISOString(),
      // A template made from a preset has its looks (a schema 1 preset: one "Default").
      defaultVariant,
      variants,
    };
  });
  return { templates, presets: loadPresets() };
}

export async function open(page: Page, path: string, scenario: MockScenario) {
  await page.emulateMedia({ colorScheme: scenario.theme, reducedMotion: "reduce" });
  await page.addInitScript(installMockTauri, scenario);
  await page.goto(path);
  await expect(page.locator("html")).toHaveAttribute("data-theme", scenario.theme);
  await page.evaluate(() => document.fonts.ready);
}

// ---- Screens ---------------------------------------------------------------------------
// Project screens live in the mock and are mirrored into the fixture's real project (templates
// added from the same presets, ids mapped), so thumbnails and the layout canvas are the real output.

const tl = (
  id: string,
  name: string,
  templateId: string,
  rect: ScreenLayer["rect"],
  fields: Record<string, string>,
): ScreenLayer => ({ id, name, kind: "template", templateId, rect, fields, vars: {} });

export const SCREEN_DOCS: ScreenDoc[] = [
  {
    id: "s1main01ab",
    name: "Main program",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      tl("l1strap", "Breaking strap", "t2breaking", { x: 0, y: 0, w: 1920, h: 1080 }, {
        label: "BREAKING",
        headline: "Storm warning for the west coast",
        subline: "Ferries cancelled until Sunday",
      }),
      {
        ...tl("l2guest", "Guest lower third", "t1guestbar", { x: 96, y: 560, w: 1040, h: 440 }, {
          name: "Grace Hopper",
          title: "Rear Admiral, computer scientist",
        }),
        bindings: {
          name: { sourceId: "src1guests", column: "name" },
          title: { sourceId: "src1guests", column: "title" },
        },
      },
      tl("l3bug", "LIVE bug", "t3liveflag", { x: 1280, y: 0, w: 640, h: 300 }, { text: "LIVE" }),
    ],
    updatedAt: new Date(Date.now() - 6 * 60_000).toISOString(),
  },
  {
    id: "s2thirds01",
    name: "Lower thirds",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      tl("l1rep", "Reporter", "t4reporter", { x: 0, y: 0, w: 1920, h: 1080 }, {
        name: "Ingrid Solberg",
        title: "Reporter, Bergen",
      }),
      tl("l2pres", "Presenter", "t5presentr", { x: 0, y: 0, w: 1920, h: 1080 }, {
        name: "Jonas Lie",
        title: "Presenter",
      }),
    ],
    updatedAt: new Date(Date.now() - 50 * 60_000).toISOString(),
  },
  {
    id: "s3opener01",
    name: "Opener",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      tl("l1open", "Show opener", "t6opener01", { x: 0, y: 0, w: 1920, h: 1080 }, {
        kicker: "Tonight",
        title: "Evening News",
        subtitle: "Live from Oslo",
      }),
    ],
    updatedAt: new Date(Date.now() - 26 * 3_600_000).toISOString(),
  },
  {
    id: "s4confid01",
    name: "Confidence monitor",
    width: 1280,
    height: 720,
    background: "#101418",
    layers: [],
    updatedAt: new Date(Date.now() - 3 * 24 * 3_600_000).toISOString(),
  },
  {
    id: "s5sports01",
    name: "Sports",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      {
        ...tl("l1board", "Standings", "t7standing", { x: 0, y: 0, w: 1920, h: 1080 }, { title: "Group A" }),
        bindings: {
          rows: { sourceId: "src3result", column: "", sort: { column: "score", numeric: true, desc: true }, limit: 5 },
        },
      },
      tl("l2score", "Match bug", "t8matchbug", { x: 0, y: 0, w: 1920, h: 1080 }, {
        home: "Bergen",
        away: "Oslo",
        home_score: "2",
        away_score: "1",
        period: "2nd",
        clock: JSON.stringify({ run: true, at: Date.now(), base: 754_000, dir: "down", end: "stop" }),
      }),
      tl("l3ticker", "News ticker", "t9newstick", { x: 0, y: 0, w: 1920, h: 1080 }, {
        label: "Latest",
        items: JSON.stringify([
          { text: "Bergen lead 2–1 after a second-half header" },
          { text: "Oslo coach: 'We still believe'" },
          { text: "Next match Saturday 18:00" },
        ]),
      }),
    ],
    updatedAt: new Date(Date.now() - 2 * 60_000).toISOString(),
  },
];
/**
 * A multi-camera wall for the layout tools shot: three tally boxes in a row with equal gaps (144),
 * a LIVE box above the middle one, two ruler guides per axis and an LED bezel safe area. Mirrored
 * into the fixture only by the layout tools scenario.
 */
export const LAYOUT_DOC: ScreenDoc = {
  id: "s8layout01",
  name: "Multi-cam wall",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers: [
    tl("l1cam1", "CAM 1", "t3liveflag", { x: 96, y: 740, w: 480, h: 240 }, { text: "CAM 1" }),
    tl("l2cam2", "CAM 2", "t3liveflag", { x: 700, y: 740, w: 480, h: 240 }, { text: "CAM 2" }),
    tl("l3cam3", "CAM 3", "t3liveflag", { x: 1344, y: 740, w: 480, h: 240 }, { text: "CAM 3" }),
    tl("l4live", "LIVE", "t3liveflag", { x: 700, y: 420, w: 480, h: 200 }, { text: "LIVE" }),
  ],
  guides: { x: [96, 1824], y: [540, 980] },
  safeAreas: [{ id: "bezel00001", name: "LED bezel", rect: { x: 1240, y: 0, w: 48, h: 1080 } }],
  updatedAt: new Date(Date.now() - 4 * 60_000).toISOString(),
};

export const ON_AIR = [
  "s1main01ab/l2guest",
  "s1main01ab/l3bug",
  "s3opener01/l1open",
  "s5sports01/l1board",
  "s5sports01/l2score",
];

/** Fixture template id per mock template id; filled once per worker. */
export const fixtureIds: Record<string, string> = {};

/** Text sources: the guest list the guest lower third is bound to, and a reporters list. */
export const SOURCE_DOCS: SourceDoc[] = [
  {
    id: "src1guests",
    name: "Guests",
    kind: "text",
    columns: [
      { key: "name", label: "Name" },
      { key: "title", label: "Title" },
    ],
    rows: [
      { name: "Grace Hopper", title: "Rear Admiral, computer scientist" },
      { name: "Ada Lovelace", title: "Mathematician" },
      { name: "Katherine Johnson", title: "NASA mathematician" },
      { name: "Margaret Hamilton", title: "Software engineer, Apollo" },
      { name: "Hedy Lamarr", title: "Inventor" },
    ],
    index: 0,
    auto: { everyMs: 8000, loop: true, transition: "update" },
    updatedAt: new Date(Date.now() - 20 * 60_000).toISOString(),
  },
  {
    id: "src2report",
    name: "Reporters",
    kind: "text",
    columns: [
      { key: "name", label: "Name" },
      { key: "city", label: "City" },
    ],
    rows: [
      { name: "Ingrid Solberg", city: "Bergen" },
      { name: "Jonas Lie", city: "Tromsø" },
    ],
    index: 0,
    updatedAt: new Date(Date.now() - 3 * 3_600_000).toISOString(),
  },
  {
    id: "src3result",
    name: "Results",
    kind: "text",
    columns: [
      { key: "name", label: "Name" },
      { key: "team", label: "Team" },
      { key: "score", label: "Score" },
    ],
    rows: [
      { name: "Bergen", team: "BRG", score: "13" },
      { name: "Oslo", team: "OSL", score: "11" },
      { name: "Tromsø", team: "TOS", score: "10" },
      { name: "Stavanger", team: "STV", score: "8" },
      { name: "Trondheim", team: "TRD", score: "7" },
      { name: "Bodø", team: "BOD", score: "5" },
    ],
    index: 0,
    updatedAt: new Date(Date.now() - 12 * 60_000).toISOString(),
  },
];
/** A linear flow ("When … then …"): the trigger wired to the steps in order, laid out left to right. */
export function linearFlow(
  id: string,
  name: string,
  trigger: NodeKind,
  steps: NodeKind[],
  patch: Partial<FlowInfo> = {},
): FlowInfo {
  const chain = [trigger, ...steps];
  const nodes = chain.map((k, i) => ({ ...k, id: `${id}n${i}`, position: { x: 240 * i, y: 0 } }) as FlowNode);
  const port = (k: NodeKind) => (k.type.startsWith("trigger.") ? "fired" : k.type === "logic.delay" ? "out" : "done");
  const edges: FlowEdge[] = nodes.slice(1).map((n, i) => ({
    id: `${id}e${i}`,
    from: { node: nodes[i].id, port: port(chain[i]) },
    to: { node: n.id, port: "in" },
    kind: "event",
  }));
  return {
    id,
    name,
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes,
    edges,
    lastFiredAt: null,
    updatedAt: new Date(Date.now() - 20 * 60_000).toISOString(),
    warnings: [],
    running: false,
    readOnly: false,
    ...patch,
  };
}

const cue = (label: string | null = null): NodeKind => ({ type: "trigger.cue", config: { label } });
const minAgo = (min: number) => new Date(Date.now() - min * 60_000).toISOString();

/**
 * The project's own flows for the Control screenshots: three cues (migrated macros, one
 * disarmed), a time-of-day rule and a branching flow (not linear, with a warning).
 */
export const FLOWS: FlowInfo[] = [
  linearFlow(
    "mac1intro0",
    "Show intro",
    cue(),
    [
      { type: "action.take", config: { screenId: "s3opener01", layerId: "l1open", forMs: null } },
      { type: "logic.delay", config: { ms: 4000 } },
      { type: "action.clear", config: { screenId: "s3opener01", layerId: "l1open" } },
      { type: "action.take", config: { screenId: "s1main01ab", layerId: "l2guest", forMs: null } },
    ],
    { lastFiredAt: minAgo(6) },
  ),
  linearFlow(
    "mac2nextg0",
    "Next guest",
    cue("Next guest"),
    [
      { type: "action.clear", config: { screenId: "s1main01ab", layerId: "l2guest" } },
      { type: "logic.delay", config: { ms: 800 } },
      { type: "action.stepSource", config: { sourceId: "src1guests", step: { to: "by", delta: 1 } } },
      { type: "action.take", config: { screenId: "s1main01ab", layerId: "l2guest", forMs: null } },
    ],
    { lastFiredAt: minAgo(2) },
  ),
  linearFlow("mac3break0", "Break", cue("Break"), [{ type: "action.clearScreen", config: { screenId: "s1main01ab" } }], {
    armed: false,
  }),
  linearFlow(
    "flw4score0",
    "Score bug on kick-off",
    { type: "trigger.timeOfDay", config: { at: "19:00", days: [5, 6] } },
    [{ type: "action.take", config: { screenId: "s5sports01", layerId: "l2score", forMs: null } }],
    { armed: false },
  ),
  {
    ...linearFlow("flw5branch", "Breaking or guest", cue(), [
      { type: "action.take", config: { screenId: "s1main01ab", layerId: "l3bug", forMs: 30_000 } },
    ]),
    nodes: [
      { id: "b0", type: "trigger.layer", config: { screenId: "s1main01ab", layerId: "l3bug", onAir: true }, position: { x: 0, y: 0 } },
      { id: "b1", type: "logic.branch", config: {}, position: { x: 240, y: 0 } },
      { id: "b2", type: "action.clear", config: { screenId: "s1main01ab", layerId: "l2guest" }, position: { x: 480, y: 0 } },
      { id: "b3", type: "action.log", config: { message: "Guest kept", toast: false }, position: { x: 480, y: 120 } },
    ],
    edges: [
      { id: "be0", from: { node: "b0", port: "fired" }, to: { node: "b1", port: "in" }, kind: "event" },
      { id: "be1", from: { node: "b1", port: "then" }, to: { node: "b2", port: "in" }, kind: "event" },
      { id: "be2", from: { node: "b1", port: "else" }, to: { node: "b3", port: "in" }, kind: "event" },
    ],
    warnings: [{ node: "b1", message: "Condition is not wired; it is false" }],
  },
];
/** Fixture source id per mock source id; filled once per worker. */
export const fixtureSourceIds: Record<string, string> = {};


// ---- Recipes, schedules and images --------------------------------------------------------
// A festival day on the "Main stage" schedule (items around the time the scenario is first
// mirrored: two finished, one NOW, UP NEXT delayed 10 min, a later one, a cancelled one, the
// fireworks), four sponsor logos plus two project images imported into the fixture's assets/, a
// "Sponsors" text source with an image column, an "Event" screen (schedule board on air, Now & Next
// and Next up ready) and a "Sponsors" screen with the slideshow on air and auto-advancing.

export const SCHEDULE_ID = "sch1stage0";
export const SPONSORS_ID = "src4sponso";
export const EVENT_SCREEN = "s6event001";
export const SPONSORS_SCREEN = "s7sponsor1";

/** Local date-time `YYYY-MM-DDTHH:MM` of an epoch ms (the fixture uses this machine's zone). */
export function local(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}

const LOGOS: (Logo & { name: string; tagline: string })[] = [
  { file: "northwind-audio.png", name: "Northwind Audio", tagline: "Sound for every stage", bg: [18, 52, 86], fg: [120, 214, 255], mark: "wave" },
  { file: "polaris-networks.png", name: "Polaris Networks", tagline: "Connections that never lag", bg: [246, 244, 238], fg: [36, 42, 120], mark: "diamond", words: [0.95, 0.6, 0.4] },
  { file: "fjord-energy.png", name: "Fjord Energy", tagline: "Powering the night", bg: [255, 186, 8], fg: [20, 24, 28], mark: "triangle", words: [0.8] },
  { file: "kite-and-key.png", name: "Kite & Key", tagline: "Keyboards built to last", bg: [226, 56, 82], fg: [255, 255, 255], mark: "bars", words: [0.7, 0.45] },
];
const IMAGES: Logo[] = [
  { file: "fjordlyd-logo.png", bg: [12, 14, 20], fg: [255, 79, 100], mark: "ring", words: [0.85, 0.5], width: 320, height: 160 },
  { file: "stage-backdrop.png", bg: [40, 28, 74], fg: [255, 150, 80], mark: "circle", words: [0.6], width: 320, height: 180 },
];

export interface M7Data {
  schedule: ScheduleDoc;
  sponsors: SourceDoc;
  assets: AssetInfo[];
}

let m7: M7Data | null = null;

function buildSchedule(): ScheduleDoc {
  const m = 60_000;
  const t = Math.floor(Date.now() / m) * m;
  const item = (
    id: string,
    title: string,
    subtitle: string,
    location: string,
    start: number,
    end: number,
    extra: Partial<ScheduleItem> = {},
  ): ScheduleItem => ({
    id,
    title,
    subtitle,
    location,
    start: local(t + start * m),
    end: local(t + end * m),
    delayMin: 0,
    state: "auto",
    ...extra,
  });
  return {
    id: SCHEDULE_ID,
    name: "Main stage",
    kind: "schedule",
    items: [
      item("it1gates00", "Gates open", "Wristbands at the north entrance", "North gate", -130, -80),
      item("it2lumen00", "Lumen Drift", "Opening set", "Main stage", -75, -35),
      item("it3northb0", "Northbound Collective", "Folk & brass", "Main stage", -30, 15),
      item("it4vale000", "Ingrid Vale", "Acoustic set", "Main stage", 5, 50, { delayMin: 10 }),
      item("it5aurora0", "Aurora Lines", "Headliner", "Main stage", 70, 140),
      item("it6disco00", "Silent disco", "Headphones at the bar", "Courtyard", 145, 200, { state: "cancelled" }),
      item("it7firewk0", "Fireworks over the fjord", "", "Harbour front", 210, 225),
    ],
    ripple: false,
    nextOverride: null,
    // Every label set: the schedule editor rejects an empty one (the server default for upcoming).
    labels: { now: "NOW", next: "UP NEXT", delayed: "DELAYED", previous: "PREVIOUS", cancelled: "CANCELLED", upcoming: "LATER" },
    updatedAt: new Date(Date.now() - 8 * 60_000).toISOString(),
  };
}

/** Imports the images, creates the schedule and the sponsors source in the fixture (once). */
async function prepareM7(server: Fixture): Promise<M7Data> {
  if (m7) return m7;
  const dir = mkdtempSync(join(tmpdir(), "kinetra-ui-"));
  const assets: AssetInfo[] = [];
  for (const [i, logo] of [...LOGOS, ...IMAGES].entries()) {
    const path = logoPng(dir, logo);
    const url = await server.cmd(`importasset ${path}`);
    assets.push({
      name: decodeURIComponent(url.slice("/assets/".length)),
      url,
      bytes: statSync(path).size,
      modifiedAt: new Date(Date.now() - (40 + i * 7) * 60_000).toISOString(),
      usedBy: i < LOGOS.length ? [SPONSORS_ID] : [],
    });
  }
  const sponsors: SourceDoc = {
    id: SPONSORS_ID,
    name: "Sponsors",
    kind: "text",
    columns: [
      { key: "logo", label: "Logo", kind: "image" },
      { key: "name", label: "Name" },
      { key: "tagline", label: "Tagline" },
    ],
    rows: LOGOS.map((l, i) => ({ logo: assets[i].url, name: l.name, tagline: l.tagline })),
    index: 0,
    auto: { everyMs: 6000, loop: true, transition: "update" },
    updatedAt: new Date(Date.now() - 30 * 60_000).toISOString(),
  };
  const schedule = buildSchedule();
  const { name, columns, rows } = sponsors;
  fixtureSourceIds[SPONSORS_ID] = await server.cmd(`addsource ${JSON.stringify({ name, columns, rows })}`);
  fixtureSourceIds[SCHEDULE_ID] = await server.cmd(
    `addschedule ${JSON.stringify({ name: schedule.name, items: schedule.items, ripple: schedule.ripple, labels: schedule.labels })}`,
  );
  m7 = { schedule, sponsors, assets };
  return m7;
}

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
const sb = (column: string, row?: "now" | "next") => ({ sourceId: SCHEDULE_ID, column, ...(row ? { row } : {}) });

export const M7_SCREEN_DOCS: ScreenDoc[] = [
  {
    id: EVENT_SCREEN,
    name: "Event",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      {
        ...tl("l1board", "Schedule board", "taboard001", FULL, { title: "Main stage" }),
        bindings: { items: sb(""), day: sb("day", "next") },
      },
      {
        ...tl("l2nownext", "Now & Next", "tbnownext1", FULL, {}),
        bindings: {
          now_label: sb("label", "now"),
          now_title: sb("title", "now"),
          now_location: sb("location", "now"),
          now_end: sb("end", "now"),
          next_label: sb("label", "next"),
          next_status: sb("status", "next"),
          next_title: sb("title", "next"),
          next_location: sb("location", "next"),
          next_start: sb("start", "next"),
          next_new_start: sb("new_start", "next"),
          next_countdown: sb("countdown", "next"),
        },
      },
      {
        ...tl("l3nextup", "Next up countdown", "tcnextup01", FULL, {}),
        bindings: {
          label: sb("label", "next"),
          status: sb("status", "next"),
          title: sb("title", "next"),
          subtitle: sb("subtitle", "next"),
          location: sb("location", "next"),
          countdown: sb("countdown", "next"),
        },
      },
    ],
    updatedAt: new Date(Date.now() - 9 * 60_000).toISOString(),
  },
  {
    id: SPONSORS_SCREEN,
    name: "Sponsors",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [
      {
        ...tl("l1slides", "Sponsor slideshow", "tdsponsor1", FULL, { label: "Presented by" }),
        vars: { interval: "6" },
        bindings: { slides: { sourceId: SPONSORS_ID, column: "" } },
      },
    ],
    updatedAt: new Date(Date.now() - 30 * 60_000).toISOString(),
  },
];
export const M7_ON_AIR = [`${EVENT_SCREEN}/l1board`, `${SPONSORS_SCREEN}/l1slides`];

const ago = (min: number) => new Date(Date.now() - min * 60_000).toISOString();

/** The two installed recipes (Control's Recipes strip). */
export const RECIPE_INSTANCES: RecipeInstance[] = [
  {
    id: "rcp1event0",
    recipeId: "event-schedule",
    name: "Main stage",
    panel: "schedule",
    sourceId: SCHEDULE_ID,
    screenId: EVENT_SCREEN,
    layerIds: ["l1board", "l2nownext", "l3nextup"],
    templateIds: ["taboard001", "tbnownext1", "tcnextup01"],
    flowIds: ["rul1start0", "rul2before"],
    createdAt: ago(35),
  },
  {
    id: "rcp2spons0",
    recipeId: "sponsor-slideshow",
    name: "Sponsors",
    panel: "slideshow",
    sourceId: SPONSORS_ID,
    screenId: SPONSORS_SCREEN,
    layerIds: ["l1slides"],
    templateIds: ["tdsponsor1"],
    flowIds: ["rul3pause0"],
    createdAt: ago(34),
  },
];

/** The recipes' flows (migrated rules), disarmed except the first. */
export const RECIPE_FLOWS: FlowInfo[] = [
  linearFlow(
    "rul1start0",
    "When an item starts, take Now & Next for 10 s",
    { type: "trigger.itemStatus", config: { sourceId: SCHEDULE_ID, status: "now", itemId: null } },
    [{ type: "action.take", config: { screenId: EVENT_SCREEN, layerId: "l2nownext", forMs: 10_000 } }],
    { origin: "rcp1event0", lastFiredAt: ago(4) },
  ),
  linearFlow(
    "rul2before",
    "5 minutes before the next item, take Next up for 1 min",
    { type: "trigger.beforeNext", config: { sourceId: SCHEDULE_ID, minutes: 5 } },
    [{ type: "action.take", config: { screenId: EVENT_SCREEN, layerId: "l3nextup", forMs: 60_000 } }],
    { origin: "rcp1event0", armed: false },
  ),
  linearFlow(
    "rul3pause0",
    "Pause the slideshow when it goes off air",
    { type: "trigger.layer", config: { screenId: SPONSORS_SCREEN, layerId: "l1slides", onAir: false } },
    [{ type: "action.pauseSource", config: { sourceId: SPONSORS_ID, mode: "pause" } }],
    { origin: "rcp2spons0", armed: false },
  ),
];

const mapBindings = (l: ScreenLayer) =>
  l.bindings
    ? {
        bindings: Object.fromEntries(
          Object.entries(l.bindings).map(([k, b]) => [k, { ...b, sourceId: fixtureSourceIds[b.sourceId] }]),
        ),
      }
    : {};

/**
 * Mirrors SCREEN_DOCS (and with `withM7`, the recipe images, schedule, sponsors and screens; `extra`
 * screens with every layer on air) into the
 * fixture's project (templates added from the same presets, ids mapped; on-air layers shown) with
 * immediate live edits, routes the app's output there, and exposes the fixture to the mock backend.
 */
export async function mirrorScreens(
  page: Page,
  context: BrowserContext,
  server: Fixture,
  withM7 = false,
  extra: ScreenDoc[] = [],
) {
  await server.cmd("liveedit immediate");
  for (const t of PROJECT_TEMPLATES) fixtureIds[t.id] ||= await server.cmd(`addtemplate ${t.preset}`);
  for (const src of SOURCE_DOCS) {
    if (!fixtureSourceIds[src.id]) {
      const { name, columns, rows } = src;
      fixtureSourceIds[src.id] = await server.cmd(`addsource ${JSON.stringify({ name, columns, rows })}`);
    }
  }
  if (withM7) await prepareM7(server);
  const docs = [...(withM7 ? [...SCREEN_DOCS, ...M7_SCREEN_DOCS] : SCREEN_DOCS), ...extra];
  const onAir = [...(withM7 ? [...ON_AIR, ...M7_ON_AIR] : ON_AIR), ...extra.flatMap((d) => d.layers.map((l) => `${d.id}/${l.id}`))];
  for (const d of docs) {
    const mapped = {
      ...d,
      layers: d.layers.map((l) => ({ ...l, templateId: fixtureIds[l.templateId], ...mapBindings(l) })),
    };
    await server.cmd(`savescreen ${JSON.stringify(mapped)}`);
    for (const l of d.layers) {
      const on = onAir.includes(`${d.id}/${l.id}`);
      await server.cmd(`${on ? "layershow" : "layerhide"} ${d.id} ${l.id}`);
    }
  }
  if (withM7) {
    // Back to the first slide (the mock's index; it gets no events) with a fresh 6 s interval, so
    // the output and Control's slideshow panel show the same slide when the shot is taken.
    const sponsors = fixtureSourceIds[SPONSORS_ID];
    await server.cmd(`autoadvance ${sponsors} null`);
    await server.cmd(`stepsource ${sponsors} -100`);
    await server.cmd(`autoadvance ${sponsors} {"everyMs":6000,"loop":true,"transition":"update"}`);
  }
  await routeOutput(context, server, new Set(), new Set(docs.map((d) => d.id)));
  await page.exposeFunction("__kinetraFixture", (line: string) => server.cmd(line));
}

/**
 * Studio mode on Main program (SCREEN_DOCS): Breaking strap cued in with a staged headline, the LIVE
 * bug cued out (Program changed since), Lower thirds with a Preview pending; the T-bar part-way.
 */
export const studioScenario = (theme: "dark" | "light", view: "single" | "grid"): MockScenario => ({
  ...screensScenario(theme),
  control: {
    view,
    selectedScreenId: "s1main01ab",
    gridColumns: 2,
    liveEdit: "onTake",
    hotkeys: { "s1main01ab/l1strap": "F1", "s1main01ab/l3bug": "F2" },
    studio: { enabled: true, previewAfterTake: "keep", transition: { kind: "outIn", ms: 500 } },
  },
  studioCues: {
    s1main01ab: [
      { layerId: "l1strap", visible: true, fields: { headline: "Storm warning lifted for the west coast" } },
      { layerId: "l3bug", visible: false, programChanged: true },
    ],
    s2thirds01: [{ layerId: "l1rep", visible: true }],
  },
  studioTbar: { s1main01ab: 0.45 },
});

/** The screens scenario: SCREEN_DOCS with the six templates, the LIVE bug staged. */
export function screensScenario(theme: "dark" | "light"): MockScenario {
  const data = loadTemplates();
  return {
    theme,
    screens: [],
    lanAccess: true,
    selectedScreenId: null,
    testOnAir: false,
    templates: data.templates,
    presets: data.presets,
    screenDocs: SCREEN_DOCS,
    fixtureTemplateIds: fixtureIds,
    onAirLayers: ON_AIR,
    pendingLayers: ["s1main01ab/l3bug"],
    sourceDocs: SOURCE_DOCS,
    fixtureSourceIds,
    flows: FLOWS,
    flowTemplates: FLOW_TEMPLATES,
  };
}

// ---- Template variants (using them) ----------------------------------------------------------
// The Guest lower third and the Breaking news strap get extra looks (same markup: the shots are
// of the pickers, not the looks), and the Main program's guest layer shows "Compact". Templates
// without `variants` (the v1 shape) become one "Default" variant, so this works either way.

const EXTRA_VARIANTS: Record<string, { id: string; name: string }[]> = {
  t1guestbar: [
    { id: "compact", name: "Compact" },
    { id: "split", name: "Split screen" },
  ],
  t2breaking: [{ id: "urgent", name: "Urgent" }],
};

export function withVariants(templates: TemplateDoc[]): TemplateDoc[] {
  return templates.map((t) => {
    const v1 = t as unknown as { html?: string; css?: string };
    const have = t.variants?.length
      ? t.variants
      : [{ id: "default", name: "Default", vars: {}, html: v1.html ?? "", css: v1.css ?? "" }];
    const base = have[0];
    const add = (EXTRA_VARIANTS[t.id] ?? []).filter((a) => !have.some((h) => h.id === a.id));
    return {
      ...t,
      defaultVariant: t.defaultVariant ?? base.id,
      variants: [...have, ...add.map((a) => ({ ...base, ...a }))],
    };
  });
}

/** A cue that shows the guest layer's "Split screen" look, out and in. */
export const VARIANT_FLOW: FlowInfo = linearFlow("flw8varian", "Guest split screen", { type: "trigger.cue", config: { label: null } }, [
  {
    type: "action.setVariant",
    config: { screenId: "s1main01ab", layerId: "l2guest", variant: "split", transition: "outIn" },
  },
  { type: "action.take", config: { screenId: "s1main01ab", layerId: "l2guest", forMs: 10_000 } },
]);

/** The screens scenario with variants: Main program's guest layer on "Compact". */
export function variantsScenario(theme: "dark" | "light"): MockScenario {
  const s = screensScenario(theme);
  return {
    ...s,
    flows: [...(s.flows ?? []), VARIANT_FLOW],
    templates: withVariants(s.templates ?? []),
    screenDocs: (s.screenDocs ?? []).map((d) =>
      d.id === "s1main01ab"
        ? { ...d, layers: d.layers.map((l) => (l.id === "l2guest" ? { ...l, variant: "compact" } : l)) }
        : d,
    ),
  };
}

/**
 * The recipes scenario (after `mirrorScreens(…, true)`): the screens scenario plus the Event and
 * Sponsors screens, the schedule, the sponsors source, the images and the two recipes.
 */
export function m7Scenario(theme: "dark" | "light"): MockScenario {
  if (!m7) throw new Error("mirrorScreens(page, context, server, true) first");
  return {
    ...screensScenario(theme),
    screenDocs: [...SCREEN_DOCS, ...M7_SCREEN_DOCS],
    onAirLayers: [...ON_AIR, ...M7_ON_AIR],
    sourceDocs: [...SOURCE_DOCS, m7.sponsors],
    scheduleDocs: [m7.schedule],
    assets: m7.assets,
    recipeInstances: RECIPE_INSTANCES,
    flows: [...FLOWS, ...RECIPE_FLOWS],
  };
}

// ---- Flow editor ---------------------------------------------------------------------------
// A non-linear flow on the Event screen: a cue asks "is the headliner on now?" (Compare of the
// schedule's NOW title with a constant) and takes the Next up countdown or the LIVE bug; a Clear
// after the take still points at no layer (an error on its node). Its draft differs from what runs.

const at = (x: number, y: number) => ({ x, y });

export const EDITOR_FLOW: FlowInfo = {
  id: "flw6editor",
  name: "Headliner or regular",
  armed: true,
  retrigger: "restart",
  origin: null,
  nodes: [
    { id: "e1cue", type: "trigger.cue", config: { label: "Next act" }, position: at(0, 60) },
    { id: "e2now", type: "data.scheduleItem", config: { sourceId: SCHEDULE_ID, which: "now" }, position: at(0, 240) },
    { id: "e3const", type: "data.constant", config: { value: { kind: "text", value: "Aurora Lines" } }, position: at(0, 520) },
    { id: "e4cmp", type: "data.compare", config: { op: "eq", a: "", b: "" }, position: at(220, 330) },
    { id: "e5if", type: "logic.branch", config: {}, position: at(440, 100) },
    { id: "e6take", type: "action.take", config: { screenId: EVENT_SCREEN, layerId: "l3nextup", forMs: 10_000 }, position: at(660, 0) },
    { id: "e7bug", type: "action.take", config: { screenId: "s1main01ab", layerId: "l3bug", forMs: null }, position: at(660, 220) },
    { id: "e8clear", type: "action.clear", config: { screenId: EVENT_SCREEN, layerId: "" }, position: at(880, 0) },
  ],
  edges: [
    { id: "w1", from: { node: "e1cue", port: "fired" }, to: { node: "e5if", port: "in" }, kind: "event" },
    { id: "w2", from: { node: "e2now", port: "title" }, to: { node: "e4cmp", port: "a" }, kind: "data" },
    { id: "w3", from: { node: "e3const", port: "value" }, to: { node: "e4cmp", port: "b" }, kind: "data" },
    { id: "w4", from: { node: "e4cmp", port: "bool" }, to: { node: "e5if", port: "condition" }, kind: "data" },
    { id: "w5", from: { node: "e5if", port: "then" }, to: { node: "e6take", port: "in" }, kind: "event" },
    { id: "w6", from: { node: "e5if", port: "else" }, to: { node: "e7bug", port: "in" }, kind: "event" },
    { id: "w7", from: { node: "e6take", port: "done" }, to: { node: "e8clear", port: "in" }, kind: "event" },
  ],
  lastFiredAt: new Date(Date.now() - 12 * 60_000).toISOString(),
  updatedAt: new Date(Date.now() - 15 * 60_000).toISOString(),
  warnings: [],
  running: false,
  readOnly: false,
};
// What runs is the same graph without the Clear; the editor shows the draft ("Not deployed").
EDITOR_FLOW.draft = { nodes: EDITOR_FLOW.nodes, edges: EDITOR_FLOW.edges, updatedAt: EDITOR_FLOW.updatedAt };
EDITOR_FLOW.nodes = EDITOR_FLOW.nodes.filter((n) => n.id !== "e8clear");
EDITOR_FLOW.edges = EDITOR_FLOW.edges.filter((e) => e.id !== "w7");

/** A flow saved by a newer app version: read-only. */
export const READONLY_FLOW: FlowInfo = linearFlow(
  "flw7future",
  "Stage manager cues (newer version)",
  { type: "trigger.cue", config: { label: null } },
  [{ type: "action.clearAll", config: {} }],
  { readOnly: true, armed: false },
);

// ---- Flow templates ----------------------------------------------------------------------
// Six of the built-in templates, mirrored from presets/flows/*.json as list_flow_templates returns them.

const slot = (key: string, label: string, kind: FlowSlot["kind"], template: string | null = null, optional = false): FlowSlot => ({
  key,
  label,
  kind,
  prefer: { template },
  optional,
});

export const FLOW_TEMPLATES: FlowTemplateInfo[] = [
  {
    id: "show-open",
    name: "Show open",
    description: "Plays the opener, then puts up the logo bug and starts the show clock.",
    category: "show",
    chain: "Cue → Take → Delay → Clear layer → Take → Set timer",
    slots: [
      slot("opener", "Opener layer", "layer", "fullscreen-title"),
      slot("bug", "Logo bug layer", "layer", "corner-bug"),
      slot("clock", "Show clock timer", "timer", "scoreboard"),
    ],
    flows: 1,
  },
  {
    id: "break-countdown",
    name: "Break with countdown",
    description: "Clears the screen and runs a 5 min break countdown; at zero the show graphic returns.",
    category: "show",
    chain: "Cue → Sequence → Clear screen → Take",
    slots: [
      slot("screen", "Program screen", "screen"),
      slot("countdown", "Break countdown layer", "layer", "countdown"),
      slot("timer", "Break countdown timer", "timer", "countdown"),
      slot("back", "Back from break layer", "layer", "corner-bug"),
    ],
    flows: 2,
  },
  {
    id: "now-next-on-item",
    name: "Now & Next on new item",
    description: "Shows Now & Next for 10 s whenever a new item starts.",
    category: "schedule",
    chain: "Schedule item status → Take",
    slots: [slot("schedule", "Run of show", "scheduleSource"), slot("nownext", "Now & Next layer", "layer", "now-next")],
    flows: 1,
  },
  {
    id: "sponsor-reel-between",
    name: "Sponsor reel between items",
    description: "Runs the sponsor reel when an item ends and pauses it off air when the next one starts.",
    category: "sponsors",
    chain: "Schedule item status → Pause/Resume source → Take",
    slots: [
      slot("schedule", "Run of show", "scheduleSource"),
      slot("slides", "Sponsor slideshow source", "textSource"),
      slot("reel", "Sponsor reel layer", "layer", "sponsor-slideshow"),
    ],
    flows: 2,
  },
  {
    id: "match-end-results",
    name: "Match end results",
    description: "When the match clock hits zero, clears the screen, shows the result for 15 s, then the standings.",
    category: "sports",
    chain: "Timer reaches zero → Clear screen → Take → Delay → Clear layer → Take",
    slots: [
      slot("clock", "Match clock timer", "timer", "scoreboard"),
      slot("screen", "Program screen", "screen"),
      slot("result", "Result layer", "layer", "fullscreen-title"),
      slot("standings", "Standings layer", "layer", "leaderboard"),
    ],
    flows: 1,
  },
  {
    id: "manual-hold",
    name: "Manual hold",
    description: "Takes Now & Next when an item starts, unless the operator pressed Hold (Release lets it through again).",
    category: "blocks",
    chain: "Schedule item status → Gate → Take",
    slots: [slot("schedule", "Run of show", "scheduleSource"), slot("layer", "Now & Next layer", "layer", "now-next")],
    flows: 3,
  },
];

// ---- Repeat and Stop flow -----------------------------------------------------------------------
// A sponsor loop: a cue starts a Repeat (forever, 2 s between rounds) whose body takes the Guest
// lower third, waits 5 s and clears it; a second cue wires into the Repeat's stop, and done logs.
// A separate "Stop loop" cue is the two-node way to stop it from elsewhere (Stop flow).

export const LOOP_FLOW: FlowInfo = {
  id: "flw9loop00",
  name: "Sponsor loop",
  armed: true,
  retrigger: "ignore",
  origin: null,
  nodes: [
    { id: "r1start", type: "trigger.cue", config: { label: "Start loop" }, position: at(0, 40) },
    { id: "r2end", type: "trigger.cue", config: { label: "End after this round" }, position: at(0, 220) },
    { id: "r3rep", type: "logic.repeat", config: { count: null, delayMs: 2000 }, position: at(220, 100) },
    { id: "r4take", type: "action.take", config: { screenId: "s1main01ab", layerId: "l2guest", forMs: null }, position: at(440, 0) },
    { id: "r5wait", type: "logic.delay", config: { ms: 5000 }, position: at(660, 0) },
    { id: "r6clear", type: "action.clear", config: { screenId: "s1main01ab", layerId: "l2guest" }, position: at(880, 0) },
    { id: "r7log", type: "action.log", config: { message: "Sponsor loop ended", toast: true }, position: at(440, 220) },
  ],
  edges: [
    { id: "rw1", from: { node: "r1start", port: "fired" }, to: { node: "r3rep", port: "in" }, kind: "event" },
    { id: "rw2", from: { node: "r2end", port: "fired" }, to: { node: "r3rep", port: "stop" }, kind: "event" },
    { id: "rw3", from: { node: "r3rep", port: "body" }, to: { node: "r4take", port: "in" }, kind: "event" },
    { id: "rw4", from: { node: "r4take", port: "done" }, to: { node: "r5wait", port: "in" }, kind: "event" },
    { id: "rw5", from: { node: "r5wait", port: "out" }, to: { node: "r6clear", port: "in" }, kind: "event" },
    { id: "rw6", from: { node: "r3rep", port: "done" }, to: { node: "r7log", port: "in" }, kind: "event" },
  ],
  lastFiredAt: new Date(Date.now() - 3 * 60_000).toISOString(),
  updatedAt: new Date(Date.now() - 10 * 60_000).toISOString(),
  warnings: [],
  running: true,
  readOnly: false,
};

/** "Stop loop": a cue and a Stop flow of the sponsor loop. */
export const STOP_LOOP_FLOW: FlowInfo = linearFlow("flw9stop00", "Stop loop", { type: "trigger.cue", config: { label: null } }, [
  { type: "action.stopFlow", config: { flowId: LOOP_FLOW.id } },
]);

/** The flow editor scenario plus the loop and its stop cue. */
export function loopScenario(theme: "dark" | "light"): MockScenario {
  const m = m9Scenario(theme);
  return { ...m, flows: [...(m.flows ?? []), LOOP_FLOW, STOP_LOOP_FLOW] };
}

/** The recipes scenario plus the node editor's flows. */
export function m9Scenario(theme: "dark" | "light"): MockScenario {
  const m = m7Scenario(theme);
  return { ...m, flows: [...(m.flows ?? []), EDITOR_FLOW, READONLY_FLOW] };
}

/**
 * A flow of `n` nodes for the editor's drag measurement: a Cue, then chains of Take / Delay /
 * Clear / Log with a Compare feeding a Branch every tenth node, laid out on a 20-column grid.
 */
export function bigFlow(n = 200): FlowInfo {
  const nodes: FlowNode[] = [];
  const edges: FlowEdge[] = [];
  const pos = (i: number) => at((i % 20) * 260, Math.floor(i / 20) * 220);
  nodes.push({ id: "b0", type: "trigger.cue", config: { label: null }, position: pos(0) });
  for (let i = 1; i < n; i++) {
    const kind: NodeKind =
      i % 10 === 5
        ? { type: "data.compare", config: { op: "eq", a: "", b: "" } }
        : i % 10 === 6
          ? { type: "logic.branch", config: {} }
          : i % 3 === 0
            ? { type: "logic.delay", config: { ms: 500 } }
            : i % 3 === 1
              ? { type: "action.take", config: { screenId: "s1main01ab", layerId: "l2guest", forMs: null } }
              : { type: "action.log", config: { message: `Step ${i}`, toast: false } };
    nodes.push({ ...kind, id: `b${i}`, position: pos(i) } as FlowNode);
  }
  const outOf = (nd: FlowNode) =>
    nd.type.startsWith("trigger.") ? "fired" : nd.type.startsWith("action.") ? "done" : nd.type === "logic.branch" ? "then" : "out";
  let prev: FlowNode = nodes[0];
  for (const nd of nodes.slice(1)) {
    if (nd.type === "data.compare") {
      edges.push({ id: `w${edges.length}`, from: { node: nd.id, port: "bool" }, to: { node: `b${Number(nd.id.slice(1)) + 1}`, port: "condition" }, kind: "data" });
      continue;
    }
    edges.push({ id: `w${edges.length}`, from: { node: prev.id, port: outOf(prev) }, to: { node: nd.id, port: "in" }, kind: "event" });
    prev = nd;
  }
  return {
    id: "flw9big000",
    name: "Two hundred nodes",
    armed: false,
    retrigger: "restart",
    origin: null,
    nodes,
    edges,
    lastFiredAt: null,
    updatedAt: new Date().toISOString(),
    warnings: [],
    running: false,
    readOnly: false,
  };
}

// ---- Control restructure ------------------------------------------------------------------------
// The state the Control side panel was redesigned for: "Screen 1" with one layer (Full-screen
// title); two "Sponsor slideshow" recipes of the same name on it whose source (and slideshow layer)
// were deleted, each with two rules carrying warnings; three cues and two own rules aimed at it.
// The recipe screens (Event with the Main stage recipe, Sponsors) and the cues of the other screens
// prove the panel's scoping. The Main stage recipe gets two rules sharing one warning (aggregated).

export const SCREEN1 = "scr1screen";

export const SCREEN1_DOC: ScreenDoc = {
  id: SCREEN1,
  name: "Screen 1",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers: [
    tl("l1title", "Full-screen title", "t6opener01", { x: 0, y: 0, w: 1920, h: 1080 }, {
      kicker: "Live",
      title: "Evening News",
      subtitle: "From Oslo",
    }),
  ],
  updatedAt: new Date(Date.now() - 3 * 60_000).toISOString(),
};

const goneSlides = (id: string, suffix: string, createdMin: number): RecipeInstance => ({
  id,
  recipeId: "sponsor-slideshow",
  name: "Sponsor slideshow",
  panel: "slideshow",
  sourceId: `srcgone${suffix}`,
  screenId: SCREEN1,
  layerIds: [`l9slide${suffix}`],
  templateIds: ["tdsponsor1"],
  flowIds: [`rp${suffix}pause`, `rp${suffix}resum`],
  createdAt: ago(createdMin),
});

export const BROKEN_RECIPES: RecipeInstance[] = [
  goneSlides("rcp3slideA", "0a", 60 * 26),
  goneSlides("rcp4slideB", "0b", 60 * 3),
];

const layerGone = "The layer it watches was deleted (screen “Screen 1”)";

/** The broken recipes' rules: pause / resume on the deleted slideshow layer and source. */
export const BROKEN_RULES: FlowInfo[] = BROKEN_RECIPES.flatMap((r) => {
  const suffix = r.id === "rcp3slideA" ? "0a" : "0b";
  const layerId = r.layerIds[0];
  const source = r.sourceId ?? "";
  return [
    linearFlow(
      `rp${suffix}pause`,
      "Pause the slideshow when it goes off air",
      { type: "trigger.layer", config: { screenId: SCREEN1, layerId, onAir: false } },
      [{ type: "action.pauseSource", config: { sourceId: source, mode: "pause" } }],
      {
        origin: r.id,
        warnings: [
          { node: `rp${suffix}pausen0`, message: layerGone },
          { node: `rp${suffix}pausen1`, message: "Source no longer exists" },
        ],
      },
    ),
    linearFlow(
      `rp${suffix}resum`,
      "Resume the slideshow when it goes on air",
      { type: "trigger.layer", config: { screenId: SCREEN1, layerId, onAir: true } },
      [{ type: "action.pauseSource", config: { sourceId: source, mode: "resume" } }],
      {
        origin: r.id,
        armed: false,
        warnings: [
          { node: `rp${suffix}resumn0`, message: layerGone },
          { node: `rp${suffix}resumn1`, message: "Source no longer exists" },
        ],
      },
    ),
  ];
});

const take1 = (forMs: number | null = null): NodeKind => ({
  type: "action.take",
  config: { screenId: SCREEN1, layerId: "l1title", forMs },
});
const clear1: NodeKind = { type: "action.clear", config: { screenId: SCREEN1, layerId: "l1title" } };

/** Screen 1's cues and own rules. */
export const SCREEN1_FLOWS: FlowInfo[] = [
  linearFlow("cue1flow01", "Flow 1", cue(), [take1()], { lastFiredAt: minAgo(14) }),
  linearFlow("cue2test01", "Test flow", cue(), [take1(), { type: "logic.delay", config: { ms: 5000 } }, clear1]),
  linearFlow("cue3sting0", "Stinger", cue("Stinger"), [take1(3000)], { lastFiredAt: minAgo(3) }),
  {
    ...linearFlow("own1sting0", "Stinger without double-fire", cue(), []),
    nodes: [
      { id: "s0", type: "trigger.layer", config: { screenId: SCREEN1, layerId: "l1title", onAir: true }, position: at(0, 0) },
      { id: "s1", type: "logic.throttle", config: { ms: 2000 }, position: at(240, 0) },
      { id: "s2", type: "action.clear", config: { screenId: SCREEN1, layerId: "l1title" }, position: at(480, 0) },
    ],
    edges: [
      { id: "se0", from: { node: "s0", port: "fired" }, to: { node: "s1", port: "in" }, kind: "event" },
      { id: "se1", from: { node: "s1", port: "out" }, to: { node: "s2", port: "in" }, kind: "event" },
    ],
    armed: false,
  },
  {
    ...linearFlow("own2leader", "New leader alert", cue(), []),
    nodes: [
      { id: "a0", type: "trigger.sourceRow", config: { sourceId: "src3result" }, position: at(0, 0) },
      { id: "a1", type: "data.sourceRow", config: { sourceId: "src3result", row: { pick: "first" } }, position: at(0, 200) },
      { id: "a2", type: "data.compare", config: { op: "ne", a: "", b: "Bergen" }, position: at(240, 200) },
      { id: "a3", type: "logic.branch", config: {}, position: at(480, 0) },
      {
        id: "a4",
        type: "action.setFields",
        config: { screenId: SCREEN1, layerId: "l1title", fields: { headline: "New leader" } },
        position: at(720, 0),
      },
      { id: "a5", type: "action.take", config: { screenId: SCREEN1, layerId: "l1title", forMs: 8000 }, position: at(960, 0) },
    ],
    edges: [
      { id: "ae0", from: { node: "a0", port: "fired" }, to: { node: "a3", port: "in" }, kind: "event" },
      { id: "ae1", from: { node: "a1", port: "name" }, to: { node: "a2", port: "a" }, kind: "data" },
      { id: "ae2", from: { node: "a2", port: "bool" }, to: { node: "a3", port: "condition" }, kind: "data" },
      { id: "ae3", from: { node: "a3", port: "then" }, to: { node: "a4", port: "in" }, kind: "event" },
      { id: "ae4", from: { node: "a4", port: "done" }, to: { node: "a5", port: "in" }, kind: "event" },
    ],
    warnings: [{ node: "a4", message: "Field “headline” isn't on the layer “Full-screen title”" }],
  },
];

const noSubtitle = "Field “subtitle” isn't on the layer “Now & Next”";

/** Two more Main stage rules sharing one warning (aggregated once per recipe). */
export const MAIN_STAGE_EXTRA: FlowInfo[] = [
  linearFlow(
    "rul4subtit",
    "When an item starts, set the strap's subtitle",
    { type: "trigger.itemStatus", config: { sourceId: SCHEDULE_ID, status: "now", itemId: null } },
    [{ type: "action.setFields", config: { screenId: EVENT_SCREEN, layerId: "l2nownext", fields: { subtitle: "Live now" } } }],
    { origin: "rcp1event0", warnings: [{ node: "rul4subtitn1", message: noSubtitle }] },
  ),
  linearFlow(
    "rul5subtit",
    "When an item ends, clear the strap's subtitle",
    { type: "trigger.itemStatus", config: { sourceId: SCHEDULE_ID, status: "previous", itemId: null } },
    [{ type: "action.setFields", config: { screenId: EVENT_SCREEN, layerId: "l2nownext", fields: { subtitle: "" } } }],
    { origin: "rcp1event0", armed: false, warnings: [{ node: "rul5subtitn1", message: noSubtitle }] },
  ),
];

/** A spare screen: no layers, nothing aimed at it (the empty states). */
export const EMPTY_DOC: ScreenDoc = {
  id: "scr9empty0",
  name: "Spare",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers: [],
  updatedAt: new Date(Date.now() - 90 * 60_000).toISOString(),
};

/**
 * The restructure scenario (after `mirrorScreens(…, true, [SCREEN1_DOC, EMPTY_DOC])`): the recipes
 * scenario plus Screen 1, the spare screen, the broken recipes and Screen 1's flows, with
 * `selectedScreenId` selected in the single view.
 */
export function restructureScenario(theme: "dark" | "light", selectedScreenId = SCREEN1): MockScenario {
  const m = m7Scenario(theme);
  return {
    ...m,
    screenDocs: [...(m.screenDocs ?? []), SCREEN1_DOC, EMPTY_DOC],
    recipeInstances: [...RECIPE_INSTANCES, ...BROKEN_RECIPES],
    flows: [...FLOWS, ...RECIPE_FLOWS, ...MAIN_STAGE_EXTRA, ...SCREEN1_FLOWS, ...BROKEN_RULES],
    control: {
      selectedScreenId,
      view: "single",
      gridColumns: 2,
      liveEdit: "onTake",
      hotkeys: { "flow/cue3sting0": "F3", "s1main01ab/l2guest": "F1" },
    },
  };
}

// ---- API sources ------------------------------------------------------------------------
// Three API sources: a JSON league table (mapped, one keychain secret stored), a CSV weather feed
// whose secret is missing on this machine and that is backing off after errors, and a paused
// match feed. The Sports screen uses all three (`apiUsedBy`), so Control's sources strip lists them.

const league = (rows: [string, string, number][]) => rows.map(([team, short, points], i) => ({ pos: String(i + 1), team, short, points: String(points) }));

export const API_DOCS: SourceDoc[] = [
  {
    id: "api1league",
    name: "League table",
    kind: "api",
    request: {
      url: "https://api.example.com/v4/competitions/eliteserien/standings",
      method: "GET",
      headers: [
        { name: "X-Auth-Token", value: null, secret: "sec1league", stored: true },
        { name: "Accept", value: "application/json" },
      ],
      body: null,
    },
    format: "json",
    mapping: {
      rows: "/standings",
      columns: [
        { key: "pos", label: "Pos", kind: "text", path: "/position" },
        { key: "team", label: "Team", kind: "text", path: "/team/name" },
        { key: "short", label: "Short", kind: "text", path: "/team/short" },
        { key: "points", label: "Points", kind: "text", path: "/points" },
        { key: "form", label: "Form", kind: "text", path: "/form" },
      ],
      csv: { header: true, delimiter: "auto" },
    },
    poll: { intervalMs: 15000, onlyWhenUsed: true, paused: false },
    columns: [
      { key: "pos", label: "Pos" },
      { key: "team", label: "Team" },
      { key: "short", label: "Short" },
      { key: "points", label: "Points" },
      { key: "form", label: "Form" },
    ],
    rows: league([
      ["Bodø/Glimt", "BOD", 61],
      ["Brann", "BRA", 55],
      ["Viking", "VIK", 52],
      ["Molde", "MOL", 47],
    ]).map((r, i) => ({ ...r, form: i === 0 ? "WWDWW" : "" })),
    index: 0,
    fetchedAt: new Date(Date.now() - 4000).toISOString(),
    updatedAt: new Date(Date.now() - 30 * 60_000).toISOString(),
  },
  {
    id: "api2weathr",
    name: "Weather",
    kind: "api",
    request: {
      url: "https://data.example.no/weather/now.csv",
      method: "GET",
      headers: [{ name: "Authorization", value: null, secret: "sec2weathr", stored: false }],
      body: null,
    },
    format: "csv",
    mapping: {
      rows: "",
      columns: [
        { key: "city", label: "City", kind: "text", path: "City" },
        { key: "temp", label: "Temp", kind: "text", path: "Temp" },
      ],
      csv: { header: true, delimiter: ";" },
    },
    poll: { intervalMs: 60000, onlyWhenUsed: false, paused: false },
    columns: [
      { key: "city", label: "City" },
      { key: "temp", label: "Temp" },
    ],
    rows: [
      { city: "Oslo", temp: "11" },
      { city: "Bergen", temp: "9" },
    ],
    index: 0,
    fetchedAt: new Date(Date.now() - 47 * 60_000).toISOString(),
    updatedAt: new Date(Date.now() - 2 * 3_600_000).toISOString(),
  },
  {
    id: "api3match0",
    name: "Match feed",
    kind: "api",
    request: { url: "https://live.example.com/matches/4471.json", method: "GET", headers: [], body: null },
    format: "json",
    mapping: {
      rows: "/events",
      columns: [
        { key: "minute", label: "Minute", kind: "text", path: "/minute" },
        { key: "text", label: "Text", kind: "text", path: "/text" },
      ],
      csv: { header: true, delimiter: "auto" },
    },
    poll: { intervalMs: 5000, onlyWhenUsed: true, paused: true },
    columns: [
      { key: "minute", label: "Minute" },
      { key: "text", label: "Text" },
    ],
    rows: [{ minute: "67", text: "Goal! Bergen 2–1" }],
    index: 0,
    fetchedAt: new Date(Date.now() - 9 * 60_000).toISOString(),
    updatedAt: new Date(Date.now() - 3 * 3_600_000).toISOString(),
  },
];

/** The screens scenario with the API sources; `selected` in Control (the Sports screen by default). */
export function apiScenario(theme: "dark" | "light", selectedScreenId = "s5sports01"): MockScenario {
  const now = Date.now();
  return {
    ...screensScenario(theme),
    sourceDocs: [...SOURCE_DOCS, ...API_DOCS],
    apiStatus: {
      api1league: { state: "ok", lastOkAt: now - 4000, lastAttemptAt: now - 4000, nextAt: now + 11_000, httpStatus: 200, message: null, rows: 4 },
      api2weathr: {
        state: "backoff",
        lastOkAt: now - 47 * 60_000,
        lastAttemptAt: now - 20_000,
        nextAt: now + 100_000,
        httpStatus: 401,
        message: "401 Unauthorized: the Authorization secret is missing on this machine. Retrying in 2 min.",
        rows: 2,
      },
      api3match0: { state: "paused", lastOkAt: now - 9 * 60_000, lastAttemptAt: now - 9 * 60_000, nextAt: null, httpStatus: 200, message: null, rows: 1 },
    },
    apiUsedBy: { api1league: ["s5sports01"], api2weathr: ["s5sports01"], api3match0: ["s5sports01"] },
    control: { selectedScreenId, view: "single", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
  };
}

// ---- Live polls ----------------------------------------------------------------------------------

function pollOf(over: Partial<import("../../src/lib/types").PollData>): import("../../src/lib/types").PollData {
  return {
    id: "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b",
    code: "K7P2QX",
    url: "https://poll.kinetra.live/K7P2QX",
    question: "Who wins tonight?",
    options: [
      { position: 0, label: "Home", votes: 912 },
      { position: 1, label: "Away", votes: 650 },
      { position: 2, label: "Draw", votes: 356 },
    ],
    status: "open",
    showResults: true,
    durationSeconds: null,
    totalVotes: 1918,
    maxVotes: 2000,
    voteLimitReached: false,
    round: 1,
    createdAt: null,
    openedAt: null,
    closesAt: null,
    closedAt: null,
    deleteAfter: null,
    expiresAt: null,
    ...over,
  };
}

/** The Polls page: an open poll with votes, a draft, a closed one; the open one in a poll source. */
export function pollsScenario(theme: "dark" | "light", over: Partial<MockScenario> = {}): MockScenario {
  const now = Date.now();
  return {
    ...screensScenario(theme),
    polls: [
      pollOf({}),
      pollOf({
        id: "0199a1b2-0000-7e5f-8a9b-000000000002",
        code: "R4MW9T",
        url: "https://poll.kinetra.live/R4MW9T",
        question: "Best goal of the half?",
        options: ["First", "Second", "Third"].map((label, position) => ({ position, label, votes: 0 })),
        status: "draft",
        totalVotes: 0,
      }),
      pollOf({
        id: "0199a1b2-0000-7e5f-8a9b-000000000003",
        code: "H8XKD3",
        url: "https://poll.kinetra.live/H8XKD3",
        question: "Player of the match?",
        options: ["Number 7", "Number 10", "Number 4", "Number 1"].map((label, i) => ({ position: i, label, votes: [312, 401, 98, 57][i] })),
        status: "closed",
        totalVotes: 868,
        deleteAfter: new Date(now + 11.5 * 3_600_000).toISOString(),
      }),
    ],
    pollQuota: {
      plan: "pro",
      planName: "Kinetra Pro",
      limits: { openPolls: 3, pollsPerMonth: 30, votesPerPoll: 500, optionsPerPoll: 8, maxDurationSeconds: 86400, lifetimeSeconds: 604800, deleteAfterCloseSeconds: 43200 },
      usage: { openPolls: 1, pollsThisMonth: 4, polls: 3 },
      monthResetsAt: "2026-11-01T00:00:00Z",
    },
    pollSources: [
      {
        id: "pollsrc01",
        name: "Halftime poll",
        kind: "poll",
        link: { id: pollOf({}).id, code: pollOf({}).code },
        cache: pollOf({}),
        updatedAt: new Date(now).toISOString(),
      },
    ],
    ...over,
  };
}
