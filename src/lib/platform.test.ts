import { describe, expect, it } from "vitest";
import { secretStoreName } from "./platform";

describe("platform", () => {
  it("names the secret store of each OS", () => {
    expect(secretStoreName(false)).toBe("Windows Credential Manager");
    expect(secretStoreName(true)).toBe("macOS Keychain");
  });
});
