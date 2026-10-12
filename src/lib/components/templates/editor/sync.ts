// Timing machinery of the template editor (autosave also used by the screen editor): preview
// drafts, autosave and the reaction to files changed on disk. No Svelte, no DOM.
// Timers exist only while something is pending; nothing runs at rest.

import type { FileKind, FilesChanged, TemplateDraft } from "$lib/types";

type Timer = ReturnType<typeof setTimeout>;
type Kind = keyof TemplateDraft;
type MapKind = "fieldValues" | "variableValues";
type Lazy<T> = T | (() => T);

// ---- Preview drafts ----------------------------------------------------------------------------

/**
 * Debounces preview drafts per kind (html, css, fields, ...): each kind sends `{ [kind]: latest }`
 * `delayMs` after its last change. The two value maps merge their changed keys instead, so only
 * changed values are sent. Values may be thunks, so large text (HTML/CSS) is only read when sent.
 */
export class DraftSender {
  #timers = new Map<Kind, Timer>();
  #pending = new Map<Kind, unknown>();

  constructor(
    private readonly delayMs: number,
    private readonly send: (draft: TemplateDraft) => Promise<unknown>,
  ) {}

  queue<K extends Exclude<Kind, MapKind>>(kind: K, value: Lazy<TemplateDraft[K]>): void {
    this.#pending.set(kind, value);
    this.#arm(kind);
  }

  merge(kind: MapKind, patch: Record<string, string>): void {
    const prev = (this.#pending.get(kind) as Record<string, string> | undefined) ?? {};
    this.#pending.set(kind, { ...prev, ...patch });
    this.#arm(kind);
  }

  pending(kind?: Kind): boolean {
    return kind ? this.#timers.has(kind) : this.#timers.size > 0;
  }

  /** Sends everything pending now. */
  flush(): Promise<void> {
    return Promise.all([...this.#timers.keys()].map((k) => this.#fire(k))).then(() => {});
  }

  /** Drops pending drafts without sending. */
  cancel(): void {
    for (const t of this.#timers.values()) clearTimeout(t);
    this.#timers.clear();
    this.#pending.clear();
  }

  #arm(kind: Kind) {
    const t = this.#timers.get(kind);
    if (t !== undefined) clearTimeout(t);
    this.#timers.set(
      kind,
      setTimeout(() => void this.#fire(kind), this.delayMs),
    );
  }

  #fire(kind: Kind): Promise<void> {
    const t = this.#timers.get(kind);
    if (t !== undefined) clearTimeout(t);
    this.#timers.delete(kind);
    if (!this.#pending.has(kind)) return Promise.resolve();
    const raw = this.#pending.get(kind);
    this.#pending.delete(kind);
    const value = typeof raw === "function" ? (raw as () => unknown)() : raw;
    return this.send({ [kind]: value } as TemplateDraft).then(
      () => {},
      () => {},
    );
  }
}

// ---- Autosave ----------------------------------------------------------------------------------

export type SaveState =
  | { kind: "saved"; at: string | null }
  | { kind: "pending" }
  | { kind: "saving" }
  | { kind: "failed"; message: string };

export interface AutosaveOptions<D> {
  delayMs: number;
  /** Builds the document to save (called only when a save starts). */
  snapshot: () => D;
  /** Null when the draft may be saved, otherwise why not. */
  blocked: () => string | null;
  save: (doc: D) => Promise<{ updatedAt: string }>;
  onState: (s: SaveState) => void;
  /** Error → message. */
  describe: (e: unknown) => string;
}

/**
 * Debounced autosave with a single save in flight. `touch()` after every edit; the save starts
 * `delayMs` after the last one. An edit during a save queues one more save with the latest doc.
 * A blocked (invalid) draft is never saved; it stays dirty until an edit makes it valid.
 */
export class Autosave<D> {
  #timer: Timer | undefined;
  #dirty = false;
  #inFlight: Promise<void> | null = null;
  #disposed = false;

  constructor(private readonly o: AutosaveOptions<D>) {}

  get dirty(): boolean {
    return this.#dirty;
  }

  get saving(): boolean {
    return this.#inFlight !== null;
  }

  touch(): void {
    if (this.#disposed) return;
    this.#dirty = true;
    this.#clear();
    this.#timer = setTimeout(() => void this.run(), this.o.delayMs);
    if (!this.#inFlight) this.o.onState({ kind: "pending" });
  }

  /** Saves now (Ctrl+S, Retry). Resolves when this save (and any queued one) has settled. */
  saveNow(): Promise<void> {
    this.#dirty = true;
    return this.run();
  }

  /** Saves now only if there are unsaved edits; resolves when nothing is in flight. */
  flush(): Promise<void> {
    if (this.#dirty) return this.run();
    return this.#inFlight ?? Promise.resolve();
  }

  /**
   * Forgets unsaved edits but keeps working (e.g. Reload from disk). A save already in flight
   * still settles.
   */
  discard(): void {
    this.#clear();
    this.#dirty = false;
  }

  /** Stops the timer and forgets unsaved edits (e.g. the template was deleted). */
  cancel(): void {
    this.#clear();
    this.#dirty = false;
    this.#disposed = true;
  }

  run(): Promise<void> {
    this.#clear();
    if (!this.#dirty) return this.#inFlight ?? Promise.resolve();
    // One save in flight; it re-runs with the latest doc when it settles (we're still dirty).
    if (this.#inFlight) return this.#inFlight;
    if (this.o.blocked() !== null) return Promise.resolve(); // stays dirty; status shows why
    this.#dirty = false;
    this.o.onState({ kind: "saving" });
    const doc = this.o.snapshot();
    const p = this.o.save(doc).then(
      (saved) => {
        this.#inFlight = null;
        if (this.#dirty) {
          // Edited while saving: save the latest doc too.
          this.o.onState({ kind: "pending" });
          return this.run();
        }
        this.o.onState({ kind: "saved", at: saved.updatedAt });
      },
      (e: unknown) => {
        this.#inFlight = null;
        this.#dirty = !this.#disposed;
        this.o.onState({ kind: "failed", message: this.o.describe(e) });
      },
    );
    this.#inFlight = p;
    return p;
  }

  #clear() {
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
  }
}

// ---- External changes ---------------------------------------------------------------------------

export type DiskReaction = "ignore" | "reload" | "conflict" | "removed" | "invalid";

/**
 * What an open editor does about a `project-files-changed` event: ignore other files; reload
 * silently when there are no unsaved edits; otherwise ask (Reload / Keep mine). A removed file is
 * announced; an invalid one is reported while the working copy is kept.
 */
export function diskReaction(e: FilesChanged, kind: FileKind, id: string, dirty: boolean): DiskReaction {
  if (e.kind !== kind || e.id !== id) return "ignore";
  if (e.change === "removed") return "removed";
  if (e.change === "invalid") return "invalid";
  return dirty ? "conflict" : "reload";
}
