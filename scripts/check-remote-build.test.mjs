import { describe, expect, it } from "vitest";
import { MARKERS, findMarkers, verdict } from "./check-remote-build.mjs";

describe("remote build check", () => {
  it("a build without remote control carries no marker, one with it all of them", () => {
    const clean = [{ name: "app.js", text: 'call("remote_view");call("cue_position")' }];
    expect(verdict("0", findMarkers(clean))).toEqual([]);
    expect(verdict("1", findMarkers(clean))).toHaveLength(MARKERS.length);
    const on = [
      { name: "remote.js", text: 'call("remote_set",{on})' },
      { name: "panel.js", text: '"Lock remote take";"Make this a separate show…"' },
    ];
    expect(verdict("1", findMarkers(on))).toEqual([]);
    expect(verdict("0", findMarkers(on))).toHaveLength(3);
    expect(() => verdict("maybe", new Map())).toThrow();
  });
});
