// The flow editor's controller (draft, autosave and validation timing, deploy / discard, external
// edits, live debug subscription) and its components that don't need the canvas: the header
// (FlowEditor with the lazy chunks stubbed) and the inspector's keyboard wiring.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { FlowCheck, FlowEdge, FlowInfo, FlowNode, LiveScreenView, NodeKind } from "$lib/types";

// Lazily loaded components these tests open: loaded once here, so the tests wait on them, not on
// Vite compiling them on first use (CLAUDE.md › Testing policy).
beforeAll(async () => {
  await Promise.all([
    import("$lib/components/ConfirmDialog.svelte"),
  ]);
});

type Handler = (payload: unknown) => void;
const events = vi.hoisted(() => ({ handlers: new Map<string, Set<Handler>>() }));
const api = vi.hoisted(() => ({
  getFlow: vi.fn(),
  listFlows: vi.fn(),
  saveFlowDraft: vi.fn(),
  checkFlow: vi.fn(),
  deployFlow: vi.fn(),
  discardFlowDraft: vi.fn(),
  renameFlow: vi.fn(),
  duplicateFlow: vi.fn(),
  setFlowArmed: vi.fn(),
  subscribeFlowDebug: vi.fn(),
  unsubscribeFlowDebug: vi.fn(),
  listSources: vi.fn(),
  getSchedule: vi.fn(),
  listScreens: vi.fn(),
  getScreen: vi.fn(),
  getTemplate: vi.fn(),
  listTemplates: vi.fn(),
}));
vi.mock("$lib/api", () => ({
  on: (event: string, h: Handler) => {
    const set = events.handlers.get(event) ?? new Set();
    set.add(h);
    events.handlers.set(event, set);
    return () => set.delete(h);
  },
  toAppError: (e: unknown) => (e && typeof e === "object" && "message" in e ? e : { code: "internal", message: String(e) }),
}));
vi.mock("$lib/api/flows", () => api);
vi.mock("$lib/api/sources", () => api);
vi.mock("$lib/api/live", () => api);
vi.mock("$lib/api/screens", () => api);
vi.mock("$lib/api/templates", () => api);
vi.mock("$app/navigation", () => ({ goto: vi.fn() }));
// The canvas and palette are lazy chunks: stubbed (jsdom can't lay out @xyflow/svelte).
vi.mock("./load", async (actual) => {
  // A component that renders nothing (Svelte 5 components are functions).
  const Stub = () => {};
  return {
    ...(await actual<typeof import("./load")>()),
    loadCanvas: () => Promise.resolve(Stub),
    loadPalette: () => Promise.resolve(Stub),
  };
});

import { toasts } from "$lib/stores/toasts.svelte";
import { CHECK_DEBOUNCE_MS, FlowEditorCtl, NOTICE_MS, SAVE_DEBOUNCE_MS } from "./editor.svelte";

const emit = (event: string, payload: unknown) => {
  for (const h of events.handlers.get(event) ?? []) h(payload);
};

const n = (id: string, k: NodeKind, x = 0): FlowNode => ({ ...k, id, position: { x, y: 0 } }) as FlowNode;
const cue = n("cue", { type: "trigger.cue", config: { label: null } });
const branch = n("if", { type: "logic.branch", config: {} }, 240);
const take = n("take", { type: "action.take", config: { screenId: "s1", layerId: "l1", forMs: null } }, 480);
const clear = n("clear", { type: "action.clear", config: { screenId: "s1", layerId: "l1" } }, 480);
const w1: FlowEdge = { id: "w1", from: { node: "cue", port: "fired" }, to: { node: "if", port: "in" }, kind: "event" };
const w2: FlowEdge = { id: "w2", from: { node: "if", port: "then" }, to: { node: "take", port: "in" }, kind: "event" };

