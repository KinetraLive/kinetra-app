<!--
  Fields tab: the template's fields as a form (key, label, type, default), with add, delete and
  move up/down. Validation messages come from the parent (`issues`, see model.ts).

  Type-specific settings: a list field shows its row count in the Default column and its columns
  under the row (ListColumns); a clock field's Default column is its format (mono, HH mm ss);
  a timer field shows its start in the Default column and direction / start / at zero under the
  row (TimerDefault). Changing the type resets these (`withFieldType`).

  Props:
    fields: FieldDef[]               current list (replaced wholesale by `onchange`)
    otherKeys: string[]              variable keys (new field keys avoid them)
    issues: Issue[]                  validation issues for the whole doc
    onchange: (next: FieldDef[]) => void
-->
<script module lang="ts">
  // List columns and timer defaults are their own chunks, loaded only when such a field exists.
  let columnsChunk: Promise<typeof import("./ListColumns.svelte")> | null = null;
  let timerChunk: Promise<typeof import("./TimerDefault.svelte")> | null = null;
  const loadColumns = () => (columnsChunk ??= import("./ListColumns.svelte"));
  const loadTimer = () => (timerChunk ??= import("./TimerDefault.svelte"));

  /** "↑ 00:00" for a timer default (direction and start); "" when it doesn't parse. */
  function timerSummary(v: string): string {
    try {
      const t = JSON.parse(v) as { base?: unknown; dir?: unknown };
      const s = Math.floor((typeof t.base === "number" ? t.base : 0) / 1000);
      const p = (n: number) => String(n).padStart(2, "0");
      const hms = s >= 3600 ? `${Math.floor(s / 3600)}:${p(Math.floor(s / 60) % 60)}:${p(s % 60)}` : `${p(Math.floor(s / 60))}:${p(s % 60)}`;
      return `${t.dir === "down" ? "↓" : "↑"} ${hms}`;
    } catch {
      return "";
    }
  }
</script>

<script lang="ts">
  import { tick } from "svelte";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Select from "$lib/components/Select.svelte";
  import { TEMPLATE_FIELDS_MAX, type FieldDef, type FieldType } from "$lib/types";
  import { withFieldType } from "./fieldTypes";
  import { FIELD_TYPES, issueFor, issueWith, newField, removeAt, replaceAt, swap, type Issue } from "./model";

  let {
    fields,
    otherKeys,
    issues,
    onchange,
  }: {
    fields: readonly FieldDef[];
    otherKeys: readonly string[];
    issues: readonly Issue[];
    onchange: (next: FieldDef[]) => void;
  } = $props();

  const uid = $props.id();
  let list: HTMLDivElement | undefined = $state();
  const full = $derived(fields.length >= TEMPLATE_FIELDS_MAX);

  function patch(i: number, p: Partial<FieldDef>) {
    onchange(replaceAt(fields, i, { ...fields[i], ...p }));
  }

  async function add() {
    if (full) return;
    onchange([...fields, newField([...fields.map((f) => f.key), ...otherKeys])]);
    await tick();
    list?.querySelector<HTMLInputElement>(`[data-row="${fields.length - 1}"] input`)?.focus();
  }

  async function move(i: number, dir: -1 | 1, what: string) {
    onchange(swap(fields, i, i + dir));
    await tick();
    const target = list?.querySelector<HTMLButtonElement>(`[data-row="${i + dir}"] [data-act="${what}"]`);
    if (target && !target.disabled) target.focus();
  }

  async function remove(i: number) {
    onchange(removeAt(fields, i));
    await tick();
    const rows = list?.querySelectorAll<HTMLElement>("[data-row]");
    if (rows && rows.length > 0) rows[Math.min(i, rows.length - 1)].querySelector<HTMLButtonElement>("[data-act=delete]")?.focus();
  }
</script>

