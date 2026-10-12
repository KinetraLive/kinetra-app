<!--
  ActionForm: the inspector's config form for action nodes (lazy chunk). Take (optionally for N s),
  Clear layer, Clear screen, Clear all screens, Set fields (field + value pairs; a wired ● input
  replaces its value), Set timer, Step source, Pause/Resume source, Schedule action (the item:
  NOW, UP NEXT, a chosen one or the wired ● item), Set variant (layer, its template's variants,
  transition), Take preview (screen, transition: the project default or a given one), Run flow (any other flow; with none, the
  picker points at Repeat), Stop flow (any flow, "This flow" included), Log, Media (layer, Play /
  Pause / Stop / Restart / Go to time, the time in seconds).

  Props: FormProps (../load.ts).
-->
<script lang="ts">
  import { untrack } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import {
    DELAY_MINUTES,
    MEDIA_ACTIONS,
    OPS,
    PAUSE,
    STEP_MODES,
    TAKE_FOR_MS,
    TIMER_ACTIONS,
    fieldDefsOf,
    modeOf,
    opFor,
    pickFor,
    pickValue,
    runFlowPick,
    scheduleOptions,
    screenOptions,
    stepFor,
    stopFlowPick,
    textOptions,
    timerFor,
    timerKeys,
    withCurrent,
  } from "../../pickers";
  import type { FormProps } from "../load";
  import Field from "./Field.svelte";
  import LayerPick from "./LayerPick.svelte";
  import { commit } from "./form";
  import { scheduleItems } from "./items.svelte";
  import { TRANSITIONS, variantCache } from "$lib/components/screens/variants.svelte";
  import { TAKE_MS, TAKE_PICK, pickOf, transitionFor } from "$lib/components/control/studio/transitions";

  let { ed, node }: FormProps = $props();
  const ro = $derived(ed.readOnly);
  // A Set variant node lists the variants as they are now (the layout or template may have changed).
  const variantNode = $derived(node.type === "action.setVariant" ? node.id : null);
  $effect(() => {
    if (variantNode) untrack(() => variantCache.reset());
  });
</script>

