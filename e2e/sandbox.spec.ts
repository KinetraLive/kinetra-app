// The template sandbox: a template that tries every escape must
// fail, twice over. (1) The hardening refuses the markup on save / import (`checkmarkup`, the same
// `core::template_safety` checks). (2) Even when the markup reaches the renderer anyway (the
// debug-only `rawtemplate` bypass), the page's CSP blocks it: no script runs, nothing loads from
// another host (a local "evil" server counts every request: it must stay at 0), no form posts,
// no Tauri IPC, no files outside assets/.

import { readFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { expect, test } from "./fixture";

const GIF = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

let evil: Server;
let hits: string[] = [];
let EVIL = "";

test.beforeAll(async () => {
  evil = createServer((req, res) => {
    hits.push(req.url ?? "");
    res.end("x");
  });
  await new Promise<void>((r) => evil.listen(0, "127.0.0.1", r));
  EVIL = `http://127.0.0.1:${(evil.address() as AddressInfo).port}`;
});
test.afterAll(() => evil.close());
test.beforeEach(() => {
  hits = [];
});

/** Each attack: markup (and CSS) a malicious template would ship. */
function attacks() {
  return [
    { name: "inline script", html: `<div class="t"><script>window.__pwned="script"</script>x</div>` },
    { name: "img onerror", html: `<div class="t"><img src="nope" onerror="window.__pwned='onerror'"></div>` },
    { name: "onload fetch", html: `<div class="t"><img src="${GIF}" onload="fetch('${EVIL}/fetch');new WebSocket('ws://127.0.0.1:1/ws');window.__pwned='onload'"></div>` },
    { name: "javascript link", html: `<div class="t"><a id="js" href="javascript:window.__pwned='js'">go</a></div>`, click: "#js" },
    { name: "remote image", html: `<div class="t"><img src="${EVIL}/img.png"></div>` },
    { name: "remote font", html: `<div class="t f">x</div>`, css: `@font-face{font-family:evil;src:url(${EVIL}/font.woff2)} .f{font-family:evil}` },
    { name: "css import", html: `<div class="t">x</div>`, css: `@import url(${EVIL}/import.css); .t{color:red}` },
    { name: "css background", html: `<div class="t bg">x</div>`, css: `.bg{background:url(${EVIL}/bg.png);width:10px;height:10px}` },
    { name: "iframe", html: `<div class="t"><iframe src="${EVIL}/frame"></iframe></div>` },
    { name: "object", html: `<div class="t"><object data="${EVIL}/object"></object><embed src="${EVIL}/embed"></div>` },
    { name: "link stylesheet", html: `<div class="t"><link rel="stylesheet" href="${EVIL}/link.css">x</div>` },
    { name: "form post", html: `<div class="t"><form action="${EVIL}/form" method="post"><button id="go">go</button></form></div>`, click: "#go" },
    { name: "tauri ipc", html: `<div class="t"><img src="nope" onerror="window.__pwned=typeof __TAURI_INTERNALS__;__TAURI_INTERNALS__.invoke('license_view')"></div>` },
    { name: "files outside assets", html: `<div class="t"><img src="/assets/..%2F..%2Fproject.json"><img src="/assets/../prefs.json"></div>` },
  ];
}

test("the hardening refuses every attack on save and import", async ({ live }) => {
  for (const a of attacks()) {
    const problems = JSON.parse(await live.cmd(`checkmarkup ${JSON.stringify({ html: a.html, css: a.css ?? "" })}`)) as string[];
    // Path tricks inside /assets are harmless markup (the server refuses them, below).
    if (a.name === "files outside assets") continue;
    expect(problems.length, a.name).toBeGreaterThan(0);
  }
});

test("renderer pages carry the CSP", async ({ live }) => {
  const screen = await fetch(`${live.base}/screen/test`);
  const csp = screen.headers.get("content-security-policy") ?? "";
  for (const d of ["default-src 'none'", "script-src 'self'", "connect-src 'self'", "img-src 'self' data: blob:", "font-src 'self'", "frame-src 'none'", "object-src 'none'", "form-action 'none'", "base-uri 'none'"]) {
    expect(csp, d).toContain(d);
  }
  expect((await fetch(`${live.base}/pattern`)).headers.get("content-security-policy")).toContain("default-src 'none'");
});

test("nothing escapes the renderer even when the markup gets through", async ({ live, page }) => {
  test.setTimeout(120_000);
  const violations: string[] = [];
  page.on("console", (m) => {
    if (/Content Security Policy/i.test(m.text())) violations.push(m.text());
  });
  await page.addInitScript(() => {
    (window as unknown as { __v: string[] }).__v = [];
    document.addEventListener("securitypolicyviolation", (e) =>
      (window as unknown as { __v: string[] }).__v.push(`${e.violatedDirective} ${e.blockedURI}`),
    );
  });
  await page.goto(`${live.base}/screen/test`);
  const results: { name: string; pwned: unknown }[] = [];
  for (const a of attacks()) {
    await live.cmd(`rawtemplate ${JSON.stringify({ html: a.html, css: a.css ?? "" })}`);
    await live.cmd("show");
    await page.waitForTimeout(400);
    if (a.click) {
      // A DOM click (the element sits in a shadow root): follows the link / submits the form.
      await page.evaluate((sel) => {
        for (const d of document.querySelectorAll("div")) {
          const t = d.shadowRoot?.querySelector<HTMLElement>(sel);
          if (t) return t.click();
        }
      }, a.click);
      await page.waitForTimeout(300);
    }
    results.push({ name: a.name, pwned: await page.evaluate(() => (window as unknown as { __pwned?: unknown }).__pwned) });
    await live.cmd("hide");
  }
  const blocked = await page.evaluate(() => (window as unknown as { __v: string[] }).__v);
  console.log(`sandbox: ${blocked.length} CSP violations blocked; evil server hits: ${hits.length}`);
  for (const b of blocked) console.log(`  blocked: ${b}`);
  // No script ran, nothing reached the other host, the page is still on its own URL.
  for (const r of results) expect(r.pwned, r.name).toBeUndefined();
  expect(hits, "requests that reached another host").toEqual([]);
  expect(page.url()).toBe(`${live.base}/screen/test`);
  expect(await page.evaluate(() => "__TAURI_INTERNALS__" in window)).toBe(false);
  expect(blocked.length).toBeGreaterThan(5);
  // Give the shared test layer its normal look back (later tests use it).
  const dev = "presets/templates/test-lower-third";
  await live.cmd(
    `rawtemplate ${JSON.stringify({ html: readFileSync(`${dev}/template.html`, "utf8"), css: readFileSync(`${dev}/template.css`, "utf8") })}`,
  );
});

test("the server serves nothing outside assets/ and no license data", async ({ live }) => {
  for (const path of [
    "/assets/..%2F..%2Fproject.json",
    "/assets/..%2Fproject.json",
    "/assets/%2e%2e/project.json",
    "/license.json",
    "/prefs.json",
    "/assets/..\\project.json",
  ]) {
    const r = await fetch(`${live.base}${path}`);
    expect([400, 404], path).toContain(r.status);
  }
});
