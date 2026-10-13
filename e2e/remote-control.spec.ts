// Remote control end to end: the real core connection (core::remote) talks to a mock of
// kinetra-cloud's relay inside the fixture server, and the real renderer shows the result.
// A remote take goes on air; a data editor is refused on an on-air layer while an off-air edit
// lands; Lock remote take refuses takes; the remote state's on_air waits for the output's
// out-animation; a remote GO runs the next cue.
import type { Page } from "@playwright/test";
import { expect, ownTest as test, type Fixture } from "./fixture";
import type { NewFlow, ScreenDoc } from "../src/lib/types";

const roots = (page: Page) => page.locator("[data-state]");
const FULL = { x: 0, y: 0, w: 1920, h: 1080 };

interface Remote {
  seq: number;
  state: {
    screens: Record<string, { layers: string[] }>;
    layers: Record<string, { screen: string; on_air: boolean; values: Record<string, string> }>;
    cue_lists: Record<string, { current: string | null; next: string | null }>;
  };
}
const remoteState = async (live: Fixture): Promise<Remote> => JSON.parse(await live.cmd("remotestate"));

let n = 0;
/** Sends a remote command and returns its result. */
async function command(live: Fixture, action: string, args: object, role = "operator") {
  const corr = `e2e-${Date.now()}-${n++}`;
  const { seq } = await remoteState(live);
  await live.cmd(
    `remotesend ${JSON.stringify({
      t: "command",
      corr,
      action,
      args,
      actor: { id: null, name: "Kari", role },
      seen_seq: seq,
      sent_ms: Date.now(),
    })}`,
  );
  return JSON.parse(await live.cmd(`remotewait result 1 corr=${corr}`)) as { ok: boolean; code?: string };
}

// Its own fixture server: the remote state lists the whole project (≤ 32 screens), and the
// shared server collects more than that over a full run.
test("remote control drives the real renderer", async ({ page, own: live }) => {
  const templateId = await live.cmd("addtemplate classic-bar");
  const layer = (id: string, name: string) => ({
    id,
    name,
    kind: "template" as const,
    templateId,
    rect: FULL,
    fields: { name, title: "" },
    vars: {},
  });
  const doc: ScreenDoc = {
    id: `rc${Date.now().toString(36)}`,
    name: "Remote",
    width: 1920,
    height: 1080,
    background: "transparent",
    layers: [layer("l1", "Ada Lovelace"), layer("l2", "Grace Hopper")],
    updatedAt: "",
  };
  await live.cmd(`savescreen ${JSON.stringify(doc)}`);
  const cue: NewFlow = {
    name: "Open",
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: [
      { id: "c", type: "trigger.cue", config: { label: null }, position: { x: 0, y: 0 } },
      { id: "t", type: "action.take", config: { screenId: doc.id, layerId: "l2", forMs: null }, position: { x: 240, y: 0 } },
    ],
    edges: [{ id: "e", from: { node: "c", port: "fired" }, to: { node: "t", port: "in" }, kind: "event" }],
  } as NewFlow;
  const flow = { id: await live.cmd(`saveflow ${JSON.stringify(cue)}`) };

  await page.goto(`${live.base}/screen/${doc.id}`);
  await live.cmd("remotemock");
  const on = JSON.parse(await live.cmd("remoteon"));
  expect(on.on).toBe(true);
  const snap = JSON.parse(await live.cmd("remotewait snapshot 1"));
  // This screen's layers in the remote state (top first). Another test's screen may use the same
  // layer ids: then these keys are screen-qualified ("<screen>_l1").
  const keys: string[] = snap.state.screens[doc.id].layers;
  expect(keys).toHaveLength(2);
  const key = (id: string) => keys.find((k) => k === id || k === `${doc.id}_${id}`)!;
  const k1 = key("l1");
  const k2 = key("l2");
  expect(snap.state.layers[k1].screen).toBe(doc.id);
  expect(JSON.parse(await live.cmd("remotewait register 1")).name.length).toBeGreaterThan(0);

  // A remote take: on air on the real output, and the relay sees on_air true.
  expect((await command(live, "take", { layer: k1 })).ok).toBe(true);
  await expect(roots(page).first()).toHaveAttribute("data-state", "in");
  await expect(page.getByText("Ada Lovelace")).toBeVisible();
  await expect.poll(async () => (await remoteState(live)).state.layers[k1].on_air).toBe(true);

  // A data editor: refused on the on-air layer, the off-air edit lands.
  expect((await command(live, "update_fields", { layer: k1, values: { name: "Typo" } }, "data_editor")).code).toBe("on_air");
  expect((await command(live, "update_fields", { layer: k2, values: { name: "Grace B. Hopper" } }, "data_editor")).ok).toBe(true);
  await expect.poll(async () => (await remoteState(live)).state.layers[k2].values.name).toBe("Grace B. Hopper");
  await expect(page.getByText("Typo")).toHaveCount(0);

  // Lock remote take: takes are refused; nothing changes on air.
  await live.cmd("remotelock on");
  expect((await command(live, "take", { layer: k2 })).code).toBe("locked");
  await expect(page.getByText("Grace B. Hopper")).toHaveCount(0);
  await live.cmd("remotelock off");

  // A remote GO runs the next cue (it takes l2) and the list moves on.
  const before = (await remoteState(live)).state.cue_lists.cues;
  expect(before.next).toBe(flow.id);
  expect((await command(live, "run_cue", { cue_list: "cues", cue: flow.id })).ok).toBe(true);
  await expect(page.getByText("Grace B. Hopper")).toBeVisible();
  await expect.poll(async () => (await remoteState(live)).state.cue_lists.cues.current).toBe(flow.id);

  // A remote clear: on_air stays true until the output's out-animation has finished.
  const t0 = Date.now();
  expect((await command(live, "clear", { layer: k1 })).ok).toBe(true);
  expect((await remoteState(live)).state.layers[k1].on_air).toBe(true);
  await expect(page.getByText("Ada Lovelace")).toHaveCount(0, { timeout: 4000 });
  await expect.poll(async () => (await remoteState(live)).state.layers[k1].on_air).toBe(false);
  expect(Date.now() - t0).toBeGreaterThan(100);

  // Off: the app ends the run.
  await live.cmd("remoteoff");
  expect(JSON.parse(await live.cmd("remotewait end 1")).reason).toBe("disabled");
  await live.cmd(`clearscreen ${doc.id}`);
});
