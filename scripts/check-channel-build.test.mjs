import { describe, expect, it } from "vitest";
import config from "../svelte.config.js";
import { MARKERS, findMarkers, verdict } from "./check-channel-build.mjs";

describe("channel build check", () => {
  it("stable must carry no canary marker, canary all of them", () => {
    const clean = [{ name: "app.js", text: 'const c={stable:"Stable",canary:"Canary"}' }];
    expect(verdict("stable", findMarkers(clean))).toEqual([]);
    expect(verdict("canary", findMarkers(clean))).toHaveLength(MARKERS.length);
    const canary = [
      { name: "badge.js", text: 't("Experimental build. Don\'t use for live shows.")' },
      { name: "badge.css", text: ".canary-badge.svelte-x1{background:#F5C400}" },
    ];
    expect(verdict("canary", findMarkers(canary))).toEqual([]);
    expect(verdict("stable", findMarkers(canary))).toHaveLength(3);
    expect(() => verdict("beta", new Map())).toThrow();
  });

  it("builds without KINETRA_CHANNEL=canary resolve the badge to the empty component", () => {
    expect(process.env.KINETRA_CHANNEL ?? "").not.toBe("canary");
    expect(config.kit.alias["$channel-badge"]).toBe("src/lib/components/brand/NoBadge.svelte");
  });
});