function flow(patch: Partial<FlowInfo> = {}): FlowInfo {
  return {
    id: "f1",
    name: "Opener",
    armed: true,
    retrigger: "restart",
    origin: null,
    nodes: [cue, branch, take],
    edges: [w1, w2],
    draft: null,
    lastFiredAt: null,
    updatedAt: "2026-10-04T10:00:00Z",
    warnings: [],
    running: false,
    readOnly: false,
    ...patch,
  };
}

const view: LiveScreenView = {
  screen: { id: "s1", name: "Main", width: 1920, height: 1080, background: "transparent" },
  layers: [{ id: "l1", name: "Guest", onAir: false, pending: false, fields: {}, fieldDefs: [] }],
};

let server: FlowInfo;
const NO_ISSUES: FlowCheck = { errors: [], warnings: [] };

beforeEach(() => {
  events.handlers.clear();
  toasts.list = [];
  server = flow();
  for (const f of Object.values(api)) f.mockReset();
  api.getFlow.mockImplementation(async () => structuredClone(server));
  api.listFlows.mockImplementation(async () => [structuredClone(server)]);
  api.saveFlowDraft.mockImplementation(async (_id: string, nodes: FlowNode[], edges: FlowEdge[]) => {
    server = { ...server, draft: { nodes, edges, updatedAt: "2026-10-04T10:05:00Z" }, updatedAt: "2026-10-04T10:05:00Z" };
    return structuredClone(server);
  });
  api.checkFlow.mockResolvedValue(NO_ISSUES);
  api.deployFlow.mockImplementation(async () => {
    server = { ...server, nodes: server.draft?.nodes ?? server.nodes, edges: server.draft?.edges ?? server.edges, draft: null };
    return structuredClone(server);
  });
  api.discardFlowDraft.mockImplementation(async () => {
    server = { ...server, draft: null };
    return structuredClone(server);
  });
  api.subscribeFlowDebug.mockResolvedValue(undefined);
  api.unsubscribeFlowDebug.mockResolvedValue(undefined);
  api.listSources.mockResolvedValue([]);
  api.listScreens.mockResolvedValue([view]);
  Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** A connected controller with the flow loaded (fake timers). */
async function open(f?: FlowInfo) {
  if (f) server = f;
  vi.useFakeTimers();
  const ed = new FlowEditorCtl("f1");
  const off = ed.connect();
  await vi.waitFor(() => expect(ed.loaded).toBe(true));
  return { ed, off };
}

describe("FlowEditorCtl: draft and autosave", () => {
  it("works on the draft when there is one, else the deployed graph", async () => {
    const { ed, off } = await open(flow({ draft: { nodes: [cue], edges: [], updatedAt: "x" } }));
    expect(ed.graph.nodes.map((x) => x.id)).toEqual(["cue"]);
    expect(ed.notDeployed).toBe(true);
    off();
    const b = await open(flow());
    expect(b.ed.graph.nodes.map((x) => x.id)).toEqual(["cue", "if", "take"]);
    expect(b.ed.notDeployed).toBe(false);
    b.off();
  });

  it("saves 800 ms after the last edit, once, with the latest graph", async () => {
    const { ed, off } = await open();
    ed.nudge(8, 0); // nothing selected: no change
    ed.setSelection(["take"]);
    ed.nudge(8, 0);
    expect(ed.saveState.kind).toBe("pending");
    expect(ed.notDeployed).toBe(true);
    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS - 100);
    ed.nudge(8, 0);
    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS - 1);
    expect(api.saveFlowDraft).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(api.saveFlowDraft).toHaveBeenCalledTimes(1);
    const [, nodes] = api.saveFlowDraft.mock.calls[0] as [string, FlowNode[]];
    expect(nodes.find((x) => x.id === "take")?.position).toEqual({ x: 496, y: 0 });
    await vi.waitFor(() => expect(ed.saveState.kind).toBe("saved"));
    expect(ed.info?.draft).toBeTruthy();
    expect(ed.notDeployed).toBe(true);
    off();
  });

  it("validates 300 ms after an edit; errors map to nodes, the rest to the flow", async () => {
    api.checkFlow.mockResolvedValue({
      errors: [
        { node: "take", message: "Pick a layer" },
        { node: null, message: "Too many wires" },
      ],
      warnings: [{ node: "if", message: "The condition is not wired" }],
    });
    const { ed, off } = await open();
    await vi.advanceTimersByTimeAsync(CHECK_DEBOUNCE_MS);
    expect(api.checkFlow).toHaveBeenCalledTimes(1);
    expect(ed.issues.get("take")).toEqual({ errors: ["Pick a layer"], warnings: [] });
    expect(ed.issues.get("if")).toEqual({ errors: [], warnings: ["The condition is not wired"] });
    expect(ed.flowIssues.map((i) => i.message)).toEqual(["Too many wires"]);
    expect(ed.deployBlocked).toBe("2 problems to fix first.");
    ed.setSelection(["take"]);
    ed.nudge(0, 8);
    ed.nudge(0, 8);
    await vi.advanceTimersByTimeAsync(CHECK_DEBOUNCE_MS - 1);
    expect(api.checkFlow).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(api.checkFlow).toHaveBeenCalledTimes(2);
    off();
  });

  it("add, connect, remove, undo and redo", async () => {
    const { ed, off } = await open();
    const id = ed.addNode("action.clear", { x: 700, y: 40 });
    expect(id).toBeTruthy();
    expect([...ed.selectedNodes]).toEqual([id]);
    expect(ed.wire({ node: "if", port: "else" }, { node: id!, port: "in" })).toBe(true);
    expect(ed.graph.edges).toHaveLength(3);
    ed.remove([id!]);
    expect(ed.graph.nodes).toHaveLength(3);
    expect(ed.graph.edges).toHaveLength(2);
    expect(ed.selectedNodes.size).toBe(0);
    ed.undo();
    expect(ed.graph.nodes).toHaveLength(4);
    expect(ed.graph.edges).toHaveLength(3);
    ed.undo();
    ed.undo();
    expect(ed.graph.nodes).toHaveLength(3);
    expect(ed.canUndo).toBe(false);
    ed.redo();
    expect(ed.graph.nodes).toHaveLength(4);
    off();
  });

  it("a refused wire says why on the canvas for a while", async () => {
    const { ed, off } = await open();
    expect(ed.wire({ node: "cue", port: "fired" }, { node: "if", port: "condition" })).toBe(false);
    expect(ed.notice).toBe("Can't connect: An event can't go into a data input.");
    await vi.advanceTimersByTimeAsync(NOTICE_MS);
    expect(ed.notice).toBeNull();
    expect(api.saveFlowDraft).not.toHaveBeenCalled();
    off();
  });

  it("highlights only ports that can take the wire being dragged", async () => {
    const { ed, off } = await open();
    ed.connecting = { node: "if", port: "else", side: "output" };
    expect(ed.canTake("take", "in", "input")).toBe(true);
    expect(ed.canTake("if", "condition", "input")).toBe(false);
    expect(ed.canTake("take", "done", "output")).toBe(false);
    expect(ed.canTake("if", "else", "output")).toBeNull();
    ed.connecting = null;
    expect(ed.canTake("take", "in", "input")).toBeNull();
    // "Connect to…": the inputs that take Branch's else.
    expect(ed.partners("if", "else", "output").map((p) => `${p.node.id}.${p.port.name}`)).toEqual(["take.in"]);
    off();
  });

  it("leaving flushes a pending save and unsubscribes", async () => {
    const { ed, off } = await open();
    expect(api.subscribeFlowDebug).toHaveBeenCalledWith("f1");
    ed.setSelection(["cue"]);
    ed.nudge(8, 0);
    off();
    expect(api.saveFlowDraft).toHaveBeenCalledTimes(1);
    expect(api.unsubscribeFlowDebug).toHaveBeenCalledWith("f1");
  });

  it("a read-only flow takes no edits", async () => {
    const { ed, off } = await open(flow({ readOnly: true }));
    expect(ed.addNode("action.log")).toBeNull();
    ed.setSelection(["cue"]);
    ed.nudge(8, 0);
    expect(ed.graph.nodes[0].position.x).toBe(0);
    expect(ed.deployBlocked).toBe("This flow is read-only.");
    off();
    expect(api.saveFlowDraft).not.toHaveBeenCalled();
  });
});

