import { describe, expect, it } from "vitest";
import { lanAccessNote, revealLabel, secretStoreName } from "./platform";

describe("platform", () => {
  it("names the file manager and the firewall note per OS", () => {
    expect(revealLabel(false)).toBe("Show in Explorer");
    expect(revealLabel(true)).toBe("Show in Finder");
    expect(lanAccessNote(false)).toContain("Windows may ask once");
    expect(lanAccessNote(true)).not.toContain("Windows");
  });

  it("names the secret store of each OS", () => {
    expect(secretStoreName(false)).toBe("Windows Credential Manager");
    expect(secretStoreName(true)).toBe("macOS Keychain");
  });
});
