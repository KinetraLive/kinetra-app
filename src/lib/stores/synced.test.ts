import { describe, expect, it } from "vitest";
import { SyncedStore } from "./synced.svelte";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("SyncedStore", () => {
  it("applies updates optimistically and keeps the stored value", async () => {
    const s = new SyncedStore<number>(
      async () => 1,
      async (v) => Math.min(v, 10), // backend clamps
    );
    await s.load();
    const p = s.update(50);
    expect(s.value).toBe(50);
    await p;
    expect(s.value).toBe(10);
  });

  it("rolls back to the last confirmed value on failure", async () => {
    const s = new SyncedStore<number>(
      async () => 1,
      async () => {
        throw { code: "invalid", message: "nope" };
      },
    );
    await s.load();
    await expect(s.update(2)).rejects.toMatchObject({ code: "invalid" });
    expect(s.value).toBe(1);
  });

  it("ignores a stale response that resolves after a newer update", async () => {
    const first = deferred<number>();
    const calls: Array<Promise<number>> = [first.promise, Promise.resolve(3)];
    const s = new SyncedStore<number>(async () => 1, () => calls.shift()!);
    await s.load();
    const a = s.update(2);
    await s.update(3);
    first.resolve(2);
    await a;
    expect(s.value).toBe(3);
  });

  it("a stale failure does not roll back a newer value", async () => {
    const first = deferred<number>();
    const calls: Array<Promise<number>> = [first.promise, Promise.resolve(3)];
    const s = new SyncedStore<number>(async () => 1, () => calls.shift()!);
    await s.load();
    const a = s.update(2);
    await s.update(3);
    first.reject({ code: "io", message: "late" });
    await expect(a).rejects.toMatchObject({ code: "io" });
    expect(s.value).toBe(3);
  });
});
