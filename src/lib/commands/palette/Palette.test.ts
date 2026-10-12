import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, CommandContext } from "../registry.svelte";

// The palette reads the command system only through ./source; a small fake stands in for it.
const fake = vi.hoisted(() => {
  const state = {
    ctx: { route: "control", project: true, screenId: "s1", layerIds: [], studio: false } as CommandContext,
    commands: [] as Command[],
    recent: [] as string[],
    keys: {} as Record<string, string[]>,
  };
  return {
    state,
    currentContext: () => state.ctx,
    paletteCommands: vi.fn(() => state.commands),
    recentIds: () => state.recent,
    keysFor: (id: string) => state.keys[id] ?? [],
    getCommand: (id: string) => state.commands.find((c) => c.id === id),
    runCommand: vi.fn(async (_id: string) => true),
    runFollowUp: vi.fn(async (_id: string, _v: string) => true),
    watchItems: vi.fn(() => () => {}),
    t: (m: string, p?: Record<string, string | number>) => (p ? m.replace(/\{(\w+)\}/g, (_, k) => String(p[k] ?? `{${k}}`)) : m),
  };
});
vi.mock("./source", () => fake);

import Palette from "./Palette.svelte";

const cmd = (id: string, title: string, section: Command["section"], more: Partial<Command> = {}): Command => ({
  id,
  title,
  section,
  run: () => {},
  ...more,
});

function setup(commands: Command[], props: { followUp?: string } = {}) {
  fake.state.commands = commands;
  const onclose = vi.fn();
  const r = render(Palette, { onclose, ...props });
  const input = screen.getByRole("combobox") as HTMLInputElement;
  return { ...r, onclose, input };
}

