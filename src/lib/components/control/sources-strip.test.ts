// Control's sources strip with API sources: the status line, its relative time and the
// Retry / Pause actions.

import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../templates/list/matchers";
import type { ApiStatus, SourceSummary } from "$lib/types";

const api = vi.hoisted(() => ({ retryApiSource: vi.fn(), setApiPaused: vi.fn() }));
vi.mock("$lib/api/api-sources", () => api);

import SourcesStrip from "./SourcesStrip.svelte";

const status = (over: Partial<ApiStatus> = {}): ApiStatus => ({
  state: "ok",
  lastOkAt: Date.now() - 4000,
  lastAttemptAt: Date.now() - 4000,
  nextAt: Date.now() + 6000,
  httpStatus: 200,
  message: null,
  rows: 3,
  ...over,
});

function src(id: string, name: string, kind: SourceSummary["kind"], st: ApiStatus | null = null): SourceSummary {
  return {
    id,
    name,
    kind,
    rowCount: 3,
    index: 0,
    current: { team: "Bergen", points: "13" },
    columns: [
      { key: "team", label: "Team" },
      { key: "points", label: "Points" },
    ],
    usedBy: ["s1"],
    updatedAt: "2026-10-06T10:00:00Z",
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
    api: st,
  };
}

const props = (sources: SourceSummary[]) => ({
  sources,
  onstep: vi.fn(),
  autoOf: () => ({ everyMs: 8000, loop: true, transition: "update" as const }),
  onauto: vi.fn(),
  onautosettings: vi.fn(),
  onapi: vi.fn(),
});

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("sources strip: API sources", () => {
  it("shows Live / Error / Paused / Waiting with a dot and text, and updated N s ago", async () => {
    render(SourcesStrip, {
      ...props([
        src("a", "Scores", "api", status()),
        src("b", "Weather", "api", status({ state: "backoff", message: "HTTP 503" })),
        src("c", "Ticker", "api", status({ state: "paused" })),
        src("d", "Polls", "api", null),
        src("t", "Guests", "text"),
      ]),
    });
    const line = (id: string) => document.querySelector(`[data-source="${id}"] .api`);
    expect(line("a")).toHaveTextContent("Live");
    expect(line("a")).toHaveTextContent("updated 4 s ago");
    expect(line("b")).toHaveTextContent("Error");
    expect(line("b")).toHaveTextContent("HTTP 503");
    expect(line("c")).toHaveTextContent("Paused");
    expect(line("d")).toHaveTextContent("Waiting");
    expect(line("t")).toBeNull();
    // The relative time follows the clock (once a second while shown).
    await vi.advanceTimersByTimeAsync(3000);
    await tick();
    expect(line("a")).toHaveTextContent(/updated [67] s ago/);
  });

  it("Retry and Pause / Resume call the backend and hand the reply over", async () => {
    const p = props([src("a", "Scores", "api", status()), src("c", "Ticker", "api", status({ state: "paused" }))]);
    const fresh = src("a", "Scores", "api", status({ state: "fetching" }));
    api.retryApiSource.mockResolvedValue(fresh);
    api.setApiPaused.mockImplementation(async (id: string, paused: boolean) => src(id, "x", "api", status({ state: paused ? "paused" : "ok" })));
    render(SourcesStrip, p);
    await fireEvent.click(screen.getByRole("button", { name: "Retry Scores now" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(api.retryApiSource).toHaveBeenCalledWith("a");
    expect(p.onapi).toHaveBeenCalledWith(fresh);
    await fireEvent.click(screen.getByRole("button", { name: "Pause Scores" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(api.setApiPaused).toHaveBeenLastCalledWith("a", true);
    await fireEvent.click(screen.getByRole("button", { name: "Resume Ticker" }));
    await vi.advanceTimersByTimeAsync(0);
    expect(api.setApiPaused).toHaveBeenLastCalledWith("c", false);
  });

  it("no timers without an API status line", () => {
    render(SourcesStrip, { ...props([src("t", "Guests", "text")]) });
    expect(vi.getTimerCount()).toBe(0);
  });
});
