import { afterEach, describe, expect, it } from "vitest";
import type { FieldDef } from "$lib/types";
import { loadPresetLook, sampleValues } from "./presets";
import { brandVars, hidePreview, previewing, showPreview } from "./preview";
import { variantThumbUrl } from "./thumbs";

// The real renderer compile and the real preset files (the legacy looks, which never change).

function host() {
  const el = document.createElement("span");
  document.body.append(el);
  return el;
}

const layerOf = (el: HTMLElement) => el.shadowRoot?.querySelector<HTMLElement>(".layer") ?? null;
const inner = (el: HTMLElement) => layerOf(el)?.shadowRoot ?? null;

afterEach(() => {
  hidePreview();
  document.body.replaceChildren();
});

describe("preset looks", () => {
  it("reads a one-look preset's fields, variables and root markup", async () => {
    const look = await loadPresetLook("classic-bar", "classic");
    expect(look).not.toBeNull();
    expect(look!.fields.map((f) => f.key)).toEqual(["name", "title"]);
    expect(look!.variables.some((v) => v.key === "accent")).toBe(true);
    expect(look!.html).toContain("{{name}}");
    // (Vitest serves .css files empty, ?raw included; the build inlines them.)
    expect(typeof look!.css).toBe("string");
    expect(look!.vars).toEqual({});
  });

  it("is null for a preset that isn't in the build", async () => {
    expect(await loadPresetLook("no-such-preset", "frame")).toBeNull();
  });

  it("sample values: the preset's sample data where given, else what a thumbnail shows", async () => {
    const fields: FieldDef[] = [
      { key: "name", label: "Name", type: "text", default: "" },
      { key: "title", label: "Title", type: "text", default: "Host" },
    ];
    // classic-bar has no sample.json: labels and defaults.
    expect(await sampleValues("classic-bar", "classic", fields)).toEqual({ name: "[Name]", title: "Host" });
  });
});

describe("hover preview", () => {
  it("draws with the renderer's compile, unfrozen, the in-animation started", async () => {
    const el = host();
    expect(await showPreview(el, "classic-bar", "classic", null)).toBe(true);
    const root = inner(el)!.querySelector(".cbar")!;
    expect(root.getAttribute("data-state")).toBe("in");
    expect(root.textContent).toContain("Name");
    // Only the template's own CSS: no freeze rule, so its animations run.
    const styles = [...inner(el)!.querySelectorAll("style")].map((s) => s.textContent ?? "");
    expect(styles).toHaveLength(1);
    expect(styles[0]).not.toMatch(/animation-play-state/);
    expect(el.hasAttribute("inert")).toBe(true);
    expect(previewing()).toBe(el);
  });

  it("one at a time: showing another releases the first; hide releases it", async () => {
    const a = host();
    const b = host();
    await showPreview(a, "classic-bar", "classic", null);
    await showPreview(b, "minimal-line", "classic", null);
    expect(inner(a)?.childNodes.length ?? 0).toBe(0);
    expect(inner(b)!.childNodes.length).toBeGreaterThan(0);
    expect(previewing()).toBe(b);

    hidePreview(a); // not the one showing: nothing happens
    expect(previewing()).toBe(b);
    hidePreview(b);
    expect(previewing()).toBeNull();
    expect(inner(b)?.childNodes.length ?? 0).toBe(0);
  });

  it("a load superseded by a newer show (or a hide) draws nothing", async () => {
    const a = host();
    const b = host();
    const first = showPreview(a, "classic-bar", "classic", null);
    const second = showPreview(b, "minimal-line", "classic", null);
    expect(await first).toBe(false);
    expect(await second).toBe(true);
    expect(inner(a)?.childNodes.length ?? 0).toBe(0);

    const c = host();
    const third = showPreview(c, "classic-bar", "classic", null);
    hidePreview(c);
    expect(await third).toBe(false);
    expect(previewing()).toBeNull();
  });

  it("the brand kit wins over the look's variables", async () => {
    const el = host();
    await showPreview(el, "classic-bar", "classic", { accent: "#ff0000", fontHeading: "IBM Plex Sans" });
    expect(layerOf(el)!.style.getPropertyValue("--accent")).toBe("#ff0000");
    expect(brandVars({ accent: "#ff0000", plate: null, fontHeading: "IBM Plex Sans", fontBody: "Barlow", logo: "/assets/l.png" })).toEqual({
      accent: "#ff0000",
      "font-heading": "IBM Plex Sans",
      "font-body": "Barlow",
    });
    expect(brandVars(undefined)).toEqual({});
  });
});

describe("variant thumbnails", () => {
  it("falls back to the preset's own thumbnail when its look has none", async () => {
    expect(await variantThumbUrl("classic-bar", "no-such-look")).toMatch(/\/presets\/thumbs\/classic-bar\.png/);
    expect(await variantThumbUrl("classic-bar", "")).toMatch(/\/presets\/thumbs\/classic-bar\.png/);
    expect(await variantThumbUrl("no-such-preset", "frame")).toBeNull();
  });
});
