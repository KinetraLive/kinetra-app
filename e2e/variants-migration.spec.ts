// Template variants: a v1 template folder migrated on open (a Default variant with the old look,
// a pre-migrate backup, idempotent), on a fixture server of its own (it reopens the project).

import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { expect, isolatedTest } from "./fixture";
import { openScreen } from "./flow-helpers";
import { LOOK, oneLayer } from "./variant-helpers";
import type { TemplateDoc } from "../src/lib/types";

isolatedTest("a v1 template becomes a Default variant on open, once", async ({ page, own }) => {
  const dir = await own.cmd("dir");
  const v1 = {
    name: "Legacy bar",
    html: '<div class="lt" data-look="v1"><b>{{name}}</b> <i>{{title}}</i></div>',
    css: LOOK.default.css,
    fields: [
      { key: "name", label: "Name", type: "text", default: "" },
      { key: "title", label: "Title", type: "text", default: "" },
    ],
  };
  // A screen already using the template (written before the template, as an older Kinetra had it).
  const screen = await oneLayer(own, "legacy0001", { name: "Old Name", title: "Old Title" });
  await own.cmd(`writev1template legacy0001 ${JSON.stringify(v1)}`);
  await own.cmd(`openproject ${dir}`);

  const folder = join(dir, "templates", "legacy0001");
  const doc = JSON.parse(await own.cmd("gettemplate legacy0001")) as TemplateDoc;
  expect(doc).toMatchObject({ id: "legacy0001", name: "Legacy bar", defaultVariant: "default" });
  expect(doc.variants).toEqual([{ id: "default", name: "Default", vars: {}, html: v1.html, css: v1.css }]);
  expect(doc.fields.map((f) => f.key)).toEqual(["name", "title"]);
  const json = readFileSync(join(folder, "template.json"), "utf8");
  expect(JSON.parse(json)).toMatchObject({
    schemaVersion: 2,
    defaultVariant: "default",
    variants: [{ id: "default", name: "Default" }],
  });
  expect(existsSync(join(folder, "template.html"))).toBe(false);
  expect(existsSync(join(folder, "template.css"))).toBe(false);
  expect(readFileSync(join(folder, "variants", "default", "template.html"), "utf8")).toBe(v1.html);
  const backups = () => readdirSync(join(dir, ".backups")).filter((d) => d.endsWith("-pre-migrate"));
  expect(backups()).toHaveLength(1);

  // The screen's layer renders the migrated look on air.
  await openScreen(page, own.base, screen.id);
  await own.cmd(`layershow ${screen.id} l1`);
  await expect(page.getByText("Old Name")).toBeVisible();
  await expect(page.locator('[data-look="v1"]')).toHaveAttribute("data-state", "in");

  // Opening it again changes nothing.
  const mtime = statSync(join(folder, "template.json")).mtimeMs;
  await own.cmd(`openproject ${dir}`);
  expect(readFileSync(join(folder, "template.json"), "utf8")).toBe(json);
  expect(statSync(join(folder, "template.json")).mtimeMs).toBe(mtime);
  expect(JSON.parse(await own.cmd("gettemplate legacy0001"))).toEqual(doc);
  expect(backups()).toHaveLength(1);
});

