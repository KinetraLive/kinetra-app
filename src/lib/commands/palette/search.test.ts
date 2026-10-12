import { describe, expect, it } from "vitest";
import type { Command, CommandContext } from "../registry.svelte";
import {
  CAP_EMPTY,
  CAP_QUERY,
  parseQuery,
  prepare,
  search,
  searchOptions,
  searchSections,
  sectionOrder,
} from "./search";

const ctx = (route: CommandContext["route"] = "dashboard", extra: Partial<CommandContext> = {}): CommandContext => ({
  route,
  project: true,
  screenId: null,
  layerIds: [],
  studio: false,
  ...extra,
});

const cmd = (id: string, title: string, section: Command["section"], more: Partial<Command> = {}): Command => ({
  id,
  title,
  section,
  run: () => {},
  ...more,
});

const base: Command[] = [
  cmd("nav.dashboard", "Go to Dashboard", "goto"),
  cmd("nav.control", "Go to Control", "goto"),
  cmd("nav.screens", "Go to Screens", "goto"),
  cmd("create.screen", "New screen", "create"),
  cmd("create.template", "New template", "create"),
  cmd("live.take:s1/l1", "Take {layer}", "live", { params: () => ({ layer: "Opening keynote" }), when: (c) => c.route === "control" }),
  cmd("live.clear:s1", "Clear {screen}", "live", { params: () => ({ screen: "Screen 1" }), when: (c) => c.route === "control" }),
  cmd("live.panic", "Panic", "live", { confirm: true, keywords: ["cut", "emergency"] }),
  cmd("item.screen:s1", "Screen 1", "items", { scope: "screen" }),
  cmd("item.template:t1", "Lower third", "items", { scope: "template" }),
  cmd("settings.theme", "Toggle theme", "settings", { scope: "setting", keywords: ["dark", "light"] }),
];

const titles = (q: string, c = ctx(), recent: string[] = []) => search(q, base, c, recent).map((m) => m.title);
const sections = (q: string, c = ctx(), recent: string[] = []) =>
  searchSections(q, prepare(base, c), c, recent).map((r) => r.section);

