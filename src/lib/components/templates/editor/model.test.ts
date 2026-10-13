import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CompletionContext, type CompletionResult, type CompletionSource } from "@codemirror/autocomplete";
import { ensureSyntaxTree } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import type { VariableDef } from "$lib/types";
import { AUTO_OUT_MAX_S, nextKey, validate, valueIssue, withKind } from "./model";
import { Autosave, DraftSender } from "./sync";

afterEach(() => {
  vi.useRealTimers();
});

const base = { fields: [], variables: [], htmlLength: 0, cssLength: 0 };

describe("validate", () => {
  it("checks range settings, select options and values", () => {
    const vars: VariableDef[] = [
      { key: "r", label: "R", type: "range", min: 10, max: 5, step: 1, unit: "px", default: "7" },
      { key: "s", label: "S", type: "select", options: [{ value: "a", label: "A" }, { value: "a", label: "B" }], default: "a" },
      { key: "f", label: "F", type: "font", default: "Comic Sans" },
      { key: "c", label: "C", type: "color", default: "#11223344" },
    ];
    const msgs = validate({ ...base, variables: vars }).map((i) => `${i.index}:${i.prop}:${i.message}`);
    expect(msgs).toContain("0:range:Max must be above min.");
    expect(msgs).toContain("1:options:Option values must be unique.");
    expect(msgs).toContain("2:value:Choose a font.");
    expect(msgs.some((m) => m.startsWith("3:"))).toBe(false); // #rrggbbaa is fine
  });

  it("flags oversized HTML/CSS and range values out of bounds", () => {
    const issues = validate({
      ...base,
      htmlLength: 300 * 1024,
      variables: [{ key: "r", label: "R", type: "range", min: 0, max: 4, step: 1, unit: "", default: "9" }],
    });
    expect(issues.map((i) => i.message)).toEqual(["HTML is over 256 KB.", "Between 0 and 4."]);
  });

  it("new keys skip used ones; kind changes reset settings", () => {
    expect(nextKey("field", ["field1", "field2"])).toBe("field3");
    const v = withKind({ key: "x", label: "X" }, "select");
    expect(v.type === "select" && v.options.length).toBe(1);
    expect(valueIssue(v)).toBeNull();
  });
});

describe("DraftSender", () => {
  it("debounces per kind and merges value maps", async () => {
    vi.useFakeTimers();
    const send = vi.fn().mockResolvedValue(undefined);
    const d = new DraftSender(150, send);
    d.queue("css", "a");
    d.merge("variableValues", { a: "1" });
    d.merge("variableValues", { b: "2" });
    d.queue("css", () => "b");
    await vi.advanceTimersByTimeAsync(150);
    expect(send.mock.calls.map((c) => c[0])).toEqual([{ variableValues: { a: "1", b: "2" } }, { css: "b" }]);
    d.queue("html", "x");
    d.cancel();
    await vi.advanceTimersByTimeAsync(500);
    expect(send).toHaveBeenCalledTimes(2);
  });
});

describe("Autosave", () => {
  it("never saves while blocked and keeps the edit dirty", async () => {
    vi.useFakeTimers();
    let blocked: string | null = "bad";
    const save = vi.fn().mockResolvedValue({ updatedAt: "t" });
    const a = new Autosave<number>({
      delayMs: 800,
      snapshot: () => 1,
      blocked: () => blocked,
      save,
      onState: () => {},
      describe: String,
    });
    a.touch();
    await vi.advanceTimersByTimeAsync(800);
    expect(save).not.toHaveBeenCalled();
    expect(a.dirty).toBe(true);
    blocked = null;
    await a.flush();
    expect(save).toHaveBeenCalledTimes(1);
    expect(a.dirty).toBe(false);
  });
});

