// Reading text pasted from a spreadsheet (the sheet's paste and the schedule editor). Pure: no
// Svelte, no DOM.
//
// `parseDelimited` reads tab-separated text (what Excel, Sheets and Numbers put on the clipboard)
// or comma-separated text (when there is no tab at all). Quoting follows RFC 4180 for both: a cell
// that starts with `"` runs to the next lone `"` and may contain the delimiter, newlines and `""`.

import type { SourceColumn } from "$lib/types";

/** The delimiter of pasted text: a tab when there is one, else a comma. */
export function detectDelimiter(text: string): "\t" | "," {
  return text.includes("\t") ? "\t" : ",";
}

/**
 * Splits delimited text into lines of cells. CRLF, CR and LF all end a line; a trailing line
 * break doesn't make an empty last line, and lines that are entirely empty are dropped.
 */
export function parseDelimited(text: string, delimiter: string = detectDelimiter(text)): string[][] {
  const lines: string[][] = [];
  let line: string[] = [];
  let cell = "";
  let i = 0;
  const n = text.length;
  let atCellStart = true;

  const endCell = () => {
    line.push(cell);
    cell = "";
    atCellStart = true;
  };
  const endLine = () => {
    endCell();
    if (line.some((c) => c !== "")) lines.push(line);
    line = [];
  };

  while (i < n) {
    const ch = text[i];
    if (atCellStart && ch === '"') {
      // Quoted cell: up to the closing quote; "" is a literal quote.
      i++;
      while (i < n) {
        if (text[i] === '"') {
          if (text[i + 1] === '"') {
            cell += '"';
            i += 2;
            continue;
          }
          i++;
          break;
        }
        cell += text[i++];
      }
      atCellStart = false;
      continue;
    }
    atCellStart = false;
    if (ch === delimiter) {
      endCell();
      i++;
    } else if (ch === "\r" || ch === "\n") {
      endLine();
      i += ch === "\r" && text[i + 1] === "\n" ? 2 : 1;
    } else {
      cell += ch;
      i++;
    }
  }
  if (cell !== "" || line.length > 0) endLine();
  return lines;
}

/** True when pasted text looks like more than one cell (so it fills rows, not one input). */
export function isMultiCell(text: string): boolean {
  const t = text.replace(/(\r\n|\r|\n)$/, "");
  return /[\t\r\n]/.test(t);
}

const norm = (s: string) => s.trim().toLowerCase();

/**
 * Index of the existing column each first-line cell names (by label or key, case-insensitive),
 * or -1. Null when the first line is not a header: no cell names a column, or two cells name the
 * same one.
 */
export function headerMatch(first: readonly string[], columns: readonly SourceColumn[]): number[] | null {
  const seen = new Set<number>();
  const match = first.map((cell) => {
    const c = norm(cell);
    if (c === "") return -1;
    return columns.findIndex((col) => norm(col.label) === c || col.key === c);
  });
  for (const m of match) {
    if (m < 0) continue;
    if (seen.has(m)) return null;
    seen.add(m);
  }
  return seen.size > 0 ? match : null;
}
