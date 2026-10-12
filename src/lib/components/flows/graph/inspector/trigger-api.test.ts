// The inspector's form for the API value changed trigger: the source picker lists API sources
// only, the column picker the source's columns ("Any row changes" first); each change is one commit.

import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FlowNode, SourceSummary } from "$lib/types";
import type { FlowEditorCtl } from "../editor.svelte";
import TriggerForm from "./TriggerForm.svelte";

function source(id: string, name: string, kind: SourceSummary["kind"], columns: SourceSummary["columns"] = []): SourceSummary {
  return {
    id,
    name,
    kind,
    rowCount: 0,
    index: 0,
    current: {},
    columns,
    usedBy: [],
    updatedAt: "",
    auto: null,
    nextAt: null,
    nowId: null,
    nextId: null,
  };
}

const sources = new Map([
  ["sp", source("sp", "Sponsors", "text")],
  ["feed", source("feed", "Scores", "api", [{ key: "home", label: "Home" }, { key: "score", label: "Score" }])],
  ["wx", source("wx", "Weather", "api", [{ key: "temp", label: "Temperature" }])],
]);

function editor(readOnly = false) {
  const setKind = vi.fn();
  const ed = { readOnly, lookups: { screens: new Map(), sources, flows: new Map() }, setKind } as unknown as FlowEditorCtl;
  return { ed, setKind };
}

const node = (column: string | null): FlowNode =>
  ({ id: "t1", type: "trigger.apiChanged", position: { x: 0, y: 0 }, config: { sourceId: "feed", column } }) as FlowNode;

afterEach(cleanup);

describe("TriggerForm: API value changed", () => {
  it("lists API sources and the source's columns; changing commits the config", async () => {
    const { ed, setKind } = editor();
    render(TriggerForm, { ed, node: node(null) });
    const src = screen.getByLabelText("API source") as HTMLSelectElement;
    expect([...src.options].map((o) => o.textContent)).toEqual(["Scores", "Weather"]);
    const col = screen.getByLabelText("Column") as HTMLSelectElement;
    expect([...col.options].map((o) => o.textContent)).toEqual(["Any row changes", "Home", "Score"]);
    expect(col.value).toBe("");

    col.value = "score";
    await fireEvent.change(col);
    expect(setKind).toHaveBeenLastCalledWith("t1", { type: "trigger.apiChanged", config: { sourceId: "feed", column: "score" } }, null);

    // Another source: its own columns, the column reset.
    src.value = "wx";
    await fireEvent.change(src);
    expect(setKind).toHaveBeenLastCalledWith("t1", { type: "trigger.apiChanged", config: { sourceId: "wx", column: null } }, null);
    expect(screen.getByText(/● value/)).toBeTruthy();
  });

  it("read-only flows can't change it", () => {
    const { ed } = editor(true);
    render(TriggerForm, { ed, node: node("home") });
    expect((screen.getByLabelText("API source") as HTMLSelectElement).disabled).toBe(true);
    expect((screen.getByLabelText("Column") as HTMLSelectElement).value).toBe("home");
  });
});
