import { afterEach, describe, expect, it } from "vitest";
import type { FieldDef, VariableDef } from "$lib/types";
import { cssValue, releaseThumb, renderThumb, thumbValue } from "./thumbs";

// The real renderer compile (renderer/src/layer.ts) runs here, in jsdom.

const fields: FieldDef[] = [
  { key: "name", label: "Name", type: "text", default: "" },
  { key: "title", label: "Title", type: "text", default: "Host" },
  { key: "logo", label: "Logo", type: "image", default: "" },
  { key: "rows", label: "Rows", type: "list", default: "", columns: [{ key: "team", label: "Team" }] },
];
const variables: VariableDef[] = [
  { key: "accent", label: "Accent", type: "color", default: "#22b8cf" },
  { key: "radius", label: "Radius", type: "range", min: 0, max: 24, step: 1, unit: "px", default: "4" },
  { key: "font", label: "Font", type: "font", default: "IBM Plex Sans" },
];

function host() {
  const el = document.createElement("span");
  document.body.append(el);
  return el;
}

const layerOf = (el: HTMLElement) => el.shadowRoot!.querySelector(".layer") as HTMLElement;
const inner = (el: HTMLElement) => layerOf(el).shadowRoot!;

afterEach(() => {
  document.body.replaceChildren();
});

describe("thumbnails", () => {
  it("renders with the renderer's compile: defaults, labels, list rows, data-state=in, frozen animations", () => {
    const el = host();
    renderThumb(
      el,
      {
        html: '<div class="bar"><b>{{name}}</b> <i>{{title}}</i><img src="{{logo}}"><ul><li data-each="rows">{{.team}}</li></ul></div>',
        css: ".bar{animation:in 1s}",
        vars: { accent: "#ff0000" },
      },
      fields,
      variables,
    );
    const root = inner(el).querySelector(".bar")!;
    expect(root.getAttribute("data-state")).toBe("in");
    expect(root.querySelector("b")!.textContent).toBe("[Name]"); // no default: a placeholder
    expect(root.querySelector("i")!.textContent).toBe("Host");
    expect(root.querySelector("img")!.getAttribute("src")).toBe(""); // media without a default: empty
    expect([...root.querySelectorAll("li")].map((l) => l.textContent)).toEqual(["Team"]);
    const styles = [...inner(el).querySelectorAll("style")].map((s) => s.textContent);
    expect(styles[0]).toBe(".bar{animation:in 1s}");
    expect(styles[1]).toMatch(/animation-play-state:paused!important/);
    // Variables: the variant's value, else the definition default; fonts as their stack.
    const layer = layerOf(el);
    expect(layer.style.getPropertyValue("--accent")).toBe("#ff0000");
    expect(layer.style.getPropertyValue("--radius")).toBe("4px");
    expect(layer.style.getPropertyValue("--font")).toMatch(/IBM Plex Sans/);
    expect(el.hasAttribute("inert")).toBe(true);
  });

  it("re-renders in place and releases", () => {
    const el = host();
    renderThumb(el, { html: "<p>{{name}}</p>", css: "", vars: { accent: "#ff0000" } }, fields, variables);
    renderThumb(el, { html: "<h1>{{title}}</h1>", css: "", vars: {} }, fields, variables);
    expect(inner(el).querySelector("p")).toBeNull();
    expect(inner(el).querySelector("h1")!.textContent).toBe("Host");
    expect(layerOf(el).style.getPropertyValue("--accent")).toBe("#22b8cf");
    const shadow = inner(el);
    releaseThumb(el);
    expect(shadow.childNodes).toHaveLength(0);
    // Rendering again after a release works on the same element.
    renderThumb(el, { html: "<p>x</p>", css: "", vars: {} }, fields, variables);
    expect(inner(el).querySelector("p")!.textContent).toBe("x");
  });

  it("CSS values follow the server's strict grammar", () => {
    const [color, range, font] = variables;
    expect(cssValue(color, "#abc")).toBe("#abc");
    expect(cssValue(color, "red")).toBeNull();
    expect(cssValue(color, "#fff;}x{")).toBeNull();
    expect(cssValue(range, "12")).toBe("12px");
    expect(cssValue(range, "99")).toBeNull();
    expect(cssValue(range, "1e2")).toBeNull();
    expect(cssValue(font, "Nope")).toBeNull();
    expect(
      cssValue(
        { key: "s", label: "S", type: "select", options: [{ value: "left", label: "Left" }], default: "left" },
        "left",
      ),
    ).toBe("left");
  });

  it("thumbnail values: clocks show their format, timers their state", () => {
    expect(thumbValue({ key: "c", label: "Clock", type: "clock", default: "", format: "HH:mm" })).toBe("HH:mm");
    expect(thumbValue({ key: "v", label: "Clip", type: "video", default: "" })).toBe("");
  });
});