{#if node.type === "action.take"}
  {@const c = node.config}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <Field label="On air for (0: stays on)">
    <NumberInput
      aria-label="Seconds on air"
      disabled={ro}
      value={c.forMs ? c.forMs / 1000 : 0}
      min={0}
      max={TAKE_FOR_MS.max / 1000}
      digits={4}
      suffix="s"
      onchange={(v) => commit(ed, node, { ...c, forMs: v > 0 ? Math.round(v * 1000) : null })}
    />
  </Field>
{:else if node.type === "action.clear"}
  <LayerPick {ed} value={node.config} disabled={ro} onchange={(next) => commit(ed, node, next)} />
{:else if node.type === "action.clearScreen"}
  {@const c = node.config}
  <Field label="Screen">
    <Select
      size="sm"
      class="grow"
      aria-label="Screen"
      disabled={ro}
      value={c.screenId}
      options={withCurrent(screenOptions(ed.lookups), c.screenId, "Choose a screen")}
      onchange={(v) => commit(ed, node, { screenId: v })}
    />
  </Field>
{:else if node.type === "action.clearAll"}
  <p class="help">Takes every layer of every screen off air, animated.</p>
{:else if node.type === "action.setFields"}
  {@const c = node.config}
  {@const defs = fieldDefsOf(ed.lookups, c.screenId, c.layerId)}
  {@const used = Object.keys(c.fields)}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <Field label="Fields (a wired input replaces the value)">
    <div class="pairs">
      {#each Object.entries(c.fields) as [k, v] (k)}
        <div class="pair">
          <Select
            size="sm"
            aria-label="Field"
            disabled={ro}
            value={k}
            options={withCurrent(
              defs.filter((f) => f.key === k || !used.includes(f.key)).map((f) => ({ value: f.key, label: f.label || f.key })),
              k,
              "Choose a field",
            )}
            onchange={(to) =>
              commit(ed, node, { ...c, fields: Object.fromEntries(Object.entries(c.fields).map(([a, b]) => (a === k ? [to, b] : [a, b]))) })}
          />
          <span class="text vf vf-within">
            <input
              type="text"
              aria-label="Value of {k}"
              autocomplete="off"
              disabled={ro}
              value={v}
              oninput={(e) => commit(ed, node, { ...c, fields: { ...c.fields, [k]: e.currentTarget.value } }, `field-${k}`)}
            />
          </span>
          <IconButton variant="danger"
            size="sm"
            label="Remove field {k}"
            tooltip="left"
            disabled={ro}
            onclick={() => commit(ed, node, { ...c, fields: Object.fromEntries(Object.entries(c.fields).filter(([a]) => a !== k)) })}
          >
            <Trash2 size={14} />
          </IconButton>
        </div>
      {/each}
      <Button
        size="sm"
        variant="ghost"
        disabled={ro || defs.every((f) => used.includes(f.key))}
        onclick={() => {
          const def = defs.find((f) => !used.includes(f.key));
          if (def) commit(ed, node, { ...c, fields: { ...c.fields, [def.key]: def.default } });
        }}
      >
        {#snippet icon()}<Plus size={14} />{/snippet}
        Add field
      </Button>
    </div>
  </Field>
{:else if node.type === "action.setTimer"}
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
  <Field label="Action">
    <Select
      size="sm"
      aria-label="Timer action"
      disabled={ro}
      value={c.action.action}
      options={c.action.action === "configure" ? [...TIMER_ACTIONS, { value: "configure", label: "Set up" }] : TIMER_ACTIONS}
      onchange={(v) => commit(ed, node, { ...c, action: timerFor(v, c.action) })}
    />
    {#if c.action.action === "set"}
      {@const a = c.action}
      <NumberInput
        aria-label="Timer seconds"
        disabled={ro}
        value={a.ms / 1000}
        min={0}
        max={86400}
        digits={5}
        suffix="s"
        onchange={(v) => commit(ed, node, { ...c, action: { action: "set", ms: Math.round(v * 1000) } })}
      />
    {/if}
  </Field>
{:else if node.type === "action.stepSource"}
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
  <Field label="Step">
    <Select size="sm" aria-label="Step" disabled={ro} value={modeOf(c.step)} options={STEP_MODES} onchange={(m) => commit(ed, node, { ...c, step: stepFor(m) })} />
    {#if c.step.to === "index"}
      <NumberInput
        aria-label="Row number"
        disabled={ro}
        value={c.step.index + 1}
        min={1}
        max={100000}
        digits={5}
        onchange={(v) => commit(ed, node, { ...c, step: { to: "index", index: v - 1 } })}
      />
    {/if}
  </Field>
{:else if node.type === "action.pauseSource"}
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
  <Field label="Action">
    <Select size="sm" class="grow" aria-label="Pause or resume" disabled={ro} value={c.mode} options={PAUSE} onchange={(v) => commit(ed, node, { ...c, mode: v })} />
  </Field>
{:else if node.type === "action.schedule"}
  {@const c = node.config}
  <Field label="Schedule">
    <Select
      size="sm"
      class="grow"
      aria-label="Schedule"
      disabled={ro}
      value={c.sourceId}
      options={withCurrent(scheduleOptions(ed.lookups), c.sourceId, "Choose a schedule")}
      onchange={(v) => commit(ed, node, { ...c, sourceId: v, item: c.item.pick === "item" ? { pick: "now" } : c.item })}
    />
  </Field>
  <Field label="Item">
    <Select
      size="sm"
      class="grow"
      aria-label="Item"
      disabled={ro}
      value={pickValue(c.item)}
      options={[
        { value: "now", label: "The NOW item" },
        { value: "next", label: "The UP NEXT item" },
        { value: "input", label: "The wired item (● item)" },
        ...withCurrent(scheduleItems.of(c.sourceId), c.item.pick === "item" ? c.item.itemId : "", "")
          .filter((o) => o.value !== "")
          .map((o) => ({ ...o, value: `item:${o.value}` })),
      ]}
      onchange={(v) => commit(ed, node, { ...c, item: pickFor(v) })}
    />
  </Field>
  <Field label="Action">
    <Select size="sm" aria-label="Schedule action" disabled={ro} value={c.op.op} options={OPS} onchange={(v) => commit(ed, node, { ...c, op: opFor(v, c.op) })} />
    {#if c.op.op === "delay"}
      <NumberInput
        aria-label="Delay minutes"
        disabled={ro}
        value={c.op.minutes}
        min={DELAY_MINUTES.min}
        max={DELAY_MINUTES.max}
        digits={5}
        suffix="min"
        onchange={(v) => commit(ed, node, { ...c, op: { op: "delay", minutes: v } })}
      />
    {/if}
  </Field>
  {#if c.op.op === "setStart" || c.op.op === "setNext"}
    {@const op = c.op}
    <Field
      label={op.op === "setStart" ? "New start" : "Start (empty: keeps its time)"}
      input={{ value: op.start ?? "", type: "datetime-local", mono: true }}
      disabled={ro}
      oninput={(v) =>
        commit(ed, node, { ...c, op: op.op === "setStart" ? { op: "setStart", start: v } : { op: "setNext", start: v === "" ? null : v } }, "start")}
    />
  {/if}
{:else if node.type === "action.setVariant"}
  {@const c = node.config}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <Field label="Variant">
    <Select
      size="sm"
      class="grow"
      aria-label="Variant"
      disabled={ro}
      value={c.variant}
      options={variantCache.pick(c.screenId, c.layerId, c.variant)}
      onchange={(v) => commit(ed, node, { ...c, variant: v })}
    />
  </Field>
  <Field label="Transition while on air">
    <Select
      size="sm"
      class="grow"
      aria-label="Transition"
      disabled={ro}
      value={c.transition}
      options={TRANSITIONS}
      onchange={(v) => commit(ed, node, { ...c, transition: v })}
    />
  </Field>
{:else if node.type === "action.takePreview"}
  {@const c = node.config}
  <Field label="Screen">
    <Select
      size="sm"
      class="grow"
      aria-label="Screen"
      disabled={ro}
      value={c.screenId}
      options={withCurrent(screenOptions(ed.lookups), c.screenId, "Choose a screen")}
      onchange={(v) => commit(ed, node, { ...c, screenId: v })}
    />
  </Field>
  <Field label="Transition">
    <Select
      size="sm"
      class="grow"
      aria-label="Transition"
      disabled={ro}
      value={pickOf(c.transition)}
      options={TAKE_PICK}
      onchange={(v) => commit(ed, node, { ...c, transition: transitionFor(v, c.transition) })}
    />
    {#if c.transition?.kind === "crossfade"}
      {@const t = c.transition}
      <NumberInput
        aria-label="Crossfade duration"
        disabled={ro}
        value={t.ms}
        min={TAKE_MS.min}
        max={TAKE_MS.max}
        step={50}
        digits={4}
        suffix="ms"
        onchange={(ms) => commit(ed, node, { ...c, transition: { ...t, ms } })}
      />
    {/if}
  </Field>
  <p class="help">Fails while Studio mode is off (● failed).</p>
{:else if node.type === "action.runFlow"}
  {@const c = node.config}
  <Field label="Flow">
    <Select
      size="sm"
      class="grow"
      aria-label="Flow to run"
      disabled={ro}
      value={c.flowId}
      options={runFlowPick(ed.lookups, ed.id, c.flowId)}
      onchange={(v) => commit(ed, node, { ...c, flowId: v })}
    />
  </Field>
  <Switch label="Wait until it finishes" disabled={ro} checked={c.wait} onchange={(on) => commit(ed, node, { ...c, wait: on })} />
{:else if node.type === "action.stopFlow"}
  {@const c = node.config}
  <Field label="Flow">
    <Select
      size="sm"
      class="grow"
      aria-label="Flow to stop"
      disabled={ro}
      value={c.flowId}
      options={stopFlowPick(ed.lookups, ed.id, c.flowId)}
      onchange={(v) => commit(ed, node, { flowId: v })}
    />
  </Field>
  <p class="help">{c.flowId === ed.id ? "Ends this run here; done doesn't fire." : "Same as its Stop button; done always fires."}</p>
{:else if node.type === "action.log"}
  {@const c = node.config}
  <Field
    label="Message (a wired ● text replaces it)"
    input={{ value: c.message, maxlength: 500 }}
    disabled={ro}
    oninput={(v) => commit(ed, node, { ...c, message: v }, "message")}
  />
  <Switch label="Also show a toast in the app" disabled={ro} checked={c.toast} onchange={(on) => commit(ed, node, { ...c, toast: on })} />
{:else if node.type === "action.media"}
  {@const c = node.config}
  <LayerPick {ed} value={c} disabled={ro} onchange={(next) => commit(ed, node, next)} />
  <Field label="Action">
    <Select
      size="sm"
      class="grow"
      aria-label="Media action"
      disabled={ro}
      value={c.action}
      options={MEDIA_ACTIONS}
      onchange={(v) => commit(ed, node, { ...c, action: v, ms: v === "seek" ? (c.ms ?? 0) : null })}
    />
  </Field>
  {#if c.action === "seek"}
    <Field label="Time">
      <NumberInput
        aria-label="Seconds into the file"
        disabled={ro}
        value={(c.ms ?? 0) / 1000}
        min={0}
        step={0.1}
        digits={6}
        suffix="s"
        onchange={(v) => commit(ed, node, { ...c, ms: Math.round(v * 1000) })}
      />
    </Field>
  {/if}
  <p class="help">Acts on what is on air at once, also in Studio mode.</p>
{/if}

<style>
  .help {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .pairs {
    display: flex;
    flex: 1 1 100%;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-1);
    min-width: 0;
  }
  .pair {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    width: 100%;
  }
  .text {
    --vf-radius: var(--r-control);
    display: flex;
    flex: 1;
    min-width: 0;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .text input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    color: var(--text);
  }
</style>
