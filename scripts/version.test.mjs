import { describe, expect, it } from "vitest";
import { agreedVersion, cargoVersion, readVersions, setCargoVersion, setLockVersion } from "./version.mjs";

const toml = `[package]\nname = "kinetra"\nversion = "0.1.0"\n\n[dependencies]\nfoo = { version = "1.0.0" }\nbar = "2"\nversion = "9.9.9"\n`;

describe("version", () => {
  it("reads and sets only the [package] version", () => {
    expect(cargoVersion(toml)).toBe("0.1.0");
    const next = setCargoVersion(toml, "0.2.0");
    expect(cargoVersion(next)).toBe("0.2.0");
    expect(next).toContain('foo = { version = "1.0.0" }');
    expect(next).toContain('version = "9.9.9"');
  });

  it("sets the kinetra package in Cargo.lock only", () => {
    const lock = `[[package]]\r\nname = "kinetra"\r\nversion = "0.1.0"\r\n\r\n[[package]]\r\nname = "kinetra-license"\r\nversion = "0.1.0"\r\n`;
    const next = setLockVersion(lock, "0.2.0");
    expect(next).toContain('name = "kinetra"\r\nversion = "0.2.0"');
    expect(next).toContain('name = "kinetra-license"\r\nversion = "0.1.0"');
  });

  it("requires all three files to agree on X.Y.Z", () => {
    expect(agreedVersion({ tauri: "1.0.0", cargo: "1.0.0", npm: "1.0.0" })).toBe("1.0.0");
    expect(() => agreedVersion({ tauri: "1.0.0", cargo: "1.0.1", npm: "1.0.0" })).toThrow(/disagree/);
    expect(() => agreedVersion({ tauri: "1.0", cargo: "1.0", npm: "1.0" })).toThrow(/X.Y.Z/);
  });

  it("the repo's files agree", () => {
    expect(() => agreedVersion(readVersions())).not.toThrow();
  });
});