describe("codemirror module (real CodeMirror in jsdom)", () => {
  function mount(lang: "html" | "css", doc: string) {
    return import("./codemirror").then(({ createEditor }) => {
      const parent = document.createElement("div");
      document.body.append(parent);
      const h = createEditor(parent, {
        lang,
        doc,
        label: "Code",
        onChange: () => {},
        completions: () => [{ key: "accent", label: "Accent" }],
      });
      const view = EditorView.findFromDOM(parent.querySelector(".cm-editor") as HTMLElement)!;
      return {
        view,
        done: () => {
          h.destroy();
          parent.remove();
        },
      };
    });
  }

  async function complete(view: EditorView, pos: number): Promise<CompletionResult[]> {
    const sources = view.state.languageDataAt<CompletionSource>("autocomplete", pos);
    const ctx = new CompletionContext(view.state, pos, true);
    const results = await Promise.all(sources.map((s) => s(ctx)));
    return results.filter((r): r is CompletionResult => !!r);
  }

  it("parses CSS inside <style> and style attributes (no JavaScript grammar)", async () => {
    const doc = '<div style="color: red"><style>.a { margin: 0 }</style></div>';
    const { view, done } = await mount("html", doc);
    const tree = ensureSyntaxTree(view.state, doc.length, 5000)!;
    expect(tree.resolveInner(doc.indexOf("color") + 1, 1).name).toBe("PropertyName");
    expect(tree.resolveInner(doc.indexOf("margin") + 1, 1).name).toBe("PropertyName");
    expect(tree.resolveInner(doc.indexOf("div") + 1, 1).name).toBe("TagName");
    done();
  });

  it("closes a tag when > is typed", async () => {
    const { view, done } = await mount("html", "<div");
    const insert = () => view.state.update({ changes: { from: 4, insert: ">" }, selection: { anchor: 5 } });
    const handled = view.state.facet(EditorView.inputHandler).some((h) => h(view, 4, 4, ">", insert));
    expect(handled).toBe(true);
    expect(view.state.doc.toString()).toBe("<div></div>");
    done();
  });

  it("completes var(--key) and loads the CSS keyword completions on demand", async () => {
    const doc = "a { color: var(--ac); }\nb { display: bl; }";
    const { view, done } = await mount("css", doc);
    const labels = async (pos: number) =>
      (await complete(view, pos)).flatMap((r) => r.options.map((o) => o.label));
    expect(await labels(doc.indexOf("ac") + 2)).toContain("--accent");
    expect(await labels(doc.indexOf("bl") + 2)).toContain("block");
    done();
  });

  it("creates a view, reports changes, and destroys it", async () => {
    const { createEditor } = await import("./codemirror");
    const parent = document.createElement("div");
    document.body.append(parent);
    const onChange = vi.fn();
    const h = createEditor(parent, {
      lang: "html",
      doc: "<div>{{name}}</div>",
      label: "Template HTML",
      onChange,
      completions: () => [{ key: "name", label: "Name" }],
    });
    expect(h.getValue()).toBe("<div>{{name}}</div>");
    expect(h.length()).toBe(19);
    const content = parent.querySelector(".cm-content");
    expect(content?.getAttribute("aria-label")).toBe("Template HTML");
    const view = EditorView.findFromDOM(parent.querySelector(".cm-editor") as HTMLElement);
    view?.dispatch({ changes: { from: 0, insert: "x" } });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(h.getValue()).toBe("x<div>{{name}}</div>");
    expect(parent.querySelector(".cm-kfield")?.textContent).toBe("{{name}}");
    h.destroy();
    expect(parent.querySelector(".cm-editor")).toBeNull();
    parent.remove();
  });
});

describe("auto-out", () => {
  it("the input's maximum is what saving accepts", () => {
    const rs = readFileSync("src-tauri/src/core/templates.rs", "utf8");
    const max = /AUTO_OUT_RANGE[^=]*= *[\d_]+\.\.=([\d_]+);/.exec(rs)?.[1];
    expect(Number(max?.replaceAll("_", ""))).toBe(AUTO_OUT_MAX_S * 1000);
  });
});
