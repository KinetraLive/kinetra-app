// Data editing against the real server and renderer: what the spreadsheet drawer and the Data
// table editor send. A list field edit (`set_layer_fields`, the drawer's path) follows Studio mode
// (Preview first, the take brings it to Program) and Live edit (pending on take, or immediate); a
// data table linked by list layers on two screens updates both on save; a pasted-and-mapped table
// keeps its column keys and current row; the Control panel width is clamped and survives a reopen.
// Every test builds its own screens; the shared test screen is untouched.

import type { Page } from "@playwright/test";
import { expect, isolatedTest, test, type Fixture } from "./fixture";
import {
  layerView,
  op,
  openBoth,
  openBus,
  studio,
  studioOff,
} from "./studio-helpers";
import type {
  ScreenDoc,
  ScreenLayer,
  SourceDoc,
  UiState,
} from "../src/lib/types";

const FULL = { x: 0, y: 0, w: 1920, h: 1080 };
/** Rows on screen (rows leaving with their out-animation excluded). */
const names = (page: Page) =>
  page.locator('.lb__row:not([data-row="out"]) .lb__name');

const ids: Record<string, string> = {};
async function preset(live: Fixture, id: string): Promise<string> {
  return (ids[id] ||= await live.cmd(`addtemplate ${id}`));
}

