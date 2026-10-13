// Dev-only screenshot harness for the app UI: `pnpm shots:ui` (see playwright.ui.config.ts).
//
// Each scenario loads the built app with a mock Tauri backend (./mock-tauri.ts) and saves PNGs at
// 1440×900 in dark and light to review/kinetra-m7/. Screen thumbnails are the REAL renderer:
// http://127.0.0.1:7770/* is routed (inside this browser only) to the fixture output server, and
// each thumbnail's WebSocket is bridged to the fixture's `test` screen when that screen is on air
// in the scenario (so on-air thumbnails show the real lower third), otherwise left silent
// (transparent output over the checkerboard). Nothing listens on 7770 and no app process is used.
//
// Shared helpers (routing, templates, the project screens mirrored into the fixture) are in ./harness.ts.
//
// Template scenarios use six templates built from the REAL preset files. The `__preview`
// screen is bridged to the fixture's `test` screen, and the mock's preview commands drive the
// fixture (`template <presetId>`, `show`, `hide`, `fields`, `vars`) through an exposed binding.
// They load presets into the fixture's only layer, so they run after every output-test scenario (those
// need the hardcoded test lower third).

import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import {
  EMPTY_DOC,
  FLOWS,
  EVENT_SCREEN,
  SCREEN1,
  SCREEN1_DOC,
  LAYOUT_DOC,
  ON_AIR,
  PROJECT_TEMPLATES,
  SCREEN_DOCS,
  SPONSORS_SCREEN,
  STYLES,
  apiScenario,
  pollsScenario,
  loadTemplates,
  m7Scenario,
  m9Scenario,
  loopScenario,
  mirrorScreens,
  open,
  restructureScenario,
  routeOutput,
  screensScenario,
  variantsScenario,
} from "./harness";
import type { MockScenario } from "./mock-tauri";
import type { DisplayInfo, OutputConfig, ScreenDoc, SourceDoc, TemplateDoc } from "../../src/lib/types";

/** A data table for the spreadsheet shots: speakers of an event (Sources page). */
const SPEAKERS: SourceDoc = {
  id: "srcspeaker",
  name: "Speakers",
  kind: "text",
  columns: [
    { key: "name", label: "Name" },
    { key: "title", label: "Title" },
    { key: "company", label: "Company" },
    { key: "session", label: "Session" },
    { key: "room", label: "Room" },
  ],
  rows: [
    ["Ingrid Vale", "Head of Product", "Nordlys", "Keynote: live graphics at scale", "Main stage"],
    ["Tomas Berg", "Lighting director", "Fjord Live", "Designing for the camera", "Studio B"],
    ["Amara Okafor", "Engineer", "Signal & Co", "IP video in practice", "Main stage"],
    ["Lena Fischer", "Producer", "Rundfunk Nord", "Run of show without stress", "Studio B"],
    ["Kenji Sato", "Graphics operator", "Pixel Harbour", "Templates that survive the show", "Workshop 1"],
    ["Sofia Lind", "Audio engineer", "Klang", "Mixing for streams", "Workshop 2"],
    ["Marco Rossi", "Director", "Studio Nove", "Multi-camera storytelling", "Main stage"],
    ["Aisha Rahman", "Streaming lead", "Wavecast", "Latency budgets", "Studio B"],
    ["Henrik Dahl", "CTO", "Kinetic Labs", "Automation for small crews", "Workshop 1"],
    ["Chloé Martin", "Designer", "Atelier Lumen", "Type for broadcast", "Workshop 2"],
    ["Daniel Novak", "Camera operator", "Fjord Live", "Shooting for vertical", "Studio B"],
    ["Priya Nair", "Data journalist", "Rundfunk Nord", "Live data on air", "Main stage"],
  ].map(([name, title, company, session, room]) => ({ name, title, company, session, room })),
  index: 1,
  updatedAt: new Date(Date.now() - 4 * 60_000).toISOString(),
};

const OUT = join(process.cwd(), "review", "kinetra-m7");
mkdirSync(OUT, { recursive: true });

const SCREENS: MockScenario["screens"] = [
  { id: "main", name: "Main program", width: 1920, height: 1080, onAir: true },
  { id: "thirds", name: "Lower thirds", width: 1920, height: 1080, onAir: false },
  { id: "score", name: "Scoreboard", width: 1920, height: 1080, onAir: false },
  { id: "confidence", name: "Confidence monitor", width: 1280, height: 720, onAir: false },
];

async function shot(page: Page, name: string) {
  // Let observers, lazy chunks and iframes settle (no animations: reduced motion).
  await page.waitForTimeout(600);
  await page.screenshot({ path: join(OUT, `${name}.png`) });
}

/** Waits until the on-air thumbnail's renderer has drawn the lower third. */
async function lowerThirdIn(page: Page, frameTitle: string) {
  const frame = page.frameLocator(`iframe[title="${frameTitle}"]`);
  await expect(frame.locator("[data-state]").first()).toBeAttached({ timeout: 15_000 });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}`, () => {
    test.beforeEach(async ({ server }) => {
      await server.cmd("clear");
      await server.cmd(`fields {"name":"Ada Lovelace","title":"Analyst & Mathematician"}`);
      await server.cmd("show");
    });

    test("dashboard, empty project", async ({ page, context, server }) => {
      await routeOutput(context, server, new Set());
      await open(page, "/dashboard", {
        theme,
        screens: [],
        lanAccess: false,
        selectedScreenId: null,
        testOnAir: true,
      });
      await expect(page.getByRole("button", { name: /Create your first screen/ })).toBeVisible();
      await shot(page, `${theme}-1-dashboard-empty`);
    });

    test("dashboard with screens", async ({ page, context, server }) => {
      await routeOutput(context, server, new Set(["main"]));
      await open(page, "/dashboard", {
        theme,
        screens: SCREENS,
        lanAccess: true,
        selectedScreenId: "thirds",
        testOnAir: true,
      });
      await expect(page.getByRole("article")).toHaveCount(SCREENS.length);
      await lowerThirdIn(page, "Main program preview");
      await shot(page, `${theme}-2-dashboard-screens`);

      // Keyboard focus on a screen card: brackets on the focused card.
      await page.getByRole("link", { name: "Scoreboard" }).focus();
      await page.keyboard.press("Shift+Tab");
      await page.keyboard.press("Tab");
      await shot(page, `${theme}-3-dashboard-keyboard-focus`);
    });

    test("project picker", async ({ page, context, server }) => {
      await routeOutput(context, server, new Set());
      const ago = (h: number) => new Date(Date.now() - h * 3_600_000).toISOString();
      await open(page, "/", {
        theme,
        screens: [],
        lanAccess: false,
        selectedScreenId: null,
        testOnAir: false,
        noProject: true,
        recents: [
          { path: "D:\\Shows\\Evening News", name: "Evening News", lastOpenedAt: ago(2), missing: false },
          { path: "D:\\Shows\\Sunday Sport", name: "Sunday Sport", lastOpenedAt: ago(30), missing: false },
          { path: "E:\\Archive\\Election 2025", name: "Election 2025", lastOpenedAt: ago(24 * 40), missing: true },
        ],
      });
      await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
      await shot(page, `${theme}-8-picker`);

      await page.getByRole("button", { name: /New project/ }).click();
      await expect(page.getByLabel("Name")).toBeVisible();
      await shot(page, `${theme}-9-picker-new-project`);
    });

    // The Preferences window (980×720, its own route): every section, then About's update states.
    test("preferences window", async ({ page, context, server }) => {
      await routeOutput(context, server, new Set());
      await page.setViewportSize({ width: 980, height: 720 });
      await open(page, "/preferences", {
        theme,
        screens: [],
        lanAccess: true,
        selectedScreenId: null,
        testOnAir: true,
        prefsUiMode: "advanced",
        update: {
          available: true,
          version: "0.2.0",
          notes: "Native menus and a Preferences window.\nCheck for updates from Help or About.",
          date: "2026-10-09T10:00:00Z",
          blocked: "Take everything off air before installing.",
        },
      });
      const sections = page.getByRole("navigation", { name: "Preferences sections" });
      await expect(page.getByRole("radiogroup", { name: "Mode" })).toBeVisible();
      await shot(page, `${theme}-5-preferences-general`);
      const steps: [string, string, () => Promise<void>][] = [
        ["Appearance", "appearance", () => expect(page.getByLabel("Accent color picker")).toBeVisible()],
        ["Network & API", "network", () => expect(page.getByRole("heading", { name: "MIDI & OSC" })).toBeVisible()],
        ["Shortcuts", "shortcuts", () => expect(page.getByRole("searchbox", { name: "Search shortcuts" })).toBeVisible()],
        ["Advanced", "advanced", () => expect(page.getByRole("list", { name: "Snapshots" })).toBeVisible()],
        ["About", "about", () => expect(page.getByText("fa23a08")).toBeVisible()],
      ];
      for (const [name, file, ready] of steps) {
        await sections.getByRole("button", { name }).click();
        await ready();
        await page.mouse.move(2, 2);
        await shot(page, `${theme}-5-preferences-${file}`);
      }

      // About: a newer version, Install blocked while on air.
      await page.getByRole("button", { name: "Check for updates" }).click();
      await expect(page.getByText("is available")).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-6-preferences-update-available`);
    });

    test("license: Preferences › License (Pro, Free signed out) and the welcome screen", async ({ page, context, server }) => {
      await routeOutput(context, server, new Set());
      await page.setViewportSize({ width: 980, height: 720 });
      const base: MockScenario = { theme, screens: [], lanAccess: false, selectedScreenId: null, testOnAir: false };
      await open(page, "/preferences?section=license", base);
      await expect(page.getByText("Works for 18 more days without a connection")).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-8-preferences-license-pro`);
      await open(page, "/preferences?section=license", {
        ...base,
        license: {
          signedIn: false, account: null, plan: "free", features: { ai: false }, entitlements: ["core.app"], endsAt: null,
          licenses: [],
        },
      });
      await expect(page.getByRole("button", { name: "AI templates: needs Kinetra Pro" })).toBeVisible();
      await page.getByRole("button", { name: "AI templates: needs Kinetra Pro" }).click();
      await expect(page.getByRole("heading", { name: "AI templates is part of Kinetra Pro" })).toBeVisible();
      await shot(page, `${theme}-8-preferences-license-free`);
      // First launch: the welcome screen over the picker.
      await page.setViewportSize({ width: 1440, height: 900 });
      await open(page, "/", { ...base, license: { signedIn: false, account: null, status: "none", plan: null, welcome: true, licenses: [] } });
      await expect(page.getByRole("heading", { name: "Welcome to Kinetra" })).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-8-welcome`);
    });

    test("preferences window: updates, Simple mode, no project", async ({ page, context, server }) => {
      await routeOutput(context, server, new Set());
      await page.setViewportSize({ width: 980, height: 720 });
      const base: MockScenario = {
        theme,
        screens: [],
        lanAccess: false,
        selectedScreenId: null,
        testOnAir: false,
      };
      // Help › Check for updates: About opens and checks; installing shows progress.
      await open(page, "/preferences?section=updates", {
        ...base,
        update: { available: true, version: "0.2.0", notes: "Native menus and a Preferences window.", date: null, blocked: null },
      });
      await page.getByRole("button", { name: "Install and restart" }).click();
      await expect(page.getByRole("progressbar", { name: "Installing the update" })).toHaveAttribute("aria-valuenow", "7340032");
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-6-preferences-update-installing`);

      await open(page, "/preferences?section=about", { ...base, update: "fail" });
      await page.getByRole("button", { name: "Check for updates" }).click();
      await expect(page.getByText("Couldn't check for updates. Try again later.")).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-6-preferences-update-failed`);

      // Simple mode: MIDI and OSC give way to the hint.
      await open(page, "/preferences?section=network", { ...base, prefsUiMode: "simple" });
      await expect(page.getByText("MIDI and OSC: available in Advanced mode")).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-7-preferences-network-simple`);

      // No project open: one line per project block.
      await open(page, "/preferences?section=advanced", { ...base, noProject: true });
      await expect(page.getByText("Open a project to change its settings.").first()).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-7-preferences-no-project`);
    });
  });
}

