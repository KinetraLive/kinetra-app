<!--
  ListColumns: the columns of a `list` field in the Fields tab (a lazy chunk, loaded only when a
  list field exists): label and mono key per column,
  move up / down, delete, and Add column (1–SOURCE_COLUMNS_MAX). A key follows its label until it
  is edited by hand (`relabelColumn`). Rows bind each column's key as `{{.key}}`.

  Props:
    field: FieldDef                   the list field
    error: string | null              the field's first column issue
    onchange: (patch: { columns, default }) => void   new columns, and the default rows reduced
                                                      to them
-->
<script lang="ts">
  import { tick } from "svelte";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { SOURCE_COLUMNS_MAX, type FieldDef, type SourceColumn } from "$lib/types";
  import { listDefault, newColumn, relabelColumn } from "./columns";
  import { removeAt, replaceAt, swap } from "./model";

  let {
    field: def,
    error,
    onchange: onpatch,
  }: {
    field: FieldDef;
    error: string | null;
    onchange: (patch: { columns: SourceColumn[]; default: string }) => void;
  } = $props();

  const field = $derived(def.label || def.key);
  const columns = $derived(def.columns ?? []);
  const onchange = (next: SourceColumn[]) => onpatch({ columns: next, default: listDefault(def, next) });

  const uid = $props.id();
  let root: HTMLDivElement | undefined = $state();
  const full = $derived(columns.length >= SOURCE_COLUMNS_MAX);

  async function focus(j: number, what: string) {
    await tick();
    const el = root?.querySelector<HTMLInputElement | HTMLButtonElement>(`[data-col="${j}"] [data-act="${what}"]`);
    if (el && !el.disabled) el.focus();
  }

  function add() {
    if (full) return;
    onchange([...columns, newColumn(columns)]);
    void focus(columns.length, "label");
  }

  function move(j: number, dir: 1 | -1, what: string) {
    onchange(swap(columns, j, j + dir));
    void focus(j + dir, what);
  }

  function remove(j: number) {
    const left = columns.length - 1;
    onchange(removeAt(columns, j));
    if (left > 0) void focus(Math.min(j, left - 1), "delete");
  }
</script>

<div class="cols" bind:this={root} role="group" aria-label="Columns of {field}">
  <p class="section-label">Columns</p>
  {#each columns as c, j (j)}
    <div class="col" data-col={j}>
      <span class="cell vf vf-within">
        <input
          type="text"
          autocomplete="off"
          data-act="label"
          aria-label="{field} column {j + 1} label"
          value={c.label}
          oninput={(e) => onchange(relabelColumn(columns, j, e.currentTarget.value))}
        />
      </span>
      <span class="cell vf vf-within">
        <input
          class="mono"
          type="text"
          spellcheck="false"
          autocomplete="off"
          aria-label="{field} column {j + 1} key"
          value={c.key}
          oninput={(e) => onchange(replaceAt(columns, j, { ...c, key: e.currentTarget.value }))}
        />
      </span>
      <span class="acts">
        <IconButton size="sm" label="Move column {j + 1} up" tooltip="none" data-act="up" disabled={j === 0} onclick={() => move(j, -1, "up")}>
          <ArrowUp size={14} />
        </IconButton>
        <IconButton
          size="sm"
          label="Move column {j + 1} down"
          tooltip="none"
          data-act="down"
          disabled={j === columns.length - 1}
          onclick={() => move(j, 1, "down")}
        >
          <ArrowDown size={14} />
        </IconButton>
        <IconButton variant="danger"
          size="sm"
          label="Delete column {j + 1}"
          tooltip="none"
          data-act="delete"
          disabled={columns.length <= 1}
          onclick={() => remove(j)}
        >
          <Trash2 size={14} />
        </IconButton>
      </span>
    </div>
  {/each}
  <div class="foot">
    <Button size="sm" variant="ghost" onclick={add} disabled={full}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Add column
    </Button>
    <span class="muted" aria-label="{columns.length} of {SOURCE_COLUMNS_MAX} columns">{columns.length} / {SOURCE_COLUMNS_MAX}</span>
  </div>
  {#if error}<p class="err" id="{uid}-err">{error}</p>{/if}
</div>

<style>
  .cols {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .col {
    display: grid;
    grid-template-columns: minmax(7rem, 1fr) minmax(6rem, 1fr) auto;
    gap: var(--s-2);
    align-items: center;
  }
  .cell {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .cell input {
    width: 100%;
    min-width: 0;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .acts {
    display: inline-flex;
    gap: 2px;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
