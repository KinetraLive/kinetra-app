<!--
  MappingSection: the API editor's Mapping. "Test request" fetches once with the working copy
  (testApiSource; nothing saved) and shows the HTTP status, any error, the response headers and:
    JSON  the response as a tree (JsonTree). Pick a node and "Use as rows" (a list: each element a
          row; an object: one row) or, inside a row, "Add column" / "Set path of" a column; the
          JSON Pointers fill in (rows absolute, column paths relative to a row) and can be typed.
          Enter or double-click does the likely one. The rows node and mapped values are
          highlighted with their column keys.
    CSV   the header names (or column numbers) as buttons that add a column; mapped ones marked.
  Below: the columns (label, key, Text/Image, path, rows with a value), editable, up to 16.

  Props:
    doc: ApiDoc
    issues: ApiIssues
    result: ApiTestResult | null      the last test (null before one)
    testing: boolean
    hits: Record<string, number> | null   per column key, for the rows shown in Preview
    rowCount: number
    link?: boolean                     a CSV link in Simple mode: "Test link", wording about the link
    ontest: () => void
    onmapping: (mapping) => void       a mapping change (the document's columns follow)
-->
<script lang="ts">
  import FlaskConical from "lucide-svelte/icons/flask-conical";
  import Plus from "lucide-svelte/icons/plus";
  import X from "lucide-svelte/icons/x";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Select from "$lib/components/Select.svelte";
  import { SOURCE_COLUMNS_MAX, type ApiMapping, type ApiMappingColumn, type ApiTestResult } from "$lib/types";
  import { deriveKey } from "../keys";
  import JsonTree from "./JsonTree.svelte";
  import { columnFor, markOf, resolve, rowsIsList, withinRow, type ApiDoc, type ApiIssues } from "./model";

  let {
    doc,
    issues,
    result,
    testing,
    hits,
    rowCount,
    link = false,
    ontest,
    onmapping,
  }: {
    doc: ApiDoc;
    issues: ApiIssues;
    result: ApiTestResult | null;
    testing: boolean;
    hits: Record<string, number> | null;
    rowCount: number;
    link?: boolean;
    ontest: () => void;
    onmapping: (mapping: ApiMapping) => void;
  } = $props();

  const uid = $props.id();
  const KINDS: { value: "text" | "image"; label: string }[] = [
    { value: "text", label: "Text" },
    { value: "image", label: "Image" },
  ];

  const mapping = $derived(doc.mapping);
  const cols = $derived(mapping.columns);
  const json = $derived(result?.json ?? null);
  const isJson = $derived(doc.format === "json" && result !== null && result.json !== null && result.json !== undefined);
  const list = $derived(isJson ? rowsIsList(json, mapping.rows) : true);
  const full = $derived(cols.length >= SOURCE_COLUMNS_MAX);

  let selected = $state<string | null>(null);
  /** Expanded to after a test and when the rows change. */
  let reveal = $state<string | null>(null);
  let note = $state<string | null>(null);

  $effect(() => {
    if (result) reveal = mapping.rows;
  });

  const sel = $derived(selected !== null && isJson && resolve(json, selected).found ? selected : null);
  const selRel = $derived(sel === null ? null : withinRow(sel, mapping.rows, list));
  const selIsContainer = $derived.by(() => {
    if (sel === null) return false;
    const v = resolve(json, sel).value;
    return v !== null && typeof v === "object";
  });

  function setColumns(next: ApiMappingColumn[]) {
    onmapping({ ...mapping, columns: next });
  }

  function patchColumn(i: number, patch: Partial<ApiMappingColumn>) {
    setColumns(cols.map((c, j) => (j === i ? { ...c, ...patch } : c)));
  }

  function useAsRows(ptr: string) {
    note = null;
    reveal = ptr;
    let next = cols;
    // No columns yet: one per plain value of the first row (up to 8).
    if (next.length === 0) {
      const at = resolve(json, ptr).value;
      const first = Array.isArray(at) ? at[0] : at;
      if (first !== null && typeof first === "object" && !Array.isArray(first)) {
        next = [];
        for (const [k, v] of Object.entries(first)) {
          if (next.length >= 8) break;
          if (v !== null && typeof v === "object") continue;
          next.push(columnFor(`/${k.replace(/~/g, "~0").replace(/\//g, "~1")}`, next, v, k));
        }
      }
    }
    onmapping({ ...mapping, rows: ptr, columns: next });
  }

  function addColumn(rel: string, sample: unknown, label?: string) {
    note = null;
    if (full) {
      note = `Up to ${SOURCE_COLUMNS_MAX} columns.`;
      return;
    }
    setColumns([...cols, columnFor(rel, cols, sample, label)]);
  }

  function addFromTree(ptr: string) {
    const rel = withinRow(ptr, mapping.rows, list);
    if (rel === null) {
      note = "Pick a value inside a row: under the rows list.";
      return;
    }
    addColumn(rel, resolve(json, ptr).value);
  }

  /** Enter / double-click: a list (or an object outside the rows) becomes the rows, a row's value a
   *  column. */
  function activate(ptr: string) {
    const rel = withinRow(ptr, mapping.rows, list);
    const v = resolve(json, ptr).value;
    const container = v !== null && typeof v === "object";
    if (ptr !== mapping.rows && (Array.isArray(v) || (container && rel === null))) useAsRows(ptr);
    else if (rel !== null && rel !== "" && !container && !cols.some((c) => c.path === rel)) addColumn(rel, v);
  }

  let pathTarget = $state("0");
  function setPathOf() {
    const i = Number(pathTarget);
    if (selRel === null || !cols[i]) return;
    patchColumn(i, { path: selRel });
  }

  function newColumn() {
    if (full) return;
    const label = `Column ${cols.length + 1}`;
    setColumns([...cols, { key: deriveKey(label, cols.map((c) => c.key)), label, kind: "text", path: doc.format === "json" ? "/" : "" }]);
  }

  function onLabel(i: number, label: string) {
    const c = cols[i];
    const others = cols.filter((_, j) => j !== i).map((x) => x.key);
    // The key follows the label until it is typed differently.
    const key = c.key === deriveKey(c.label, others) ? deriveKey(label, others) : c.key;
    patchColumn(i, { label, key });
  }

  const csvMapped = (name: string) => cols.filter((c) => c.path === name).map((c) => c.key);
  const hitText = (key: string) => (hits && key in hits ? `${hits[key]} / ${rowCount}` : "");
</script>

<section class="section" aria-labelledby="{uid}-title">
  <div class="head">
    <h3 class="section-label" id="{uid}-title">Mapping</h3>
    <Button size="sm" variant={result ? "secondary" : "primary"} loading={testing} disabled={!doc.request.url.trim() || !!issues.url} onclick={ontest}>
      {#snippet icon()}<FlaskConical size={14} />{/snippet}
      {link ? "Test link" : "Test request"}
    </Button>
    {#if result && !testing}
      <span class="result small" role="status">
        {#if result.httpStatus !== null}<span class="mono">HTTP {result.httpStatus}</span>{/if}
        {#if result.error}<span class="bad">{result.error}</span>{:else}<span class="muted">{rowCount} {rowCount === 1 ? "row" : "rows"}</span>{/if}
      </span>
    {/if}
  </div>

  {#if !result}
    <p class="small muted">
      {#if link}
        {doc.request.url.trim() ? "Test the link, then pick the columns to use." : "Paste the link, then test it."}
      {:else}
        {doc.request.url.trim()
          ? "Test the request, then pick rows and columns from the response."
          : "Enter the URL, then test the request."}
      {/if}
    </p>
  {:else}
    {#if result.headers.length > 0}
      <details class="resp">
        <summary class="small muted">Response headers ({result.headers.length})</summary>
        <dl class="mono small">
          {#each result.headers as [k, v], i (i)}<dt>{k}</dt><dd>{v}</dd>{/each}
        </dl>
      </details>
    {/if}

    {#if doc.format === "json"}
      <div class="rows-row">
        <label class="small muted" for="{uid}-rows">Rows</label>
        <span class="input grow vf vf-within">
          <input
            id="{uid}-rows"
            class="mono"
            type="text"
            autocomplete="off"
            spellcheck="false"
            value={mapping.rows}
            oninput={(e) => onmapping({ ...mapping, rows: e.currentTarget.value })}
          />
        </span>
      </div>
      {#if isJson}
        <div class="pick-bar" role="toolbar" aria-label="Selected node">
          {#if sel !== null}
            <span class="mono small sel-ptr" title={sel}>{sel || "/ (root)"}</span>
            {#if selIsContainer}
              <Button size="sm" variant="secondary" disabled={sel === mapping.rows} onclick={() => sel !== null && useAsRows(sel)}>Use as rows</Button>
            {/if}
            {#if selRel !== null}
              <Button size="sm" variant="secondary" disabled={full} onclick={() => sel !== null && addFromTree(sel)}>
                {#snippet icon()}<Plus size={14} />{/snippet}
                Add as column
              </Button>
              {#if cols.length > 0}
                <span class="set-path">
                  <Button size="sm" variant="ghost" onclick={setPathOf}>Set path of</Button>
                  <Select
                    size="sm"
                    aria-label="Column to set"
                    value={pathTarget}
                    options={cols.map((c, i) => ({ value: String(i), label: c.label || c.key }))}
                    onchange={(v) => (pathTarget = v)}
                  />
                </span>
              {/if}
            {/if}
          {:else}
            <span class="small muted">Pick a list for the rows, then values in a row as columns. Double-click picks the likely one.</span>
          {/if}
        </div>
        {#if note}<p class="warn small" role="status">{note}</p>{/if}
        <div class="tree-box">
          <JsonTree
            json={result.json}
            label="Response"
            bind:selected
            {reveal}
            mark={(p) => markOf(p, mapping, list)}
            onactivate={activate}
          />
        </div>
      {:else if !result.error}
        <p class="small muted">The response is too large to show as a tree; type the pointers.</p>
      {/if}
    {:else if result.csvColumns}
      <div class="csv" role="group" aria-label="CSV columns">
        {#each result.csvColumns as name, i (i)}
          {@const keys = csvMapped(name)}
          <button
            type="button"
            class="chip vf"
            class:mapped={keys.length > 0}
            disabled={full && keys.length === 0}
            aria-label="Add column {doc.mapping.csv.header ? name : `${i + 1}`}"
            onclick={() => addColumn(name, undefined, doc.mapping.csv.header ? name : `Column ${i + 1}`)}
          >
            <span class="mono">{doc.mapping.csv.header ? name : `#${i + 1}`}</span>
            {#each keys as k (k)}<span class="tag mono">{k}</span>{/each}
          </button>
        {/each}
      </div>
      {#if note}<p class="warn small" role="status">{note}</p>{/if}
    {/if}
  {/if}

  <div class="cols" role="group" aria-labelledby="{uid}-cols">
    <div class="sub-head">
      <span class="small muted" id="{uid}-cols">Columns {cols.length} / {SOURCE_COLUMNS_MAX}</span>
      <Button size="sm" variant="ghost" disabled={full} onclick={newColumn}>
        {#snippet icon()}<Plus size={14} />{/snippet}
        Add column
      </Button>
    </div>
    {#if cols.length > 0}
      <div class="col-grid">
        <span class="small muted">Label</span>
        <span class="small muted">Key</span>
        <span class="small muted">Kind</span>
        <span class="small muted">{doc.format === "json" ? "Path in a row" : "CSV column"}</span>
        <span class="small muted" title="Rows with a value">Values</span>
        <span></span>
        {#each cols as c, i (i)}
          <span class="input vf vf-within">
            <input
              type="text"
              aria-label="Column {i + 1} label"
              autocomplete="off"
              value={c.label}
              aria-invalid={issues.columns[i] ? "true" : undefined}
              oninput={(e) => onLabel(i, e.currentTarget.value)}
            />
          </span>
          <span class="input vf vf-within">
            <input
              type="text"
              class="mono"
              aria-label="Column {i + 1} key"
              autocomplete="off"
              spellcheck="false"
              value={c.key}
              oninput={(e) => patchColumn(i, { key: e.currentTarget.value })}
            />
          </span>
          <Select size="sm" aria-label="Column {i + 1} kind" value={c.kind} options={KINDS} onchange={(v) => patchColumn(i, { kind: v })} />
          <span class="input vf vf-within">
            <input
              type="text"
              class="mono"
              aria-label="Column {i + 1} path"
              autocomplete="off"
              spellcheck="false"
              value={c.path}
              oninput={(e) => patchColumn(i, { path: e.currentTarget.value })}
            />
          </span>
          <span class="small muted hits" data-hits={c.key}>{hitText(c.key)}</span>
          <IconButton variant="danger" size="sm" label="Remove column {c.label || i + 1}" tooltip="left" onclick={() => setColumns(cols.filter((_, j) => j !== i))}>
            <X size={14} />
          </IconButton>
          {#if issues.columns[i]}<p class="bad small full-row">{issues.columns[i]}</p>{/if}
        {/each}
      </div>
    {/if}
    {#if issues.table}<p class="bad small">{issues.table}</p>{/if}
  </div>
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  .head,
  .rows-row,
  .pick-bar,
  .sub-head,
  .set-path {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .head .section-label {
    margin-right: auto;
  }
  .sub-head {
    justify-content: space-between;
  }
  .pick-bar {
    min-height: calc(var(--control-h-sm) + var(--s-2));
    padding: var(--s-1) var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
  }
  .sel-ptr {
    max-width: 22rem;
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .set-path {
    gap: var(--s-1);
  }
  .result {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .tree-box {
    max-height: 26rem;
    overflow: auto;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .resp dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 2px var(--s-3);
    margin: var(--s-2) 0 0;
    color: var(--text-muted);
  }
  .resp dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .resp summary {
    cursor: pointer;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    min-width: 0;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .rows-row .input {
    height: var(--control-h);
  }
  .input input {
    flex: 1;
    width: 100%;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    font-size: var(--fs-sm);
  }
  .input:has([aria-invalid="true"]) {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .csv {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1);
  }
  .chip {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .chip:hover {
    background: var(--surface-2);
  }
  .chip.mapped {
    background: var(--accent-soft);
  }
  .tag {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    background: var(--accent);
    color: var(--accent-ink);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .cols {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .col-grid {
    display: grid;
    grid-template-columns: minmax(6rem, 1.2fr) minmax(5rem, 1fr) auto minmax(6rem, 1.4fr) 4.5rem auto;
    align-items: center;
    gap: var(--s-1) var(--s-2);
  }
  .full-row {
    grid-column: 1 / -1;
    margin: 0;
  }
  .hits {
    text-align: right;
  }
  .small {
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
</style>
