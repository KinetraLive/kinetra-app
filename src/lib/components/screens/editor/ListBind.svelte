<!--
  ListBind: "Link to data table…" for a `list` field (lazy chunk). A linked list shows every row
  of a data table (a schedule: every item, with its status), columns matched by key; several
  layers can share one table. A small popover (non-modal dialog) picks the table (data tables
  first, then schedules) and the options: sort by a column (text, or as numbers), descending, and
  a row limit (1–LIST_ROWS_MAX). Opened on a linked field it edits that link's options. Unlink
  (in the Inspector) keeps a copy of the rows as the layer's own value.

  Keyboard: the button opens it with focus on the source select; Escape closes and returns focus
  to the button; a press outside closes it (the window listener exists only while open).

  Props:
    field: string                       the field's label (accessible names)
    columns: SourceColumn[]             the field's columns (to show how many the source matches)
    sources: SourceSummary[] | null     null while loading
    current?: SourceBinding | null      the binding being edited (bound fields)
    onopen: () => void                  opened (the editor loads the sources lazily)
    onpick: (binding: SourceBinding) => void
-->
<script lang="ts">
  import { tick } from "svelte";
  import Link from "lucide-svelte/icons/link";
  import Settings2 from "lucide-svelte/icons/settings-2";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { bindableColumns } from "$lib/components/fields/binding-label";
  import { LIST_ROWS_MAX, type SourceBinding, type SourceColumn, type SourceSummary } from "$lib/types";

  let {
    field,
    columns,
    sources,
    current = null,
    onopen,
    onpick,
  }: {
    field: string;
    columns: readonly SourceColumn[];
    sources: readonly SourceSummary[] | null;
    current?: SourceBinding | null;
    onopen: () => void;
    onpick: (binding: SourceBinding) => void;
  } = $props();

  const uid = $props.id();
  let open = $state(false);
  let root: HTMLDivElement | undefined = $state();
  let panel: HTMLDivElement | undefined = $state();

  // The form, reset from `current` on every open.
  let sourceId = $state("");
  let sortColumn = $state("");
  let numeric = $state(false);
  let desc = $state(false);
  let limited = $state(false);
  let limit = $state(10);

  const chosen = $derived(sources?.find((s) => s.id === sourceId) ?? null);
  /** The chosen source's columns (a schedule's fixed ones, without the countdown). */
  const sourceColumns = $derived(bindableColumns(chosen).filter((c) => c.key !== "countdown"));
  /** Data tables first, then schedules (marked). */
  const ordered = $derived([
    ...(sources ?? []).filter((s) => s.kind === "text"),
    ...(sources ?? []).filter((s) => s.kind !== "text"),
  ]);
  const sourceOptions = $derived([
    ...(sourceId === "" ? [{ value: "", label: "Choose a data table", disabled: true }] : []),
    ...ordered.map((s) => ({ value: s.id, label: s.kind === "text" ? s.name : `${s.name} · schedule` })),
  ]);
  const sortOptions = $derived([
    { value: "", label: "Table order" },
    ...sourceColumns.map((c) => ({ value: c.key, label: c.label })),
  ]);
  const matched = $derived(chosen ? columns.filter((c) => sourceColumns.some((x) => x.key === c.key)).length : 0);
  const schedule = $derived(chosen?.kind === "schedule");

  function reset() {
    sourceId = current?.sourceId ?? ordered[0]?.id ?? "";
    sortColumn = current?.sort?.column ?? "";
    numeric = current?.sort?.numeric ?? false;
    desc = current?.sort?.desc ?? false;
    limited = current?.limit !== undefined;
    limit = current?.limit ?? 10;
  }

  async function show() {
    reset();
    open = true;
    onopen();
    await tick();
    panel?.querySelector<HTMLElement>("select, button")?.focus();
  }

  function close(refocus: boolean) {
    open = false;
    if (refocus) root?.querySelector<HTMLButtonElement>("button")?.focus();
  }

  function submit() {
    if (!chosen) return;
    const b: SourceBinding = { sourceId: chosen.id, column: "" };
    if (sortColumn !== "") b.sort = { column: sortColumn, numeric, desc };
    if (limited) b.limit = limit;
    close(true);
    onpick(b);
  }

  // Sources arrive after opening (first open): pick the first one.
  $effect(() => {
    if (open && sources && sourceId === "" && ordered.length > 0) sourceId = ordered[0].id;
  });

  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) close(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close(true);
    }
  }