/** Waits until the template preview iframe's renderer has the graphic in. */
async function previewIn(page: Page) {
  const frame = page.frameLocator('iframe[title="Template preview"]');
  await expect(frame.locator('[data-state="in"]').first()).toBeAttached({ timeout: 15_000 });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, templates`, () => {
    const data = loadTemplates();
    const scenario = (extra: Partial<MockScenario> = {}): MockScenario => ({
      theme,
      screens: [],
      lanAccess: false,
      selectedScreenId: null,
      testOnAir: false,
      templates: data.templates,
      presets: data.presets,
      ...extra,
    });

    test.beforeEach(async ({ page, context, server }) => {
      await server.cmd("clear");
      await routeOutput(context, server, new Set());
      await page.exposeFunction("__kinetraFixture", (line: string) => server.cmd(line));
    });

    test("templates list and gallery", async ({ page }) => {
      await open(page, "/templates", scenario());
      await expect(page.getByRole("option")).toHaveCount(PROJECT_TEMPLATES.length);
      await page.getByRole("option", { name: /Guest lower third/ }).click();
      await expect(page.getByRole("option", { name: /Guest lower third/ })).toHaveAttribute(
        "aria-selected",
        "true",
      );
      await previewIn(page);
      await shot(page, `${theme}-10-templates-list`);

      await page.getByRole("button", { name: "New template" }).click();
      await expect(page.getByRole("heading", { name: "New template" })).toBeVisible();
      // Blank and every preset but the Legacy ones (collapsed).
      await expect(page.getByRole("dialog").getByRole("option")).toHaveCount(
        data.presets.filter((p) => !p.legacy).length + 1,
      );
      await page.getByRole("dialog").getByRole("option", { name: /^Corner bug/ }).click();
      await shot(page, `${theme}-11-gallery`);
    });

    test("template editor", async ({ page }) => {
      await open(page, "/templates/t1guestbar", scenario());
      await expect(page.getByRole("tab", { name: "HTML" })).toHaveAttribute("aria-selected", "true");
      await expect(page.locator(".cm-content").first()).toBeVisible({ timeout: 15_000 });
      await expect(page.getByText("On air", { exact: true })).toBeVisible(); // the editor opens with the graphic shown
      await expect(page.getByText("On air", { exact: true })).toBeVisible();
      await previewIn(page);
      await shot(page, `${theme}-12-editor-html`);

      await page.getByRole("tab", { name: "CSS" }).click();
      await expect(page.locator(".cm-content").last()).toBeVisible();
      await shot(page, `${theme}-13-editor-css`);

      await page.getByRole("tab", { name: "Fields" }).click();
      await shot(page, `${theme}-14-editor-fields`);

      await page.getByRole("tab", { name: "Variables" }).click();
      await expect(page.getByRole("heading", { name: "Values", exact: true })).toBeVisible();
      await shot(page, `${theme}-15-editor-variables`);
    });

    test("Simple mode: template editor without code, Flows hidden", async ({ page }) => {
      await open(page, "/templates/t1guestbar", scenario({ prefsUiMode: "simple" }));
      await expect(page.getByRole("tab", { name: "Fields" })).toHaveAttribute("aria-selected", "true");
      await expect(page.getByRole("tab", { name: "HTML" })).toHaveCount(0);
      await expect(page.getByText("HTML and CSS: available in Advanced mode")).toBeVisible();
      await expect(page.getByText("On air", { exact: true })).toBeVisible(); // the editor opens with the graphic shown
      await previewIn(page);
      await shot(page, `${theme}-100-simple-editor`);
      await open(page, "/flows", scenario({ prefsUiMode: "simple" }));
      await expect(page.getByRole("heading", { name: "Flows" })).toBeVisible();
      await expect(page.getByRole("link", { name: "Flows" })).toHaveCount(0);
      await shot(page, `${theme}-101-simple-flows`);
    });

    test("dashboard with recent templates", async ({ page }) => {
      await open(page, "/dashboard", scenario());
      await expect(page.getByRole("heading", { name: "Recent templates" })).toBeVisible();
      await expect(page.getByRole("button", { name: /Create your first screen/ })).toBeVisible();
      await shot(page, `${theme}-16-dashboard-recent-templates`);
    });
  });
}

// Template variants in the editor: the variant tabs with thumbnails (the renderer's compile, static),
// a non-default variant that leaves a field unused, the ⋯ menu, and the list's variant counts.
// The Guest lower third gets "Compact" (no title line, smaller, cyan) and "Sponsor" (right-aligned,
// light bar); the Breaking news strap gets a second look.
function editorVariants(templates: TemplateDoc[]): TemplateDoc[] {
  return templates.map((t) => {
    const base = t.variants[0];
    if (t.id === "t1guestbar")
      return {
        ...t,
        variants: [
          base,
          {
            id: "compact",
            name: "Compact",
            vars: { scale: "0.75", accent: "#33d6ff" },
            html: base.html.replace(/\s*<div class="cbar__title">\{\{title\}\}<\/div>/, ""),
            css: base.css,
          },
          {
            id: "sponsor",
            name: "Sponsor",
            vars: { align: "right", accent: "#e64980", panel: "#ffffffee", text: "#101116" },
            html: base.html,
            css: base.css,
          },
        ],
      };
    if (t.id === "t2breaking") return { ...t, variants: [base, { ...base, id: "urgent", name: "Urgent" }] };
    return t;
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, template variants`, () => {
    const data = loadTemplates();
    const scenario = (): MockScenario => ({
      theme,
      screens: [],
      lanAccess: false,
      selectedScreenId: null,
      testOnAir: false,
      templates: editorVariants(data.templates),
      presets: data.presets,
    });

    test.beforeEach(async ({ page, context, server }) => {
      await server.cmd("clear");
      await routeOutput(context, server, new Set());
      await page.exposeFunction("__kinetraFixture", (line: string) => server.cmd(line));
    });

    test("editor: variant tabs, unused-field warning, the ⋯ menu", async ({ page }) => {
      await open(page, "/templates/t1guestbar", scenario());
      const tabs = page.getByRole("tablist", { name: "Variants" }).getByRole("tab");
      await expect(tabs).toHaveCount(3);
      await expect(page.locator(".cm-content").first()).toBeVisible({ timeout: 15_000 });
      await tabs.nth(1).click();
      await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
      await expect(page.getByText(/Not used in Compact/)).toBeVisible();
      await expect(page.getByText("On air", { exact: true })).toBeVisible(); // the editor opens with the graphic shown
      await previewIn(page);
      await shot(page, `${theme}-17-editor-variants`);

      await page.getByRole("button", { name: /^Variant actions/ }).click();
      await expect(page.getByRole("menu")).toBeVisible();
      await shot(page, `${theme}-18-editor-variant-menu`);
      await page.keyboard.press("Escape");
    });

    test("templates list: variant counts", async ({ page }) => {
      await open(page, "/templates", scenario());
      await expect(page.getByRole("option", { name: /Guest lower third/ })).toContainText("3 variants");
      await shot(page, `${theme}-19-templates-list-variants`);
    });
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, screen layout tools`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, false, [LAYOUT_DOC]);
    });

    test("rulers, guides, grid, safe areas, crop, multi-selection drag, command palette", async ({ page }) => {
      await open(page, `/screens/${LAYOUT_DOC.id}`, {
        ...screensScenario(theme),
        screenDocs: [...SCREEN_DOCS, LAYOUT_DOC],
        onAirLayers: [...ON_AIR, ...LAYOUT_DOC.layers.map((l) => `${LAYOUT_DOC.id}/${l.id}`)],
        screenEditor: { grid: true, gridSize: 32, actionSafe: true, titleSafe: true, centerCross: true, crop: "4:3" },
      });
      const out = page.frameLocator('iframe[title="Screen output"]');
      await expect(out.locator('[data-state="in"]')).toHaveCount(4, { timeout: 15_000 });
      await expect(page.getByRole("toolbar", { name: "Layout tools" })).toBeVisible();
      await expect(page.locator(".ruler.top")).toBeVisible();

      // Select CAM 2 and LIVE, then drag them together towards the gap's middle (mouse still down).
      const cam2 = page.getByRole("button", { name: /^CAM 2,/ });
      await cam2.click();
      await page.getByRole("button", { name: /^LIVE,/ }).click({ modifiers: ["Shift"] });
      const b = await cam2.boundingBox();
      if (!b) throw new Error("no layer box");
      const x = b.x + b.width / 2;
      const y = b.y + b.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 8; i++) {
        await page.mouse.move(x + i, y + (i % 2));
        await page.waitForTimeout(20);
      }
      await page.waitForTimeout(300);
      await expect(page.locator('[data-smart="spacing"]').first()).toBeAttached();
      await shot(page, `${theme}-38-layout-tools`);
      await page.mouse.up();
      await page.waitForTimeout(700);

      await page.keyboard.press("Control+K");
      const palette = page.getByRole("dialog", { name: "Command palette" });
      await expect(palette).toBeVisible();
      await page.keyboard.type("align");
      await shot(page, `${theme}-39-command-palette`);
      await page.keyboard.press("Escape");
    });

    test("free drag: X / Y readout and smart guides mid-drag", async ({ page }) => {
      await open(page, `/screens/${LAYOUT_DOC.id}`, {
        ...screensScenario(theme),
        screenDocs: [...SCREEN_DOCS, LAYOUT_DOC],
        onAirLayers: [...ON_AIR, ...LAYOUT_DOC.layers.map((l) => `${LAYOUT_DOC.id}/${l.id}`)],
      });
      const out = page.frameLocator('iframe[title="Screen output"]');
      await expect(out.locator('[data-state="in"]')).toHaveCount(4, { timeout: 15_000 });
      await expect(page.locator(".ruler.top")).toBeVisible();

      // Drag LIVE alone up and to the left (mouse still down): readout, distances, snap lines.
      const live = page.getByRole("button", { name: /^LIVE,/ });
      await live.click();
      const b = await live.boundingBox();
      if (!b) throw new Error("no layer box");
      const x = b.x + b.width / 2;
      const y = b.y + b.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      for (let i = 1; i <= 12; i++) {
        await page.mouse.move(x - i * 6, y - i * 5);
        await page.waitForTimeout(16);
      }
      await page.waitForTimeout(300);
      const readout = page.locator("[data-readout]");
      await expect(readout).toHaveText(/^X -?\d+ Y -?\d+$/);
      await expect(page.locator('[data-smart="measure"]').first()).toBeAttached();
      await shot(page, `${theme}-86-layout-drag-readout`);
      await page.mouse.up();
      await expect(readout).toHaveCount(0);
    });
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, screens`, () => {
    const scenario = () => screensScenario(theme);

    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("screens list and new screen", async ({ page }) => {
      await open(page, "/screens", scenario());
      await expect(page.getByRole("heading", { name: "Screens", level: 1 })).toBeVisible();
      await expect(page.getByText("Confidence monitor")).toBeVisible();
      await page.getByText("Main program").first().click();
      await page.waitForTimeout(1200);
      await shot(page, `${theme}-17-screens-list`);

      await page.getByRole("button", { name: "New screen" }).first().click();
      await expect(page.getByRole("heading", { name: "New screen" })).toBeVisible();
      await shot(page, `${theme}-18-new-screen`);
    });

    test("screen layout editor", async ({ page }) => {
      await open(page, "/screens/s1main01ab", scenario());
      const out = page.frameLocator('iframe[title="Screen output"]');
      await expect(out.locator('[data-state="in"]')).toHaveCount(2, { timeout: 15_000 });
      const guest = page.getByRole("button", { name: /^Guest lower third,/ });
      await guest.click();
      await page.waitForTimeout(800);
      await shot(page, `${theme}-19-screen-editor`);

      await page.getByRole("complementary", { name: "Inspector" }).evaluate((el) => {
        const s = [el, ...el.querySelectorAll("*")].find((x) => x.scrollHeight > x.clientHeight + 4);
        if (s) s.scrollTop = s.scrollHeight;
      });
      await shot(page, `${theme}-20-screen-editor-inspector`);

      // A drag in progress: snap guides and the moving box (mouse still down).
      const box = await guest.boundingBox();
      if (!box) throw new Error("no layer box");
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
      await page.mouse.down();
      for (let i = 1; i <= 12; i++) {
        await page.mouse.move(box.x + box.width / 2 + i * 9, box.y + box.height / 2 - i * 5);
      }
      await page.waitForTimeout(300);
      await shot(page, `${theme}-21-screen-editor-drag`);
      await page.mouse.up();
    });

    test("anchor grid: a centred layer", async ({ page }) => {
      await open(page, "/screens/s1main01ab", scenario());
      const out = page.frameLocator('iframe[title="Screen output"]');
      await expect(out.locator('[data-state="in"]')).toHaveCount(2, { timeout: 15_000 });
      await page.getByRole("button", { name: /^Guest lower third,/ }).click();
      const inspector = page.getByRole("complementary", { name: "Inspector" });
      const anchor = inspector.getByRole("group", { name: "Anchor" });
      await anchor.getByRole("button", { name: "Center", exact: true }).click();
      await expect(anchor.getByRole("button", { name: "Center", exact: true })).toHaveAttribute("aria-pressed", "true");
      await inspector.getByRole("switch", { name: "Respect safe area" }).click();
      await anchor.scrollIntoViewIfNeeded();
      // Keyboard focus in the grid (the tooltip of the focused cell shows).
      await anchor.getByRole("button", { name: "Center", exact: true }).focus();
      await page.keyboard.press("ArrowRight");
      await page.waitForTimeout(600);
      await shot(page, `${theme}-87-anchor-inspector`);
    });

    test("control", async ({ page }) => {
      await open(page, "/control", {
        ...scenario(),
        control: {
          selectedScreenId: "s1main01ab",
          gridColumns: 2,
          liveEdit: "onTake",
          hotkeys: { "s1main01ab/l2guest": "F1", "s1main01ab/l3bug": "F2", "s3opener01/l1open": "Shift+F1" },
        },
      });
      await expect(page.getByRole("button", { name: "Take Guest lower third" })).toBeVisible();
      await page.waitForTimeout(1500);
      await shot(page, `${theme}-22-control-grid`);

      await page.getByRole("button", { name: "Fields and hotkey of Guest lower third" }).click();
      await page.getByRole("button", { name: "Fields and hotkey of Breaking strap" }).click();
      await page.waitForTimeout(400);
      await shot(page, `${theme}-23-control-fields`);

      await page.getByRole("radio", { name: "Single" }).or(page.getByRole("button", { name: "Single" })).first().click();
      await page.waitForTimeout(1200);
      await shot(page, `${theme}-24-control-single`);
    });

    test("sources", async ({ page }) => {
      await open(page, "/sources", scenario());
      await expect(page.getByRole("listbox", { name: "Sources" })).toBeVisible();
      await page.getByRole("option", { name: /Guests/ }).click();
      await page.waitForTimeout(500);
      await shot(page, `${theme}-25-sources`);

      await page.goto("/screens/s1main01ab");
      await page.getByRole("button", { name: /^Guest lower third,/ }).click();
      await page.waitForTimeout(800);
      await shot(page, `${theme}-26-screen-editor-bound`);
    });

    test("data table: the sheet with a selection, the paste mapping", async ({ page }) => {
      const base = scenario();
      await open(page, "/sources", { ...base, sourceDocs: [...(base.sourceDocs ?? []), SPEAKERS] });
      await page.getByRole("option", { name: /Speakers/ }).click();
      const grid = page.getByRole("grid", { name: "Rows of Speakers" });
      await expect(grid.locator('[role="gridcell"][data-r="0"][data-c="0"]')).toHaveText("Ingrid Vale");
      // Rows 4–6 selected, the active cell in row 6.
      await grid.locator('[role="rowheader"][data-r="3"]').click();
      await grid.locator('[role="gridcell"][data-r="5"][data-c="2"]').click({ modifiers: ["Shift"] });
      await shot(page, `${theme}-25b-data-table`);

      await page.evaluate(() => {
        const dt = new DataTransfer();
        dt.setData(
          "text/plain",
          "Name\tCompany\tSession\tTwitter\r\nJonas Lie\tNordlys\tClosing panel\t@jlie\r\nMaja Holm\tKlang\tSound for small rooms\t@majah\r\nOle Brun\tPixel Harbour\tLower thirds 101\t\r\n",
        );
        document
          .querySelector('[role="grid"]')!
          .dispatchEvent(new ClipboardEvent("paste", { clipboardData: dt, bubbles: true, cancelable: true }));
      });
      const dialog = page.getByRole("dialog", { name: "Paste 3 rows" });
      await expect(dialog).toBeVisible();
      await shot(page, `${theme}-25c-paste-mapping`);
    });

    test("automation: control, cues, flows, list and timer editors", async ({ page }) => {
      await open(page, "/control", {
        ...scenario(),
        control: {
          selectedScreenId: "s5sports01",
          gridColumns: 2,
          liveEdit: "onTake",
          hotkeys: { "flow/mac1intro0": "F5", "macro/mac2nextg0": "F6", "s5sports01/l2score": "F2" },
        },
      });
      // The project's cues aim at other screens: show them all.
      await page.getByRole("button", { name: "Show all cues" }).first().click();
      await expect(page.getByRole("button", { name: "Run Show intro" })).toBeVisible();
      await page.getByRole("button", { name: "Fields and hotkey of Match bug" }).click();
      await page.waitForTimeout(1500);
      await shot(page, `${theme}-27-control-automation`);

      // A cue running, the screen's own rules open under Automation.
      await page.getByRole("button", { name: "Run Next guest" }).click();
      const automation = page.getByRole("region", { name: "Automation" });
      await automation.locator('[data-row="__other"] .toggle').click();
      await expect(automation.locator("[data-flow]")).toHaveCount(1);
      await page.getByRole("region", { name: "Cues" }).evaluate((el) => el.scrollIntoView({ block: "start" }));
      await expect(page.locator('[data-cue="mac2nextg0"]')).toContainText("Running");
      await shot(page, `${theme}-28-control-flows`);

      // The linear editor on a rule.
      await automation.locator('[data-flow="flw4score0"]').getByRole("button", { name: "Actions for Score bug on kick-off" }).click();
      await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
      await expect(page.getByRole("dialog", { name: "Edit rule" })).toBeVisible();
      await page.waitForTimeout(400);
      await shot(page, `${theme}-41-flow-editor`);
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog", { name: "Edit rule" })).toBeHidden();

      // Automation off: the amber switch next to PANIC and the pill in the top bar.
      await page.getByRole("switch", { name: "Automation off" }).click();
      await expect(page.getByRole("link", { name: "Automation off" })).toBeVisible();
      await page.waitForTimeout(300);
      await shot(page, `${theme}-42-automation-off`);
      // On every page (client-side navigation: a reload would reset the mock backend).
      await page.keyboard.press("Control+4");
      await expect(page).toHaveURL(/\/screens$/);
      await expect(page.getByRole("link", { name: "Automation off" })).toBeVisible();
      await page.waitForTimeout(600);
      await shot(page, `${theme}-43-automation-off-screens`);

      await page.goto("/templates/t7standing");
      await page.getByRole("tab", { name: "Fields" }).click();
      await page.waitForTimeout(800);
      await shot(page, `${theme}-29-template-list-field`);

      await page.goto("/screens/s5sports01");
      // The three layers fill the screen; pick Standings from the layers panel.
      await page.getByRole("complementary", { name: "Layers" }).getByText("Standings", { exact: true }).first().click();
      await page.waitForTimeout(1200);
      await shot(page, `${theme}-30-screen-editor-list`);
    });

    test("data editing in Control: list summaries, the drawer, a time input, the resized panel", async ({ page }) => {
      await open(page, "/control", {
        ...scenario(),
        control: {
          selectedScreenId: "s5sports01",
          view: "single",
          gridColumns: 2,
          liveEdit: "onTake",
          hotkeys: {},
          panelWidth: 520,
        },
      });
      // The resized panel and the summary lines: own rows (ticker) and linked (standings).
      await page.getByRole("button", { name: "Fields and hotkey of Standings" }).click();
      await page.getByRole("button", { name: "Fields and hotkey of News ticker" }).click();
      await expect(page.getByRole("button", { name: "Edit data table Results" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Edit items of Items" })).toBeVisible();
      await expect(page.getByRole("separator", { name: "Resize the side panel" })).toHaveAttribute("aria-valuenow", "520");
      await page.waitForTimeout(800);
      await shot(page, `${theme}-60-control-list-summaries`);

      // The drawer on the ticker's own rows.
      await page.getByRole("button", { name: "Edit items of Items" }).click();
      const drawer = page.getByRole("dialog", { name: "Items" });
      await expect(drawer.getByRole("grid")).toBeVisible();
      await page.waitForTimeout(500);
      await shot(page, `${theme}-61-control-list-drawer`);
      await page.keyboard.press("Escape");
      await expect(drawer).toBeHidden();

      // The drawer on the linked data table, with the note.
      await page.getByRole("button", { name: "Edit data table Results" }).click();
      const table = page.getByRole("dialog", { name: "Data table Results" });
      await expect(table.getByText(/Changes affect \d+ layers?\./)).toBeVisible();
      await page.waitForTimeout(500);
      await shot(page, `${theme}-62-control-table-drawer`);
      await table.getByRole("button", { name: "Close" }).click();
      await expect(table).toBeHidden();

      // A time that isn't one: kept in the box, marked, Save blocked.
      const automation = page.getByRole("region", { name: "Automation" });
      await automation.locator('[data-row="__other"] .toggle').click();
      await automation.locator('[data-flow="flw4score0"]').getByRole("button", { name: "Actions for Score bug on kick-off" }).click();
      await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
      const rule = page.getByRole("dialog", { name: "Edit rule" });
      const time = rule.getByLabel("Time of day");
      await time.fill("24:00");
      await time.blur();
      await expect(time).toHaveAttribute("aria-invalid", "true");
      await expect(rule.getByRole("button", { name: "Save" })).toBeDisabled();
      await page.waitForTimeout(400);
      await shot(page, `${theme}-63-time-input-invalid`);
      await time.fill("1915");
      await time.blur();
      await expect(time).toHaveValue("19:15");
      await page.keyboard.press("Escape");
    });
  });
}

// Recipes (schedule and slideshow panels, gallery, their flows), the schedule editor, the asset
// picker and the Event screen's real schedule board. The schedule's statuses come from the
// fixture's real engine (`scheduleview`), images from its assets/ folder.

/** Waits until every <img> in `scope` has decoded (thumbnails from the fixture server). */
async function imagesLoaded(page: Page, selector: string, min = 1) {
  await expect
    .poll(
      () =>
        page.locator(selector).evaluateAll((imgs) =>
          imgs.filter((i) => (i as HTMLImageElement).complete && (i as HTMLImageElement).naturalWidth > 0).length,
        ),
      { timeout: 15_000 },
    )
    .toBeGreaterThanOrEqual(min);
}

/** Scrolls Control's side panel so the recipe's panel starts at the top. */
async function revealRecipe(page: Page, recipeId: string) {
  await page.locator(`[data-recipe="${recipeId}"]`).evaluate((el) => el.scrollIntoView({ block: "start" }));
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, recipes`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, true);
    });

    const control = (selectedScreenId: string): MockScenario => ({
      ...m7Scenario(theme),
      control: { selectedScreenId, view: "single", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
    });

    test("control: schedule panel", async ({ page }) => {
      await open(page, "/control", control(EVENT_SCREEN));
      await page.locator('[data-recipe="rcp1event0"] .toggle').click();
      const schedule = page.getByRole("region", { name: "Main stage: schedule" });
      await expect(schedule).toBeVisible({ timeout: 15_000 });
      // The view (from the real engine) and the document (titles) are in.
      await expect(schedule.locator('[data-status="now"]')).toBeVisible();
      await expect(schedule.locator("[data-next]")).toContainText("Ingrid Vale");
      await expect(schedule.locator('[data-badge="delayed"]').first()).toBeVisible();
      const out = page.frameLocator('iframe[title="Event preview"]');
      await expect(out.getByText("Northbound Collective").first()).toBeVisible({ timeout: 15_000 });
      await revealRecipe(page, "rcp1event0");
      await shot(page, `${theme}-31-control-recipes`);

    });

    test("control: slideshow panel", async ({ page }) => {
      await open(page, "/control", control(SPONSORS_SCREEN));
      await page.locator('[data-recipe="rcp2spons0"] .toggle').click();
      const slideshow = page.getByRole("region", { name: "Slideshow Sponsors" });
      await expect(slideshow).toBeVisible({ timeout: 15_000 });
      await expect(slideshow.locator("[data-progress]")).toBeAttached();
      await imagesLoaded(page, '[aria-label="Slideshow Sponsors"] img');
      const sponsors = page.frameLocator('iframe[title="Sponsors preview"]');
      await expect(sponsors.locator('[data-state="in"]')).toBeAttached({ timeout: 15_000 });
      await revealRecipe(page, "rcp2spons0");
      await shot(page, `${theme}-32-control-slideshow`);
    });

    test("recipe gallery and a recipe flow in the linear editor", async ({ page }) => {
      await open(page, "/control", control(EVENT_SCREEN));
      await page.locator('[data-recipe="rcp1event0"] .toggle').click();
      await expect(page.getByRole("region", { name: "Main stage: schedule" })).toBeVisible({ timeout: 15_000 });
      await page.getByRole("button", { name: "Add automation…" }).click();
      const gallery = page.getByRole("dialog", { name: "Add automation" });
      const cards = gallery.getByRole("listbox", { name: "Recipes" });
      await expect(cards.getByRole("option")).toHaveCount(4);
      await cards.getByRole("option", { name: /^Event schedule/ }).click();
      await imagesLoaded(page, "dialog[open] img", 4);
      await shot(page, `${theme}-33-recipe-gallery`);
      await page.keyboard.press("Escape");
      await expect(gallery).toBeHidden();

      const recipe = page.locator('[data-recipe="rcp1event0"]');
      await expect(recipe.locator("[data-flow]")).toHaveCount(2);
      await recipe.locator('[data-flow="rul2before"]').getByRole("button", { name: /^Actions for/ }).click();
      await page.getByRole("menuitem", { name: "Edit", exact: true }).click();
      const editor = page.getByRole("dialog", { name: "Edit rule" });
      await expect(editor).toBeVisible();
      await expect(editor.getByLabel("Minutes before")).toHaveValue("5");
      await page.waitForTimeout(400);
      await shot(page, `${theme}-34-recipe-flow-editor`);
      await page.keyboard.press("Escape");
      await expect(editor).toBeHidden();

      // A rule of the recipe's own: aimed at its screen and schedule; a schedule step moving
      // the next item's start.
      await recipe.getByRole("button", { name: "New rule for Main stage" }).click();
      const rule = page.getByRole("dialog", { name: "New rule" });
      await expect(rule).toBeVisible();
      await rule.getByLabel("Step 1 type").selectOption("action.schedule");
      await rule.getByLabel("Step 1 item").selectOption("next");
      await rule.getByLabel("Step 1 schedule action").selectOption("setStart");
      await rule.getByLabel("Step 1 new start").fill("2026-10-04T14:15");
      await page.waitForTimeout(400);
      await shot(page, `${theme}-44-recipe-new-rule`);
      await rule.getByRole("button", { name: "Save" }).click();
      await expect(rule).toBeHidden();
      await expect(recipe.locator("[data-flow]")).toHaveCount(3);
    });

    test("schedule editor and asset picker", async ({ page }) => {
      await open(page, "/sources", m7Scenario(theme));
      await page.getByRole("option", { name: /Main stage/ }).click();
      const editor = page.getByRole("region", { name: "Source editor" });
      await expect(editor.locator('[role="row"][data-status="now"]')).toBeVisible({ timeout: 15_000 });
      await expect(editor.locator('[role="row"][data-status="delayed"]')).toBeVisible();
      await shot(page, `${theme}-35-schedule-editor`);

      await page.getByRole("option", { name: /Sponsors/ }).click();
      // The data table's sheet: the image cell (Logo, row 1) opens the asset picker on Enter.
      const sheet = page.getByRole("grid", { name: "Rows of Sponsors" });
      await sheet.locator('[role="gridcell"][data-r="0"][data-c="0"]').click({ timeout: 15_000 });
      await page.keyboard.press("Enter");
      const picker = page.getByRole("dialog", { name: "Choose image for Logo, row 1" });
      await expect(picker.getByRole("option")).toHaveCount(6);
      await imagesLoaded(page, "dialog[open] img", 6);
      await shot(page, `${theme}-36-asset-picker`);
    });

    test("event screen output", async ({ page }) => {
      await open(page, `/screens/${EVENT_SCREEN}`, m7Scenario(theme));
      const out = page.frameLocator('iframe[title="Screen output"]');
      await expect(out.locator('[data-state="in"]')).toHaveCount(1, { timeout: 15_000 });
      await expect(out.getByText("Ingrid Vale").first()).toBeVisible();
      // The three layers fill the screen; pick the board from the layers panel.
      await page.getByRole("complementary", { name: "Layers" }).getByText("Schedule board", { exact: true }).first().click();
      await page.waitForTimeout(800);
      await shot(page, `${theme}-37-event-output`);
    });
  });
}

// The Flows list (own flows, then per recipe) and the node editor on a non-linear flow: data
// wires (schedule NOW title vs a constant into a Branch), an error on a node, live debug flashes
// (static under reduced motion), the palette searched, the inspector, a read-only flow.

// Template variants in use: the layout editor's Variant select, Control's expanded layer (variant +
// transition, staged under Live edit "On take") and the palette's "Switch … to …", the flow editor's
// Set variant node.
for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, variants`, () => {
    const scenario = () => variantsScenario(theme);

    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("layout editor: Variant select", async ({ page }) => {
      await open(page, "/screens/s1main01ab", scenario());
      await page.getByRole("button", { name: /^Guest lower third,/ }).click();
      const inspector = page.getByRole("complementary", { name: "Inspector" });
      const select = inspector.getByLabel("Variant", { exact: true });
      await expect(select).toHaveValue("compact");
      await select.focus();
      await shot(page, `${theme}-60-variant-inspector`);
    });

    test("control: expanded layer and the palette", async ({ page }) => {
      await open(page, "/control", {
        ...scenario(),
        control: { selectedScreenId: "s1main01ab", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
      });
      await page.getByRole("button", { name: "Fields and hotkey of Guest lower third" }).click();
      const select = page.getByLabel("Variant of Guest lower third");
      await expect(select).toHaveValue("compact");
      await page.getByRole("radio", { name: "Out and in" }).click();
      await select.selectOption("split");
      await expect(page.getByText("Changes on next take").first()).toBeVisible();
      await shot(page, `${theme}-61-variant-control`);

      await page.keyboard.press("Control+K");
      await expect(page.getByRole("dialog", { name: "Command palette" })).toBeVisible();
      await page.keyboard.type("switch");
      await shot(page, `${theme}-62-variant-palette`);
      await page.keyboard.press("Escape");
    });

    test("flow editor: Set variant node", async ({ page }) => {
      await open(page, "/flows/flw8varian", scenario());
      const canvas = page.getByRole("application", { name: "Flow canvas" });
      await expect(canvas.locator(".svelte-flow__node")).toHaveCount(3, { timeout: 15_000 });
      await canvas.locator('[data-node="flw8variann1"]').click();
      const inspector = page.getByRole("complementary", { name: "Inspector" });
      await expect(inspector.getByRole("heading", { name: "Set variant" })).toBeVisible();
      await expect(inspector.getByLabel("Variant", { exact: true })).toHaveValue("split");
      await shot(page, `${theme}-63-variant-flow-node`);
    });
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, flows`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, true);
    });

    test("flows list and node editor", async ({ page }) => {
      await open(page, "/flows", m9Scenario(theme));
      await expect(page.getByRole("list", { name: "Project flows" })).toBeVisible({ timeout: 15_000 });
      await expect(page.getByRole("list", { name: "Main stage flows" })).toBeVisible();
      await expect(page.locator('[data-flow="flw6editor"]')).toContainText("Not deployed");
      await shot(page, `${theme}-50-flows-list`);

      // The editor: the canvas, the error on the Clear, the flow's warnings.
      await page.locator('[data-flow="flw6editor"] a.name').click();
      await expect(page).toHaveURL(/\/flows\/flw6editor$/);
      const canvas = page.getByRole("application", { name: "Flow canvas" });
      await expect(canvas.locator(".svelte-flow__node")).toHaveCount(8, { timeout: 15_000 });
      await expect(canvas.locator('[data-node="e8clear"]')).toContainText("Pick a layer");
      await page.waitForTimeout(500);
      await shot(page, `${theme}-51-flow-editor`);

      // Live debug: the run's nodes and wires light up (held for 1 s under reduced motion).
      await page.evaluate(() => (window as unknown as { __kinetraFlowDebug: (id: string) => void }).__kinetraFlowDebug("flw6editor"));
      await expect(canvas.locator('[data-node="e5if"].flash')).toBeVisible();
      await page.screenshot({ path: join(OUT, `${theme}-52-flow-debug.png`) });

      // The inspector on the Compare node, and the palette searched.
      await canvas.locator('[data-node="e4cmp"]').click();
      const inspector = page.getByRole("complementary", { name: "Inspector" });
      await expect(inspector.getByRole("heading", { name: "Compare" })).toBeVisible();
      await page.getByRole("searchbox", { name: "Search nodes" }).fill("time");
      await page.waitForTimeout(400);
      await shot(page, `${theme}-53-flow-inspector-palette`);

      // Connect to… from the Branch's else output, by keyboard.
      await canvas.locator('[data-node="e5if"]').click();
      await inspector.getByRole("button", { name: "Connect to… (else)" }).click();
      await page.waitForTimeout(300);
      await shot(page, `${theme}-54-flow-connect-menu`);
      await page.keyboard.press("Escape");

      // A read-only flow (from a newer version).
      await page.goto("/flows/flw7future");
      await expect(page.getByText("so it's read-only")).toBeVisible({ timeout: 15_000 });
      await shot(page, `${theme}-55-flow-read-only`);
    });

    test("flow editor: Repeat loop (paced warning) and Stop flow", async ({ page }) => {
      await open(page, "/flows/flw9loop00", loopScenario(theme));
      const canvas = page.getByRole("application", { name: "Flow canvas" });
      await expect(canvas.locator(".svelte-flow__node")).toHaveCount(7, { timeout: 15_000 });
      await canvas.locator('[data-node="r3rep"]').click();
      const inspector = page.getByRole("complementary", { name: "Inspector" });
      await expect(inspector.getByRole("heading", { name: "Repeat" })).toBeVisible();
      await expect(inspector.getByRole("switch", { name: "Forever" })).toHaveAttribute("aria-checked", "true");
      // The engine paces a fast loop: a warning on the node (flow-debug) and in the header (flow-state).
      const paced = "Looping faster than every 100 ms — paced.";
      await page.evaluate((message) => {
        const emit = (window as unknown as { __kinetraEmit: (e: string, p: unknown) => void }).__kinetraEmit;
        emit("flow-debug", { id: "flw9loop00", nodes: [], edges: [], errors: [{ node: "r3rep", message }] });
        emit("flow-state", { id: "flw9loop00", state: "running", node: "r3rep", message, at: new Date().toISOString() });
      }, paced);
      await expect(canvas.locator('[data-node="r3rep"]')).toContainText(paced);
      await expect(page.getByRole("status").filter({ hasText: paced })).toBeVisible();
      await shot(page, `${theme}-64-flow-repeat`);

      // Stop loop: a cue and a Stop flow of the loop.
      await page.goto("/flows/flw9stop00");
      await expect(canvas.locator(".svelte-flow__node")).toHaveCount(2, { timeout: 15_000 });
      await canvas.locator('[data-node="flw9stop00n1"]').click();
      await expect(inspector.getByRole("heading", { name: "Stop flow" })).toBeVisible();
      await expect(inspector.getByRole("combobox", { name: "Flow to stop" })).toHaveValue("flw9loop00");
      await shot(page, `${theme}-65-flow-stop-flow`);
    });

    test("flows list, empty", async ({ page }) => {
      await open(page, "/flows", { ...m9Scenario(theme), flows: [] });
      await expect(page.getByRole("button", { name: "Create your first flow" })).toBeVisible({ timeout: 15_000 });
      await shot(page, `${theme}-56-flows-empty`);
    });

    test("flow templates: gallery and the Point it at form", async ({ page }) => {
      await open(page, "/flows", m9Scenario(theme));
      await page.getByRole("button", { name: "New flow" }).click();
      const blank = page.getByRole("dialog", { name: "New flow" });
      await expect(blank.getByRole("button", { name: /^From a template/ })).toBeVisible({ timeout: 15_000 });
      await shot(page, `${theme}-57-new-flow-choice`);

      await blank.getByRole("button", { name: /^From a template/ }).click();
      const gallery = page.getByRole("dialog", { name: "New flow from a template" });
      await expect(gallery.locator("[data-flow-template]")).toHaveCount(6, { timeout: 15_000 });
      await shot(page, `${theme}-58-flow-templates`);

      // Show open: the smart defaults find the opener, the LIVE bug and the match clock.
      await gallery.locator('[data-flow-template="show-open"]').click();
      const form = page.getByRole("dialog", { name: "Show open" });
      await expect(form.locator('[data-slot="opener"] select')).toHaveValue("s3opener01/l1open");
      await expect(form.locator('[data-slot="bug"] select')).toHaveValue("s1main01ab/l3bug");
      await expect(form.locator('[data-slot="clock"] select')).toHaveValue("s5sports01/l2score/clock");
      await shot(page, `${theme}-59-flow-template-form`);

      await form.getByRole("button", { name: "Add" }).click();
      await expect(page).toHaveURL(/\/flows\/[a-z0-9]+$/);
    });
  });
}

// Control restructure: the side panel of a selected screen (layers, cues, automation), captured
// whole (a tall viewport, the panel element only) into review/control-restructure/.

const CR_OUT = join(process.cwd(), "review", "control-restructure");
mkdirSync(CR_OUT, { recursive: true });

async function panelShot(page: Page, name: string) {
  await page.waitForTimeout(600);
  // The panel down to its last child (the page is tall so nothing scrolls), not the empty rest.
  const side = page.getByRole("complementary", { name: "Selected screen" });
  const box = await side.boundingBox();
  const end = await side.evaluate((el) => (el.lastElementChild as HTMLElement).getBoundingClientRect().bottom);
  if (!box) throw new Error("no panel");
  await page.screenshot({ path: join(CR_OUT, `${name}.png`), clip: { x: box.x, y: box.y, width: box.width, height: Math.ceil(end - box.y) + 8 } });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, control restructure`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, true, [SCREEN1_DOC, EMPTY_DOC]);
      await page.setViewportSize({ width: 1440, height: 4600 });
    });

    // review/control-restructure/before-<theme>.png came from the pre-restructure code (the same
    // scenario with the old Flows section opened); these are the after shots.

    test("after: Screen 1, scoped, with the attention banner", async ({ page }) => {
      await open(page, "/control", restructureScenario(theme, SCREEN1));
      await expect(page.getByRole("button", { name: "Take Full-screen title" })).toBeVisible({ timeout: 15_000 });
      await expect(page.getByText("2 automations need attention")).toBeVisible();
      await expect(page.getByRole("button", { name: "Run Stinger" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Run Show intro" })).toHaveCount(0);
      await panelShot(page, `after-${theme}`);

      // Other rules open: the reasons inline, once.
      await page.locator('[data-row="__other"] .toggle').click();
      await expect(page.getByText("Field “headline” isn't on the layer “Full-screen title”")).toBeVisible();
      await panelShot(page, `after-${theme}-other-rules`);

      // Show all cues.
      await page.getByRole("button", { name: "Show all cues" }).first().click();
      await expect(page.getByRole("button", { name: "Run Show intro" })).toBeVisible();
      await panelShot(page, `after-${theme}-show-all-cues`);
    });

    test("automation strip at 280, 400 and 640 px panel widths", async ({ page }) => {
      const base = restructureScenario(theme, SCREEN1);
      for (const panelWidth of [280, 400, 640]) {
        await open(page, "/control", { ...base, control: { ...base.control!, panelWidth } });
        const other = page.locator('[data-row="__other"]');
        await expect(other).toBeVisible({ timeout: 15_000 });
        // "Other rules" keeps its full text; the summary gives way instead.
        const name = other.locator(".name");
        await expect(name).toHaveText("Other rules");
        const clipped = await name.evaluate((el) => el.scrollWidth > el.clientWidth + 0.5 || el.getBoundingClientRect().right > el.parentElement!.getBoundingClientRect().right + 0.5);
        expect(clipped, `"Other rules" clipped at ${panelWidth} px`).toBe(false);
        await panelShot(page, `after-${theme}-automation-${panelWidth}`);
      }
    });

    test("after: a recipe expanded with its aggregated warnings", async ({ page }) => {
      await open(page, "/control", restructureScenario(theme, EVENT_SCREEN));
      await page.locator('[data-recipe="rcp1event0"] .toggle').click();
      await expect(page.getByRole("region", { name: "Main stage: schedule" })).toBeVisible({ timeout: 15_000 });
      await expect(page.getByText("Affects When an item starts, set the strap's subtitle, When an item ends, clear the strap's subtitle")).toBeVisible();
      await panelShot(page, `after-${theme}-recipe-warnings`);
    });

    test("after: a screen with nothing aimed at it", async ({ page }) => {
      await open(page, "/control", restructureScenario(theme, EMPTY_DOC.id));
      await expect(page.getByText("No cues for this screen.")).toBeVisible({ timeout: 15_000 });
      await expect(page.getByText("Nothing runs on its own here.")).toBeVisible();
      await panelShot(page, `after-${theme}-empty`);
    });

    test("after: Fix pickers", async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      // The Sponsors screen with its slideshow's source deleted (its layer is still there).
      const m = restructureScenario(theme, SPONSORS_SCREEN);
      await open(page, "/control", { ...m, sourceDocs: (m.sourceDocs ?? []).filter((d) => d.id !== "src4sponso") });
      await expect(page.getByText("1 automation needs attention")).toBeVisible({ timeout: 15_000 });
      await page.getByRole("button", { name: "Fix Sponsors" }).click();
      const fix = page.getByRole("dialog", { name: "Fix Sponsors" });
      await expect(fix.getByLabel("New source for Sponsors")).toBeVisible();
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(CR_OUT, `after-${theme}-fix-source.png`) });
      await fix.getByLabel("New source for Sponsors").selectOption({ label: "Guests" });
      await fix.getByRole("button", { name: "Use this source" }).click();
      await expect(page.getByText(/automation needs attention/)).toHaveCount(0);
      await expect(page.locator('[data-recipe="rcp2spons0"]')).toBeVisible();

    });

    test("after: Fix with the layer gone", async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      // Screen 1: the slideshow layer is gone too, so Fix explains and offers Delete.
      await open(page, "/control", restructureScenario(theme, SCREEN1));
      await page.locator('[data-broken="rcp3slideA"]').getByRole("button", { name: /^Fix/ }).click();
      await expect(page.getByRole("dialog", { name: "Fix Sponsor slideshow" })).toBeVisible();
      await page.waitForTimeout(400);
      await page.screenshot({ path: join(CR_OUT, `after-${theme}-fix-layer.png`) });
    });
  });
}

