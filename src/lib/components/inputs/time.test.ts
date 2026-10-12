import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isTime, parseTime } from "./time";
import TimeInput from "./TimeInput.svelte";

describe("parseTime", () => {
  it.each([
    ["12:00", "12:00"],
    ["12.00", "12:00"],
    ["12h00", "12:00"],
    ["1200", "12:00"],
    ["12", "12:00"],
    ["9", "09:00"],
    ["09", "09:00"],
    ["930", "09:30"],
    ["0930", "09:30"],
    ["9:30", "09:30"],
    ["9.30", "09:30"],
    ["  7:05 ", "07:05"],
    ["0", "00:00"],
    ["000", "00:00"],
    ["23:59", "23:59"],
    ["2359", "23:59"],
  ])("%s → %s", (text, want) => {
    expect(parseTime(text)).toEqual({ ok: true, value: want });
  });

  it.each([
    ["24:00", "Hours go from 00 to 23."],
    ["2400", "Hours go from 00 to 23."],
    ["25", "Hours go from 00 to 23."],
    ["12:60", "Minutes go from 00 to 59."],
    ["1260", "Minutes go from 00 to 59."],
    ["975", "Minutes go from 00 to 59."],
    ["12:5", "Use two digits for the minutes, like 09:30."],
    ["12:", "Use two digits for the minutes, like 09:30."],
    ["12345", "Too many digits: use HH:mm, like 09:30."],
    ["noon", "Use numbers, like 09:30."],
    ["12:3a", "Use numbers, like 09:30."],
    ["-1", "Use numbers, like 09:30."],
    ["", "Enter a time."],
    ["   ", "Enter a time."],
  ])("%s is invalid", (text, reason) => {
    expect(parseTime(text)).toEqual({ ok: false, reason });
  });

  it("isTime accepts only canonical HH:mm", () => {
    expect(isTime("09:30")).toBe(true);
    expect(isTime("23:59")).toBe(true);
    expect(isTime("9:30")).toBe(false);
    expect(isTime("24:00")).toBe(false);
    expect(isTime("")).toBe(false);
  });
});

describe("TimeInput", () => {
  afterEach(cleanup);

  function setup(props: Partial<{ value: string; optional: boolean; compact: boolean }> = {}) {
    const onchange = vi.fn();
    const oninvalid = vi.fn();
    render(TimeInput, { value: "10:00", onchange, oninvalid, "aria-label": "Start", ...props });
    const input = screen.getByRole("textbox", { name: "Start" }) as HTMLInputElement;
    return { input, onchange, oninvalid };
  }

  it("commits a normalised time on blur, not per keystroke", async () => {
    const { input, onchange, oninvalid } = setup();
    expect(input.value).toBe("10:00");
    expect(input.classList.contains("mono")).toBe(true);
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "930" } });
    expect(onchange).not.toHaveBeenCalled();
    expect(oninvalid).not.toHaveBeenCalled();
    await fireEvent.blur(input);
    expect(onchange).toHaveBeenCalledExactlyOnceWith("09:30");
    expect(input.value).toBe("09:30");
  });

  it("commits on Enter (before a form submits)", async () => {
    const { input, onchange } = setup();
    await fireEvent.input(input, { target: { value: "12.15" } });
    await fireEvent.keyDown(input, { key: "Enter" });
    expect(onchange).toHaveBeenCalledWith("12:15");
  });

  it("keeps an invalid time in the box, marks it and reports the reason", async () => {
    const { input, onchange, oninvalid } = setup();
    await fireEvent.focus(input);
    await fireEvent.input(input, { target: { value: "24:00" } });
    expect(oninvalid).toHaveBeenLastCalledWith("Hours go from 00 to 23.");
    await fireEvent.blur(input);
    expect(onchange).not.toHaveBeenCalled();
    expect(input.value).toBe("24:00");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    const why = document.getElementById(input.getAttribute("aria-describedby") ?? "");
    expect(why?.textContent).toBe("Hours go from 00 to 23.");
    // Fixed: valid again, reported once.
    await fireEvent.input(input, { target: { value: "23" } });
    expect(oninvalid).toHaveBeenLastCalledWith(null);
    expect(input.getAttribute("aria-invalid")).toBeNull();
    await fireEvent.blur(input);
    expect(onchange).toHaveBeenCalledWith("23:00");
  });

  it("compact puts the reason in a tooltip", async () => {
    const { input } = setup({ compact: true });
    await fireEvent.input(input, { target: { value: "12:60" } });
    expect(input.title).toBe("Minutes go from 00 to 59.");
  });

  it("empty is invalid unless optional", async () => {
    const req = setup();
    await fireEvent.input(req.input, { target: { value: "" } });
    await fireEvent.blur(req.input);
    expect(req.oninvalid).toHaveBeenLastCalledWith("Enter a time.");
    expect(req.onchange).not.toHaveBeenCalled();
    cleanup();
    const opt = setup({ optional: true });
    await fireEvent.input(opt.input, { target: { value: " " } });
    await fireEvent.blur(opt.input);
    expect(opt.oninvalid).not.toHaveBeenCalled();
    expect(opt.onchange).toHaveBeenCalledWith("");
  });

  it("follows the value from outside while not editing", async () => {
    const onchange = vi.fn();
    const r = render(TimeInput, { value: "08:00", onchange, "aria-label": "At" });
    await r.rerender({ value: "08:30", onchange, "aria-label": "At" });
    expect((screen.getByRole("textbox", { name: "At" }) as HTMLInputElement).value).toBe("08:30");
  });
});

describe("hosts: the flow inspector's time fields", () => {
  afterEach(cleanup);

  it("commit a normalised time on blur; an invalid one is never committed", async () => {
    const { default: Field } = await import("$lib/components/flows/graph/inspector/Field.svelte");
    const oninput = vi.fn();
    render(Field, { label: "Time", input: { value: "08:00", type: "time" }, oninput });
    const box = screen.getByLabelText("Time") as HTMLInputElement;
    await fireEvent.focus(box);
    await fireEvent.input(box, { target: { value: "24:00" } });
    await fireEvent.blur(box);
    expect(oninput).not.toHaveBeenCalled();
    expect(box.getAttribute("aria-invalid")).toBe("true");
    await fireEvent.focus(box);
    await fireEvent.input(box, { target: { value: "2015" } });
    await fireEvent.blur(box);
    expect(oninput).toHaveBeenCalledExactlyOnceWith("20:15");
  });

  it("optional ones commit empty", async () => {
    const { default: Field } = await import("$lib/components/flows/graph/inspector/Field.svelte");
    const oninput = vi.fn();
    render(Field, { label: "Until", input: { value: "08:00", type: "time", optional: true }, oninput });
    const box = screen.getByLabelText("Until");
    await fireEvent.input(box, { target: { value: "" } });
    await fireEvent.blur(box);
    expect(oninput).toHaveBeenCalledWith("");
  });
});
