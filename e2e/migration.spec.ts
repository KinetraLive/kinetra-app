// Flow migration against the real server: an old project (recipe rules, macros.json and a macro
// hotkey) opened on a fixture server of its own becomes flows, with a `pre-migrate` backup, and the
// migrated macro runs as a cue.

import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, isolatedTest } from "./fixture";
import { openScreen, roots, twoLayers } from "./flow-helpers";
import type { FlowInfo, UiState } from "../src/lib/types";

isolatedTest("an old project's rules and macros become flows on open", async ({ page, own }) => {
  const fixtureDir = await own.cmd("dir");
  const dir = mkdtempSync(join(tmpdir(), "kinetra-legacy-"));
  const projectFile = JSON.parse(readFileSync(join(fixtureDir, "project.json"), "utf8")) as Record<string, unknown>;
  writeFileSync(join(dir, "project.json"), JSON.stringify({ ...projectFile, name: "Legacy" }));
  // A macro (take, wait, clear) with a hotkey, and a recipe with an enabled rule (schemaVersion 1).
  writeFileSync(
    join(dir, "macros.json"),
    JSON.stringify({
      schemaVersion: 1,
      macros: [
        {
          id: "mac0000001",
          name: "Opener",
          steps: [
            { do: "take", screenId: "scr0000001", layerId: "a" },
            { do: "wait", ms: 800 },
            { do: "clear", screenId: "scr0000001", layerId: "a" },
          ],
        },
      ],
    }),
  );
  writeFileSync(
    join(dir, "ui-state.json"),
    JSON.stringify({ schemaVersion: 1, control: { hotkeys: { "macro/mac0000001": "F6", "scr0000001/a": "F1" } } }),
  );
  mkdirSync(join(dir, "recipes"));
  writeFileSync(
    join(dir, "recipes", "inst000001.json"),
    JSON.stringify({
      schemaVersion: 1,
      id: "inst000001",
      recipeId: "event-schedule",
      name: "Day 1",
      panel: "schedule",
      sourceId: "src0000001",
      screenId: "scr0000001",
      layerIds: ["a"],
      templateIds: [],
      rules: [
        {
          id: "r1",
          name: "On NOW",
          enabled: true,
          when: { on: "itemStatus", sourceId: "src0000001", status: "now" },
          then: [{ do: "take", screenId: "scr0000001", layerId: "b", forMs: 10_000 }, { do: "runMacro", macroId: "mac0000001" }],
          lastFiredAt: "2026-09-01T12:00:00Z",
        },
      ],
      createdAt: "2026-09-01T10:00:00Z",
    }),
  );

  await own.cmd(`openproject ${dir}`);

  const flows = JSON.parse(await own.cmd("flows")) as FlowInfo[];
  const opener = flows.find((f) => f.id === "mac0000001")!;
  expect(opener).toMatchObject({ name: "Opener", armed: true, origin: null, retrigger: "restart" });
  expect(opener.nodes.map((n) => n.type)).toEqual(["trigger.cue", "action.take", "logic.delay", "action.clear"]);
  const rule = flows.find((f) => f.origin === "inst000001")!;
  expect(rule).toMatchObject({ name: "On NOW", armed: true, lastFiredAt: "2026-09-01T12:00:00Z" });
  expect(rule.nodes.map((n) => n.type)).toEqual(["trigger.itemStatus", "action.take", "action.runFlow"]);
  expect(rule.nodes[2].config).toMatchObject({ flowId: "mac0000001" });
  expect(existsSync(join(dir, "flows", `${rule.id}.json`))).toBe(true);

  const recipe = JSON.parse(readFileSync(join(dir, "recipes", "inst000001.json"), "utf8"));
  expect(recipe.schemaVersion).toBe(2);
  expect(recipe.flowIds).toEqual([rule.id]);
  expect(recipe.rules).toBeUndefined();
  expect(JSON.parse(readFileSync(join(dir, "macros.json"), "utf8"))).toEqual({ schemaVersion: 2, migratedTo: "flows" });
  const ui = JSON.parse(await own.cmd("uistate")) as UiState;
  expect(ui.control.hotkeys).toEqual({ "flow/mac0000001": "F6", "scr0000001/a": "F1" });
  expect(readdirSync(join(dir, ".backups")).filter((d) => d.endsWith("-pre-migrate"))).toHaveLength(1);

  // The migrated macro runs as a cue: take, wait, clear.
  await twoLayers(own, "scr0000001");
  await openScreen(page, own.base, "scr0000001");
  await own.cmd("runflow mac0000001");
  await expect(page.getByText("Layer A")).toBeVisible();
  await expect(roots(page)).toHaveCount(0, { timeout: 4_000 });
  await expect.poll(() => own.cmd("flowstate mac0000001")).toBe("idle");
});
