// Web images (core::web_images): an http(s) image link bound into a template's `src` shows on air
// from Kinetra's copy. The renderer asks the output server (`/web-image`), never the web: a local
// "web" server sees exactly one request, from Kinetra (its user agent), none from the browser, and
// the copy serves every later load. A link no layer holds is never fetched.

import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { expect, test } from "./fixture";

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64",
);

let web: Server;
let hits: { url: string; agent: string }[] = [];
let WEB = "";

test.beforeAll(async () => {
  web = createServer((req, res) => {
    hits.push({ url: req.url ?? "", agent: req.headers["user-agent"] ?? "" });
    if (req.url === "/logo.png") {
      res.setHeader("content-type", "application/octet-stream");
      res.end(PNG);
    } else {
      res.statusCode = 404;
      res.end();
    }
  });
  await new Promise<void>((r) => web.listen(0, "127.0.0.1", r));
  WEB = `http://127.0.0.1:${(web.address() as AddressInfo).port}`;
});
test.afterAll(() => web.close());

/** The test layer's <img>, read from its shadow root. */
const pic = () => {
  for (const d of document.querySelectorAll("div")) {
    const img = d.shadowRoot?.querySelector<HTMLImageElement>("img.pic");
    if (img) return { src: img.getAttribute("src"), w: img.naturalWidth, done: img.complete };
  }
  return null;
};

test("a bound web image shows on air from Kinetra's copy", async ({ live, page }) => {
  hits = [];
  // A link no layer holds: refused, nothing fetched.
  const unused = await fetch(`${live.base}/web-image?u=${encodeURIComponent(`${WEB}/logo.png`)}`);
  expect(unused.status).toBe(404);
  expect(hits).toEqual([]);

  await live.cmd(`rawtemplate ${JSON.stringify({ html: `<div class="t"><img class="pic" src="{{name}}"></div>` })}`);
  await live.cmd(`fields ${JSON.stringify({ name: `${WEB}/logo.png` })}`);
  await page.goto(`${live.base}/screen/test`);
  await live.cmd("show");
  await expect.poll(() => page.evaluate(pic)).toMatchObject({ w: 1, done: true });
  const p = await page.evaluate(pic);
  expect(p?.src).toBe(`/web-image?u=${encodeURIComponent(`${WEB}/logo.png`)}`);
  // One request reached the web, from Kinetra, not from the browser.
  expect(hits.map((h) => h.url)).toEqual(["/logo.png"]);
  expect(hits[0].agent).toMatch(/^Kinetra\//);

  // A reload and a second renderer use the copy.
  await page.reload();
  await expect.poll(() => page.evaluate(pic)).toMatchObject({ w: 1, done: true });
  expect(hits).toHaveLength(1);

  // A link that isn't an image: nothing shows, nothing is kept, and the copy route says why.
  await live.cmd(`fields ${JSON.stringify({ name: `${WEB}/missing.png` })}`);
  const failed = await fetch(`${live.base}/web-image?u=${encodeURIComponent(`${WEB}/missing.png`)}`);
  expect(failed.status).toBe(502);
  expect(await failed.text()).toContain("HTTP 404");

  await live.cmd("hide");
  // Give the shared test layer its normal look back (later tests use it).
  const dev = "presets/templates/test-lower-third";
  await live.cmd(
    `rawtemplate ${JSON.stringify({ html: readFileSync(`${dev}/template.html`, "utf8"), css: readFileSync(`${dev}/template.css`, "utf8") })}`,
  );
  await live.cmd(`fields ${JSON.stringify({ name: "" })}`);
});
