import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LayerEdits } from "./edits";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("LayerEdits", () => {
  it("sends one call per layer with every field typed within the delay", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const e = new LayerEdits(150, send);
    e.push("s/a", "name", "A");
    e.push("s/a", "name", "Ada");
    e.push("s/a", "title", "CTO");
    e.push("s/b", "name", "Bob");
    expect(e.pending("s/a")).toBe(true);
    await vi.advanceTimersByTimeAsync(149);
    expect(send).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(send).toHaveBeenCalledTimes(2);
    expect(send).toHaveBeenCalledWith("s/a", { name: "Ada", title: "CTO" });
    expect(send).toHaveBeenCalledWith("s/b", { name: "Bob" });
    expect(e.pending("s/a")).toBe(false);
  });

  it("flush sends now and waits; a failing send never rejects the flush", async () => {
    const send = vi.fn().mockRejectedValue(new Error("nope"));
    const e = new LayerEdits(150, send);
    e.push("s/a", "name", "Ada");
    await expect(e.flush("s/a")).resolves.toBeUndefined();
    expect(send).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(500);
    expect(send).toHaveBeenCalledOnce();
  });

  it("flush with nothing pending waits for a send in flight", async () => {
    let release!: () => void;
    const send = vi.fn(() => new Promise<void>((r) => (release = r)));
    const e = new LayerEdits(150, send);
    e.push("s/a", "name", "Ada");
    void e.flush("s/a");
    let done = false;
    void e.flush("s/a").then(() => (done = true));
    await Promise.resolve();
    expect(done).toBe(false);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(done).toBe(true);
  });

  it("cancel drops pending edits", async () => {
    const send = vi.fn().mockResolvedValue(undefined);
    const e = new LayerEdits(150, send);
    e.push("s/a", "name", "Ada");
    e.cancel();
    await vi.advanceTimersByTimeAsync(500);
    expect(send).not.toHaveBeenCalled();
  });
});
