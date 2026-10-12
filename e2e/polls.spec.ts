// Live polls against the real server and renderer, with the Poll
// preset's Frame look added through "Add to screen" (every field bound to the poll source):
// - a test poll: the open state (question, QR, short link), bars that follow the votes, then the
//   results (the total in the tile, the winner bracketed);
// - a live poll from the fixture's poll API mock: the stream's data on air, the last results kept
//   when the stream drops (the layer stays on air), and new data after it reconnects.

import type { Page } from "@playwright/test";
import { expect, isolatedTest as test, type Fixture } from "./fixture";
import { openBus } from "./studio-helpers";

// The first test may compile the fixture server.
test.describe.configure({ timeout: 300_000 });

const PID = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

/** An empty screen, the Poll graphic added for `source`, on air with the renderer open. */
async function onAir(page: Page, live: Fixture, source: string): Promise<string> {
  const id = `poll${Date.now().toString(36)}`;
  await live.cmd(
    `savescreen ${JSON.stringify({ id, name: "Polls", width: 1920, height: 1080, background: "transparent", layers: [], updatedAt: "" })}`,
  );
  const layer = await live.cmd(`polltoscreen ${source} ${id}`);
  await openBus(page, live.base, id, "program");
  await live.cmd(`layershow ${id} ${layer}`);
  await expect(page.locator(".pl")).toHaveAttribute("data-state", "in");
  return id;
}

/** The rounded shares shown, in answer order. */
const shares = (page: Page) => page.locator(".pl__opt:not([data-row='out']) .pl__pct--round");
/** A bar's horizontal scale once its transition settled. */
const scaleX = (page: Page, i: number) =>
  page
    .locator(".pl__bar")
    .nth(i)
    .evaluate((el) => new DOMMatrix(getComputedStyle(el).transform).a);
/** The word after a count (the graphic's own "vote" / "votes", drawn by CSS). */
const unit = (page: Page, sel: string) =>
  page.locator(sel).evaluate((el) => getComputedStyle(el, "::after").content.replace(/"/g, "").trim());
const stateLabel = (page: Page) =>
  page.locator(".pl__state").evaluate((el) => getComputedStyle(el, "::after").content.replace(/"/g, ""));

function wire(votes: number[], status = "open") {
  const labels = ["Home", "Away", "Draw"];
  return JSON.stringify({
    id: PID,
    code: "K7P2QX",
    url: "https://poll.kinetra.live/K7P2QX",
    question: "Who wins tonight?",
    options: votes.map((v, i) => ({ position: i, label: labels[i], votes: v })),
    status,
    show_results: true,
    duration_seconds: null,
    total_votes: votes.reduce((a, b) => a + b, 0),
    max_votes: 2000,
    vote_limit_reached: false,
    round: 1,
    created_at: null,
    opened_at: null,
    closes_at: null,
    closed_at: null,
    delete_after: null,
    expires_at: null,
  });
}

test("a test poll on air: the voting tile, bars that follow the votes, then the results", async ({ page, own }) => {
  const source = await own.cmd(
    `testpoll ${JSON.stringify({ question: "Best snack?", options: ["Popcorn", "Nachos", "Candy"], votes: [1, 1, 0], status: "open" })}`,
  );
  await onAir(page, own, source);
  // Open: the state, the question, the QR and the short link.
  await expect(page.locator(".pl")).toHaveAttribute("data-poll", "open");
  expect(await stateLabel(page)).toBe("Live poll");
  await expect(page.locator(".pl__q")).toHaveText("Best snack?");
  await expect(page.locator(".pl__url")).toHaveText("poll.kinetra.live/TEST00");
  await expect(page.locator(".pl__qr")).toHaveAttribute("src", /^data:image\/svg\+xml,/);
  await expect(page.locator(".pl__qr")).toBeVisible();
  await expect(shares(page)).toHaveText(["50", "50", "0"]);
  await expect.poll(() => scaleX(page, 0)).toBeCloseTo(0.5, 2);

  // One vote reads "1 vote"; more read "votes".
  await own.cmd(`testpollvotes ${source} [1,0,0]`);
  await expect(shares(page)).toHaveText(["100", "0", "0"]);
  await expect.poll(() => unit(page, ".pl__total")).toBe("vote");
  expect(await unit(page, ".pl__suml")).toBe("vote");

  // Votes arrive: shares and bars follow (no replayed animation: still "in").
  await own.cmd(`testpollvotes ${source} [1,6,1]`);
  await expect(shares(page)).toHaveText(["13", "75", "12"]);
  await expect.poll(() => scaleX(page, 1)).toBeCloseTo(0.75, 2);
  await expect(page.locator(".pl")).toHaveAttribute("data-state", "in");
  await expect(page.locator(".pl__opt").nth(1)).toHaveAttribute("data-leader", "true");

  // Closed: the results; the QR tile leaves, the total takes its place, the winner is marked.
  await own.cmd(`testpollstatus ${source} closed`);
  await expect(page.locator(".pl")).toHaveAttribute("data-poll", "closed");
  expect(await stateLabel(page)).toBe("Results");
  await expect(page.locator(".pl__opt").nth(1)).toHaveAttribute("data-winner", "true");
  await expect(page.locator(".pl__opt").nth(0)).not.toHaveAttribute("data-winner", "true");
  await expect(page.locator(".pl__sumn")).toHaveText("8");
  expect(await unit(page, ".pl__suml")).toBe("votes");
  expect(await unit(page, ".pl__total")).toBe("votes");
  await expect.poll(() => page.locator(".pl__vote").evaluate((el) => getComputedStyle(el).opacity)).toBe("0");
  await expect.poll(() => page.locator(".pl__sum").evaluate((el) => getComputedStyle(el).opacity)).toBe("1");
});

test("a live poll keeps its last results on air when the stream drops, and updates after it reconnects", async ({
  page,
  own,
}) => {
  const source = await own.cmd(`pollmock ${wire([2, 1, 1])}`);
  await onAir(page, own, source);
  await expect.poll(async () => JSON.parse(await own.cmd(`pollconn ${PID}`))).toBe("live");
  await expect(shares(page)).toHaveText(["50", "25", "25"]);
  await expect(page.locator(".pl__url")).toHaveText("poll.kinetra.live/K7P2QX");

  await own.cmd(`pollpush totals ${wire([3, 0, 1])}`);
  await expect(shares(page)).toHaveText(["75", "0", "25"]);

  // The stream drops: nothing on air changes; the client reconnects on its own.
  await own.cmd("polldrop");
  await expect.poll(async () => Number(await own.cmd("pollconnects")), { timeout: 15_000 }).toBeGreaterThanOrEqual(2);
  await expect(shares(page)).toHaveText(["75", "0", "25"]);
  await expect(page.locator(".pl")).toHaveAttribute("data-state", "in");

  // After the reconnect, new totals arrive again.
  await expect.poll(async () => JSON.parse(await own.cmd(`pollconn ${PID}`))).toBe("live");
  await own.cmd(`pollpush totals ${wire([1, 3, 0])}`);
  await expect(shares(page)).toHaveText(["25", "75", "0"]);

  // Closed on the server: the results state.
  await own.cmd(`pollpush status ${wire([1, 3, 0], "closed")}`);
  await expect(page.locator(".pl")).toHaveAttribute("data-poll", "closed");
  await expect(page.locator(".pl__opt").nth(1)).toHaveAttribute("data-winner", "true");
});
