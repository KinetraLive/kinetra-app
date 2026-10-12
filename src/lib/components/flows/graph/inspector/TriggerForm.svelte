<!--
  TriggerForm: the inspector's config form for trigger nodes (lazy chunk). Cue: the button label
  (the hotkey is assigned in Control). Schedule item status: schedule, status, any or a chosen item.
  Before next item: minutes and schedule. Time of day: HH:MM and days (none = every day). Timer
  reaches zero: screen, layer, timer field. Layer on/off air: screen, layer, on or off. Slideshow
  wrap / Source row changed: the source. API value changed: an API source and optionally one column
  of its current row (none: any row change). Run by flow: nothing to set. MIDI in: device, channel,
  Note or CC, number, Learn (MidiTriggerFields). OSC in: the address (mono; * matches one part).
  Media ended: screen, layer.

  Props: FormProps (../load.ts).
-->
<script lang="ts">
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import MidiTriggerFields from "$lib/components/midi/MidiTriggerFields.svelte";
  import { DAYS } from "../../linear";
  import {
    BEFORE_MINUTES,
    ON_AIR,
    STATUS_OPTIONS,
    apiColumnOptions,
    apiOptions,
    scheduleOptions,
    textOptions,
    timerKeys,
    toggleDay,
    withCurrent,
  } from "../../pickers";
  import type { FormProps } from "../load";
  import Field from "./Field.svelte";
  import LayerPick from "./LayerPick.svelte";
  import { commit } from "./form";
  import { scheduleItems } from "./items.svelte";

  let { ed, node }: FormProps = $props();
  const ro = $derived(ed.readOnly);
</script>

{#if node.type === "trigger.cue"}
  {@const c = node.config}
  <Field
    label="Button label (empty: the flow's name)"
    input={{ value: c.label ?? "", maxlength: 100 }}
    disabled={ro}
    oninput={(v) => commit(ed, node, { label: v === "" ? null : v }, "label")}
  />
  <p class="help">The hotkey is assigned in Control.</p>
{:else if node.type === "trigger.itemStatus"}
  {@const c = node.config}
  <Field label="Schedule">
    <Select
      size="sm"
      class="grow"
      aria-label="Schedule"
      disabled={ro}
      value={c.sourceId}
      options={withCurrent(scheduleOptions(ed.lookups), c.sourceId, "Choose a schedule")}
      onchange={(v) => commit(ed, node, { ...c, sourceId: v, itemId: null })}
    />
  </Field>
  <Field label="Becomes">
    <Select size="sm" class="grow" aria-label="Status" disabled={ro} value={c.status} options={STATUS_OPTIONS} onchange={(v) => commit(ed, node, { ...c, status: v })} />
  </Field>
  <Field label="Item">
    <Select
      size="sm"
      class="grow"
      aria-label="Item"
      disabled={ro}
      value={c.itemId ?? ""}
      options={[
        { value: "", label: "Any item" },
        ...withCurrent(scheduleItems.of(c.sourceId), c.itemId ?? "", "").filter((o) => o.value !== ""),
      ]}
      onchange={(v) => commit(ed, node, { ...c, itemId: v === "" ? null : v })}
    />
  </Field>
{:else if node.type === "trigger.beforeNext"}
  {@const c = node.config}
  <Field label="Minutes before the next item">
    <NumberInput
      aria-label="Minutes before"
      disabled={ro}
      value={c.minutes}
      min={BEFORE_MINUTES.min}
      max={BEFORE_MINUTES.max}
      digits={4}
      suffix="min"
      onchange={(v) => commit(ed, node, { ...c, minutes: v })}
    />
  </Field>
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
{:else if node.type === "trigger.timeOfDay"}
  {@const c = node.config}
  <Field
    label="Time"
    input={{ value: c.at, type: "time" }}
    disabled={ro}
    oninput={(v) => v && commit(ed, node, { ...c, at: v }, "at")}
  />
  <Field label="Days (none: every day)">
    <span class="days" role="group" aria-label="Days">
      {#each DAYS as d, i (d)}
        <button
          type="button"
          class="day vf"
          aria-pressed={c.days.includes(i)}
          disabled={ro}
          onclick={() => commit(ed, node, { ...c, days: toggleDay(c.days, i) })}
        >
          {d}
        </button>
      {/each}
    </span>
  </Field>
{:else if node.type === "trigger.timerZero"}
  {@const c = node.config}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <Field label="Timer field">
    <Select
      size="sm"
      class="grow"
      aria-label="Timer field"
      disabled={ro}
      value={c.key}
      options={withCurrent(timerKeys(ed.lookups, c.screenId, c.layerId), c.key, "Choose a timer")}
      onchange={(v) => commit(ed, node, { ...c, key: v })}
    />
  </Field>
{:else if node.type === "trigger.layer"}
  {@const c = node.config}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <Field label="When it">
    <Select
      size="sm"
      class="grow"
      aria-label="On or off air"
      disabled={ro}
      value={c.onAir ? "on" : "off"}
      options={ON_AIR}
      onchange={(v) => commit(ed, node, { ...c, onAir: v === "on" })}
    />
  </Field>
{:else if node.type === "trigger.slideshowWrap" || node.type === "trigger.sourceRow"}
  {@const c = node.config}
  <Field label="Source">
    <Select
      size="sm"
      class="grow"
      aria-label="Source"
      disabled={ro}
      value={c.sourceId}
      options={withCurrent(textOptions(ed.lookups), c.sourceId, "Choose a source")}
      onchange={(v) => commit(ed, node, { sourceId: v })}
    />
  </Field>
{:else if node.type === "trigger.apiChanged"}
  {@const c = node.config}
  <Field label="API source">
    <Select
      size="sm"
      class="grow"
      aria-label="API source"
      disabled={ro}
      value={c.sourceId}
      options={withCurrent(apiOptions(ed.lookups), c.sourceId, "Choose an API source")}
      onchange={(v) => commit(ed, node, { sourceId: v, column: null })}
    />
  </Field>
  <Field label="When">
    <Select
      size="sm"
      class="grow"
      aria-label="Column"
      disabled={ro}
      value={c.column ?? ""}
      options={apiColumnOptions(ed.lookups, c.sourceId, c.column ?? null)}
      onchange={(v) => commit(ed, node, { ...c, column: v === "" ? null : v })}
    />
  </Field>
  <p class="help">With a column: when its value in the current row changes (● value). Otherwise when any row changes (● row).</p>
{:else if node.type === "trigger.midi"}
  <Field label="Control">
    <MidiTriggerFields config={node.config} disabled={ro} onchange={(c) => commit(ed, node, c)} />
  </Field>
  <p class="help">Fires on a key press or a knob move. ● value is 0–127.</p>
{:else if node.type === "trigger.osc"}
  <Field
    label="Address"
    input={{ value: node.config.address, mono: true, maxlength: 200, placeholder: "/kinetra/go" }}
    disabled={ro}
    oninput={(v) => commit(ed, node, { address: v }, "address")}
  />
  <p class="help">* matches one part, like /show/*/go. ● value is the first argument.</p>
{:else if node.type === "trigger.runByFlow"}
  <p class="help">Started by another flow's Run flow. Its input comes out of ● value.</p>
{:else if node.type === "trigger.mediaEnded"}
  <LayerPick {ed} value={node.config} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <p class="help">Fires at the out point when the layer holds or clears (not when it loops).</p>
{/if}

<style>
  .help {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .days {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 2px;
  }
  .day {
    --vf-radius: var(--r-control);
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .day[aria-pressed="true"] {
    background: var(--accent-soft);
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
</style>
