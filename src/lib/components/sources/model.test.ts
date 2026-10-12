import { describe, expect, it } from "vitest";
import { bindingLabel } from "$lib/components/screens/editor/model";
import {
  SOURCE_CELL_MAX,
  SOURCE_ROWS_MAX,
  TEMPLATE_KEY_PATTERN,
  type SourceDoc,
} from "$lib/types";
import {
  blockedReason,
  byteLength,
  defaultSourceName,
  deriveKey,
  kindLabel,
  sortSources,
  sourceIssues,
  summaryOf,
  usedByLabel,
  validSourceName,
} from "./model";
import { detectDelimiter, headerMatch, isMultiCell, parseDelimited } from "./paste";

function source(over: Partial<Extract<SourceDoc, { kind: "text" }>> = {}): SourceDoc {
  return {
    id: "g1",
    name: "Guests",
    kind: "text",
    columns: [
      { key: "name", label: "Name" },
      { key: "title", label: "Title" },
    ],
    rows: [
      { name: "Grace Hopper", title: "Rear Admiral" },
      { name: "Ada Lovelace", title: "Analyst" },
      { name: "Alan Turing", title: "Mathematician" },
    ],
    index: 1,
    updatedAt: "2026-10-01T10:00:00Z",
    ...over,
  };
}

// ---- Keys ---------------------------------------------------------------------------------------

describe("deriveKey", () => {
  it("lowercases, replaces invalid runs with _ and starts with a letter", () => {
    expect(deriveKey("Name")).toBe("name");
    expect(deriveKey("Guest name")).toBe("guest_name");
    expect(deriveKey("  Job / Title!! ")).toBe("job_title");
    expect(deriveKey("2nd line")).toBe("nd_line");
    expect(deriveKey("Crème brûlée")).toBe("creme_brulee");
    expect(deriveKey("lower-third_x")).toBe("lower-third_x");
  });

  it("falls back to col and caps at 32 characters", () => {
    expect(deriveKey("")).toBe("col");
    expect(deriveKey("123 !!")).toBe("col");
    expect(deriveKey("名前")).toBe("col");
    const long = deriveKey("a".repeat(50));
    expect(long).toHaveLength(32);
  });

  it("is unique against taken keys and always valid", () => {
    expect(deriveKey("Name", ["name"])).toBe("name_2");
    expect(deriveKey("Name", new Set(["name", "name_2"]))).toBe("name_3");
    const k = deriveKey("b".repeat(40), ["b".repeat(32)]);
    expect(k).toHaveLength(32);
    expect(k.endsWith("_2")).toBe(true);
    for (const label of ["Name", "9", "Ünïcødé", "a b c", "x".repeat(40), "-x"]) {
      expect(deriveKey(label)).toMatch(TEMPLATE_KEY_PATTERN);
    }
  });
});

// ---- Validation ---------------------------------------------------------------------------------

describe("validation", () => {
  it("accepts a valid source", () => {
    expect(blockedReason(source())).toBeNull();
  });

  it("name: empty or too long", () => {
    expect(sourceIssues(source({ name: "  " })).name).toBe("Enter a name.");
    expect(sourceIssues(source({ name: "x".repeat(101) })).name).toMatch(/100/);
    expect(blockedReason(source({ name: "" }))).toBe("Name: Enter a name.");
  });

  it("columns: empty label, invalid key, duplicate key", () => {
    const d = source({
      columns: [
        { key: "name", label: "" },
        { key: "9x", label: "Bad" },
        { key: "dup", label: "A" },
        { key: "dup", label: "B" },
      ],
    });
    const i = sourceIssues(d);
    expect(i.columns[0]).toBe("Enter a label.");
    expect(i.columns[1]).toMatch(/start with a letter/);
    expect(i.columns[2]).toMatch(/Another column/);
    expect(i.columns[3]).toMatch(/Another column/);
    expect(blockedReason(d)).toBe("Column 1: Enter a label.");
  });

  it("limits: no columns, rows, cell bytes", () => {
    expect(sourceIssues(source({ columns: [] })).table).toMatch(/at least one column/);
    const many = source({ rows: Array.from({ length: SOURCE_ROWS_MAX + 1 }, () => ({})) });
    expect(blockedReason(many)).toBe(`Up to ${SOURCE_ROWS_MAX} rows.`);
    const big = "é".repeat(SOURCE_CELL_MAX / 2 + 1); // 2 bytes each
    expect(byteLength(big)).toBe(SOURCE_CELL_MAX + 2);
    expect(byteLength("abc")).toBe(3);
    const d = source({ rows: [{}, {}, { name: big }] });
    expect([...sourceIssues(d).cells]).toEqual(["2:name"]);
    expect(blockedReason(d)).toMatch(/^Row 3: a cell is over 4 KB/);
  });
});

