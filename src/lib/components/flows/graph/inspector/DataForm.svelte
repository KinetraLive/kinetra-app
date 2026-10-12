<!--
  DataForm: the inspector's config form for data nodes (lazy chunk). Source row (which row; a wired
  ● index picks it), Schedule now/next, Field value, Layer state, Constant (type + value), Time now,
  Format text (pattern), Math and Compare (operation, and the values used while ● a / ● b are not
  wired).

  Props: FormProps (../load.ts).
-->
<script lang="ts">
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import type { CompareOp, FlowConstant, MathOp, RowPick } from "$lib/types";
  import { fieldKeys, scheduleOptions, textOptions, withCurrent } from "../../pickers";
  import type { FormProps } from "../load";
  import Field from "./Field.svelte";
  import LayerPick from "./LayerPick.svelte";
  import { commit } from "./form";

  let { ed, node }: FormProps = $props();
  const ro = $derived(ed.readOnly);

  const ROWS: { value: RowPick["pick"]; label: string }[] = [
    { value: "current", label: "The current row" },
    { value: "first", label: "The first row" },
    { value: "last", label: "The last row" },
    { value: "index", label: "Row number" },
  ];
  const KINDS: { value: FlowConstant["kind"]; label: string }[] = [
    { value: "text", label: "Text" },
    { value: "number", label: "Number" },
    { value: "bool", label: "Yes / no" },
  ];
  const MATH: { value: MathOp; label: string }[] = [
    { value: "add", label: "a + b" },
    { value: "sub", label: "a − b" },
    { value: "mul", label: "a × b" },
    { value: "div", label: "a ÷ b" },
    { value: "min", label: "The smaller" },
    { value: "max", label: "The larger" },
    { value: "round", label: "Round a" },
  ];
  const COMPARE: { value: CompareOp; label: string }[] = [
    { value: "eq", label: "a = b" },
    { value: "ne", label: "a ≠ b" },
    { value: "lt", label: "a < b" },
    { value: "le", label: "a ≤ b" },
    { value: "gt", label: "a > b" },
    { value: "ge", label: "a ≥ b" },
    { value: "contains", label: "a contains b" },
    { value: "empty", label: "a is empty" },
  ];

  const constantOf = (kind: FlowConstant["kind"]): FlowConstant =>
    kind === "text" ? { kind, value: "" } : kind === "number" ? { kind, value: 0 } : { kind, value: true };
</script>

