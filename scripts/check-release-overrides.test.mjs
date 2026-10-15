import { describe, expect, it } from "vitest";
import { OVERRIDES, verdict } from "./check-release-overrides.mjs";

const err = (code, message) => ({ reason: "compiler-message", message: { level: "error", code: code && { code }, message } });

describe("release overrides check", () => {
  it("passes only when every override is missing in the release build", () => {
    const missing = [
      err("E0599", "no method named `set_test_entitlements` found for reference `&Cloud`"),
      err("E0425", "cannot find function `load_unchecked_for_tests` in module `preview`"),
      err("E0425", "cannot find function `dev_profile` in module `devprofile`"),
      err(null, "aborting due to 3 previous errors"),
    ];
    expect(verdict(missing)).toEqual([]);
  });

  it("fails when the guard compiles (an override is in release builds)", () => {
    expect(verdict([])).toHaveLength(1);
    // Only one override missing: the others are reported.
    const one = [err("E0599", "no method named `set_test_entitlements` found")];
    expect(verdict(one)).toEqual([
      "load_unchecked_for_tests exists in release builds",
      "dev_profile exists in release builds",
    ]);
  });

  it("fails on any other error (nothing is proven then)", () => {
    const other = [
      err("E0599", "no method named `set_test_entitlements` found"),
      err("E0425", "cannot find function `load_unchecked_for_tests`"),
      err("E0425", "cannot find function `dev_profile`"),
      err("E0432", "unresolved import `kinetra_lib::core::gone`"),
    ];
    expect(verdict(other)).toHaveLength(1);
    expect(OVERRIDES).toContain("set_test_entitlements");
  });
});
