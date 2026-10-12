// Colour values (hex or transparent) and the "Transparent" toggle of the variable forms.

import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import "$lib/components/templates/list/matchers";
import VariablesForm from "$lib/components/templates/editor/VariablesForm.svelte";
import { valueIssue } from "$lib/components/templates/editor/model";
import { cssValue } from "$lib/components/templates/editor/thumbs";
import type { VariableDef } from "$lib/types";
import VariableValuesForm from "./VariableValuesForm.svelte";
import { alphaOf, isColor, isHex, isTransparent, normalizeColor, pickerHex } from "./color";

const TILE: VariableDef = { key: "tile", label: "Tiles", type: "color", default: "#ffffff" };

afterEach(() => cleanup());

describe("color values", () => {
  it("hex or transparent in any case", () => {
    for (const ok of ["#12abef", "#12ABEF80", "transparent", "Transparent", "TRANSPARENT"]) expect(isColor(ok)).toBe(true);
    for (const bad of ["", "red", "#12ab", " transparent", "transparent;", "transparentx"]) expect(isColor(bad)).toBe(false);
    expect(isHex("transparent")).toBe(false);
    expect(isTransparent("Transparent")).toBe(true);
    expect(isTransparent(undefined)).toBe(false);
    expect(normalizeColor("TransParent")).toBe("transparent");
    expect(normalizeColor("#ABCDEF")).toBe("#ABCDEF");
    expect(pickerHex("transparent")).toBe("#000000");
    expect(alphaOf("#11223380")).toBe("80");
    expect(alphaOf("transparent")).toBe("");
  });

  it("variable defaults and thumbnails accept it", () => {
    expect(valueIssue({ ...TILE, default: "transparent" })).toBeNull();
    expect(valueIssue({ ...TILE, default: "clear" })).toBe("Enter a color like #1a1a1a.");
    expect(cssValue(TILE, "Transparent")).toBe("transparent");
    expect(cssValue(TILE, "transparent;x")).toBeNull();
  });
});

describe("VariableValuesForm: Transparent", () => {
  const toggle = () => screen.getByRole("button", { name: "Transparent Tiles" });
  const swatch = () => screen.getByLabelText("Tiles color picker").closest(".swatch")!;

  it("sets transparent; shown as the checkerboard and the word; off restores the last color", async () => {
    const onvalue = vi.fn();
    const { rerender } = render(VariableValuesForm, { variables: [TILE], values: { tile: "#112233" }, onvalue });
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
    expect(swatch().classList.contains("checker")).toBe(false);
    await fireEvent.click(toggle());
    expect(onvalue).toHaveBeenLastCalledWith("tile", "transparent", 0);

    await rerender({ variables: [TILE], values: { tile: "transparent" }, onvalue });
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    expect(swatch().classList.contains("checker")).toBe(true);
    expect((screen.getByLabelText("Tiles") as HTMLInputElement).value).toBe("transparent");
    await fireEvent.click(toggle());
    expect(onvalue).toHaveBeenLastCalledWith("tile", "#112233", 0);
  });

  it("off without a remembered color uses the default; picking or typing a color turns it off", async () => {
    const onvalue = vi.fn();
    render(VariableValuesForm, { variables: [TILE], values: { tile: "transparent" }, onvalue });
    await fireEvent.click(toggle());
    expect(onvalue).toHaveBeenLastCalledWith("tile", "#ffffff", 0);
    await fireEvent.input(screen.getByLabelText("Tiles color picker"), { target: { value: "#ff0000" } });
    expect(onvalue).toHaveBeenLastCalledWith("tile", "#ff0000", 0);
    await fireEvent.input(screen.getByLabelText("Tiles"), { target: { value: " Transparent " } });
    expect(onvalue).toHaveBeenLastCalledWith("tile", "transparent", 0);
  });

  it("an override can be transparent; Reset stays", async () => {
    const onvalue = vi.fn();
    const onreset = vi.fn();
    render(VariableValuesForm, {
      variables: [TILE],
      values: { tile: "transparent" },
      overrides: { tile: "transparent" },
      onvalue,
      onreset,
    });
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    await fireEvent.click(screen.getByRole("button", { name: "Reset Tiles to the template value" }));
    expect(onreset).toHaveBeenCalledWith("tile");
  });
});

describe("VariablesForm: Transparent", () => {
  it("the template editor's value row sets the variant value", async () => {
    const onvalue = vi.fn();
    render(VariablesForm, { variables: [TILE], otherKeys: [], issues: [], onvalue, ondefs: vi.fn() });
    await fireEvent.click(screen.getByRole("button", { name: "Transparent Tiles" }));
    expect(onvalue).toHaveBeenCalledWith(0, "transparent");
  });
});
