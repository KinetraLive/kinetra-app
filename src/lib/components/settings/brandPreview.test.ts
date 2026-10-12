import { afterEach, describe, expect, it, vi } from "vitest";
import { BrandPreview, PREVIEW_MS, inheritedBrand, loadStyleLook } from "./brandPreview";

// The real preset files (presets/templates/lower-third) and the renderer's compile.

const layer = (el: HTMLElement) => el.shadowRoot?.querySelector<HTMLElement>(".layer") ?? null;
const cssVar = (el: HTMLElement, key: string) => layer(el)?.style.getPropertyValue(`--${key}`) ?? "";

afterEach(() => {
  vi.useRealTimers();
  document.body.replaceChildren();
});

describe("inherited brand values", () => {
  it("are the style's variant values of the lower third", async () => {
    const s = await loadStyleLook("inline");
    expect(s?.inherited).toEqual({
      accent: "#2b59ff",
      secondary: "#5b6270",
      text: "#12141a",
      plate: "#ffffff",
      fontHeading: "Plus Jakarta Sans",
      fontBody: "Plus Jakarta Sans",
    });
    expect(s?.values.name).toBe("Alex Morgan");
  });

  it("fall back to the variable defaults for a style the lower third has no look for", async () => {
    const s = await loadStyleLook("classic");
    expect(s?.look.vars).toEqual({});
    expect(s?.inherited.accent).toBe("#33d6ff");
    expect(s?.inherited.plate).toBe("#0c0d11d9");
    expect(inheritedBrand({ ...s!.look, vars: { accent: "#010203" } }).accent).toBe("#010203");
  });
});

describe("BrandPreview", () => {
  it("draws a new look at once and debounces value changes", async () => {
    const look = (await loadStyleLook("frame"))!;
    const el = document.createElement("span");
    document.body.append(el);
    vi.useFakeTimers();
    const p = new BrandPreview(el);
    p.set(look, { brand: {}, logo: null, fontBase: null });
    expect(cssVar(el, "accent")).toBe("#33d6ff");

    p.set(look, { brand: { accent: "#111111" }, logo: null, fontBase: null });
    p.set(look, { brand: { accent: "#222222" }, logo: null, fontBase: null });
    vi.advanceTimersByTime(PREVIEW_MS - 1);
    expect(cssVar(el, "accent")).toBe("#33d6ff");
    vi.advanceTimersByTime(1);
    expect(cssVar(el, "accent")).toBe("#222222");
    expect(vi.getTimerCount()).toBe(0); // nothing scheduled at rest

    // The brand logo replaces the sample logo.
    p.set(look, { brand: {}, logo: "http://127.0.0.1:7770/assets/logo.png", fontBase: null });
    vi.advanceTimersByTime(PREVIEW_MS);
    const img = [...(layer(el)?.shadowRoot?.querySelectorAll("img") ?? [])].map((i) => i.getAttribute("src"));
    expect(img).toContain("http://127.0.0.1:7770/assets/logo.png");

    p.set(look, { brand: { accent: "#333333" }, logo: null, fontBase: null });
    p.destroy();
    expect(vi.getTimerCount()).toBe(0);
    expect(layer(el)?.shadowRoot?.childNodes.length ?? 0).toBe(0);
  });
});