let seq = 0;
async function screenWith(
  live: Fixture,
  layer: ScreenLayer,
): Promise<ScreenDoc> {
  const doc: ScreenDoc = {
    id: `de${Date.now().toString(36)}${seq++}`,
    name: "Data editing",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  return doc;
}

const layer = (
  templateId: string,
  fields: Record<string, string>,
  bindings?: ScreenLayer["bindings"],
): ScreenLayer => ({
  id: "l1",
  name: "Board",
  kind: "template",
  templateId,
  rect: FULL,
  fields,
  vars: {},
  ...(bindings ? { bindings } : {}),
});

/** A list value of `n` rows named `<prefix>1`…; what the drawer sends. */
const board = (prefix: string, n: number) =>
  JSON.stringify(
    Array.from({ length: n }, (_, i) => ({
      name: `${prefix}${i + 1}`,
      team: `T${i + 1}`,
      score: String(100 - i),
    })),
  );
const expected = (prefix: string, n: number) =>
  Array.from({ length: n }, (_, i) => `${prefix}${i + 1}`);

const COLUMNS = [
  { key: "name", label: "Name" },
  { key: "team", label: "Team" },
  { key: "score", label: "Score" },
];

test.describe("live data", () => {
  test.beforeEach(async ({ live }) => {
    await studioOff(live);
    await live.cmd("liveedit onTake");
  });
  test.afterEach(async ({ live }) => {
    await studioOff(live);
    await live.cmd("liveedit onTake");
  });

  test("a list edit lands in Preview under Studio mode; off it follows Live edit (on take, immediate)", async ({
    page,
    context,
    live,
  }) => {
    const doc = await screenWith(
      live,
      layer(await preset(live, "leaderboard"), {
        title: "Standings",
        rows: board("A", 2),
      }),
    );
    const { prog, pv } = await openBoth(page, context, live.base, doc.id);
    await live.cmd(`layershow ${doc.id} l1`);
    await expect(names(prog)).toHaveText(expected("A", 2));
    await expect(names(pv)).toHaveText(expected("A", 2));

    // Studio mode: the drawer's edit goes to the Preview only.
    await studio(live);
    await live.cmd(
      `setfields ${doc.id} l1 ${JSON.stringify({ rows: board("B", 4) })}`,
    );
    await expect(names(pv)).toHaveText(expected("B", 4));
    await prog.waitForTimeout(300);
    await expect(names(prog)).toHaveText(expected("A", 2));
    // The take brings it to Program.
    await op(live, doc.id, { op: "take" });
    await expect(names(prog)).toHaveText(expected("B", 4));

    // Studio off, Live edit "On take": the on-air layer holds the edit as pending.
    await studioOff(live);
    await live.cmd(
      `setfields ${doc.id} l1 ${JSON.stringify({ rows: board("C", 3) })}`,
    );
    await expect
      .poll(async () => (await layerView(live, doc.id, "l1")).pending)
      .toBe(true);
    await prog.waitForTimeout(300);
    await expect(names(prog)).toHaveText(expected("B", 4));
    await live.cmd(`layershow ${doc.id} l1`);
    await expect(names(prog)).toHaveText(expected("C", 3));
    expect(await layerView(live, doc.id, "l1")).toMatchObject({
      onAir: true,
      pending: false,
    });

    // Immediate: on air at once.
    await live.cmd("liveedit immediate");
    await live.cmd(
      `setfields ${doc.id} l1 ${JSON.stringify({ rows: board("D", 5) })}`,
    );
    await expect(names(prog)).toHaveText(expected("D", 5));
    expect(await layerView(live, doc.id, "l1")).toMatchObject({
      onAir: true,
      pending: false,
    });
  });

  test("a data table linked by list layers on two screens: saving it updates both renderers", async ({
    page,
    context,
    live,
  }) => {
    await live.cmd("liveedit immediate");
    const rows = [
      { name: "Ada", team: "A", score: "12" },
      { name: "Grace", team: "B", score: "40" },
    ];
    const src = await live.cmd(
      `addsource ${JSON.stringify({ name: "Standings", columns: COLUMNS, rows })}`,
    );
    const tpl = await preset(live, "leaderboard");
    const linked = { rows: { sourceId: src, column: "" } };
    const one = await screenWith(live, layer(tpl, { title: "One" }, linked));
    const two = await screenWith(live, layer(tpl, { title: "Two" }, linked));
    const other = await context.newPage();
    await openBus(page, live.base, one.id, "program");
    await openBus(other, live.base, two.id, "program");
    await live.cmd(`layershow ${one.id} l1`);
    await live.cmd(`layershow ${two.id} l1`);
    await expect(names(page)).toHaveText(["Ada", "Grace"]);
    await expect(names(other)).toHaveText(["Ada", "Grace"]);

    // The Data table editor's save: an edited cell, a new row, a reorder.
    const doc = JSON.parse(await live.cmd(`getsource ${src}`)) as SourceDoc;
    if (doc.kind !== "text") throw new Error("not a data table");
    doc.rows = [
      { name: "Hedy", team: "C", score: "7" },
      { name: "Grace Hopper", team: "B", score: "41" },
      { name: "Ada", team: "A", score: "12" },
    ];
    await live.cmd(`savesource ${JSON.stringify(doc)}`);
    const want = ["Hedy", "Grace Hopper", "Ada"];
    await expect(names(page)).toHaveText(want);
    await expect(names(other)).toHaveText(want);
  });

  test("a pasted-and-mapped table keeps its column keys and current row", async ({
    page,
    live,
  }) => {
    await live.cmd("liveedit immediate");
    const columns = [
      { key: "name", label: "Name" },
      { key: "title", label: "Title" },
    ];
    const src = await live.cmd(
      `addsource ${JSON.stringify({
        name: "Guests",
        columns,
        rows: [
          { name: "Ada", title: "Analyst" },
          { name: "Grace", title: "Admiral" },
          { name: "Hedy", title: "Inventor" },
        ],
      })}`,
    );
    expect(await live.cmd(`stepsource ${src} 1`)).toBe("1");
    const doc = await screenWith(
      live,
      layer(
        await preset(live, "classic-bar"),
        { name: "", title: "" },
        {
          name: { sourceId: src, column: "name" },
          title: { sourceId: src, column: "title" },
        },
      ),
    );
    const ws = page.waitForEvent("websocket");
    await page.goto(`${live.base}/screen/${doc.id}`);
    await (
      await ws
    ).waitForEvent("framereceived", (f) =>
      String(f.payload).includes('"t":"snapshot"'),
    );
    await live.cmd(`layershow ${doc.id} l1`);
    await expect(page.locator(".cbar__name")).toHaveText("Grace");

    // Pasted "Role<TAB>Guest" columns, mapped Role → title and Guest → name, replacing every row:
    // the rows arrive keyed by the table's own column keys.
    const pasted = [
      ["Host", "Katherine"],
      ["Engineer", "Margaret"],
      ["Pioneer", "Radia"],
      ["Scientist", "Barbara"],
    ];
    const map = ["title", "name"];
    const before = JSON.parse(await live.cmd(`getsource ${src}`)) as SourceDoc;
    if (before.kind !== "text") throw new Error("not a data table");
    before.rows = pasted.map((cells) =>
      Object.fromEntries(cells.map((v, i) => [map[i], v])),
    );
    const saved = JSON.parse(
      await live.cmd(`savesource ${JSON.stringify(before)}`),
    ) as SourceDoc;
    if (saved.kind !== "text") throw new Error("not a data table");

    expect(saved.columns.map((c) => c.key)).toEqual(["name", "title"]);
    expect(saved.index).toBe(1);
    expect(saved.rows[1]).toEqual({ name: "Margaret", title: "Engineer" });
    const back = JSON.parse(await live.cmd(`getsource ${src}`)) as SourceDoc;
    expect(back).toMatchObject({ kind: "text", index: 1, columns });
    // The bound layer shows the new current row.
    await expect(page.locator(".cbar__name")).toHaveText("Margaret");
    await expect(page.locator(".cbar__title")).toHaveText("Engineer");
  });
});

isolatedTest(
  "the Control panel width is clamped to 280–640 and survives a project reopen",
  async ({ own }) => {
    const set = async (panelWidth: number) => {
      const ui = JSON.parse(await own.cmd("uistate")) as UiState;
      ui.control.panelWidth = panelWidth;
      return (
        JSON.parse(await own.cmd(`setuistate ${JSON.stringify(ui)}`)) as UiState
      ).control.panelWidth;
    };
    expect(await set(100)).toBe(280);
    expect(await set(9999)).toBe(640);
    expect(await set(452)).toBe(452);
    expect(await set(9999)).toBe(640);

    const dir = await own.cmd("dir");
    await own.cmd(`openproject ${dir}`);
    expect(
      (JSON.parse(await own.cmd("uistate")) as UiState).control.panelWidth,
    ).toBe(640);
  },
);