{#if node.type === "data.sourceRow"}
  {@const c = node.config}
  <Field label="Source">
    <Select
      size="sm"
      class="grow"
      aria-label="Source"
      disabled={ro}
      value={c.sourceId}
      options={withCurrent(textOptions(ed.lookups), c.sourceId, "Choose a source")}
      onchange={(v) => commit(ed, node, { ...c, sourceId: v })}
    />
  </Field>
  <Field label="Row (a wired ● index picks it)">
    <Select
      size="sm"
      aria-label="Which row"
      disabled={ro}
      value={c.row.pick}
      options={ROWS}
      onchange={(v) => commit(ed, node, { ...c, row: v === "index" ? { pick: "index", index: 0 } : { pick: v } })}
    />
    {#if c.row.pick === "index"}
      <NumberInput
        aria-label="Row number"
        disabled={ro}
        value={c.row.index + 1}
        min={1}
        max={100000}
        digits={5}
        onchange={(v) => commit(ed, node, { ...c, row: { pick: "index", index: v - 1 } })}
      />
    {/if}
  </Field>
{:else if node.type === "data.scheduleItem"}
  {@const c = node.config}
  <Field label="Schedule">
    <Select
      size="sm"
      class="grow"
      aria-label="Schedule"
      disabled={ro}
      value={c.sourceId}
      options={withCurrent(scheduleOptions(ed.lookups), c.sourceId, "Choose a schedule")}
      onchange={(v) => commit(ed, node, { ...c, sourceId: v })}
    />
  </Field>
  <Field label="Item">
    <Select
      size="sm"
      class="grow"
      aria-label="Now or next"
      disabled={ro}
      value={c.which}
      options={[
        { value: "now", label: "The NOW item" },
        { value: "next", label: "The UP NEXT item" },
      ]}
      onchange={(v) => commit(ed, node, { ...c, which: v })}
    />
  </Field>
{:else if node.type === "data.fieldValue"}
  {@const c = node.config}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, { ...next, key: "" })} />
  <Field label="Field">
    <Select
      size="sm"
      class="grow"
      aria-label="Field"
      disabled={ro}
      value={c.key}
      options={withCurrent(fieldKeys(ed.lookups, c.screenId, c.layerId), c.key, "Choose a field")}
      onchange={(v) => commit(ed, node, { ...c, key: v })}
    />
  </Field>
{:else if node.type === "data.layerState"}
  <LayerPick {ed} value={node.config} disabled={ro} onchange={(next) => commit(ed, node, next)} />
{:else if node.type === "data.constant"}
  {@const v = node.config.value}
  <Field label="Type">
    <Select size="sm" class="grow" aria-label="Type" disabled={ro} value={v.kind} options={KINDS} onchange={(k) => commit(ed, node, { value: constantOf(k) })} />
  </Field>
  {#if v.kind === "text"}
    <Field label="Value" input={{ value: v.value }} disabled={ro} oninput={(t) => commit(ed, node, { value: { kind: "text", value: t } }, "value")} />
  {:else if v.kind === "number"}
    <Field label="Value">
      <NumberInput aria-label="Value" disabled={ro} value={v.value} step={0.001} digits={8} onchange={(n) => commit(ed, node, { value: { kind: "number", value: n } })} />
    </Field>
  {:else}
    <Switch label="Yes" disabled={ro} checked={v.value} onchange={(on) => commit(ed, node, { value: { kind: "bool", value: on } })} />
  {/if}
{:else if node.type === "data.format"}
  {@const c = node.config}
  <Field
    label="Pattern ({'{a}'}…{'{h}'}; {'{a:HH:mm}'} formats a time)"
    input={{ value: c.pattern, mono: true }}
    disabled={ro}
    oninput={(t) => commit(ed, node, { pattern: t }, "pattern")}
  />
{:else if node.type === "data.math"}
  {@const c = node.config}
  <Field label="Operation">
    <Select size="sm" class="grow" aria-label="Operation" disabled={ro} value={c.op} options={MATH} onchange={(v) => commit(ed, node, { ...c, op: v })} />
  </Field>
  <Field label="a and b when not wired">
    <NumberInput aria-label="a" disabled={ro} value={c.a} step={0.001} digits={7} onchange={(n) => commit(ed, node, { ...c, a: n })} />
    {#if c.op !== "round"}
      <NumberInput aria-label="b" disabled={ro} value={c.b} step={0.001} digits={7} onchange={(n) => commit(ed, node, { ...c, b: n })} />
    {/if}
  </Field>
{:else if node.type === "data.compare"}
  {@const c = node.config}
  <Field label="Comparison (numbers compare as numbers)">
    <Select size="sm" class="grow" aria-label="Comparison" disabled={ro} value={c.op} options={COMPARE} onchange={(v) => commit(ed, node, { ...c, op: v })} />
  </Field>
  <Field label="a when not wired" input={{ value: c.a }} disabled={ro} oninput={(t) => commit(ed, node, { ...c, a: t }, "a")} />
  {#if c.op !== "empty"}
    <Field label="b when not wired" input={{ value: c.b }} disabled={ro} oninput={(t) => commit(ed, node, { ...c, b: t }, "b")} />
  {/if}
{:else if node.type === "data.timeNow"}
  <p class="help">The server clock, as ● now.</p>
{/if}

<style>
  .help {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
</style>
