import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_TIMEOUT_MS, MAX_TOASTS, toasts } from "./toasts.svelte";

describe("toasts", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    toasts.clear();
    vi.useRealTimers();
  });

  it("caps the list and drops the oldest", () => {
    const ids = Array.from({ length: MAX_TOASTS + 2 }, (_, i) =>
      toasts.push({ kind: "info", message: `m${i}` }),
    );
    expect(toasts.list.length).toBe(MAX_TOASTS);
    expect(toasts.list.map((t) => t.id)).toEqual(ids.slice(2));
    // Dropped toasts' timers are cleared, so only the visible ones remain pending.
    expect(vi.getTimerCount()).toBe(MAX_TOASTS);
  });

  it("auto-dismisses info after the default timeout", () => {
    toasts.push({ kind: "info", message: "hi" });
    vi.advanceTimersByTime(DEFAULT_TIMEOUT_MS - 1);
    expect(toasts.list.length).toBe(1);
    vi.advanceTimersByTime(1);
    expect(toasts.list.length).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps errors until dismissed", () => {
    const id = toasts.push({ kind: "error", message: "boom" });
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(60_000);
    expect(toasts.list.length).toBe(1);
    toasts.dismiss(id);
    expect(toasts.list.length).toBe(0);
  });

  it("dismiss clears the pending timer", () => {
    const id = toasts.push({ kind: "success", message: "ok", timeoutMs: 1000 });
    expect(vi.getTimerCount()).toBe(1);
    toasts.dismiss(id);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps an action (e.g. Undo) on the toast", () => {
    const run = vi.fn();
    toasts.push({ kind: "success", message: "Applied", action: { label: "Undo", run } });
    expect(toasts.list[0].action?.label).toBe("Undo");
    toasts.list[0].action?.run();
    expect(run).toHaveBeenCalledTimes(1);
  });
});
