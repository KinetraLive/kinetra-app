// Live debug state of the open flow: which nodes and wires are flashing, each node's last-fired
// time and its last run-time error or warning, from `flow-debug` batches.
//
// Most run-time messages are errors (--danger), cleared when the node runs again without one. A few
// are information (`debugSeverity`: Repeat's "Looping faster than every 100 ms — paced."): those are
// warnings (--warning), sent once per run, and stay until the flow starts a new run (`newRun`).
//
// A flash lasts FLASH_MS (the CSS fades in over 120 ms and out over 220 ms) or, with reduced
// motion, REDUCED_FLASH_MS of static highlight. One timer, to the earliest expiry, exists only while
// something is flashing: nothing runs at rest.

import type { FlowDebug } from "$lib/types";

export const FLASH_MS = 340;

/** Run-time messages that report, not fail (by their start, as the engine words them). */
const WARNING_STARTS = ["Looping faster"];

/** Whether a run-time message from `flow-debug` is an error or a warning. */
export function debugSeverity(message: string): "error" | "warning" {
  return WARNING_STARTS.some((s) => message.startsWith(s)) ? "warning" : "error";
}
export const REDUCED_FLASH_MS = 1000;

export class DebugState {
  /** Node ids currently flashing. Replaced wholesale. */
  nodes = $state.raw<ReadonlySet<string>>(new Set());
  /** Edge ids currently flashing. */
  edges = $state.raw<ReadonlySet<string>>(new Set());
  /** Node id → RFC 3339 of its last run seen in this session. */
  fired = $state.raw<ReadonlyMap<string, string>>(new Map());
  /** Node id → its last run-time error (cleared when it runs again without one). */
  errors = $state.raw<ReadonlyMap<string, string>>(new Map());
  /** Node id → its run-time warning in the current run (kept while it runs again). */
  warnings = $state.raw<ReadonlyMap<string, string>>(new Map());

  #until = new Map<string, number>();
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(private readonly reducedMotion: () => boolean = () => false) {}

  /** Applies one batch of the open flow. */
  apply(b: FlowDebug): void {
    const now = Date.now();
    const ms = this.reducedMotion() ? REDUCED_FLASH_MS : FLASH_MS;
    if (b.nodes.length > 0) {
      const fired = new Map(this.fired);
      for (const n of b.nodes) fired.set(n.node, n.at);
      this.fired = fired;
    }
    const ran = new Set(b.nodes.map((n) => n.node));
    const failed = b.errors.filter((e) => e.node && debugSeverity(e.message) === "error");
    if (ran.size > 0 || failed.length > 0) {
      const errors = new Map(this.errors);
      for (const id of ran) errors.delete(id);
      for (const e of failed) if (e.node) errors.set(e.node, e.message);
      if (errors.size !== this.errors.size || failed.length > 0) this.errors = errors;
    }
    const warned = b.errors.filter((e) => e.node && debugSeverity(e.message) === "warning");
    if (warned.some((w) => this.warnings.get(w.node ?? "") !== w.message)) {
      const warnings = new Map(this.warnings);
      for (const w of warned) if (w.node) warnings.set(w.node, w.message);
      this.warnings = warnings;
    }
    for (const id of ran) this.#until.set(`n:${id}`, now + ms);
    for (const id of b.edges) this.#until.set(`e:${id}`, now + ms);
    this.#publish();
    this.#arm(now);
  }

  /** The flow started a new run: the last run's warnings go. */
  newRun(): void {
    if (this.warnings.size > 0) this.warnings = new Map();
  }

  /** Clears the flashes (e.g. the window was hidden); keeps last-fired times and errors. */
  clearFlashes(): void {
    this.#until.clear();
    this.#stop();
    this.#publish();
  }

  dispose(): void {
    this.#until.clear();
    this.#stop();
  }

  get pending(): boolean {
    return this.#timer !== undefined;
  }

  #publish() {
    const nodes = new Set<string>();
    const edges = new Set<string>();
    for (const k of this.#until.keys()) (k.startsWith("n:") ? nodes : edges).add(k.slice(2));
    if (!sameSet(nodes, this.nodes)) this.nodes = nodes;
    if (!sameSet(edges, this.edges)) this.edges = edges;
  }

  #stop() {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
  }

  #arm(now: number) {
    this.#stop();
    if (this.#until.size === 0) return;
    const next = Math.min(...this.#until.values());
    this.#timer = setTimeout(() => this.#expire(), Math.max(0, next - now));
  }

  #expire() {
    this.#timer = undefined;
    const now = Date.now();
    for (const [k, t] of this.#until) if (t <= now) this.#until.delete(k);
    this.#publish();
    this.#arm(now);
  }
}

function sameSet(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const x of a) if (!b.has(x)) return false;
  return true;
}