// ---- Display ------------------------------------------------------------------------------------

describe("display helpers", () => {
  it("labels, sort, names and summaries", () => {
    expect(usedByLabel(0)).toBe("Not used");
    expect(usedByLabel(1)).toBe("Used by 1 screen");
    expect(usedByLabel(3)).toBe("Used by 3 screens");
    expect(kindLabel({ kind: "text" })).toBe("Data table");
    expect(kindLabel({ kind: "schedule" })).toBe("Schedule");
    expect(kindLabel({ kind: "api", format: "json" })).toBe("API");
    expect(kindLabel({ kind: "api", format: "csv" })).toBe("CSV link");
    expect(kindLabel({ kind: "api" })).toBe("API");
    expect(defaultSourceName([])).toBe("Source 1");
    expect(defaultSourceName(["Source 2"])).toBe("Source 3");
    expect(defaultSourceName(["Guests", "source 3"])).toBe("Source 4");
    expect(validSourceName(" ")).toBe(false);
    expect(validSourceName("Guests")).toBe(true);
    const s = summaryOf(source(), ["m1"]);
    expect(s).toMatchObject({ rowCount: 3, index: 1, current: { name: "Ada Lovelace" }, usedBy: ["m1"] });
    expect(summaryOf(source({ rows: [], index: 4 }), []).current).toEqual({});
    expect(sortSources([s, { ...s, id: "a", name: "agenda" }]).map((x) => x.name)).toEqual(["agenda", "Guests"]);
    expect(bindingLabel(s, "name")).toBe("Guests · Name");
    expect(bindingLabel(s, "gone")).toBe("Guests · gone (missing)");
    expect(bindingLabel(null, "name")).toBe("Missing source · name");
  });
});

// ---- Paste --------------------------------------------------------------------------------------

describe("parseDelimited", () => {
  it("TSV with CRLF and a trailing newline", () => {
    expect(parseDelimited("Grace\tAdmiral\r\nAda\tAnalyst\r\n")).toEqual([
      ["Grace", "Admiral"],
      ["Ada", "Analyst"],
    ]);
    expect(detectDelimiter("a\tb")).toBe("\t");
    expect(detectDelimiter("a,b")).toBe(",");
  });

  it("CSV with quotes, escaped quotes, commas and newlines in cells", () => {
    expect(parseDelimited('name,title\n"Hopper, Grace","Says ""hi""\nline 2"\nAda,')).toEqual([
      ["name", "title"],
      ["Hopper, Grace", 'Says "hi"\nline 2'],
      ["Ada", ""],
    ]);
  });

  it("TSV quoted cells (Excel multi-line) and empty lines", () => {
    expect(parseDelimited('"a\nb"\tc\n\n\t\nd\te')).toEqual([
      ["a\nb", "c"],
      ["d", "e"],
    ]);
  });

  it("lone CR line breaks and a single value", () => {
    expect(parseDelimited("a\rb")).toEqual([["a"], ["b"]]);
    expect(parseDelimited("solo")).toEqual([["solo"]]);
    expect(parseDelimited("")).toEqual([]);
  });

  it("multi-cell detection ignores one trailing newline", () => {
    expect(isMultiCell("Grace")).toBe(false);
    expect(isMultiCell("Grace\n")).toBe(false);
    expect(isMultiCell("Grace\r\n")).toBe(false);
    expect(isMultiCell("Grace\tAdmiral")).toBe(true);
    expect(isMultiCell("a\nb")).toBe(true);
  });
});