describe("search", () => {
  it("parses prefixes", () => {
    expect(parseQuery(">  take ")).toEqual({ scope: "command", text: "take" });
    expect(parseQuery("@scr")).toEqual({ scope: "screen", text: "scr" });
    expect(parseQuery("#")).toEqual({ scope: "template", text: "" });
    expect(parseQuery("/dark")).toEqual({ scope: "setting", text: "dark" });
    expect(parseQuery(" grid")).toEqual({ scope: null, text: "grid" });
  });

  it("translates titles with params and filters `when`", () => {
    expect(titles("", ctx("control"))).toContain("Take Opening keynote");
    expect(titles("", ctx("dashboard"))).not.toContain("Take Opening keynote");
  });

  it("orders sections: Live first in Control, after Project items elsewhere", () => {
    expect(sectionOrder(ctx("control"))[0]).toBe("live");
    expect(sections("", ctx("control"))).toEqual(["live", "goto", "create", "items", "settings"]);
    expect(sections("", ctx("dashboard"))).toEqual(["goto", "create", "items", "live", "settings"]);
  });

  it("puts Recent first and doesn't repeat those commands in their section", () => {
    const r = searchSections("", prepare(base, ctx()), ctx(), ["create.template", "gone", "nav.control"]);
    expect(r[0].section).toBe("recent");
    expect(r[0].matches.map((m) => m.command.id)).toEqual(["create.template", "nav.control"]);
    const goto = r.find((x) => x.section === "goto");
    expect(goto?.matches.map((m) => m.command.id)).toEqual(["nav.dashboard", "nav.screens"]);
  });

  it("Recent only lists commands that match the query", () => {
    expect(sections("screen", ctx(), ["create.template"])).not.toContain("recent");
    expect(sections("templ", ctx(), ["create.template"])[0]).toBe("recent");
  });

  it("scopes by prefix", () => {
    expect(titles("@")).toEqual(["Screen 1"]);
    expect(titles("#")).toEqual(["Lower third"]);
    expect(titles("/")).toEqual(["Toggle theme"]);
    expect(titles(">screen")).toEqual(["Go to Screens", "New screen"]);
  });

  it("matches keywords at a lower weight without highlighting", () => {
    const m = search("dark", base, ctx(), []);
    expect(m.map((x) => x.title)).toEqual(["Toggle theme"]);
    expect(m[0].ranges).toEqual([]);
    const c = [cmd("a", "Dark mode", "settings"), cmd("b", "Toggle theme", "settings", { keywords: ["dark"] })];
    expect(search("dark", c, ctx(), []).map((x) => x.command.id)).toEqual(["a", "b"]);
  });

  it("highlights title ranges", () => {
    const m = search("ctrl", base, ctx(), []);
    expect(m[0].title).toBe("Go to Control");
    expect(m[0].ranges.length).toBeGreaterThan(0);
  });

  it("drops scattered matches once something matches by word starts", () => {
    const c = [cmd("a", "Take screen", "live"), cmd("b", "Rotate selection", "live")];
    // "ts": word starts in "Take screen"; scattered in "Rotate selection" (t…s).
    expect(search("ts", c, ctx(), []).map((x) => x.command.id)).toEqual(["a"]);
  });

  it("adds rank(ctx) and keeps registration order on ties", () => {
    const c = [
      cmd("a", "Clear one", "live"),
      cmd("b", "Clear two", "live", { rank: (x) => (x.route === "control" ? 1000 : 0) }),
      cmd("c", "Clear six", "live"),
    ];
    expect(search("clear", c, ctx("control"), []).map((x) => x.command.id)).toEqual(["b", "a", "c"]);
    expect(search("clear", c, ctx(), []).map((x) => x.command.id)).toEqual(["a", "b", "c"]);
    // Empty query: rank, then registration order.
    expect(search("", c, ctx("control"), []).map((x) => x.command.id)).toEqual(["b", "a", "c"]);
  });

  it("keeps disabled commands listed", () => {
    const c = [cmd("a", "Take", "live", { disabled: () => "Nothing selected" })];
    expect(search("take", c, ctx(), []).length).toBe(1);
  });

  it("caps sections and reports the total", () => {
    const many = Array.from({ length: 30 }, (_, i) => cmd(`s${i}`, `Screen ${i}`, "items", { scope: "screen" }));
    const prepared = prepare(many, ctx());
    const empty = searchSections("", prepared, ctx(), []);
    expect(empty[0].matches.length).toBe(CAP_EMPTY);
    expect(empty[0].total).toBe(30);
    const q = searchSections("screen", prepared, ctx(), []);
    expect(q[0].matches.length).toBe(CAP_QUERY);
    expect(q[0].matches.map((m) => m.command.id).slice(0, 3)).toEqual(["s0", "s1", "s2"]);
    const more = searchSections("screen", prepared, ctx(), [], { items: 20 });
    expect(more[0].matches.length).toBe(20);
    expect(searchSections("@", prepared, ctx(), [])[0].matches.length).toBe(30);
  });

  it("skips commands whose callbacks throw", () => {
    const c = [cmd("a", "Fine", "goto"), cmd("b", "Broken", "goto", { when: () => { throw new Error("x"); } })];
    expect(search("", c, ctx(), []).map((x) => x.command.id)).toEqual(["a"]);
  });

  it("searches follow-up options", () => {
    const opts = [
      { id: "minimal", title: "Minimal" },
      { id: "pill", title: "Pill", keywords: ["rounded"] },
      { id: "inline", title: "Inline" },
    ];
    expect(searchOptions("", opts).map((m) => m.option.id)).toEqual(["minimal", "pill", "inline"]);
    expect(searchOptions("in", opts).map((m) => m.option.id)).toEqual(["inline", "minimal"]);
    expect(searchOptions("round", opts).map((m) => m.option.id)).toEqual(["pill"]);
  });

  it("stays well inside one frame for 2000 entries", () => {
    const words = ["Opening", "keynote", "Lower", "third", "Screen", "Sponsor", "Panel", "Guest", "Ticker", "Clock"];
    const many: Command[] = Array.from({ length: 2000 }, (_, i) =>
      cmd(`item:${i}`, `${words[i % 10]} ${words[(i * 7) % 10]} ${i}`, i % 3 === 0 ? "items" : i % 3 === 1 ? "live" : "goto", {
        keywords: [words[(i * 3) % 10].toLowerCase()],
      }),
    );
    const c = ctx("control");
    const prepared = prepare(many, c);
    const queries = ["o", "op", "ope", "open", "openi", "opening k", "lt", "sp gu", "zzz", "screen 19"];
    for (const q of queries) searchSections(q, prepared, c, []); // warm up
    // Each query's best of 5 rounds (a GC pause or a busy parallel test run is not the search's
    // cost), then the worst query.
    let worst = 0;
    for (const q of queries) {
      let best = Infinity;
      for (let round = 0; round < 5; round++) {
        const t0 = performance.now();
        searchSections(q, prepared, c, []);
        best = Math.min(best, performance.now() - t0);
      }
      worst = Math.max(worst, best);
    }
    // Budget: one frame (16 ms) per keystroke including rendering; scripting must leave room.
    expect(worst).toBeLessThan(8);
  });
});