describe("FlowEditorCtl: deploy and discard", () => {
  it("deploy saves the draft first, then deploys", async () => {
    const { ed, off } = await open();
    ed.setSelection(["take"]);
    ed.nudge(0, 16);
    const order: string[] = [];
    api.saveFlowDraft.mockImplementationOnce(async (_id: string, nodes: FlowNode[], edges: FlowEdge[]) => {
      order.push("save");
      server = { ...server, draft: { nodes, edges, updatedAt: "y" } };
      return structuredClone(server);
    });
    api.deployFlow.mockImplementationOnce(async () => {
      order.push("deploy");
      server = { ...server, nodes: server.draft!.nodes, edges: server.draft!.edges, draft: null };
      return structuredClone(server);
    });
    expect(await ed.deploy()).toBe(true);
    expect(order).toEqual(["save", "deploy"]);
    expect(ed.notDeployed).toBe(false);
    expect(ed.info?.nodes.find((x) => x.id === "take")?.position.y).toBe(16);
    off();
  });

  it("errors block deploy", async () => {
    api.checkFlow.mockResolvedValue({ errors: [{ node: "take", message: "Pick a layer" }], warnings: [] });
    const { ed, off } = await open();
    expect(await ed.deploy()).toBe(false);
    expect(api.deployFlow).not.toHaveBeenCalled();
    off();
  });

  it("discard drops the draft and shows what runs", async () => {
    const { ed, off } = await open(flow({ draft: { nodes: [cue, clear], edges: [], updatedAt: "x" } }));
    expect(ed.graph.nodes.map((x) => x.id)).toEqual(["cue", "clear"]);
    expect(await ed.discard()).toBe(true);
    expect(api.discardFlowDraft).toHaveBeenCalledWith("f1");
    expect(ed.graph.nodes.map((x) => x.id)).toEqual(["cue", "if", "take"]);
    expect(ed.canUndo).toBe(false);
    expect(ed.notDeployed).toBe(false);
    off();
  });
});