<div class="fields">
  {#if fields.length > 0}
    <div class="head" aria-hidden="true">
      <span>Key</span><span>Label</span><span>Type</span><span>Default</span><span></span>
    </div>
  {/if}
  <div class="rows" bind:this={list}>
    {#each fields as f, i}
      {@const keyErr = issueFor(issues, "field", i, "key")}
      {@const labelErr = issueFor(issues, "field", i, "label")}
      {@const formatErr = f.type === "clock" ? issueFor(issues, "field", i, "format") : null}
      <div class="row" data-row={i}>
        <span class="cell vf vf-within">
          <input
            class="mono"
            type="text"
            aria-label="Field {i + 1} key"
            spellcheck="false"
            autocomplete="off"
            value={f.key}
            aria-invalid={keyErr ? "true" : undefined}
            aria-describedby={keyErr ? `${uid}-k${i}` : undefined}
            oninput={(e) => patch(i, { key: e.currentTarget.value })}
          />
        </span>
        <span class="cell vf vf-within">
          <input
            type="text"
            aria-label="Field {i + 1} label"
            autocomplete="off"
            value={f.label}
            aria-invalid={labelErr ? "true" : undefined}
            aria-describedby={labelErr ? `${uid}-l${i}` : undefined}
            oninput={(e) => patch(i, { label: e.currentTarget.value })}
          />
        </span>
        <Select
          class="type"
          aria-label="Field {i + 1} type"
          value={f.type}
          options={FIELD_TYPES}
          onchange={(type: FieldType) => onchange(replaceAt(fields, i, withFieldType(f, type)))}
        />
        {#if f.type === "list"}
          {@const n = f.columns?.length ?? 0}
          <span class="summary">{n} {n === 1 ? "column" : "columns"}</span>
        {:else if f.type === "timer"}
          <span class="summary mono">{timerSummary(f.default)}</span>
        {:else if f.type === "clock"}
          <span class="cell vf vf-within">
            <input
              type="text"
              class="mono"
              spellcheck="false"
              aria-label="Field {i + 1} clock format"
              autocomplete="off"
              maxlength="40"
              value={f.format ?? ""}
              aria-invalid={formatErr ? "true" : undefined}
              aria-describedby={formatErr ? `${uid}-c${i}` : undefined}
              oninput={(e) => patch(i, { format: e.currentTarget.value, default: e.currentTarget.value })}
            />
          </span>
        {:else}
          <span class="cell vf vf-within">
            <input
              type="text"
              class:mono={f.type !== "text" && f.type !== "multiline"}
              aria-label="Field {i + 1} default"
              autocomplete="off"
              value={f.default}
              oninput={(e) => patch(i, { default: e.currentTarget.value })}
            />
          </span>
        {/if}
        <span class="acts">
          <IconButton
            size="sm"
            label="Move up"
            tooltip="top"
            data-act="up"
            disabled={i === 0}
            onclick={() => move(i, -1, "up")}><ArrowUp size={14} /></IconButton
          >
          <IconButton
            size="sm"
            label="Move down"
            tooltip="top"
            data-act="down"
            disabled={i === fields.length - 1}
            onclick={() => move(i, 1, "down")}><ArrowDown size={14} /></IconButton
          >
          <IconButton variant="danger" size="sm" label="Delete field" tooltip="top" data-act="delete" onclick={() => remove(i)}>
            <Trash2 size={14} />
          </IconButton>
        </span>
        {#if keyErr}<p class="err" id="{uid}-k{i}">{keyErr}</p>{/if}
        {#if labelErr}<p class="err" id="{uid}-l{i}">{labelErr}</p>{/if}
        {#if formatErr}<p class="err" id="{uid}-c{i}">{formatErr}</p>{/if}
        {#if f.type === "list"}
          <div class="detail">
            {#await loadColumns() then { default: ListColumns }}
              <ListColumns field={f} error={issueWith(issues, i, "column")} onchange={(p) => patch(i, p)} />
            {/await}
          </div>
        {:else if f.type === "timer"}
          {@const timerErr = issueFor(issues, "field", i, "default")}
          <div class="detail">
            {#await loadTimer() then { default: TimerDefault }}
              <TimerDefault field={f.label || f.key} value={f.default} onchange={(v) => patch(i, { default: v })} />
            {/await}
            {#if timerErr}<p class="err">{timerErr}</p>{/if}
          </div>
        {/if}
      </div>
    {/each}
  </div>

  <div class="foot">
    <Button size="sm" onclick={add} disabled={full}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Add field
    </Button>
    <span class="count">{fields.length} / {TEMPLATE_FIELDS_MAX}</span>
  </div>
  {#if fields.length === 0}
    <p class="hint">
      Fields are the values an operator fills in, used in the HTML as <code>{"{{key}}"}</code>.
    </p>
  {/if}
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-4);
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(7rem, 1fr) minmax(8rem, 1.4fr) 8.5rem minmax(7rem, 1.2fr) auto;
    gap: var(--s-2);
    align-items: center;
  }
  .head {
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .cell {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .cell input {
    width: 100%;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .cell input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .row :global(.type) {
    width: 100%;
  }
  .acts {
    display: inline-flex;
    gap: 2px;
  }
  .summary {
    padding: 0 var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .detail {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0 0 var(--s-2) var(--s-4);
    padding: var(--s-2) 0 var(--s-2) var(--s-3);
    border-left: 1px solid var(--line);
  }
  .err {
    grid-column: 1 / -1;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    margin-top: var(--s-2);
  }
  .count {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  @media (max-width: 720px) {
    .head {
      display: none;
    }
    .row {
      grid-template-columns: 1fr 1fr;
    }
  }
</style>
