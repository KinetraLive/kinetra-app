// App-wide toast queue, rendered once by <ToastHost /> in the root layout.
//
//   const id = toasts.push({ kind: "success", message: "Project created" });
//   toasts.dismiss(id);
//
// `error` toasts stay until dismissed; `info`/`success` auto-dismiss after 4 s. Pass `timeoutMs`
// to override (0 = sticky). At most MAX_TOASTS are visible; pushing more drops the oldest.
// A timer exists only while its toast is visible and is cleared on dismiss. An `action` (e.g.
// Undo) shows as a button; choosing it dismisses the toast, then runs it.

export type ToastKind = "info" | "success" | "error";

export interface ToastAction {
  label: string;
  run: () => void;
}

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  action?: ToastAction;
}

export interface ToastInput {
  kind: ToastKind;
  message: string;
  action?: ToastAction;
  /** Auto-dismiss delay. Defaults to 4000 for info/success and 0 (sticky) for errors. */
  timeoutMs?: number;
}

export const MAX_TOASTS = 4;
export const DEFAULT_TIMEOUT_MS = 4000;

class ToastStore {
  /** Replaced wholesale on every change, so `$state.raw`. */
  list = $state.raw<Toast[]>([]);

  #nextId = 1;
  #timers = new Map<number, ReturnType<typeof setTimeout>>();

  push(input: ToastInput): number {
    const id = this.#nextId++;
    const timeout = input.timeoutMs ?? (input.kind === "error" ? 0 : DEFAULT_TIMEOUT_MS);
    let next = [...this.list, { id, kind: input.kind, message: input.message, action: input.action }];
    while (next.length > MAX_TOASTS) {
      this.#clearTimer(next[0].id);
      next = next.slice(1);
    }
    this.list = next;
    if (timeout > 0 && Number.isFinite(timeout)) {
      this.#timers.set(
        id,
        setTimeout(() => this.dismiss(id), timeout),
      );
    }
    return id;
  }

  dismiss(id: number) {
    this.#clearTimer(id);
    if (this.list.some((t) => t.id === id)) this.list = this.list.filter((t) => t.id !== id);
  }

  clear() {
    for (const id of this.#timers.keys()) this.#clearTimer(id);
    this.list = [];
  }

  #clearTimer(id: number) {
    const t = this.#timers.get(id);
    if (t !== undefined) {
      clearTimeout(t);
      this.#timers.delete(id);
    }
  }
}

export const toasts = new ToastStore();
