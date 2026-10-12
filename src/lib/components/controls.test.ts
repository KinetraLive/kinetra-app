import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { createRawSnippet, tick } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import Button from "./Button.svelte";
import NumberInput from "./NumberInput.svelte";
import Switch from "./Switch.svelte";

afterEach(cleanup);

const text = (s: string) => createRawSnippet(() => ({ render: () => `<span>${s}</span>` }));

describe("NumberInput", () => {
  it("clamps and snaps on change, reverting invalid input", async () => {
    const onchange = vi.fn();
    render(NumberInput, { value: 10, min: 1, max: 240, suffix: "min", "aria-label": "Interval", onchange });
    const input = screen.getByLabelText("Interval") as HTMLInputElement;

    input.value = "999";
    await fireEvent.change(input);
    expect(onchange).toHaveBeenLastCalledWith(240);
    expect(input.value).toBe("240");

    input.value = "0";
    await fireEvent.change(input);
    expect(onchange).toHaveBeenLastCalledWith(1);

    input.value = "";
    await fireEvent.change(input);
    expect(input.value).toBe("1");
    expect(onchange).toHaveBeenCalledTimes(2);
  });
});

describe("Switch", () => {
  it("toggles aria-checked and reports the new value", async () => {
    const onchange = vi.fn();
    render(Switch, { label: "Backups", onchange });
    const sw = screen.getByRole("switch", { name: "Backups" });
    expect(sw.getAttribute("aria-checked")).toBe("false");
    await fireEvent.click(sw);
    expect(sw.getAttribute("aria-checked")).toBe("true");
    expect(onchange).toHaveBeenCalledWith(true);
  });
});

describe("Button", () => {
  afterEach(() => vi.useRealTimers());

  it("shows the spinner only after 300 ms of loading", async () => {
    vi.useFakeTimers();
    const { container } = render(Button, { loading: true, children: text("Save") });
    const btn = screen.getByRole("button");
    expect(btn.hasAttribute("disabled")).toBe(true);
    expect(btn.getAttribute("aria-busy")).toBe("true");
    expect(container.querySelector(".spinner")).toBeNull();
    vi.advanceTimersByTime(299);
    await tick();
    expect(container.querySelector(".spinner")).toBeNull();
    vi.advanceTimersByTime(1);
    await tick();
    expect(container.querySelector(".spinner")).not.toBeNull();
  });

  it("renders a link when href is set", () => {
    render(Button, { href: "/settings", children: text("Settings") });
    expect(screen.getByRole("link", { name: "Settings" }).getAttribute("href")).toBe("/settings");
  });
});
