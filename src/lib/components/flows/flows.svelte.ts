// The open project's flows, for Control (cue buttons, the Automation strip). Held by
// ControlCtl, so it lives exactly as long as the Control page:
//
//   const flows = new FlowsState();
//   $effect(() => flows.connect());   // loads the list, follows flow-state and files-changed
//
// `flow-state` updates a flow's running state in place (and `lastFiredAt` from `at` when it starts);
// the last failure per flow is kept to show its message. A flow file changed on disk
// (`project-files-changed`, kind "flow") reloads that flow or drops it. Run is optimistic and
// rolls back on failure (only when nothing newer arrived meanwhile). What only the lazy chunks
// (Automation strip, editor) do (arm, create, save, duplicate, delete) is in ./ops.ts.

import { on, toAppError } from "$lib/api";
import { getFlow, listFlows, runFlow, stopFlow } from "$lib/api/flows";
import { toasts } from "$lib/stores/toasts.svelte";
import type { Flow, FlowInfo, FlowState } from "$lib/types";

/** A flow's Cue trigger config, or null when it has none (then it is no cue button). */
export function cueOf(flow: Pick<Flow, "nodes">): { label?: string | null } | null {
  for (const n of flow.nodes) if (n.type === "trigger.cue") return n.config;
  return null;
}

/** A flow aims at a screen when any node (trigger, data or action) names it. */
export function aimsAt(flow: Pick<Flow, "nodes">, screenId: string): boolean {
  return flow.nodes.some((n) => "screenId" in n.config && n.config.screenId === screenId);
}

/** The text on a cue button: its label, else the flow's name. */
export function cueLabel(flow: Pick<Flow, "nodes" | "name">): string {
  return cueOf(flow)?.label?.trim() || flow.name;
}

/** "12 s ago", "3 min ago", "2 h ago", "4 d ago" (formatted on render; no timers). */
export function relTime(iso: string, now = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return `${s} s ago`;
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

export class FlowsState {
  /** In the backend's order. */
  list = $state.raw<FlowInfo[]>([]);
  loaded = $state(false);
  /** Flow id → its last failure (cleared when it runs again or finishes). */
  failed = $state.raw<ReadonlyMap<string, FlowState>>(new Map());
  /** Flows with a Cue trigger: Control's cue buttons. */
  cues = $derived(this.list.filter((f) => cueOf(f) !== null));

  connect(): () => void {
    const offState = on("flow-state", (s) => this.#onState(s));
    const offFiles = on("project-files-changed", (c) => {
      if (c.kind !== "flow") return;
      if (c.change === "removed") this.drop(c.id);
      else void this.reload(c.id);
    });
    void this.load();
    return () => {
      offFiles();
      offState();
    };
  }

  async load(): Promise<void> {
    try {
      this.list = await listFlows();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't load the flows: ${toAppError(e).message}` });
    } finally {
      this.loaded = true;
    }
  }

  /** Reloads one flow (gone: dropped). */
  async reload(id: string): Promise<void> {
    try {
      this.put(await getFlow(id));
    } catch {
      this.drop(id);
    }
  }

  get(id: string): FlowInfo | undefined {
    return this.list.find((f) => f.id === id);
  }

  /** Replaces a flow (or adds it at the end). */
  put(f: FlowInfo): void {
    const known = this.list.some((x) => x.id === f.id);
    this.list = known ? this.list.map((x) => (x.id === f.id ? f : x)) : [...this.list, f];
  }

  drop(id: string): void {
    this.list = this.list.filter((f) => f.id !== id);
    this.#setFailed(id, null);
  }

  #setFailed(id: string, s: FlowState | null) {
    if (!s && !this.failed.has(id)) return;
    const next = new Map(this.failed);
    if (s) next.set(id, s);
    else next.delete(id);
    this.failed = next;
  }

  #onState(s: FlowState) {
    const f = this.get(s.id);
    const running = s.state === "running";
    this.#setFailed(s.id, s.state === "failed" ? s : null);
    if (f) this.put({ ...f, running, ...(running ? { lastFiredAt: s.at } : {}) });
  }

  /** Runs (or restarts) a cue; shown running at once. A disarmed flow doesn't run. */
  run = async (id: string): Promise<void> => {
    const before = this.get(id);
    if (!before) return;
    if (!before.armed) {
      toasts.push({ kind: "error", message: `${cueLabel(before)} is disarmed.` });
      return;
    }
    const optimistic = { ...before, running: true };
    this.put(optimistic);
    this.#setFailed(id, null);
    try {
      await runFlow(id);
    } catch (e) {
      if (this.get(id) === optimistic) this.put(before);
      toasts.push({ kind: "error", message: `Couldn't run ${cueLabel(before)}: ${toAppError(e).message}` });
    }
  };

  stop = async (id: string): Promise<void> => {
    try {
      await stopFlow(id);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't stop the flow: ${toAppError(e).message}` });
    }
  };
}