describe("FlowEditorCtl: files changed on disk, live debug", () => {
  it("without unsaved edits the file is taken silently (one undo step)", async () => {
    const { ed, off } = await open();
    server = flow({ nodes: [cue] , edges: [] });
    emit("project-files-changed", { kind: "flow", id: "f1", change: "changed", message: null });
    await vi.waitFor(() => expect(ed.graph.nodes).toHaveLength(1));
    expect(ed.disk).toBeNull();
    ed.undo();
    expect(ed.graph.nodes).toHaveLength(3);
    off();
  });

  it("with unsaved edits: Reload / Keep mine", async () => {
    const { ed, off } = await open();
    ed.setSelection(["cue"]);
    ed.nudge(8, 0);
    emit("project-files-changed", { kind: "flow", id: "f1", change: "changed", message: null });
    expect(ed.disk).toBe("conflict");
    expect(ed.deployBlocked).toMatch(/Reload or Keep mine/);
    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS);
    expect(api.saveFlowDraft).not.toHaveBeenCalled();
    await ed.diskKeep();
    expect(api.saveFlowDraft).toHaveBeenCalledTimes(1);
    expect(ed.disk).toBeNull();
    // Another conflict, resolved with Reload: the file wins.
    ed.nudge(8, 0);
    emit("project-files-changed", { kind: "flow", id: "f1", change: "changed", message: null });
    server = flow({ nodes: [cue], edges: [] });
    await ed.diskReload();
    expect(ed.graph.nodes).toHaveLength(1);
    expect(ed.disk).toBeNull();
    off();
  });

  it("deleted on disk: a banner; nothing saves", async () => {
    const { ed, off } = await open();
    emit("project-files-changed", { kind: "flow", id: "f1", change: "removed", message: null });
    expect(ed.disk).toBe("removed");
    ed.setSelection(["cue"]);
    ed.nudge(8, 0);
    off();
    expect(api.saveFlowDraft).not.toHaveBeenCalled();
  });

  it("flow-debug of this flow flashes its nodes; others are ignored; hidden window unsubscribes", async () => {
    const { ed, off } = await open();
    emit("flow-debug", { id: "other", nodes: [{ node: "cue", at: "2026-10-04T10:00:00Z" }], edges: [], errors: [] });
    expect(ed.debug.nodes.size).toBe(0);
    emit("flow-debug", { id: "f1", nodes: [{ node: "cue", at: "2026-10-04T10:00:00Z" }], edges: ["w1"], errors: [] });
    expect([...ed.debug.nodes]).toEqual(["cue"]);
    expect(ed.debug.fired.get("cue")).toBe("2026-10-04T10:00:00Z");
    emit("flow-debug", { id: "f1", nodes: [], edges: [], errors: [{ node: "take", message: "Skipped: no layer" }] });
    expect(ed.issues.get("take")?.errors).toEqual(["Skipped: no layer"]);
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(api.unsubscribeFlowDebug).toHaveBeenCalledTimes(1);
    expect(ed.debug.nodes.size).toBe(0);
    Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(api.subscribeFlowDebug).toHaveBeenCalledTimes(2);
    off();
    expect(api.unsubscribeFlowDebug).toHaveBeenCalledTimes(2);
  });
});