// Style library (phase 1): the gallery filtered to Frame with a hover preview, the Legacy group
// opened, Preferences > Style & brand, and "Apply style: Frame" from the palette in Control (the
// on-air confirm, then the result toast with Undo).
for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, style library`, () => {
    const scenario = (): MockScenario => ({
      ...screensScenario(theme),
      styles: STYLES,
      projectStyle: "classic",
      brand: { accent: "#ff5a36", fontHeading: "Space Grotesk" },
    });

    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("gallery: Frame, a hover preview, Legacy", async ({ page }) => {
      await open(page, "/templates", scenario());
      await page.getByRole("button", { name: "New template" }).click();
      const dialog = page.getByRole("dialog");
      await expect(dialog.getByRole("heading", { name: "New template" })).toBeVisible();
      await dialog.getByRole("group", { name: "Style" }).getByRole("button", { name: "Frame" }).click();
      const lt = dialog.getByRole("option", { name: /^Lower third/ });
      await lt.hover();
      await expect(lt.locator("[data-preview]")).toBeAttached();
      await page.waitForTimeout(1200);
      await shot(page, `${theme}-50-gallery-frame`);

      await dialog.getByRole("group", { name: "Style" }).getByRole("button", { name: "All" }).click();
      await dialog.getByRole("button", { name: /Legacy/ }).click();
      await expect(dialog.getByRole("listbox", { name: "Legacy" })).toBeVisible();
      await dialog.getByRole("listbox", { name: "Legacy" }).scrollIntoViewIfNeeded();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-51-gallery-legacy`);
    });

    test("preferences: style & brand", async ({ page }) => {
      await open(page, "/preferences?section=style", scenario());
      await expect(page.getByLabel("Accent color picker")).toBeVisible();
      await page.getByLabel("Secondary color", { exact: true }).fill("#12");
      await shot(page, `${theme}-52-preferences-style`);
    });

    test("preferences: third-party licenses", async ({ page }) => {
      await open(page, "/preferences?section=about", scenario());
      const view = page.getByRole("button", { name: "View licenses" });
      await view.scrollIntoViewIfNeeded();
      await shot(page, `${theme}-66-preferences-about`);
      await view.click();
      const dialog = page.getByRole("dialog", { name: "Third-party licenses" });
      await expect(dialog.getByRole("heading", { name: /Core \(Rust\)/ })).toBeVisible();
      await shot(page, `${theme}-67-licenses`);
      await dialog.getByRole("searchbox", { name: "Search licenses" }).fill("xyflow");
      await dialog.getByRole("button", { name: /@xyflow\/svelte/ }).click();
      await expect(dialog.getByText("webkid GmbH", { exact: false })).toBeVisible();
      await shot(page, `${theme}-68-licenses-expanded`);
    });

    test("apply style to the project: confirm on air, then the toast", async ({ page }) => {
      await open(page, "/control", { ...scenario(), control: { selectedScreenId: "s1main01ab", liveEdit: "onTake" } });
      await expect(page.getByRole("button", { name: "Take Guest lower third" })).toBeVisible();
      await page.keyboard.press("Control+K");
      await expect(page.getByRole("dialog", { name: "Command palette" })).toBeVisible();
      // "Apply style…" opens its pick step (the styles), searched the same way.
      await page.keyboard.type("apply style");
      await expect(page.getByRole("option", { name: /Apply style…/ })).toBeVisible();
      await page.keyboard.press("Enter");
      await page.keyboard.type("frame");
      await expect(page.getByRole("option", { name: /^Frame/ })).toBeVisible();
      await page.keyboard.press("Enter");
      const confirm = page.getByRole("dialog", { name: /Apply Frame to the project/ });
      await expect(confirm).toBeVisible();
      await shot(page, `${theme}-53-apply-confirm`);
      await confirm.getByRole("button", { name: "Apply" }).click();
      await expect(page.getByText(/Frame applied · \d+ switched/)).toBeVisible();
      await shot(page, `${theme}-54-apply-toast`);
      await page.keyboard.press("Control+Z");
      await expect(page.getByText("Frame undone")).toBeVisible();
    });
  });
}

