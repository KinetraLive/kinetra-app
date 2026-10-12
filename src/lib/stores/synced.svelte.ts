// A value mirrored from the Rust side with optimistic writes.
//
// `update` shows the new value immediately and sends it to the backend. Only the latest write
// decides the outcome: an older request resolving late never overwrites a newer value, and a
// failure rolls back to the last value the backend confirmed, not to whatever was on screen.
// The backend coalesces disk writes, so calling `update` often is cheap; still prefer committing
// on `change` rather than every `input` event for sliders.

export class SyncedStore<T> {
  /** `$state.raw`: values are replaced wholesale, never mutated, so no deep proxy is needed. */
  value = $state.raw<T | null>(null);

  #confirmed: T | null = null;
  #seq = 0;
  #load: () => Promise<T>;
  #save: (v: T) => Promise<T>;

  constructor(load: () => Promise<T>, save: (v: T) => Promise<T>) {
    this.#load = load;
    this.#save = save;
  }

  async load(): Promise<T> {
    const seq = ++this.#seq;
    const v = await this.#load();
    if (seq === this.#seq) {
      this.#confirmed = v;
      this.value = v;
    }
    return v;
  }

  /** Sets the value without a backend round trip (e.g. after a command returned it). */
  set(v: T | null) {
    this.#seq++;
    this.#confirmed = v;
    this.value = v;
  }

  /**
   * Applies `next` optimistically. Rejects with the backend's `AppError` after rolling back.
   * Resolves with the stored value (the backend may clamp it).
   */
  async update(next: T): Promise<T> {
    const seq = ++this.#seq;
    this.value = next;
    try {
      const stored = await this.#save(next);
      if (seq === this.#seq) {
        this.#confirmed = stored;
        this.value = stored;
      }
      return stored;
    } catch (e) {
      if (seq === this.#seq) this.value = this.#confirmed;
      throw e;
    }
  }
}
