import { describe, expect, it } from "vitest";
import { MARKERS, findMarkers, verdict } from "./check-cloud-build.mjs";

describe("cloud build check", () => {
  it("a build without cloud backup carries no marker, one with it all of them", () => {
    const clean = [{ name: "app.js", text: 'call("remote_view");"cloud-changed"' }];
    expect(verdict("0", findMarkers(clean))).toEqual([]);
    expect(verdict("1", findMarkers(clean))).toHaveLength(MARKERS.length);
    const on = [
      { name: "cloud.js", text: 'call("cloud_backup_now");call("cloud_restore_start",{request})' },
      { name: "picker.js", text: '"Download a project from your cloud backup."' },
      { name: "exit.js", text: 'title:t("Back up before quitting?")' },
    ];
    expect(verdict("1", findMarkers(on))).toEqual([]);
    expect(verdict("0", findMarkers(on))).toHaveLength(4);
    expect(() => verdict("maybe", new Map())).toThrow();
  });
});