</script>

<div class="bind" bind:this={root}>
  <IconButton
    size="sm"
    label={current ? `Link options of ${field}` : `Link ${field} to a data table…`}
    tooltip="left"
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-controls={open ? `${uid}-panel` : undefined}
    onclick={() => (open ? close(false) : void show())}
  >
    {#if current}<Settings2 size={14} />{:else}<Link size={14} />{/if}
  </IconButton>

  {#if open}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="panel"
      id="{uid}-panel"
      role="dialog"
      aria-label="Link {field} to a data table"
      tabindex="-1"
      bind:this={panel}
      onkeydown={onKey}
    >
      {#if sources === null}
        <p class="note" role="status">Loading sources…</p>
      {:else if sources.length === 0}
        <p class="note">This project has no data tables yet. Create one on the <a href="/sources">Sources</a> page.</p>
      {:else}
        <form
          class="form"
          onsubmit={(e) => {
            e.preventDefault();
            submit();
          }}
        >
          <p class="note">
            {schedule
              ? "Shows every schedule item with its status."
              : "Shows every row of the table."}
          </p>
          <label class="row">
            <span>Table</span>
            <Select
              size="sm"
              class="grow"
              value={sourceId}
              options={sourceOptions}
              onchange={(v: string) => {
                sourceId = v;
                if (!sourceColumns.some((c) => c.key === sortColumn)) sortColumn = "";
              }}
            />
          </label>
          {#if chosen}
            <p class="meta" aria-label="{matched} of {columns.length} columns match">
              {chosen.rowCount} {schedule ? (chosen.rowCount === 1 ? "item" : "items") : chosen.rowCount === 1 ? "row" : "rows"} · {matched}/{columns.length} columns match
            </p>
          {/if}
          <label class="row">
            <span>Sort by</span>
            <Select size="sm" class="grow" value={sortColumn} options={sortOptions} onchange={(v: string) => (sortColumn = v)} />
          </label>
          {#if sortColumn !== ""}
            <Switch label="Compare as numbers" checked={numeric} onchange={(on) => (numeric = on)} />
            <Switch label="Descending" checked={desc} onchange={(on) => (desc = on)} />
          {/if}
          <div class="row">
            <Switch label="Limit rows" checked={limited} onchange={(on) => (limited = on)} />
            <NumberInput
              aria-label="Most rows"
              value={limit}
              min={1}
              max={LIST_ROWS_MAX}
              digits={3}
              disabled={!limited}
              onchange={(n) => (limit = n)}
            />
          </div>
          <div class="acts">
            <Button size="sm" variant="ghost" onclick={() => close(true)}>Cancel</Button>
            <Button size="sm" variant="primary" type="submit" disabled={!chosen}>{current ? "Apply" : "Link"}</Button>
          </div>
        </form>
      {/if}
    </div>
  {/if}
</div>

<style>
  .bind {
    position: relative;
    flex: none;
  }
  .panel {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    right: 0;
    width: 18rem;
    padding: var(--s-3);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }
  .row > span {
    flex: none;
    width: 4.5rem;
    color: var(--text-muted);
  }
  .row :global(.grow) {
    flex: 1;
    min-width: 0;
  }
  .meta {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .acts {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-2);
    margin-top: var(--s-1);
  }
  .note {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .note a {
    color: var(--accent-text);
  }
</style>
