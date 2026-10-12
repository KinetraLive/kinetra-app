// Command palette cost: `pnpm perf:ui`. The built app with the mock backend, on Control.
//   - Ctrl+K keydown → panel painted < 50 ms: the first open (lazy chunk loaded from the built app)
//     and later opens.
//   - With ~2000 entries (synthetic items through the palette's measurement seam,
//     `__KINETRA_PALETTE_EXTRA__`): keydown → results painted within one frame budget; no long
//     tasks while typing.
//   - Closed: no timers or animation frames scheduled by the palette.
// "Painted" = the first setTimeout after the next animation frame following the DOM change.

import type { Page } from "@playwright/test";
import { expect, test } from "../fixture";
import { mirrorScreens, open, screensScenario } from "./harness";

const WORDS = ["Opening", "keynote", "Lower", "third", "Screen", "Sponsor", "Panel", "Guest", "Ticker", "Clock", "Stage", "Break"];

type Pal = { down: number; opens: number[]; keys: number[]; long: number[]; pending: "open" | "key" | null };

/** The keydown probe, installed before the app's scripts: its capture listener on window must run
 * before the shell's (same target and phase: registration order), which may open the palette
 * synchronously. */
async function keyProbe(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __pal: Pal };
    const m = (w.__pal = { down: 0, opens: [], keys: [], long: [], pending: null });
    window.addEventListener(
      "keydown",
      (e) => {
        m.down = performance.now();
        const open = (e.ctrlKey || e.metaKey) && e.code === "KeyK";
        const typing = e.key !== "Escape" && e.key !== "Shift" && !!document.querySelector("dialog.palette[open]");
        m.pending = open ? "open" : typing ? "key" : null;
      },
      { capture: true },
    );
  });
}

/** Installs the other probes: long tasks, DOM-change → painted times. */
async function probe(page: Page) {
  await page.evaluate(() => {
    const m = (window as unknown as { __pal: Pal }).__pal;
    m.opens.length = 0;
    m.keys.length = 0;
    m.pending = null;
    new PerformanceObserver((l) => l.getEntries().forEach((e) => m.long.push(e.duration))).observe({ type: "longtask" });
    const painted = (kind: "open" | "key") => {
      const t0 = m.down;
      requestAnimationFrame(() =>
        setTimeout(() => {
          (kind === "open" ? m.opens : m.keys).push(performance.now() - t0);
        }),
      );
    };
    new MutationObserver(() => {
      const kind = m.pending;
      if (!kind) return;
      if (kind === "open" && !document.querySelector("dialog.palette[open]")) return;
      m.pending = null;
      painted(kind);
    }).observe(document.body, { childList: true, subtree: true, characterData: true, attributes: true });
  });
}

test("palette: open < 50 ms (first and later), a keystroke over ~2000 entries within a frame", async ({
  page,
  context,
  server,
}) => {
  test.setTimeout(120_000);
  await mirrorScreens(page, context, server);
  await keyProbe(page);
  // ~2000 synthetic project items across the sections that hold them.
  await page.addInitScript((words: string[]) => {
    const sections = ["items", "items", "items", "live", "goto"];
    (globalThis as unknown as { __KINETRA_PALETTE_EXTRA__: unknown[] }).__KINETRA_PALETTE_EXTRA__ = Array.from(
      { length: 2000 },
      (_, i) => ({
        id: `perf.item:${i}`,
        title: `${words[i % words.length]} ${words[(i * 7) % words.length]} ${i}`,
        section: sections[i % sections.length],
        scope: i % 2 ? "screen" : "template",
        keywords: [words[(i * 5) % words.length].toLowerCase()],
        run: () => {},
      }),
    );
  }, WORDS);
  await open(page, "/control", {
    ...screensScenario("dark"),
    control: { selectedScreenId: "s1main01ab", view: "single", gridColumns: 2, liveEdit: "onTake", hotkeys: {} },
  });
  await expect(page.getByRole("button", { name: "Take Guest lower third" })).toBeVisible();
  await page.waitForTimeout(1500);
  await probe(page);

  const dialog = page.getByRole("dialog", { name: "Command palette" });
  const input = dialog.getByRole("combobox");

  // Opens: the first loads the lazy chunk.
  for (let i = 0; i < 4; i++) {
    await page.keyboard.press("Control+K");
    await expect(input).toBeFocused();
    await page.waitForTimeout(300);
    if (i === 0) {
      // Keystrokes over ~2000 entries (no query yet → a query → refined → a prefix scope).
      for (const ch of "opening k") {
        await page.keyboard.type(ch);
        await page.waitForTimeout(60);
      }
      for (let k = 0; k < 9; k++) {
        await page.keyboard.press("Backspace");
        await page.waitForTimeout(60);
      }
      for (const ch of "@st 1") {
        await page.keyboard.type(ch);
        await page.waitForTimeout(60);
      }
      await expect(dialog.getByRole("option").first()).toBeVisible();
    }
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await page.waitForTimeout(200);
  }

  const m = (await page.evaluate(() => (window as unknown as { __pal: unknown }).__pal)) as {
    opens: number[];
    keys: number[];
    long: number[];
  };
  const f = (xs: number[]) => xs.map((x) => x.toFixed(1)).join(", ");
  const sorted = [...m.keys].sort((a, b) => a - b);
  const p95 = sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.95))] ?? 0;
  console.log(
    `palette: open ${f(m.opens)} ms (first includes the lazy chunk); ` +
      `keystroke → painted over ~2000 entries: median ${sorted[Math.floor(sorted.length / 2)]?.toFixed(1)} ms, ` +
      `p95 ${p95.toFixed(1)} ms, max ${sorted[sorted.length - 1]?.toFixed(1)} ms (${m.keys.length} keys); ` +
      `long tasks ${m.long.length ? f(m.long) : "none"}`,
  );
  expect(m.opens.length).toBe(4);
  for (const t of m.opens) expect(t).toBeLessThan(50);
  expect(m.keys.length).toBeGreaterThanOrEqual(20);
  // Painted includes waiting for the next frame (vsync); scripting + layout stay inside it.
  expect(p95).toBeLessThan(34);
  expect(m.long.length).toBe(0);

  // Closed: nothing scheduled.
  const sched = await page.evaluate(async () => {
    let n = 0;
    const st = window.setTimeout;
    const raf = window.requestAnimationFrame;
    window.setTimeout = ((...a: Parameters<typeof setTimeout>) => (n++, st(...a))) as typeof setTimeout;
    window.requestAnimationFrame = (cb) => (n++, raf(cb));
    await new Promise((r) => st(r, 1000));
    window.setTimeout = st;
    window.requestAnimationFrame = raf;
    return n;
  });
  expect(sched).toBe(0);
});
