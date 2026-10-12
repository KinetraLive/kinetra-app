// The open project's flows for the Flows pages (/flows and the editor): the list in the backend's
// order, each flow's running state from `flow-state` (and `lastFiredAt` when it starts), its last
// failure, and the flow files changed on disk (`project-files-changed`, kind flow: reloaded or
// dropped). Arming is optimistic and rolls back on failure (only when nothing newer arrived).
//
// The same job as Control's FlowsState (../flows.svelte.ts), kept apart on purpose: a module both
// Control and these routes import is split out of Control's route chunk, which costs its budget.
//
//   const flows = new FlowList();
//   $effect(() => flows.connect());
//
// The screens list (ScreenViews) is in ./screens.svelte.ts: the /flows list loads it lazily.

import { on, toAppError } from "$lib/api";
import { getFlow, listFlows, setFlowArmed } from "$lib/api/flows";
import { toasts } from "$lib/stores/toasts.svelte";
import type { FlowInfo, FlowState } from "$lib/types";

export class FlowList {
  list = $state.raw<FlowInfo[]>([]);
  loaded = $state(false);
  /** Flow id → its last failure (cleared when it runs again or finishes). */
  failed = $state.raw<ReadonlyMap<string, FlowState>>(new Map());
  /** Flow id → a note on its current run (`running` with a message, e.g. a paced Repeat). */
  notes = $state.raw<ReadonlyMap<string, string>>(new Map());

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
      this.list = (await listFlows()) ?? [];
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't load the flows: ${toAppError(e).message}` });
    } finally {
      this.loaded = true;
    }
  }

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

  /** Arm or disarm (optimistic; rolled back on failure, only when nothing newer arrived). */
  setArmed = async (id: string, armed: boolean): Promise<void> => {
    const before = this.get(id);
    if (!before) return;
    const optimistic = { ...before, armed, running: armed && before.running };
    this.put(optimistic);
    try {
      this.put(await setFlowArmed(id, armed));
    } catch (e) {
      if (this.get(id) === optimistic) this.put(before);
      toasts.push({ kind: "error", message: `Couldn't ${armed ? "arm" : "disarm"} ${before.name}: ${toAppError(e).message}` });
    }
  };

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
    const note = running ? s.message : null;
    if ((this.notes.get(s.id) ?? null) !== note) {
      const notes = new Map(this.notes);
      if (note) notes.set(s.id, note);
      else notes.delete(s.id);
      this.notes = notes;
    }
    // A note mid-run (running + message) isn't a new start.
    if (f) this.put({ ...f, running, ...(running && !s.message ? { lastFiredAt: s.at } : {}) });
  }
}
