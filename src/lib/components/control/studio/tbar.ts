// The T-bar's wire side (part of the lazy studio chunk): a rate limit for positions and the studio
// ops of one drag, sent strictly in order. Plain TS, unit-tested.
//
//   const tb = new TbarOps(send);          // send: (op) => Promise<unknown>
//   tb.begin(transition); tb.set(0.4); …; tb.end(p >= TBAR_COMMIT);
//
// Positions go out at most TBAR_RATE per second (a pending position is sent once the gap has
// passed; `end` drops one still waiting: the commit or abort says where the bar ends). Ops queue
// behind each other, so `tbarSet` never overtakes `tbarBegin` and `tbarEnd` comes last.

import type { StudioOp, TakeTransition } from "$lib/types";

/** Most `tbarSet` messages per second. */
export const TBAR_RATE = 30;
/** Released at or past this position, the T-bar commits the take. */
export const TBAR_COMMIT = 0.999;
/** Keyboard steps: arrows, PageUp / PageDown. */
export const TBAR_STEP = 0.05;
export const TBAR_PAGE = 0.25;

export const clamp01 = (p: number): number => (p < 0 ? 0 : p > 1 ? 1 : p);

/** Calls `send` at most `perSec` times a second with the latest value. */
export class Throttle<T> {
  readonly #gap: number;
  readonly #send: (v: T) => void;
  #last = -Infinity;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #next: { v: T } | null = null;

  constructor(perSec: number, send: (v: T) => void) {
    this.#gap = 1000 / perSec;
    this.#send = send;
  }

  push(v: T): void {
    this.#next = { v };
    const wait = this.#last + this.#gap - performance.now();
    if (wait <= 0) this.#fire();
    else this.#timer ??= setTimeout(() => this.#fire(), Math.ceil(wait));
  }

  /** Drops a waiting value. */
  cancel(): void {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    this.#next = null;
  }

  #fire(): void {
    if (this.#timer !== null) clearTimeout(this.#timer);
    this.#timer = null;
    const n = this.#next;
    if (!n) return;
    this.#next = null;
    this.#last = performance.now();
    this.#send(n.v);
  }
}

/** One screen's T-bar ops, serialized. */
export class TbarOps {
  readonly #send: (op: StudioOp) => Promise<unknown>;
  #q: Promise<unknown> = Promise.resolve();
  readonly #pos = new Throttle<number>(TBAR_RATE, (p) => this.#enqueue({ op: "tbarSet", p }));
  /** A drag (or keyboard run) is in progress. */
  active = false;

  constructor(send: (op: StudioOp) => Promise<unknown>) {
    this.#send = send;
  }

  #enqueue(op: StudioOp): void {
    this.#q = this.#q.then(
      () => this.#send(op),
      () => this.#send(op),
    );
  }

  begin(transition: TakeTransition | null): void {
    if (this.active) return;
    this.active = true;
    this.#enqueue({ op: "tbarBegin", transition });
  }

  set(p: number): void {
    if (this.active) this.#pos.push(clamp01(p));
  }

  /** Released: the commit (or abort); a position still waiting for its slot is dropped. */
  end(commit: boolean): Promise<unknown> {
    if (!this.active) return this.#q;
    this.active = false;
    this.#pos.cancel();
    this.#enqueue({ op: "tbarEnd", commit });
    return this.#q;
  }

  /** The screen changed under a running T-bar: abort it. */
  dispose(): void {
    this.#pos.cancel();
    if (this.active) void this.end(false);
  }
}
