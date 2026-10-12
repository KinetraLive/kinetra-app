// Per-layer trailing debounce for Control field edits: every field typed into one layer within
// `delayMs` of quiet goes out as ONE `setLayerFields` call with all changed fields.
//
//   const edits = new LayerEdits(150, (key, fields) => setLayerFields(screenId, layerId, fields));
//   edits.push("s1/l1", "name", "Ada");
//   await edits.flush("s1/l1");   // before TAKE, on blur
//
// A timer exists only while an edit is pending; nothing runs at rest. `send` reports its own
// errors; flush never rejects.

export class LayerEdits {
  readonly #delay: number;
  readonly #send: (key: string, fields: Record<string, string>) => Promise<unknown>;
  readonly #timers = new Map<string, ReturnType<typeof setTimeout>>();
  readonly #fields = new Map<string, Record<string, string>>();
  /** Sends in flight per key, so a flush also waits for an earlier send of the same layer. */
  readonly #inflight = new Map<string, Promise<void>>();

  constructor(delayMs: number, send: (key: string, fields: Record<string, string>) => Promise<unknown>) {
    this.#delay = delayMs;
    this.#send = send;
  }

  push(key: string, field: string, value: string): void {
    this.#fields.set(key, { ...this.#fields.get(key), [field]: value });
    const t = this.#timers.get(key);
    if (t !== undefined) clearTimeout(t);
    this.#timers.set(
      key,
      setTimeout(() => void this.flush(key), this.#delay),
    );
  }

  pending(key: string): boolean {
    return this.#timers.has(key);
  }

  /** Sends the key's pending edits now (if any) and waits until they and earlier sends are done. */
  flush(key: string): Promise<void> {
    const t = this.#timers.get(key);
    if (t === undefined) return this.#inflight.get(key) ?? Promise.resolve();
    clearTimeout(t);
    this.#timers.delete(key);
    const fields = this.#fields.get(key) ?? {};
    this.#fields.delete(key);
    const before = this.#inflight.get(key) ?? Promise.resolve();
    // Keep the order per layer: the next send starts after the previous one settled.
    const p: Promise<void> = before
      .then(() => this.#send(key, fields))
      .then(
        () => {},
        () => {},
      )
      .finally(() => {
        if (this.#inflight.get(key) === p) this.#inflight.delete(key);
      });
    this.#inflight.set(key, p);
    return p;
  }

  async flushAll(): Promise<void> {
    await Promise.all([...new Set([...this.#timers.keys(), ...this.#inflight.keys()])].map((k) => this.flush(k)));
  }

  /** Drops pending edits without sending them. */
  cancel(): void {
    for (const t of this.#timers.values()) clearTimeout(t);
    this.#timers.clear();
    this.#fields.clear();
  }
}