// ---- Components ------------------------------------------------------------------------------------

import FlowEditor from "./FlowEditor.svelte";
import Inspector from "./Inspector.svelte";
import { installShortcuts } from "$lib/commands/testing";

describe("FlowEditor header", () => {
  it("shows Saved / Not deployed and deploys with Ctrl+Enter", async () => {
    server = flow({ draft: { nodes: [cue, branch, take], edges: [w1], updatedAt: "x" } });
    render(FlowEditor, { id: "f1" });
    await screen.findByRole("textbox", { name: "Flow name" });
    const status = screen.getAllByRole("status")[0];
    expect(status.textContent).toContain("Saved");
    expect(status.textContent).toContain("Not deployed");
    await waitFor(() => expect(api.checkFlow).toHaveBeenCalled());
    await waitFor(() => expect((screen.getByRole("button", { name: "Deploy" }) as HTMLButtonElement).disabled).toBe(false));
    // Through the shell's keydown and the editor's "Deploy flow" command (works while typing).
    const off = installShortcuts({ route: "flowEditor" });
    try {
      await fireEvent.keyDown(screen.getByRole("textbox", { name: "Flow name" }), { key: "Enter", ctrlKey: true });
      await waitFor(() => expect(api.deployFlow).toHaveBeenCalledWith("f1"));
    } finally {
      off();
    }
  });

  it("Deploy is disabled with the reason while there are errors", async () => {
    server = flow({ draft: { nodes: [cue, take], edges: [], updatedAt: "x" } });
    api.checkFlow.mockResolvedValue({ errors: [{ node: "take", message: "Pick a layer" }], warnings: [] });
    render(FlowEditor, { id: "f1" });
    await waitFor(() => expect(screen.getByText("1 problem to fix first.")).toBeTruthy());
    expect((screen.getByRole("button", { name: "Deploy" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("Discard draft asks first", async () => {
    server = flow({ draft: { nodes: [cue], edges: [], updatedAt: "x" } });
    render(FlowEditor, { id: "f1" });
    await fireEvent.click(await screen.findByRole("button", { name: "Discard draft" }));
    const dialog = await screen.findByRole("dialog", { name: "Discard the draft?" });
    expect(api.discardFlowDraft).not.toHaveBeenCalled();
    await fireEvent.click(within(dialog).getByRole("button", { name: "Discard draft" }));
    await waitFor(() => expect(api.discardFlowDraft).toHaveBeenCalledWith("f1"));
  });

  it("a read-only flow: a note, Duplicate, no Deploy", async () => {
    server = flow({ readOnly: true });
    render(FlowEditor, { id: "f1" });
    await screen.findByText(/it's read-only/);
    expect(screen.getByRole("button", { name: "Duplicate" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Deploy" })).toBeNull();
    expect((screen.getByRole("textbox", { name: "Flow name" }) as HTMLInputElement).disabled).toBe(true);
  });
});

describe("Inspector", () => {
  it("wires by keyboard: Connect to… lists the ports that fit and adds the wire", async () => {
    const { ed, off } = await open();
    vi.useRealTimers();
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "if")! });
    const button = screen.getByRole("button", { name: "Connect to… (else)" });
    await fireEvent.keyDown(button, { key: "ArrowDown" });
    const menu = await screen.findByRole("menu", { name: "Connect to… (else)" });
    const items = within(menu).getAllByRole("menuitem");
    expect(items.map((i) => i.textContent?.trim())).toEqual(["Take: Take Guest › in"]);
    await fireEvent.click(items[0]);
    expect(ed.graph.edges.some((e) => e.from.port === "else" && e.to.node === "take")).toBe(true);
    // Its wires are listed with Remove.
    await tick();
    await fireEvent.click(screen.getByRole("button", { name: "Remove the wire from else to Take in" }));
    expect(ed.graph.edges.some((e) => e.from.port === "else")).toBe(false);
    off();
  });

  it("shows the node's problems and its config form", async () => {
    api.checkFlow.mockResolvedValue({ errors: [{ node: "take", message: "Pick a layer" }], warnings: [] });
    const { ed, off } = await open();
    await vi.advanceTimersByTimeAsync(CHECK_DEBOUNCE_MS);
    vi.useRealTimers();
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "take")! });
    expect(screen.getByRole("heading", { name: "Take" })).toBeTruthy();
    expect(screen.getByText("Pick a layer")).toBeTruthy();
    // The config form is a lazy chunk: its first import (cold transform) can take over a second.
    expect(await screen.findByRole("combobox", { name: "Layer" }, { timeout: 5_000 })).toBeTruthy();
    off();
  });

  it("Set variant: the layer, its template's variants (default marked) and the transition", async () => {
    const look = (id: string, name: string) => ({ id, name, vars: {}, html: "", css: "" });
    api.getScreen.mockResolvedValue({
      doc: { id: "s1", layers: [{ id: "l1", templateId: "t1" }] },
      live: view,
      warnings: [],
    });
    api.getTemplate.mockResolvedValue({ id: "t1", defaultVariant: "full", variants: [look("full", "Full"), look("compact", "Compact")] });
    const sv = n("sv", { type: "action.setVariant", config: { screenId: "s1", layerId: "l1", variant: "compact", transition: "outIn" } }, 720);
    const { ed, off } = await open(flow({ nodes: [cue, branch, take, sv] }));
    vi.useRealTimers();
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "sv")! });
    expect(screen.getByRole("heading", { name: "Set variant" })).toBeTruthy();
    const variant = (await screen.findByRole("combobox", { name: "Variant" })) as HTMLSelectElement;
    await waitFor(() => expect([...variant.options].map((o) => o.textContent)).toEqual(["Full (default)", "Compact"]));
    expect(variant.value).toBe("compact");
    expect(api.getScreen).toHaveBeenCalledWith("s1");
    expect(api.getTemplate).toHaveBeenCalledWith("t1");
    await fireEvent.change(variant, { target: { value: "full" } });
    expect(ed.graph.nodes.find((x) => x.id === "sv")!.config).toMatchObject({ variant: "full", transition: "outIn" });
    const transition = screen.getByRole("combobox", { name: "Transition" }) as HTMLSelectElement;
    expect(transition.value).toBe("outIn");
    await fireEvent.change(transition, { target: { value: "crossfade" } });
    // (The rendered node prop is fixed here, so only the changed key is checked.)
    expect(ed.graph.nodes.find((x) => x.id === "sv")!.config).toMatchObject({ transition: "crossfade" });
    off();
  });

  it("Repeat: rounds 1 to 10 000, Forever sets null (and back to 3), the wait between rounds", async () => {
    const rep = n("rep", { type: "logic.repeat", config: { count: 5, delayMs: 0 } }, 720);
    const { ed, off } = await open(flow({ nodes: [cue, branch, take, rep] }));
    vi.useRealTimers();
    const cfg = () => ed.graph.nodes.find((x) => x.id === "rep")!.config;
    const r = render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "rep")! });
    // The inspector gets the edited node back (the test passes it by hand).
    const refresh = () => r.rerender({ ed, node: ed.graph.nodes.find((x) => x.id === "rep")! });
    expect(screen.getByRole("heading", { name: "Repeat" })).toBeTruthy();
    const rounds = (await screen.findByLabelText("Rounds", {}, { timeout: 5_000 })) as HTMLInputElement;
    expect(rounds.value).toBe("5");
    expect([rounds.min, rounds.max]).toEqual(["1", "10000"]);
    await fireEvent.change(rounds, { target: { value: "20000" } });
    expect(cfg()).toEqual({ count: 10000, delayMs: 0 });
    await refresh();
    await fireEvent.change(rounds, { target: { value: "0" } });
    expect(cfg()).toEqual({ count: 1, delayMs: 0 });
    await refresh();
    await fireEvent.change(screen.getByLabelText("Wait between rounds"), { target: { value: "1.5" } });
    expect(cfg()).toEqual({ count: 1, delayMs: 1500 });
    await refresh();
    expect(screen.getByText("Each round waits for the body to finish. Rounds start at most every 100 ms.")).toBeTruthy();
    const forever = screen.getByRole("switch", { name: "Forever" });
    expect(forever.getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(forever);
    expect(cfg()).toEqual({ count: null, delayMs: 1500 });
    // Forever: no rounds field; switching it off starts again at 3.
    await refresh();
    expect(screen.queryByLabelText("Rounds")).toBeNull();
    await fireEvent.click(screen.getByRole("switch", { name: "Forever" }));
    expect(cfg()).toEqual({ count: 3, delayMs: 1500 });
    off();
  });

  it("Stop flow: any flow, this one listed as \"This flow\"; Run flow hides it", async () => {
    const stop = n("stop", { type: "action.stopFlow", config: { flowId: "" } }, 720);
    const run = n("run", { type: "action.runFlow", config: { flowId: "", wait: false } }, 960);
    const { ed, off } = await open(flow({ nodes: [cue, branch, take, stop, run] }));
    vi.useRealTimers();
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "stop")! });
    expect(screen.getByRole("heading", { name: "Stop flow" })).toBeTruthy();
    const pick = (await screen.findByRole("combobox", { name: "Flow to stop" }, { timeout: 5_000 })) as HTMLSelectElement;
    expect([...pick.options].map((o) => o.textContent)).toEqual(["Choose a flow", "This flow"]);
    await fireEvent.change(pick, { target: { value: "f1" } });
    expect(ed.graph.nodes.find((x) => x.id === "stop")!.config).toEqual({ flowId: "f1" });
    cleanup();
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "run")! });
    const runPick = (await screen.findByRole("combobox", { name: "Flow to run" }, { timeout: 5_000 })) as HTMLSelectElement;
    expect([...runPick.options].map((o) => o.textContent)).toEqual(["No other flows — use the Repeat node to loop this flow."]);
    off();
  });

  it("a paced Repeat from live debug is a warning on the node and a note in the header", async () => {
    const rep = n("rep", { type: "logic.repeat", config: { count: null, delayMs: 0 } }, 720);
    const { ed, off } = await open(flow({ nodes: [cue, branch, take, rep] }));
    vi.useRealTimers();
    const paced = "Looping faster than every 100 ms — paced.";
    emit("flow-debug", { id: "f1", nodes: [{ node: "rep", at: "2026-10-04T10:00:00Z" }], edges: [], errors: [{ node: "rep", message: paced }] });
    expect(ed.issues.get("rep")).toEqual({ errors: [], warnings: [paced] });
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "rep")! });
    const li = screen.getByText(paced).closest("li")!;
    expect(li.className).toContain("warn");
    // A new run clears it.
    emit("flow-state", { id: "f1", state: "running", node: null, message: null, at: "2026-10-04T10:01:00Z" });
    expect(ed.issues.get("rep")).toBeUndefined();
    off();
  });

  it("header: a running flow's note (flow-state message) shows next to Running", async () => {
    render(FlowEditor, { id: "f1" });
    await screen.findByRole("textbox", { name: "Flow name" });
    emit("flow-state", { id: "f1", state: "running", node: null, message: null, at: "2026-10-04T10:00:00Z" });
    emit("flow-state", { id: "f1", state: "running", node: "rep", message: "Looping faster than every 100 ms — paced.", at: "2026-10-04T10:00:01Z" });
    await tick();
    const status = screen.getAllByRole("status")[0];
    await waitFor(() => expect(status.textContent).toContain("Running"));
    const note = within(status).getByText("Looping faster than every 100 ms — paced.");
    expect(note.className).toContain("warn");
    emit("flow-state", { id: "f1", state: "stopped", node: null, message: "Stopped", at: "2026-10-04T10:00:05Z" });
    await tick();
    expect(within(status).queryByText("Looping faster than every 100 ms — paced.")).toBeNull();
  });

  it("Take preview: the screen, and the transition (project default, a kind, a crossfade's duration)", async () => {
    const tp = n("tp", { type: "action.takePreview", config: { screenId: "s1", transition: { kind: "crossfade", ms: 800 } } }, 720);
    const { ed, off } = await open(flow({ nodes: [cue, branch, take, tp] }));
    vi.useRealTimers();
    render(Inspector, { ed, node: ed.graph.nodes.find((x) => x.id === "tp")! });
    expect(screen.getByRole("heading", { name: "Take preview" })).toBeTruthy();
    const scr = (await screen.findByRole("combobox", { name: "Screen" })) as HTMLSelectElement;
    expect(scr.value).toBe("s1");
    const tr = screen.getByRole("combobox", { name: "Transition" }) as HTMLSelectElement;
    expect([...tr.options].map((o) => o.textContent)).toEqual(["Project default", "Cut", "Crossfade", "Play out & in"]);
    expect(tr.value).toBe("crossfade");
    const ms = screen.getByLabelText("Crossfade duration") as HTMLInputElement;
    expect(ms.value).toBe("800");
    await fireEvent.change(ms, { target: { value: "1200" } });
    expect(ed.graph.nodes.find((x) => x.id === "tp")!.config).toEqual({ screenId: "s1", transition: { kind: "crossfade", ms: 1200 } });
    await fireEvent.change(tr, { target: { value: "default" } });
    expect(ed.graph.nodes.find((x) => x.id === "tp")!.config).toEqual({ screenId: "s1", transition: null });
    await fireEvent.change(tr, { target: { value: "outIn" } });
    expect(ed.graph.nodes.find((x) => x.id === "tp")!.config).toEqual({ screenId: "s1", transition: { kind: "outIn", ms: 800 } });
    off();
  });
});
