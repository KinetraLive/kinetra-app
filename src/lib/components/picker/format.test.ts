import { describe, expect, it } from "vitest";
import { describeError, parentDir, relativeTime, splitPath } from "./format";
import { joinPath, slugify } from "./newProject";

describe("picker format helpers", () => {
  it("slugify approximates the backend", () => {
    expect(slugify("My Show!")).toBe("my-show");
    expect(slugify("  trailing dots... ")).toBe("trailing-dots");
    expect(slugify("a  -  b")).toBe("a-b");
    expect(slugify("!!!")).toBe("project");
    expect(slugify("Nyheter Ø")).toBe("nyheter-ø");
  });

  it("joins and splits paths", () => {
    expect(joinPath("C:\\Shows\\", "x")).toBe("C:\\Shows\\x");
    expect(joinPath("/home/a", "x")).toBe("/home/a/x");
    expect(parentDir("C:\\Shows\\x")).toBe("C:\\Shows");
    expect(parentDir("C:\\x")).toBe("C:\\");
    expect(splitPath("C:\\Shows\\x")).toEqual(["C:\\Shows\\", "x"]);
  });

  it("relative time", () => {
    const now = Date.parse("2026-10-03T12:00:00Z");
    expect(relativeTime("2026-10-03T11:59:30Z", now)).toBe("just now");
    expect(relativeTime("2026-10-03T10:00:00Z", now)).toBe("2 hours ago");
    expect(relativeTime("2026-10-02T12:00:00Z", now)).toBe("yesterday");
    expect(relativeTime("2026-09-03T12:00:00Z", now)).toBe("last month");
  });

  it("maps error codes", () => {
    expect(describeError({ code: "cancelled", message: "" })).toBeNull();
    expect(describeError({ code: "newerVersion", message: "From a newer version" })).toBe(
      "From a newer version",
    );
    expect(describeError({ code: "notAProject", message: "x" })).toMatch(/isn't a Kinetra/);
  });
});