const options = () => screen.queryAllByRole("option");
const selected = () => options().find((o) => o.getAttribute("aria-selected") === "true");
const key = (el: Element, k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(el, { key: k, ...init });

async function type(input: HTMLInputElement, value: string) {
  await fireEvent.input(input, { target: { value } });
  await tick();
}

const BASIC = [
  cmd("nav.control", "Go to Control", "goto"),
  cmd("nav.screens", "Go to Screens", "goto"),
  cmd("live.take", "Take {layer}", "live", { params: () => ({ layer: "Opening keynote" }) }),
  cmd("live.clearAll", "Clear all", "live", { confirm: true }),
  cmd("settings.theme", "Toggle theme", "settings", { scope: "setting" }),
];

beforeEach(() => {
  fake.state.recent = [];
  fake.state.keys = {};
  fake.runCommand.mockClear();
  fake.runFollowUp.mockClear();
  fake.paletteCommands.mockClear();
});
afterEach(() => cleanup());

describe("Palette", () => {
  it("\"Show all commands\" lists every command in place instead of closing", async () => {
    const many = Array.from({ length: 12 }, (_, i) => cmd(`nav.x${i}`, `Go to Page ${i}`, "goto"));
    const { input, onclose } = setup([cmd("palette.open", "Show all commands", "settings"), ...many]);
    await type(input, "show all");
    await key(input, "Enter");
    await tick();
    expect(onclose).not.toHaveBeenCalled();
    expect(fake.runCommand).not.toHaveBeenCalled();
    expect(input.value).toBe(">");
    // Every command, no "Show N more" row.
    expect(options().filter((o) => /Go to Page/.test(o.textContent ?? ""))).toHaveLength(12);
    expect(options().some((o) => /Show d+ more/.test(o.textContent ?? ""))).toBe(false);
  });

  it("is a combobox over a grouped listbox; Live first in Control", () => {
    const { input } = setup(BASIC);
    expect(input.getAttribute("aria-expanded")).toBe("true");
    const list = screen.getByRole("listbox");
    expect(input.getAttribute("aria-controls")).toBe(list.id);
    const groups = screen.getAllByRole("group");
    expect(groups[0].getAttribute("aria-labelledby")).toBeTruthy();
    expect(document.getElementById(groups[0].getAttribute("aria-labelledby")!)?.textContent).toBe("Live");
    expect(options()[0].textContent).toContain("Take Opening keynote");
    expect(input.getAttribute("aria-activedescendant")).toBe(options()[0].id);
    expect(input.placeholder).toBe("Type a command or search…");
    expect(fake.paletteCommands).toHaveBeenCalledWith(fake.state.ctx);
  });

  it("shows keys in mono and highlights matches", async () => {
    fake.state.keys = { "nav.control": ["Ctrl+3"] };
    const { input } = setup(BASIC);
    await type(input, "control");
    const row = options()[0];
    expect(row.textContent).toContain("Go to Control");
    expect(row.querySelector("mark")?.textContent).toBe("Control");
    expect(row.querySelector("kbd.mono")?.textContent).toBe("Ctrl+3");
  });

  it("moves with arrows, Home/End and Tab between sections", async () => {
    const { input } = setup(BASIC);
    await key(input, "ArrowDown");
    expect(selected()?.textContent).toContain("Clear all");
    await key(input, "End");
    expect(selected()?.textContent).toContain("Toggle theme");
    await key(input, "Home");
    expect(selected()?.textContent).toContain("Take Opening keynote");
    await key(input, "Tab");
    expect(selected()?.textContent).toContain("Go to Control");
    await key(input, "Tab", { shiftKey: true });
    expect(selected()?.textContent).toContain("Take Opening keynote");
    await key(input, "ArrowUp");
    expect(selected()?.textContent).toContain("Toggle theme");
    expect(input.getAttribute("aria-activedescendant")).toBe(selected()?.id);
  });

  it("runs on Enter and closes", async () => {
    const { input, onclose } = setup(BASIC);
    await type(input, "screens");
    await key(input, "Enter");
    expect(onclose).toHaveBeenCalled();
    expect(fake.runCommand).toHaveBeenCalledWith("nav.screens");
  });

  it("confirm commands need a second Enter; any other key cancels", async () => {
    const { input, onclose } = setup(BASIC);
    await type(input, "clear all");
    await key(input, "Enter");
    expect(fake.runCommand).not.toHaveBeenCalled();
    expect(selected()?.classList.contains("armed")).toBe(true);
    expect(selected()?.textContent).toContain("Press Enter again to confirm");
    await key(input, "ArrowDown");
    expect(screen.queryByText("Press Enter again to confirm")).toBeNull();
    await key(input, "ArrowUp");
    await key(input, "Enter");
    await key(input, "Enter");
    expect(fake.runCommand).toHaveBeenCalledWith("live.clearAll");
    expect(onclose).toHaveBeenCalled();
  });

  it("ignores Enter key repeats", async () => {
    const { input } = setup(BASIC);
    await type(input, "clear all");
    await key(input, "Enter");
    await key(input, "Enter", { repeat: true });
    expect(fake.runCommand).not.toHaveBeenCalled();
  });

  it("lists disabled commands with their reason and doesn't run them", async () => {
    const { input } = setup([cmd("live.update", "Update", "live", { disabled: () => "Nothing changed" })]);
    expect(options()[0].getAttribute("aria-disabled")).toBe("true");
    expect(options()[0].textContent).toContain("Nothing changed");
    await key(input, "Enter");
    expect(fake.runCommand).not.toHaveBeenCalled();
  });

  it("follow-up pick: a searched sub-list; Esc steps back", async () => {
    const pick = cmd("style.apply", "Apply style…", "create", {
      run: undefined,
      next: {
        kind: "pick",
        placeholder: "Pick a style",
        options: () => [
          { id: "minimal", title: "Minimal" },
          { id: "pill", title: "Pill" },
          { id: "inline", title: "Inline" },
        ],
        run: () => {},
      },
    });
    const { input, onclose } = setup([...BASIC, pick]);
    await type(input, "apply");
    await key(input, "Enter");
    expect(input.value).toBe("");
    expect(input.placeholder).toBe("Pick a style");
    expect(options().map((o) => o.textContent?.trim())).toEqual(["Minimal", "Pill", "Inline"]);
    await type(input, "pil");
    expect(options().map((o) => o.textContent?.trim())).toEqual(["Pill"]);
    await key(input, "Escape");
    expect(onclose).not.toHaveBeenCalled();
    expect(input.value).toBe("apply");
    await key(input, "Enter");
    await key(input, "Backspace");
    expect(input.value).toBe("apply");
    await key(input, "Enter");
    await key(input, "ArrowDown");
    await key(input, "Enter");
    expect(fake.runFollowUp).toHaveBeenCalledWith("style.apply", "pill");
    expect(onclose).toHaveBeenCalled();
  });

  it("follow-up text: validation blocks Enter", async () => {
    const rename = cmd("screen.rename", "Rename screen…", "create", {
      run: undefined,
      next: {
        kind: "text",
        placeholder: "Screen name",
        value: () => "Main",
        validate: (s) => (s.trim() === "" ? "Enter a name" : null),
        run: () => {},
      },
    });
    const { input } = setup([rename]);
    await key(input, "Enter");
    const box = screen.getByRole("textbox") as HTMLInputElement;
    expect(box.value).toBe("Main");
    expect(screen.queryByRole("listbox")).toBeNull();
    await type(box, " ");
    expect(screen.getByRole("alert").textContent).toBe("Enter a name");
    await key(box, "Enter");
    expect(fake.runFollowUp).not.toHaveBeenCalled();
    await type(box, "Stage");
    await key(box, "Enter");
    expect(fake.runFollowUp).toHaveBeenCalledWith("screen.rename", "Stage");
  });

  it("opens straight into a follow-up step; stepping back closes", async () => {
    const pick = cmd("style.apply", "Apply style…", "create", {
      run: undefined,
      next: { kind: "pick", placeholder: "Pick a style", options: () => [{ id: "pill", title: "Pill" }], run: () => {} },
    });
    const { input, onclose } = setup([pick], { followUp: "style.apply" });
    expect(input.placeholder).toBe("Pick a style");
    await key(input, "Escape");
    expect(onclose).toHaveBeenCalled();
  });

  it("caps a section with a Show more row", async () => {
    const many = Array.from({ length: 12 }, (_, i) => cmd(`s${i}`, `Screen ${i}`, "items", { scope: "screen" }));
    const { input } = setup(many);
    expect(options().length).toBe(6);
    expect(options()[5].textContent).toContain("Show 7 more");
    await key(input, "End");
    await key(input, "Enter");
    expect(options().length).toBe(12);
  });

  it("Recent comes first; the empty state says No matches", async () => {
    fake.state.recent = ["settings.theme"];
    const { input } = setup(BASIC);
    expect(options()[0].textContent).toContain("Toggle theme");
    await type(input, "zzzz");
    expect(options().length).toBe(0);
    expect(screen.getByText("No matches")).toBeTruthy();
  });

  it("closes on Esc and gives focus back", async () => {
    const before = document.createElement("button");
    document.body.append(before);
    before.focus();
    const { input, onclose } = setup(BASIC);
    expect(document.activeElement).toBe(input);
    await key(input, "Escape");
    expect(onclose).toHaveBeenCalled();
    expect(document.activeElement).toBe(before);
    before.remove();
  });
});