// ---- API sources -------------------------------------------------------------------------
for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, api sources`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("api editor: a test request, the mapping in the tree; a failing source", async ({ page }) => {
      await open(page, "/sources", apiScenario(theme));
      await page.getByRole("option", { name: /League table/ }).click();
      await page.getByRole("button", { name: "Test request" }).click();
      const tree = page.getByRole("tree", { name: "Response" });
      await expect(tree).toBeVisible();
      // A mapped value selected: the pick bar offers Add as column / Set path of.
      await tree.locator('[data-ptr="/standings/0/team"]').click();
      await page.keyboard.press("ArrowRight");
      await tree.locator('[data-ptr="/standings/0/team/name"]').click();
      await expect(page.getByRole("grid", { name: "Preview of League table" })).toBeVisible();
      await shot(page, `${theme}-70-api-editor`);

      await page.getByRole("grid", { name: "Preview of League table" }).scrollIntoViewIfNeeded();
      await shot(page, `${theme}-70b-api-preview`);

      await page.getByRole("option", { name: /Weather/ }).click();
      await expect(page.getByText("Secret missing on this machine — enter it again")).toBeVisible();
      await page.getByRole("button", { name: "Test request" }).click();
      await expect(page.getByRole("group", { name: "CSV columns" })).toBeVisible();
      await shot(page, `${theme}-71-api-status-error`);
    });

    test("control: API sources in the sources strip", async ({ page }) => {
      await open(page, "/control", apiScenario(theme));
      const strip = page.getByRole("region", { name: "Sources" });
      await expect(strip.locator('[data-source="api1league"] .api')).toContainText("Live");
      await strip.evaluate((el) => el.scrollIntoView({ block: "end" }));
      await page.waitForTimeout(800);
      await shot(page, `${theme}-72-control-api-sources`);
    });

    test("new source: API", async ({ page }) => {
      await open(page, "/sources", apiScenario(theme));
      await page.getByRole("button", { name: "New source" }).click();
      const dialog = page.getByRole("dialog", { name: "New source" });
      await dialog.getByRole("radio", { name: /^API/ }).check();
      await shot(page, `${theme}-73-new-api-source`);
    });

    test("Simple mode: a JSON source, a CSV link, New source", async ({ page }) => {
      await open(page, "/sources", { ...apiScenario(theme), prefsUiMode: "simple" });
      await page.getByRole("option", { name: /League table/ }).click();
      await expect(page.getByText("The request and mapping: available in Advanced mode")).toBeVisible();
      await shot(page, `${theme}-102-simple-api-json`);
      await page.getByRole("option", { name: /Weather/ }).click();
      await expect(page.getByRole("heading", { name: "Link" })).toBeVisible();
      await shot(page, `${theme}-103-simple-csv-link`);
      await page.getByRole("button", { name: "New source" }).click();
      const dialog = page.getByRole("dialog", { name: "New source" });
      await dialog.getByRole("radio", { name: /^CSV link/ }).check();
      await shot(page, `${theme}-104-simple-new-source`);
    });
  });
}

// ---- Command palette ---------------------------------------------------------------------------
// In Control: live actions first, a prefix scope ("@" screens), a confirm row (Panic: "Press Enter
// again to confirm" in --danger) and a follow-up sub-list ("Set variant…"). Preferences > Keyboard
// shortcuts with a conflict (the theme toggle rebound to F1, which a layer of Main program uses),
// and a key being recorded.
for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, command palette`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("control: live first, a scope, a confirm row, a follow-up step", async ({ page }) => {
      await open(page, "/control", {
        ...variantsScenario(theme),
        control: { selectedScreenId: "s1main01ab", gridColumns: 2, liveEdit: "onTake", hotkeys: { "s1main01ab/l2guest": "F1" } },
      });
      await expect(page.getByRole("button", { name: "Take Guest lower third" })).toBeVisible();
      const dialog = page.getByRole("dialog", { name: "Command palette" });
      const input = dialog.getByRole("combobox");

      await page.keyboard.press("Control+K");
      await expect(input).toBeFocused();
      await expect(dialog.getByRole("group").first()).toBeVisible();
      await shot(page, `${theme}-80-palette-control`);

      await page.keyboard.type("@");
      await shot(page, `${theme}-81-palette-scope`);

      await input.fill("panic");
      await page.keyboard.press("Enter");
      await expect(dialog.getByText("Press Enter again to confirm")).toBeVisible();
      await shot(page, `${theme}-82-palette-confirm`);
      await page.keyboard.press("ArrowDown");

      await input.fill("set variant");
      await page.keyboard.press("Enter");
      await expect(dialog.getByRole("listbox")).not.toHaveAttribute("aria-label", "Commands");
      await shot(page, `${theme}-83-palette-follow-up`);
      await page.keyboard.press("Escape");
      await page.keyboard.press("Escape");
      await expect(dialog).toBeHidden();
    });

    test("keyboard shortcuts: a conflict, recording a key", async ({ page }) => {
      await open(page, "/preferences?section=shortcuts", {
        ...variantsScenario(theme),
        control: { selectedScreenId: "s1main01ab", gridColumns: 2, liveEdit: "onTake", hotkeys: { "s1main01ab/l2guest": "F1" } },
        keybindings: { "shell.theme": ["F1"], "nav.flows": ["Ctrl+5", "Ctrl+Alt+F"] },
      });
      const panel = page.getByRole("region", { name: "1 conflict" });
      await expect(panel).toBeVisible();
      await page.getByRole("heading", { name: "Keyboard shortcuts" }).evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.locator(".scroller").evaluate((el) => el.scrollBy(0, -24));
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-84-shortcuts-conflict`);

      await panel.getByRole("button", { name: "Change Toggle light / dark theme" }).click();
      await expect(page.getByText("Press the new keys. Esc cancels.")).toBeVisible();
      await page.getByText("Press the new keys. Esc cancels.").evaluate((el) => el.scrollIntoView({ block: "center" }));
      await shot(page, `${theme}-85-shortcuts-recording`);
      await page.keyboard.press("Control+Alt+T");
      await expect(panel).toBeHidden();
    });
  });
}

// ---- Resizing scales (88) -------------------------------------------------------------------------
// A lower third and a leaderboard panel placed at their design boxes as reflow layers (no sizing,
// as in an existing project), each dragged by a corner handle to about half size and to about
// 1.5x: the graphic itself scales with the box (it becomes Scale to fit) and the handles stay on
// the graphic's edges.

/** A preset's design box on 1920×1080 (as a new layer gets it). */
function designBox(preset: string) {
  const d = JSON.parse(readFileSync(join(process.cwd(), "presets", "templates", preset, "template.json"), "utf8")).design;
  const x = Math.round(Math.min(Math.max(d.x ?? (1920 - d.w) / 2, 0), 1920 - d.w));
  const y = Math.round(Math.min(Math.max(d.y ?? (1080 - d.h) / 2, 0), 1080 - d.h));
  return { x, y, w: d.w as number, h: d.h as number };
}

const RESIZE_DOC: ScreenDoc = {
  id: "s9resize01",
  name: "Resize",
  width: 1920,
  height: 1080,
  background: "transparent",
  layers: [
    {
      id: "l1guest",
      name: "Guest lower third",
      kind: "template",
      templateId: "t1guestbar",
      rect: designBox("classic-bar"),
      fields: { name: "Grace Hopper", title: "Rear Admiral, computer scientist" },
      vars: {},
    },
    {
      id: "l2board",
      name: "Standings",
      kind: "template",
      templateId: "t7standing",
      rect: designBox("leaderboard"),
      fields: {
        title: "Standings",
        rows: JSON.stringify([
          { name: "Ingrid Vale", team: "Nordlys", score: "128" },
          { name: "Jonas Berg", team: "Fjell", score: "117" },
          { name: "Mari Lund", team: "Brygge", score: "96" },
          { name: "Ola Nordmann", team: "Kite & Key", score: "74" },
        ]),
      },
      vars: {},
    },
  ],
  updatedAt: new Date(Date.now() - 3 * 60_000).toISOString(),
};

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, resizing scales`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server, false, [RESIZE_DOC]);
    });

    test("a corner handle scales the graphic: half size and 1.5x", async ({ page }) => {
      await open(page, `/screens/${RESIZE_DOC.id}`, {
        ...screensScenario(theme),
        screenDocs: [...SCREEN_DOCS, RESIZE_DOC],
        onAirLayers: [...ON_AIR, ...RESIZE_DOC.layers.map((l) => `${RESIZE_DOC.id}/${l.id}`)],
      });
      const out = page.frameLocator('iframe[title="Screen output"]');
      await expect(out.locator('[data-state="in"]')).toHaveCount(2, { timeout: 15_000 });
      const inspector = page.getByRole("complementary", { name: "Inspector" });

      /** The rendered graphic's root box (canvas px) of a layer, by its position in the output. */
      const root = (i: number) =>
        out.locator("[data-state]").nth(i).evaluate((e) => {
          const r = e.getBoundingClientRect();
          return { w: r.width, h: r.height };
        });

      /** Drags the selected layer's handle so its box becomes k × the start box (the opposite corner fixed). */
      async function scaleBy(name: RegExp, handle: "ne" | "nw", k: number, base: { x: number; y: number; width: number; height: number }) {
        const box = page.getByRole("button", { name });
        const hb = await box.locator(`[data-handle="${handle}"]`).boundingBox();
        const b = await box.boundingBox();
        if (!hb || !b) throw new Error("no handle");
        const sx = hb.x + hb.width / 2;
        const sy = hb.y + hb.height / 2;
        // The fixed corner and where the dragged one should end.
        const fx = handle === "ne" ? b.x : b.x + b.width;
        const fy = b.y + b.height;
        const tx = handle === "ne" ? fx + base.width * k : fx - base.width * k;
        const ty = fy - base.height * k;
        await page.mouse.move(sx, sy);
        await page.mouse.down();
        for (let i = 1; i <= 14; i++) {
          await page.mouse.move(sx + ((tx - sx) * i) / 14, sy + ((ty - sy) * i) / 14);
          await page.waitForTimeout(16);
        }
        await page.mouse.up();
        await page.waitForTimeout(700);
      }

      for (const [i, name, handle, label] of [
        [0, /^Guest lower third,/, "ne", "lt"],
        [1, /^Standings,/, "nw", "board"],
      ] as const) {
        const box = page.getByRole("button", { name });
        await box.click();
        await page.waitForTimeout(400);
        const base = await box.boundingBox();
        if (!base) throw new Error("no layer box");
        const r0 = await root(i);
        await shot(page, `${theme}-88-resize-scale-${label}-before`);

        await scaleBy(name, handle, 0.5, base);
        await expect(inspector.locator("[data-scale]")).toHaveText(/^(4[5-9]|5[0-5]) %$/);
        const half = await root(i);
        expect(half.w / r0.w).toBeGreaterThan(0.42);
        expect(half.w / r0.w).toBeLessThan(0.58);
        expect(half.h / r0.h).toBeGreaterThan(0.42);
        expect(half.h / r0.h).toBeLessThan(0.58);
        await shot(page, `${theme}-88-resize-scale-${label}-half`);

        await scaleBy(name, handle, 1.5, base);
        await expect(inspector.locator("[data-scale]")).toHaveText(/^1(4[0-9]|5[0-9]) %$/);
        const large = await root(i);
        expect(large.w / r0.w).toBeGreaterThan(1.4);
        expect(large.h / r0.h).toBeGreaterThan(1.4);
        await shot(page, `${theme}-88-resize-scale-${label}-large`);
      }
    });
  });
}

