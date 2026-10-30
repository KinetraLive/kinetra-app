<!--
  PasteDialog: the column-mapping preview for a paste into the sheet (lazy chunk of SheetEditor;
  native modal <dialog>: focus trap, Esc, inert background).

    - "First line names the columns" (detected: the first line names existing columns)
    - per pasted column: its name and a target, an existing column, Ignore, or New column
      (editable columns only)
    - Append to the end, or Replace from row N down (N = the active row; rows past the end are added)
    - the first 5 rows as a preview, and what Apply does against the row limit; a paste past a
      limit can't be applied and says why

  Props:
    plan: PastePlan                    from planPaste
    rows, columns                      the sheet as it is
    limits: SheetLimits
    at: number                         the active row (Replace starts there)
    editableColumns?: boolean
    onapply: (next, order, added, replaced) => void
    oncancel: () => void
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import type { SourceColumn } from "$lib/types";
  import { IGNORE, NEW_COLUMN, applyPaste, bodyLines, type PastePlan } from "./paste";
  import type { Table } from "./rows";
  import type { SheetLimits, SheetRow } from "./types";

  let {
    plan,
    rows,
    columns,
    limits,
    at,
    editableColumns = false,
    onapply,
    oncancel,
  }: {
    plan: PastePlan;
    rows: readonly SheetRow[];
    columns: readonly SourceColumn[];
    limits: SheetLimits;
    at: number;
    editableColumns?: boolean;
    onapply: (next: Table, order: number[], added: number, replaced: number) => void;
    oncancel: () => void;
  } = $props();

  const uid = $props.id();
  const PREVIEW = 5;

  let dialog: HTMLDialogElement | undefined = $state();
  // svelte-ignore state_referenced_locally
  let header = $state(plan.header);
  // svelte-ignore state_referenced_locally
  let map = $state<number[]>([...plan.map]);
  let mode = $state<"append" | "replace">("append");

  const names = $derived(
    Array.from({ length: plan.width }, (_, i) => (header ? (plan.lines[0][i] ?? "").trim() : "") || `Column ${i + 1}`),
  );
  const options = $derived([
    { value: String(IGNORE), label: "Ignore" },
    ...columns.map((c, i) => ({ value: String(i), label: c.label || c.key })),
    ...(editableColumns ? [{ value: String(NEW_COLUMN), label: "New column" }] : []),
  ]);
  const body = $derived(bodyLines(plan, header));
  const preview = $derived(body.slice(0, PREVIEW));
  const outcome = $derived(applyPaste(rows, columns, plan, { header, map, mode, at }, limits));
  const rowsText = (k: number) => `${k} ${k === 1 ? "row" : "rows"}`;
  const targetName = (t: number, i: number) =>
    t === NEW_COLUMN ? `${names[i]} (new)` : t >= 0 ? (columns[t]?.label ?? "") : "Ignored";

  $effect(() => {
    const d = dialog;
    if (!d || d.open) return;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    d.querySelector<HTMLElement>("[data-apply]")?.focus();
  });

  function close() {
    if (dialog?.open && typeof dialog.close === "function") dialog.close();
    oncancel();
  }

  function apply() {
    const o = outcome;
    if (!o.ok) return;
    if (dialog?.open && typeof dialog.close === "function") dialog.close();
    onapply({ rows: o.rows, columns: o.columns }, o.order, o.added, o.replaced);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  oncancel={(e) => {
    e.preventDefault();
    close();
  }}
  onclick={(e) => e.target === dialog && close()}
>
  <form
    class="inner"
    onsubmit={(e) => {
      e.preventDefault();
      apply();
    }}
  >
    <h2 class="title" id="{uid}-title">Paste {body.length} {body.length === 1 ? "row" : "rows"}</h2>

    <label class="check">
      <input type="checkbox" bind:checked={header} />
      First line names the columns
    </label>

    <fieldset class="map">
      <legend class="section-label">Columns</legend>
      {#each names as name, i (i)}
        <div class="pair">
          <span class="from" title={name}>{name}</span>
          <span class="arrow" aria-hidden="true">→</span>
          <Select
            size="sm"
            class="grow"
            aria-label="Target for pasted column {i + 1}, {name}"
            value={String(map[i] ?? IGNORE)}
            {options}
            onchange={(v: string) => (map[i] = Number(v))}
          />
        </div>
      {/each}
    </fieldset>

    <fieldset class="modes">
      <legend class="section-label">Rows</legend>
      <label class="check">
        <input type="radio" name="{uid}-mode" value="append" bind:group={mode} />
        Append to the end
      </label>
      <label class="check">
        <input type="radio" name="{uid}-mode" value="replace" bind:group={mode} disabled={rows.length === 0} />
        Replace from row {at + 1} down
      </label>
    </fieldset>

    <div class="preview" role="table" aria-label="Preview, first {Math.min(PREVIEW, body.length)} rows" style:--n={plan.width}>
      <div class="prow phead" role="row">
        {#each names as _, i (i)}
          <span role="columnheader" class:ignored={(map[i] ?? IGNORE) === IGNORE}>{targetName(map[i] ?? IGNORE, i)}</span>
        {/each}
      </div>
      {#each preview as line, r (r)}
        <div class="prow" role="row">
          {#each names as _, i (i)}
            <span role="cell" class:ignored={(map[i] ?? IGNORE) === IGNORE}>{line[i] ?? ""}</span>
          {/each}
        </div>
      {/each}
    </div>
    {#if body.length > PREVIEW}<p class="muted small">and {body.length - PREVIEW} more</p>{/if}

    <p class="outcome small" class:bad={!outcome.ok} role="status">
      {#if outcome.ok}
        {[outcome.added > 0 ? `Adds ${rowsText(outcome.added)}` : "", outcome.replaced > 0 ? `replaces ${rowsText(outcome.replaced)}` : ""]
          .filter(Boolean)
          .join(", ")}:
        {outcome.rows.length} / {limits.rows} rows.
      {:else}
        {outcome.reason}
      {/if}
    </p>

    <div class="actions">
      <Button variant="ghost" onclick={close}>Cancel</Button>
      <Button variant="primary" type="submit" data-apply disabled={!outcome.ok}>Apply</Button>
    </div>
  </form>
</dialog>

<style>
  .dialog {
    width: min(720px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: auto;
  }
  .dialog::backdrop {
    background: var(--overlay);
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-5);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  fieldset {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }
  legend {
    margin-bottom: var(--s-1);
  }
  .map {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
    gap: var(--s-1) var(--s-4);
  }
  .map legend {
    grid-column: 1 / -1;
  }
  .pair {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .from {
    flex: 0 1 8rem;
    min-width: 0;
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .arrow {
    color: var(--text-muted);
  }
  .pair :global(.grow) {
    flex: 1;
    min-width: 0;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .check input {
    accent-color: var(--accent);
  }
  .preview {
    overflow: auto;
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line);
    font-size: var(--fs-sm);
  }
  .prow {
    display: grid;
    grid-template-columns: repeat(var(--n), minmax(7rem, 1fr));
    min-width: max-content;
    border-bottom: 1px solid var(--line);
  }
  .prow:last-child {
    border-bottom: 0;
  }
  .prow span {
    min-width: 0;
    padding: var(--s-1) var(--s-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .prow span + span {
    border-left: 1px solid var(--line);
  }
  .phead {
    background: var(--surface-1);
    font-weight: 600;
  }
  .ignored {
    color: var(--text-muted);
    text-decoration: line-through;
  }
  .phead .ignored {
    text-decoration: none;
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
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-2);
    /* Stays in view when a long dialog scrolls. */
    position: sticky;
    bottom: 0;
    margin-bottom: calc(-1 * var(--s-5));
    padding-bottom: var(--s-5);
    background: var(--surface-2);
  }
</style>
