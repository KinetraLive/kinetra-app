import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Badge from "./Badge.svelte";
import Button from "./Button.svelte";
import Card from "./Card.svelte";
import IconButton from "./IconButton.svelte";
import NumberInput from "./NumberInput.svelte";
import Select from "./Select.svelte";
import Switch from "./Switch.svelte";
import TallyDot from "./TallyDot.svelte";

afterEach(cleanup);

const text = (s: string) => createRawSnippet(() => ({ render: () => `<span>${s}</span>` }));

describe("NumberInput digits", () => {
  it("sizes the field to the digit count and hides the spin buttons", () => {
    render(NumberInput, { value: 7770, min: 1024, max: 65535, digits: 5, "aria-label": "Port" });
    const input = screen.getByLabelText("Port") as HTMLInputElement;
    expect(input.style.getPropertyValue("--digits")).toBe("5");
    expect(input.classList.contains("sized")).toBe(true);
    expect(input.classList.contains("no-spin")).toBe(true);
  });

  it("has no digit sizing without the prop", () => {
    render(NumberInput, { value: 1, "aria-label": "N" });
    const input = screen.getByLabelText("N") as HTMLInputElement;
    expect(input.style.getPropertyValue("--digits")).toBe("");
    expect(input.classList.contains("sized")).toBe(false);
    expect(input.classList.contains("no-spin")).toBe(true);
  });

  it("still clamps with digits set", async () => {
    const onchange = vi.fn();
    render(NumberInput, { value: 7770, min: 1024, max: 65535, digits: 5, "aria-label": "Port", onchange });
    const input = screen.getByLabelText("Port") as HTMLInputElement;
    input.value = "70000";
    await fireEvent.change(input);
    expect(onchange).toHaveBeenLastCalledWith(65535);
    expect(input.value).toBe("65535");
    input.value = "80";
    await fireEvent.change(input);
    expect(onchange).toHaveBeenLastCalledWith(1024);
  });
});

describe("Badge tally", () => {
  it("onAir + dot renders a pulsing program TallyDot", () => {
    const { container } = render(Badge, { tone: "onAir", dot: true, children: text("On air") });
    const dot = container.querySelector(".tally");
    expect(dot?.classList.contains("program")).toBe(true);
    expect(dot?.classList.contains("pulse")).toBe(true);
  });

  it("maps dot tones: warning -> warning, accent -> selected, success -> idle", () => {
    const states = (["warning", "accent", "success"] as const).map((tone) => {
      const { container, unmount } = render(Badge, { tone, dot: true, children: text("x") });
      const cls = [...(container.querySelector(".tally")?.classList ?? [])];
      unmount();
      return cls;
    });
    expect(states[0]).toContain("warning");
    expect(states[1]).toContain("selected");
    expect(states[2]).toContain("idle");
  });

  it("tally prop renders that state and wins over dot", () => {
    const { container } = render(Badge, { tone: "success", dot: true, tally: "preview", children: text("Ready") });
    const dots = container.querySelectorAll(".tally");
    expect(dots.length).toBe(1);
    expect(dots[0].classList.contains("preview")).toBe(true);
  });

  it("renders no dot by default", () => {
    const { container } = render(Badge, { children: text("Plain") });
    expect(container.querySelector(".tally")).toBeNull();
  });
});

describe("TallyDot", () => {
  it("with a label it is an img with that name", () => {
    render(TallyDot, { state: "preview", label: "Connected" });
    const dot = screen.getByRole("img", { name: "Connected" });
    expect(dot.getAttribute("aria-hidden")).toBeNull();
  });

  it("without a label it is decorative", () => {
    const { container } = render(TallyDot, { state: "idle" });
    const dot = container.querySelector(".tally");
    expect(dot?.getAttribute("aria-hidden")).toBe("true");
    expect(dot?.getAttribute("role")).toBeNull();
  });

  it("program pulses by default, pulse={false} turns it off, others don't pulse", () => {
    const a = render(TallyDot, { state: "program" });
    expect(a.container.querySelector(".tally")?.classList.contains("pulse")).toBe(true);
    a.unmount();
    const b = render(TallyDot, { state: "program", pulse: false });
    expect(b.container.querySelector(".tally")?.classList.contains("pulse")).toBe(false);
    b.unmount();
    const c = render(TallyDot, { state: "preview" });
    expect(c.container.querySelector(".tally")?.classList.contains("pulse")).toBe(false);
  });

  it("passes title through", () => {
    const { container } = render(TallyDot, { state: "warning", title: "Port in use" });
    expect(container.querySelector(".tally")?.getAttribute("title")).toBe("Port in use");
  });
});

describe("viewfinder brackets", () => {
  it("Button and link Button carry vf", () => {
    render(Button, { children: text("Save") });
    expect(screen.getByRole("button").classList.contains("vf")).toBe(true);
    cleanup();
    render(Button, { href: "/x", children: text("Go") });
    expect(screen.getByRole("link").classList.contains("vf")).toBe(true);
  });

  it("IconButton carries vf and keeps its tooltip label", () => {
    render(IconButton, { label: "Close", children: text("x") });
    const btn = screen.getByRole("button", { name: "Close" });
    expect(btn.classList.contains("vf")).toBe(true);
    expect(btn.getAttribute("data-tip")).toBe("Close");
  });

  it("IconButton tooltip='none' drops the tooltip", () => {
    render(IconButton, { label: "Close", tooltip: "none", children: text("x") });
    expect(screen.getByRole("button").hasAttribute("data-tip")).toBe(false);
  });

  it("Switch button carries vf", () => {
    render(Switch, { label: "On", hideLabel: true });
    expect(screen.getByRole("switch").classList.contains("vf")).toBe(true);
  });

  it("Select and NumberInput wrappers carry vf vf-within", () => {
    render(Select, {
      value: "a",
      options: [
        { value: "a", label: "A" },
        { value: "b", label: "B" },
      ],
      "aria-label": "Pick",
    });
    const wrap = screen.getByLabelText("Pick").parentElement;
    expect(wrap?.classList.contains("vf")).toBe(true);
    expect(wrap?.classList.contains("vf-within")).toBe(true);
    cleanup();
    render(NumberInput, { value: 1, "aria-label": "Num" });
    const nwrap = screen.getByLabelText("Num").parentElement;
    expect(nwrap?.classList.contains("vf")).toBe(true);
    expect(nwrap?.classList.contains("vf-within")).toBe(true);
  });

  it("Card carries vf and `selected` sets data-selected (selection border, not brackets)", () => {
    render(Card, { interactive: true, selected: true, children: text("Screen") });
    const card = screen.getByRole("button");
    expect(card.classList.contains("vf")).toBe(true);
    expect(card.classList.contains("vf-live")).toBe(false);
    expect(card.hasAttribute("data-selected")).toBe(true);
  });

  it("IconButton variant='danger' marks a destructive icon button", () => {
    render(IconButton, { label: "Delete layer", variant: "danger", children: text("x") });
    const btn = screen.getByRole("button", { name: "Delete layer" });
    expect(btn.classList.contains("danger")).toBe(true);
    expect(btn.classList.contains("ghost")).toBe(false);
  });
});