// MIDI & OSC: Preferences (devices with status and LED feedback; this project's mappings grouped,
// with a conflict; Learn waiting for a message), Server > OSC (allowlist, on-air status target, the
// address list with real ids), and Control (MIDI chips, the right-click menu, Assign MIDI…).
const MIDI_SCENARIO = (theme: "dark" | "light"): MockScenario => ({
  ...variantsScenario(theme),
  control: { selectedScreenId: "s1main01ab", gridColumns: 2, liveEdit: "onTake", hotkeys: { "s1main01ab/l2guest": "F2" } },
  midiDevices: [
    { name: "APC40 mkII", input: true, output: true, active: true },
    { name: "Launchpad X", input: true, output: true, active: true },
    { name: "loopMIDI Port", input: true, output: true, active: false },
  ],
  midiPrefs: {
    inputs: { "nanoKONTROL2": { enabled: true } },
    feedback: { "APC40 mkII": { output: null, profile: "apc40" } },
  },
  controls: {
    mappings: [
      { id: "m1guest000", source: { device: "APC40 mkII", channel: 1, kind: "note", number: 53 }, target: { kind: "command", commandId: "live.toggle:s1main01ab/l2guest" }, feedback: true },
      { id: "m2strap000", source: { device: "APC40 mkII", channel: 1, kind: "note", number: 54 }, target: { kind: "command", commandId: "live.toggle:s1main01ab/l1strap" }, feedback: true },
      { id: "m3intro000", source: { device: "APC40 mkII", channel: 1, kind: "note", number: 82 }, target: { kind: "command", commandId: "cue.go:mac1intro0" }, feedback: true },
      { id: "m4next0000", source: { device: null, channel: 1, kind: "note", number: 82 }, target: { kind: "command", commandId: "cue.go:mac2nextg0" }, feedback: true },
      { id: "m5panic000", source: { device: "APC40 mkII", channel: 1, kind: "note", number: 98 }, target: { kind: "command", commandId: "live.panic" }, feedback: false },
      { id: "m6fader000", source: { device: "APC40 mkII", channel: 1, kind: "cc", number: 7 }, target: { kind: "value", screenId: "s1main01ab", layerId: "l3bug", target: { kind: "opacity" } }, feedback: false },
    ],
  },
  osc: { enabled: true, port: 9000, allow: ["192.168.1.31", "192.168.1.40"], feedback: { host: "192.168.1.31", port: 9001 } },
  midiLearnNext: { device: "Launchpad X", channel: 1, kind: "note", number: 11, value: 127 },
  midiLearnDelayMs: null,
});

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, midi and osc`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("preferences: devices, mappings with a conflict, learning", async ({ page }) => {
      await open(page, "/preferences?section=midi", MIDI_SCENARIO(theme));
      const panel = page.getByRole("region", { name: "1 conflict" });
      await expect(panel).toBeVisible();
      await expect(page.getByText("Connected").first()).toBeVisible();
      await page.getByRole("heading", { name: "MIDI & OSC" }).evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.locator(".scroller").evaluate((el) => el.scrollBy(0, -24));
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-90-midi-devices-mappings`);

      await page.getByRole("button", { name: "Learn a new control for Take or clear Breaking strap (Main program)" }).click();
      await expect(page.getByText("Press a key, pad or fader…")).toBeVisible();
      await page.getByText("Press a key, pad or fader…").evaluate((el) => el.scrollIntoView({ block: "center" }));
      await shot(page, `${theme}-91-midi-learning`);
      await page.keyboard.press("Escape");
      await expect(page.getByText("Press a key, pad or fader…")).toBeHidden();
    });

    test("preferences: server with OSC", async ({ page }) => {
      await open(page, "/preferences?section=server", MIDI_SCENARIO(theme));
      await expect(page.getByText("/kinetra/layer/s1main01ab/", { exact: false }).first()).toBeVisible();
      await expect(page.getByRole("status").filter({ hasText: "Listening on" })).toBeVisible();
      await page.getByRole("heading", { name: "Server" }).evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.locator(".scroller").evaluate((el) => el.scrollBy(0, -24));
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-92-osc-server`);
    });

    test("control: MIDI chips, the context menu, Assign MIDI", async ({ page }) => {
      await open(page, "/control", { ...MIDI_SCENARIO(theme), midiLearnDelayMs: 900 });
      const row = page.locator('li[data-layer="l2guest"]');
      await expect(row.locator(".chip")).toHaveAttribute("title", "Note 53 · Ch 1 · APC40 mkII");
      await row.locator(".names").click({ button: "right" });
      const menu = page.getByRole("menu", { name: "Actions for Guest lower third" });
      await expect(menu).toBeVisible();
      await expect(menu.getByRole("menuitem", { name: "Remove MIDI" })).toBeVisible();
      await shot(page, `${theme}-93-control-context-menu`);

      await menu.getByRole("menuitem", { name: /Assign MIDI/ }).click();
      const dialog = page.getByRole("dialog", { name: "Assign MIDI to Guest lower third" });
      await expect(dialog.getByText("Press a key, pad or fader…")).toBeVisible();
      await shot(page, `${theme}-94-control-assign-waiting`);
      await expect(dialog.getByText("Note 11 · Ch 1 · Launchpad X")).toBeVisible();
      await shot(page, `${theme}-95-control-assign-done`);
      await dialog.getByRole("button", { name: "Done" }).click();
      await expect(dialog).toBeHidden();

      // The keyboard opens it too, on the focused cue.
      const cue = page.locator('li[data-cue="mac2nextg0"]');
      await cue.getByRole("button", { name: "Run Next guest" }).focus();
      await page.keyboard.press("Shift+F10");
      await expect(page.getByRole("menu", { name: "Actions for Next guest" })).toBeVisible();
      await page.keyboard.press("Escape");
    });

    test("layout editor: opacity in the Inspector, the Layers panel menu", async ({ page }) => {
      await open(page, "/screens/s1main01ab", MIDI_SCENARIO(theme));
      await page.getByRole("button", { name: /^LIVE bug,/ }).click();
      const inspector = page.getByRole("complementary", { name: "Inspector" });
      await inspector.getByRole("slider", { name: "Opacity slider" }).fill("60");
      await expect(inspector.getByLabel("Opacity", { exact: true })).toHaveValue("60");
      await page.locator('aside[aria-label="Layers"] li[data-layer="l3bug"] .main').click({ button: "right" });
      await expect(page.getByRole("menu", { name: "Actions for LIVE bug" })).toBeVisible();
      await shot(page, `${theme}-96-editor-opacity-menu`);
      await page.keyboard.press("Escape");
    });
  });
}

// ---- Display outputs: Preferences › Outputs and Control's outputs strip -------------------------
const DISPLAYS = (keyHz = 60): DisplayInfo[] => [
  { id: "disp-main", name: "DELL U2720Q", edidKey: "DEL-A0F1-1", x: 0, y: 0, width: 2560, height: 1440, refreshHz: 60, primary: true, hasMainWindow: true },
  { id: "disp-fill", name: "ASUS PA248QV", edidKey: "AUS-24A1-7", x: 2560, y: 0, width: 1920, height: 1080, refreshHz: 60, primary: false, hasMainWindow: false },
  { id: "disp-key", name: "ASUS PA248QV", edidKey: "AUS-24A1-9", x: 4480, y: 0, width: 1920, height: 1080, refreshHz: keyHz, primary: false, hasMainWindow: false },
];
const OUTPUTS: OutputConfig[] = [
  {
    id: "out1fill00",
    kind: "display",
    name: "Program fill",
    enabled: true,
    display: { id: "disp-fill", name: "ASUS PA248QV", edidKey: "AUS-24A1-7", width: 1920, height: 1080 },
    screenId: "s1main01ab",
    mode: "fill",
    background: { kind: "black" },
    keyOf: null,
    alwaysOnTop: true,
  },
  {
    id: "out2key000",
    kind: "display",
    name: "Program key",
    enabled: true,
    display: { id: "disp-key", name: "ASUS PA248QV", edidKey: "AUS-24A1-9", width: 1920, height: 1080 },
    screenId: null,
    mode: "key",
    background: { kind: "black" },
    keyOf: "out1fill00",
    alwaysOnTop: true,
  },
  {
    id: "out3conf00",
    kind: "display",
    name: "Confidence",
    enabled: true,
    display: { id: "disp-gone", name: "LG 24MB35", edidKey: "GSM-5A3B-2", width: 1920, height: 1080 },
    screenId: "s4confid01",
    mode: "fill",
    background: { kind: "color", color: "#1f2a44" },
    keyOf: null,
    alwaysOnTop: false,
  },
];

for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, display outputs`, () => {
    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("preferences: outputs empty, configured (frame-locked, disconnected), not frame-locked with a pattern", async ({ page }) => {
      await page.setViewportSize({ width: 1100, height: 1000 });
      await open(page, "/preferences?section=outputs", { ...screensScenario(theme), displays: DISPLAYS() });
      await expect(page.getByRole("list", { name: "Displays" })).toBeVisible();
      await expect(page.getByText("Show a screen on a display, as fill or key.")).toBeVisible();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-100-outputs-empty`);

      // Add output picks the first free display that Kinetra's window isn't on.
      await page.getByRole("button", { name: "Add output" }).click();
      const row = page.locator("li[data-output]");
      await expect(row).toHaveCount(1);
      await expect(row.getByLabel("Output name")).toHaveValue("Output 1");
      await expect(row.getByRole("combobox").first()).toHaveValue("disp-fill");

      await open(page, "/preferences?section=outputs", { ...screensScenario(theme), displays: DISPLAYS(), outputs: OUTPUTS });
      const outs = page.getByRole("list", { name: "Outputs" });
      await expect(outs.getByText("Frame-locked")).toHaveCount(2);
      await expect(outs.getByText("Display disconnected")).toBeVisible();
      await expect(outs.getByRole("option", { name: "LG 24MB35 (disconnected)" })).toBeAttached();
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-101-outputs-configured`);

      await open(page, "/preferences?section=outputs", { ...screensScenario(theme), displays: DISPLAYS(59), outputs: OUTPUTS });
      await expect(outs.getByText("Not frame-locked: displays differ")).toHaveCount(2);
      const fill = page.locator('li[data-output="out1fill00"]');
      await fill.getByRole("button", { name: "Bars" }).click();
      await expect(fill.getByRole("button", { name: "Bars" })).toHaveAttribute("aria-pressed", "true");
      await expect(fill.getByRole("button", { name: "Back to screen" })).toBeVisible();
      await expect(fill.getByRole("status")).toHaveText(/Test pattern/);
      await page.mouse.move(2, 2);
      await shot(page, `${theme}-102-outputs-pattern-not-locked`);
    });

    test("control: outputs strip with a disconnected display", async ({ page }) => {
      await open(page, "/control", {
        ...screensScenario(theme),
        displays: DISPLAYS(),
        outputs: OUTPUTS,
        control: { selectedScreenId: "s1main01ab", gridColumns: 2 },
      });
      const strip = page.getByRole("group", { name: "Outputs" });
      await expect(strip.getByText("Program fill · Main program")).toBeVisible();
      await expect(strip.getByText("Confidence: display disconnected")).toBeVisible();
      await page.waitForTimeout(1200);
      await shot(page, `${theme}-103-control-outputs-strip`);
    });
  });
}

