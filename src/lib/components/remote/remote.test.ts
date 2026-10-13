import { afterEach, describe, expect, it, vi } from "vitest";
import type { RemoteActivity, RemoteView } from "$lib/types";
import { ActivityLine, LINE_MS, PER_PERSON_MS, chipOf } from "./chip.svelte";

const base: RemoteView = {
  state: "off",
  on: false,
  takeLocked: false,
  crew: [],
  showId: null,
  crewUrl: null,
  controllerUrl: null,
  serverVersion: null,
  rttMs: null,
  licenseWarning: null,
  lastError: null,
  hasShowId: false,
};
const v = (over: Partial<RemoteView> & Pick<RemoteView, "state">) => ({ ...base, on: true, ...over }) as RemoteView;
const act = (name: string, text: string, at = 0): RemoteActivity => ({ at, name, role: "operator", text });

afterEach(() => vi.useRealTimers());

describe("chipOf", () => {
  it("shows nothing while off", () => {
    expect(chipOf(null)).toBeNull();
    expect(chipOf(base)).toBeNull();
  });

  it("says how many are connected, in green, while live", () => {
    const crew = [{ id: "a", name: "Kari", role: "operator", anonymous: false, sinceMs: 1 }];
    expect(chipOf(v({ state: "live", runId: "r", resumed: false }))).toEqual({ text: "On", label: "Remote control: On", tally: "preview" });
    expect(chipOf(v({ state: "live", runId: "r", resumed: false, crew }))?.label).toBe("Remote control: 1 connected");
  });

  it("is amber while reconnecting, asking or stopped", () => {
    expect(chipOf(v({ state: "reconnecting", attempt: 1, nextMs: 0 }))?.tally).toBe("warning");
    expect(chipOf(v({ state: "showInUse", machine: "Studio PC" }))?.text).toBe("Move it here?");
    expect(chipOf(v({ state: "stopped", code: "unauthorized", message: "Sign in again" }))?.tally).toBe("warning");
    expect(chipOf(v({ state: "ended" }))?.label).toBe("Remote control ended");
  });
});

describe("activity line", () => {
  it("shows each person at most every 2 s, the latest winning, for a few seconds", () => {
    vi.useFakeTimers();
    const r = new ActivityLine();
    const t0 = Date.now();
    r.add(act("Kari", "Kari took A"), t0);
    expect(r.line?.text).toBe("Kari took A");
    // Within 2 s: waits; a later one replaces it.
    r.add(act("Kari", "Kari took B"), t0 + 100);
    r.add(act("Kari", "Kari took C"), t0 + 200);
    expect(r.line?.text).toBe("Kari took A");
    // Someone else shows at once.
    r.add(act("Ola", "Ola ran Open"), t0 + 300);
    expect(r.line?.text).toBe("Ola ran Open");
    vi.advanceTimersByTime(PER_PERSON_MS);
    expect(r.line?.text).toBe("Kari took C");
    vi.advanceTimersByTime(LINE_MS);
    expect(r.line).toBeNull();
  });
});
