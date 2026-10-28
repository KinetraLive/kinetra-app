// Pure logic of the API source editor: JSON Pointers, the mapping applied to a test
// response (rows, cells, per-column hits), which tree nodes the mapping uses, checks against the
// backend's limits, and small document edits. No Svelte, no I/O.

import {
  API_POLL_MAX_MS,
  API_POLL_MIN_MS,
  SOURCE_COLUMNS_MAX,
  SOURCE_NAME_MAX,
  SOURCE_ROWS_MAX,
  TEMPLATE_KEY_PATTERN,
  type ApiHeader,
  type ApiMapping,
  type ApiMappingColumn,
  type SourceColumn,
  type SourceDoc,
} from "$lib/types";
import { deriveKey } from "../keys";

export type ApiDoc = Extract<SourceDoc, { kind: "api" }>;

// ---- JSON Pointer (RFC 6901) ------------------------------------------------------------------------

const escape = (t: string) => t.replace(/~/g, "~0").replace(/\//g, "~1");
const unescape = (t: string) => t.replace(/~1/g, "/").replace(/~0/g, "~");

/** "/data/items/0" from ["data", "items", 0]; "" = the root. */
export const pointerOf = (path: readonly (string | number)[]): string =>
  path.map((t) => `/${escape(String(t))}`).join("");

/** The tokens of a pointer, or null when it isn't one ("" → []). */
export function tokensOf(ptr: string): string[] | null {
  if (ptr === "") return [];
  if (!ptr.startsWith("/")) return null;
  return ptr.slice(1).split("/").map(unescape);
}

/** The value at `ptr` (found: false when the pointer leads nowhere). */
export function resolve(json: unknown, ptr: string): { found: boolean; value: unknown } {
  const tokens = tokensOf(ptr);
  if (!tokens) return { found: false, value: undefined };
  let cur = json;
  for (const t of tokens) {
    if (Array.isArray(cur)) {
      if (!/^(0|[1-9]\d*)$/.test(t) || Number(t) >= cur.length) return { found: false, value: undefined };
      cur = cur[Number(t)];
    } else if (cur !== null && typeof cur === "object" && Object.hasOwn(cur, t)) {
      cur = (cur as Record<string, unknown>)[t];
    } else return { found: false, value: undefined };
  }
  return { found: true, value: cur };
}

/** A cell's text: strings as they are, numbers and booleans as written, null empty, objects as JSON. */
export function cellText(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  return JSON.stringify(v);
}

// ---- Mapping ----------------------------------------------------------------------------------------

export interface Mapped {
  rows: Record<string, string>[];
  /** Per column key: rows that had a value there. */
  hits: Record<string, number>;
  error: string | null;
}

/** The rows the mapping makes of a JSON response (as the backend does; at most SOURCE_ROWS_MAX). */
export function mapJson(json: unknown, mapping: Pick<ApiMapping, "rows" | "columns">): Mapped {
  const hits = Object.fromEntries(mapping.columns.map((c) => [c.key, 0]));
  const at = resolve(json, mapping.rows);
  if (!at.found) return { rows: [], hits, error: `Nothing at ${mapping.rows || "the root"}.` };
  const list = Array.isArray(at.value)
    ? at.value
    : at.value !== null && typeof at.value === "object"
      ? [at.value]
      : null;
  if (!list) return { rows: [], hits, error: "The rows must point to a list or an object." };
  const rows = list.slice(0, SOURCE_ROWS_MAX).map((item) => {
    const row: Record<string, string> = {};
    for (const c of mapping.columns) {
      const v = resolve(item, c.path);
      row[c.key] = v.found ? cellText(v.value) : "";
      if (v.found && v.value !== null) hits[c.key]++;
    }
    return row;
  });
  return { rows, hits, error: null };
}

/** Whether the rows pointer names a list (each element a row) rather than one object (one row). */
export function rowsIsList(json: unknown, rowsPtr: string): boolean {
  return Array.isArray(resolve(json, rowsPtr).value);
}

/**
 * The pointer of `ptr` within a row ("/name"; "" = the row itself), or null when `ptr` isn't
 * inside a row.
 */
export function withinRow(ptr: string, rowsPtr: string, list: boolean): string | null {
  if (list) {
    const prefix = `${rowsPtr}/`;
    if (!ptr.startsWith(prefix)) return null;
    const rest = ptr.slice(prefix.length);
    const slash = rest.indexOf("/");
    const index = slash < 0 ? rest : rest.slice(0, slash);
    if (!/^(0|[1-9]\d*)$/.test(index)) return null;
    return slash < 0 ? "" : rest.slice(slash);
  }
  if (ptr === rowsPtr) return "";
  return ptr.startsWith(`${rowsPtr}/`) ? ptr.slice(rowsPtr.length) : null;
}

/** What the mapping does with a tree node: it is the rows, and/or the value of these columns. */
export interface Mark {
  rows: boolean;
  keys: string[];
}

export function markOf(ptr: string, mapping: Pick<ApiMapping, "rows" | "columns">, list: boolean): Mark | null {
  const rows = ptr === mapping.rows;
  const rel = withinRow(ptr, mapping.rows, list);
  const keys = rel === null || (rows && list) ? [] : mapping.columns.filter((c) => c.path === rel).map((c) => c.key);
  return rows || keys.length > 0 ? { rows, keys } : null;
}

const IMAGE_URL = /^(https?:\/\/|\/assets\/)\S+\.(png|jpe?g|gif|webp|svg)(\?\S*)?$/i;

/** A new column for `path` (a pointer within a row, or a CSV header): label from the last token,
 * a unique key, image when the sample looks like an image URL. */
export function columnFor(path: string, taken: readonly ApiMappingColumn[], sample?: unknown, label?: string): ApiMappingColumn {
  const last = label ?? (tokensOf(path)?.at(-1) || "Value");
  const pretty = last.charAt(0).toUpperCase() + last.slice(1);
  return {
    key: deriveKey(last, taken.map((c) => c.key)),
    label: pretty.slice(0, 64),
    kind: typeof sample === "string" && IMAGE_URL.test(sample) ? "image" : "text",
    path,
  };
}

/** The document's table columns follow the mapping's (API and live data sources). */
export function withMapping<D extends { mapping: ApiMapping; columns: SourceColumn[] }>(doc: D, mapping: ApiMapping): D {
  return {
    ...doc,
    mapping,
    columns: mapping.columns.map(({ key, label, kind }) => (kind === "image" ? { key, label, kind } : { key, label })),
  };
}

// ---- Polling ----------------------------------------------------------------------------------------

/** Seconds typed → interval ms, clamped to 1 s – 1 h (`clamped`: which end it was past). */
export function intervalFrom(seconds: number): { ms: number; clamped: false | "min" | "max" } {
  const ms = Math.round(seconds * 1000);
  if (!Number.isFinite(ms) || ms < API_POLL_MIN_MS) return { ms: API_POLL_MIN_MS, clamped: "min" };
  if (ms > API_POLL_MAX_MS) return { ms: API_POLL_MAX_MS, clamped: "max" };
  return { ms, clamped: false };
}

// ---- Checks -----------------------------------------------------------------------------------------

const HEADER_NAME = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;

export interface ApiIssues {
  name: string | null;
  url: string | null;
  /** Per header: what's wrong with its name, or null. */
  headers: (string | null)[];
  /** Per mapping column: what's wrong with it, or null. */
  columns: (string | null)[];
  table: string | null;
}

export function apiIssues(doc: ApiDoc): ApiIssues {
  const trimmed = doc.name.trim();
  const name =
    trimmed === "" ? "Enter a name." : [...trimmed].length > SOURCE_NAME_MAX ? `Up to ${SOURCE_NAME_MAX} characters.` : null;
  const u = doc.request.url.trim();
  const url = u === "" || /^https?:\/\/[^\s/?#]+\S*$/i.test(u) ? null : "Enter an http:// or https:// address.";
  const headers = doc.request.headers.map((h) =>
    h.name.trim() === "" ? "Enter a header name." : HEADER_NAME.test(h.name.trim()) ? null : "Letters, digits and - only.",
  );
  const { columns, table } = columnIssues(doc.mapping.columns, doc.format === "json");
  return { name, url, headers, columns, table };
}

/** Per mapping column: what's wrong with it, or null; `table`: too many columns. */
export function columnIssues(cols: readonly ApiMappingColumn[], json: boolean): { columns: (string | null)[]; table: string | null } {
  const counts = new Map<string, number>();
  for (const c of cols) counts.set(c.key, (counts.get(c.key) ?? 0) + 1);
  const columns = cols.map((c) => {
    if (c.label.trim() === "") return "Enter a label.";
    if (!TEMPLATE_KEY_PATTERN.test(c.key)) return "The key must start with a letter: a–z, 0–9, _ or -.";
    if ((counts.get(c.key) ?? 0) > 1) return "Another column has this key.";
    if (json && c.path !== "" && !c.path.startsWith("/")) return "A path starts with /, like /name.";
    return null;
  });
  const table = cols.length > SOURCE_COLUMNS_MAX ? `Up to ${SOURCE_COLUMNS_MAX} columns.` : null;
  return { columns, table };
}

/** A name's problem, or null. */
export function nameIssue(name: string): string | null {
  const trimmed = name.trim();
  return trimmed === "" ? "Enter a name." : [...trimmed].length > SOURCE_NAME_MAX ? `Up to ${SOURCE_NAME_MAX} characters.` : null;
}

/** What MappingSection needs of the editor's checks. */
export type MappingIssues = Pick<ApiIssues, "url" | "columns" | "table">;

/** A test result MappingSection shows: an API test request or a live data test connection. */
export interface MappingResult {
  json: unknown | null;
  error: string | null;
  httpStatus?: number | null;
  headers?: [string, string][];
  csvColumns?: string[] | null;
}

/** Why the working copy can't be saved, or null. */
export function apiBlocked(doc: ApiDoc, issues: ApiIssues = apiIssues(doc)): string | null {
  if (issues.name) return `Name: ${issues.name}`;
  if (issues.url) return `URL: ${issues.url}`;
  const hi = issues.headers.findIndex((e) => e !== null);
  if (hi >= 0) return `Header ${hi + 1}: ${issues.headers[hi]}`;
  const ci = issues.columns.findIndex((e) => e !== null);
  if (ci >= 0) return `${doc.mapping.columns[ci].label.trim() || `Column ${ci + 1}`}: ${issues.columns[ci]}`;
  return issues.table;
}

// ---- Headers ----------------------------------------------------------------------------------------

/** How a header's secret stands: none (a plain value), stored on this machine, or missing here. */
export function secretState(h: ApiHeader): "none" | "stored" | "missing" {
  if (!h.secret) return "none";
  return h.stored === false ? "missing" : "stored";
}

/** The headers of `from` (a reply to a secret command) for the names it knows, the rest kept. */
export function mergeSecrets(mine: readonly ApiHeader[], from: readonly ApiHeader[]): ApiHeader[] {
  return mine.map((h) => {
    const s = from.find((x) => x.name === h.name);
    return s ? { ...h, value: s.secret ? null : (h.value ?? null), secret: s.secret ?? null, stored: s.stored } : h;
  });
}