for (const theme of ["dark", "light"] as const) {
  test.describe(`live polls (${theme})`, () => {
    test("polls: an open poll with live results, the quota, the voting link", async ({ page }) => {
      await open(page, "/polls", pollsScenario(theme));
      await expect(page.getByLabel("Your plan's poll limits")).toContainText("Open now 1 of 3");
      await expect(page.getByRole("region", { name: "Results" })).toContainText("1,918 votes");
      await shot(page, `${theme}-110-polls-open`);
    });

    test("polls: a closed poll awaiting deletion", async ({ page }) => {
      await open(page, "/polls", pollsScenario(theme));
      await page.getByRole("option", { name: /Player of the match/ }).click();
      await expect(page.getByText(/Deleted in \d+ h/)).toBeVisible();
      await shot(page, `${theme}-111-polls-closed`);
    });

    test("polls: new poll", async ({ page }) => {
      await open(page, "/polls", pollsScenario(theme));
      await page.getByRole("button", { name: "New poll", exact: true }).click();
      await page.getByLabel("Question").fill("Who scores next?");
      await page.getByLabel("Answer 1", { exact: true }).fill("Home");
      await page.getByLabel("Answer 2", { exact: true }).fill("Away");
      await shot(page, `${theme}-112-polls-new`);
    });

    test("polls: Free, the lock", async ({ page }) => {
      await open(
        page,
        "/polls",
        pollsScenario(theme, {
          polls: [],
          pollQuota: null,
          pollSources: [],
          license: { plan: "free", access: "free", entitlements: ["core.app"], features: { ai: false, polls: false, control: false } },
        }),
      );
      await page.getByRole("button", { name: "New poll: needs Kinetra Pro" }).first().click();
      await expect(page.getByText("New poll is part of Kinetra Pro").first()).toBeVisible();
      await shot(page, `${theme}-114-polls-free`);
    });

    test("sources: a poll source", async ({ page }) => {
      await open(page, "/sources", pollsScenario(theme));
      await page.getByRole("option", { name: /Halftime poll/ }).click();
      await expect(page.getByRole("region", { name: "Results" })).toBeVisible();
      await shot(page, `${theme}-115-sources-poll`);
    });
  });
}

// Remote control (step 2): Control's chip with its panel open (people, lock, recent actions) and
// the cue strip's Next; Preferences › Network & API › Remote control with Connection details and
// a Test connection result.
for (const theme of ["dark", "light"] as const) {
  test.describe(`${theme}, remote control`, () => {
    const now = Date.UTC(2026, 9, 13, 18, 0);
    const scenario = (): MockScenario => ({
      ...screensScenario(theme),
      flows: FLOWS,
      cueNext: "mac3break0",
      control: { selectedScreenId: "s1main01ab", liveEdit: "onTake" },
      remote: {
        on: true,
        state: "live",
        runId: "0193b2c4-5d6e-7f80-9a1b-2c3d4e5f6a7b",
        resumed: false,
        hasShowId: true,
        rttMs: 38,
        serverVersion: "kinetra-cloud 0.9.2",
        crewUrl: "https://control.kinetra.live/s/0193/crew",
        controllerUrl: "https://control.kinetra.live/s/0193",
        crew: [
          { id: "u1", name: "Kari Nordmann", role: "operator", anonymous: false, sinceMs: now - 3_600_000 },
          { id: "u2", name: "Jonas Berg", role: "data_editor", anonymous: false, sinceMs: now - 1_800_000 },
          { id: null, name: "Guest 3", role: "viewer", anonymous: true, sinceMs: now - 600_000 },
        ],
      },
      remoteActivity: [
        { at: now - 60_000, name: "Kari Nordmann", role: "operator", text: "Kari Nordmann took Guest lower third" },
        { at: now - 240_000, name: "Jonas Berg", role: "data_editor", text: "Jonas Berg updated Name, Title on Guest lower third" },
      ],
    });

    test.beforeEach(async ({ page, context, server }) => {
      await mirrorScreens(page, context, server);
    });

    test("control: the chip, its panel and Next in the cue strip", async ({ page }) => {
      await open(page, "/control", scenario());
      const chip = page.getByRole("button", { name: "Remote control: 3 connected" });
      await expect(chip).toBeVisible();
      await page.getByRole("button", { name: "Show all cues" }).click();
      await expect(page.getByText("Next", { exact: true })).toBeVisible();
      await shot(page, `${theme}-90-remote-chip`);
      await chip.click();
      await expect(page.getByText("3 remote users connected")).toBeVisible();
      await expect(page.getByText("Kari Nordmann took Guest lower third")).toBeVisible();
      await shot(page, `${theme}-91-remote-panel`);
    });

    test("control at 1024 px: the collapsed toolbar and its More menu", async ({ page }) => {
      await page.setViewportSize({ width: 1024, height: 768 });
      await open(page, "/control", scenario());
      const more = page.getByRole("button", { name: "More Control options" });
      await expect(more).toBeVisible();
      await page.waitForTimeout(300);
      await shot(page, `${theme}-94-control-1024`);
      await more.click();
      await expect(page.getByRole("menuitemcheckbox", { name: "Grid view" })).toBeVisible();
      await shot(page, `${theme}-95-control-1024-more`);
    });

    test("preferences: remote control and connection details", async ({ page }) => {
      await open(page, "/preferences?section=remote", scenario());
      await expect(page.getByRole("heading", { name: "Connection details" })).toBeVisible();
      await page.getByRole("button", { name: "Test connection" }).click();
      await expect(page.getByText("Connected", { exact: true })).toBeVisible();
      await page.locator("#remote").scrollIntoViewIfNeeded();
      await shot(page, `${theme}-92-preferences-remote`);
      await page.getByRole("heading", { name: "Connection details" }).scrollIntoViewIfNeeded();
      await shot(page, `${theme}-93-preferences-remote-details`);
    });
  });
}
