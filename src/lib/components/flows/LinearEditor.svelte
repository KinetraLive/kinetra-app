<!--
  LinearEditor: "When … then …" for one flow (lazy chunk; opened through ctl.flowEditor). A native
  modal <dialog> (focus trap, Esc, inert background). One editor for cues (manual / hotkey) and
  automatic rules alike: it replaces the macro and rules editors.

    Name, and for a cue its retrigger ("Running again restarts it" / "Ignored while it runs").
    When   the trigger type and its fields: a cue's button label, a schedule item's status (+ a
           specific item), minutes before the next item, a time of day + days, a timer reaching
           zero, a layer going on or off air, a slideshow wrapping, a source's row changing, an
           API source's data (or one column of it) changing, or being run by another flow.
    Then   ordered steps: Take (optionally for N s), Clear layer, Clear screen, Clear all screens,
           Set fields, Set timer, Step source, Pause / resume source, Schedule action, Set variant
           (a layer's look, with its transition while on air), Take preview (a screen's Preview to
           Program, with the project's or a given transition), Wait, Run flow (another flow; with none,
           the picker points at Repeat), Stop flow (any flow, "This flow" included), Log. Each with Move up / Move down (or Alt+↑/↓ anywhere in the step) and Delete.

  A missing target or a bad value is an error (Save is disabled); a target that no longer exists is
  a warning (saved, skipped when the flow runs). Save creates the flow (a cue armed, a rule
  disarmed) or saves it, deployed at once; the server's message shows inline if it refuses. A new
  rule from a recipe panel belongs to that recipe (`origin`) and starts on its screen and source.

  Props: LinearEditorProps (./launcher.svelte.ts).
-->
<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import Plus from "lucide-svelte/icons/plus";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { toAppError } from "$lib/api";
  import { getSchedule } from "$lib/api/sources";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TimeInput from "$lib/components/inputs/TimeInput.svelte";
  import MidiTriggerFields from "$lib/components/midi/MidiTriggerFields.svelte";
  import { hhmm } from "$lib/components/recipes/schedule";
  import { TRANSITIONS, variantCache } from "$lib/components/screens/variants.svelte";
  import { TAKE_MS, TAKE_PICK, pickOf, transitionFor } from "$lib/components/control/studio/transitions";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { FLOW_NAME_MAX, type Retrigger } from "$lib/types";
  import {
    BEFORE_MINUTES,
    DELAY_MINUTES,
    RETRIGGER_OPTIONS,
    STATUS_OPTIONS,
    STEP_OPTIONS,
    TAKE_FOR_MS,
    TRIGGER_OPTIONS,
    check,
    defaultName,
    nameError,
    newLinear,
    stepIssue,
    stepOfType,
    toNewFlow,
    toSaveFlow,
    triggerIssue,
    triggerOfType,
    type Defaults,
    type Issue,
  } from "./edit";
  import type { LinearEditorProps } from "./launcher.svelte";
  import * as P from "./pickers";
  import {
    ON_AIR,
    OPS,
    PAUSE,
    STEP_MODES,
    TIMER_ACTIONS,
    modeOf,
    opFor,
    pickFor,
    pickValue,
    stepFor,
    timerFor,
    toggleDay,
    withCurrent,
    type Layered,
    type Opt,
  } from "./pickers";
  import { create, save as saveFlow } from "./ops";
  import { DAYS, newNodeId, toLinear, type LinearFlow, type LinearStep, type Lookups, type StepType, type TriggerType } from "./linear";

  let { target, ctl, onclose }: LinearEditorProps = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let saving = $state(false);
  let error = $state<string | null>(null);
  /** Schedule source id → its items as options (for "a specific item"). */
  let scheduleItems = $state.raw<Record<string, Opt[]>>({});
  let returnFocus: HTMLElement | null = null;

  const existing = untrack(() => (target.mode === "edit" ? (ctl.flows.get(target.id) ?? null) : null));
  /** A recipe's own new rule: owned by it, aimed at its screen, first layer and source. */
  const origin = untrack(() => (target.mode === "new" ? (target.origin ?? null) : null));
  /** A slideshow recipe's rule starts on "a slideshow wraps". */
  let originTrigger: TriggerType | undefined;
  const defaults = untrack((): Defaults => {
    const view = (origin && ctl.screens.find((v) => v.screen.id === origin.screenId)) || ctl.selected || ctl.screens[0] || null;
    const own = origin?.sourceId ? ctl.sources.find((s) => s.id === origin.sourceId) : undefined;
    if (own && own.kind !== "schedule") originTrigger = "trigger.slideshowWrap";
    const layerId = origin && view?.layers.some((l) => l.id === origin.layerId) ? origin.layerId : view?.layers.at(-1)?.id;
    return {
      screenId: view?.screen.id ?? "",
      layerId: layerId ?? "",
      scheduleId: (own?.kind === "schedule" ? own : ctl.sources.find((s) => s.kind === "schedule"))?.id ?? "",
      textId: (own && own.kind !== "schedule" ? own : ctl.sources.find((s) => s.kind !== "schedule"))?.id ?? "",
      apiId: ctl.sources.find((s) => s.kind === "api")?.id ?? "",
    };
  });
  const initial = untrack((): LinearFlow | null =>
    existing ? toLinear(existing) : newLinear(target.mode === "new" ? target.kind : "cue", defaults, originTrigger),
  );

  let name = $state(
    untrack(() =>
      existing?.name ??
        defaultName(target.mode === "new" && target.kind === "rule" ? "Rule" : "Cue", ctl.flows.list.map((f) => f.name)),
    ),
  );
  let retrigger = $state<Retrigger>(untrack(() => existing?.retrigger ?? "restart"));
  let draft = $state<LinearFlow | null>(initial ? structuredClone(initial) : null);

  const selfId = $derived(existing?.id ?? null);
  const lookups = $derived<Lookups>({
    screens: new Map(ctl.screens.map((v) => [v.screen.id, v])),
    sources: new Map(ctl.sources.map((s) => [s.id, s])),
    flows: new Map(ctl.flows.list.map((f) => [f.id, f])),
    self: selfId,
  });
  /** A time typed that isn't one yet (TimeInput keeps it in the box): counts as a problem. */
  let timeInvalid = $state(false);
  const checked = $derived(draft ? check(name, draft, lookups, selfId) : { errors: 1, warnings: 0 });
  const result = $derived(timeInvalid ? { ...checked, errors: checked.errors + 1 } : checked);
  const title = $derived(
    existing ? (existing.nodes.some((n) => n.type === "trigger.cue") ? "Edit cue" : "Edit rule") : target.mode === "new" && target.kind === "rule" ? "New rule" : "New cue",
  );

  const screenOptions = $derived<Opt[]>(ctl.screens.map((v) => ({ value: v.screen.id, label: v.screen.name })));
  const scheduleOptions = $derived<Opt[]>(
    ctl.sources.filter((s) => s.kind === "schedule").map((s) => ({ value: s.id, label: s.name })),
  );
  const textOptions = $derived<Opt[]>(
    ctl.sources.filter((s) => s.kind !== "schedule").map((s) => ({ value: s.id, label: s.name })),
  );
  onMount(() => {
    // Variant pickers list the layers' templates as they are now.
    variantCache.reset();
    const d = dialog;
    if (!d) return;
    returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    void tick().then(() => {
      const input = document.getElementById(`${uid}-name`) as HTMLInputElement | null;
      input?.focus();
      if (!existing) input?.select();
    });
    return () => {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
      const el = returnFocus;
      returnFocus = null;
      if (el?.isConnected) el.focus();
    };
  });

  // Items of the schedules the trigger and schedule steps point at (loaded once per source).
  $effect(() => {
    if (!draft) return;
    const ids = new Set<string>();
    const t = draft.trigger;
    if (t.type === "trigger.itemStatus" && t.config.sourceId) ids.add(t.config.sourceId);
    for (const s of draft.steps) if (s.type === "action.schedule" && s.config.sourceId) ids.add(s.config.sourceId);
    for (const id of ids) {
      if (untrack(() => id in scheduleItems)) continue;
      untrack(() => (scheduleItems = { ...scheduleItems, [id]: [] }));
      getSchedule(id).then(
        (doc) => {
          scheduleItems = {
            ...scheduleItems,
            [id]: doc.items.map((i) => ({ value: i.id, label: `${hhmm(i.start)} ${i.title || "Untitled"}` })),
          };
        },
        () => {},
      );
    }
  });

  function cancel() {
    if (!saving) onclose();
  }

  function onCancelEvent(e: Event) {
    e.preventDefault();
    cancel();
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === dialog) cancel();
  }

  async function save(e?: SubmitEvent) {
    e?.preventDefault();
    if (!draft || result.errors > 0 || saving) return;
    saving = true;
    error = null;
    const l = $state.snapshot(draft) as LinearFlow;
    try {
      if (existing) await saveFlow(ctl.flows, toSaveFlow(existing, name, retrigger, l));
      else await create(ctl.flows, toNewFlow(name, retrigger, l, origin?.id));
    } catch (err) {
      error = toAppError(err).message;
      saving = false;
      return;
    }
    saving = false;
    onclose();
  }

  // ---- Steps --------------------------------------------------------------------------------------

  function move<T>(arr: T[], i: number, delta: number): T[] {
    const j = Math.max(0, Math.min(arr.length - 1, i + delta));
    if (i === j) return arr;
    const next = [...arr];
    const [x] = next.splice(i, 1);
    next.splice(j, 0, x);
    return next;
  }

  async function keepFocus() {
    const el = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    await tick();
    if (el && el.isConnected && document.activeElement !== el) el.focus();
  }

  function moveStep(i: number, delta: number) {
    if (!draft) return;
    draft.steps = move(draft.steps, i, delta);
    void keepFocus();
  }

  function onStepKey(e: KeyboardEvent, i: number) {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    moveStep(i, e.key === "ArrowUp" ? -1 : 1);
  }

  function addStep() {
    if (!draft) return;
    const last = draft.steps.at(-1);
    const type: StepType = !last || last.type === "logic.delay" ? "action.take" : last.type;
    // Targets carry over from the last step; the id is new.
    draft.steps.push({ ...stepOfType(type, defaults, last), id: newNodeId() });
  }

  /** A timer step or trigger starts on the layer's first timer field. */
  function withTimer<T extends { config: object }>(x: T): T {
    const c = x.config as Partial<Layered>;
    if (c.key === "" && c.screenId && c.layerId) c.key = timerKeys(c.screenId, c.layerId)[0]?.value ?? "";
    return x;
  }

  function setStepType(i: number, type: StepType) {
    if (!draft || draft.steps[i].type === type) return;
    draft.steps[i] = withTimer(stepOfType(type, defaults, draft.steps[i]));
  }

  function setTriggerType(type: TriggerType) {
    if (!draft || draft.trigger.type === type) return;
    draft.trigger = withTimer(triggerOfType(type, defaults, draft.trigger));
  }

  const layersOf = (screenId: string) => P.layersOf(lookups, screenId);
  const fieldDefsOf = (screenId: string, layerId: string) => P.fieldDefsOf(lookups, screenId, layerId);
  const timerKeys = (screenId: string, layerId: string) => P.timerKeys(lookups, screenId, layerId);
  const setScreen = (x: Layered, screenId: string) => P.setScreen(lookups, x, screenId);
  const setLayer = (x: Layered, layerId: string) => P.setLayer(lookups, x, layerId);

  type SetFields = Extract<LinearStep, { type: "action.setFields" }>["config"];

  function addField(c: SetFields) {
    const def = fieldDefsOf(c.screenId, c.layerId).find((f) => !(f.key in c.fields));
    if (def) c.fields = { ...c.fields, [def.key]: def.default };
  }

  function renameField(c: SetFields, from: string, to: string) {
    c.fields = Object.fromEntries(Object.entries(c.fields).map(([k, v]) => (k === from ? [to, v] : [k, v])));
  }

  function removeField(c: SetFields, key: string) {
    const next = { ...c.fields };
    delete next[key];
    c.fields = next;
  }

  /** Simple mode hides the MIDI / OSC triggers (a flow that already uses one keeps it). */
  const HIDDEN_SIMPLE: readonly string[] = ["trigger.midi", "trigger.osc"];
  function triggerOptions(current: string) {
    return isAdvanced() ? TRIGGER_OPTIONS : TRIGGER_OPTIONS.filter((o) => o.value === current || !HIDDEN_SIMPLE.includes(o.value));
  }
</script>

{#snippet issueLine(issue: Issue | null)}
  {#if issue}
    <p class="issue" class:error={issue.level === "error"} class:warning={issue.level === "warning"}>
      {#if issue.level === "warning"}<TriangleAlert size={12} />{/if}
      {issue.message}
    </p>
  {/if}
{/snippet}

{#snippet layerPickers(x: Layered, label: string)}
  <Select
    size="sm"
    aria-label="{label} screen"
    value={x.screenId}
    options={withCurrent(screenOptions, x.screenId, "Choose a screen")}
    onchange={(v) => setScreen(x, v)}
  />
  <Select
    size="sm"
    aria-label="{label} layer"
    value={x.layerId}
    options={withCurrent(layersOf(x.screenId), x.layerId, "Choose a layer")}
    onchange={(v) => setLayer(x, v)}
  />
{/snippet}

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog bind:this={dialog} class="dialog" aria-labelledby="{uid}-title" oncancel={onCancelEvent} onclick={onBackdropClick}>
  <form class="inner" onsubmit={save} novalidate>
    <h2 id="{uid}-title" class="title">{title}</h2>

    {#if !draft}
      {#if isAdvanced()}
        <p class="muted">Too complex to edit here. Use Edit in Flows.</p>
      {:else}
        <p class="muted">Too complex to edit here.</p>
        {#await import("$lib/components/AdvancedHint.svelte") then { default: AdvancedHint }}<AdvancedHint feature="The Flow editor" />{/await}
      {/if}
    {:else}
      {@const t = draft.trigger}
      {@const nErr = nameError(name)}
      <div class="name-row">
        <label for="{uid}-name">Name</label>
        <div class="input vf vf-within">
          <input
            id="{uid}-name"
            type="text"
            bind:value={name}
            maxlength={FLOW_NAME_MAX}
            autocomplete="off"
            spellcheck="false"
            aria-invalid={nErr !== null}
          />
        </div>
        {#if t.type === "trigger.cue"}
          <Select size="sm" aria-label="When it runs again" value={retrigger} options={RETRIGGER_OPTIONS} onchange={(v) => (retrigger = v)} />
        {/if}
      </div>
      {#if nErr}<p class="error">{nErr}</p>{/if}

      <span class="section-label">When</span>
      <div class="block">
        <div class="row">
          <Select size="sm" aria-label="Trigger" value={t.type} options={triggerOptions(t.type)} onchange={setTriggerType} />
          {#if t.type === "trigger.cue"}
            {@const c = t.config}
            <label for="{uid}-cue">Button label</label>
            <div class="input sm grow vf vf-within">
              <input
                id="{uid}-cue"
                type="text"
                aria-describedby="{uid}-label-help"
                value={c.label ?? ""}
                oninput={(e) => (c.label = e.currentTarget.value === "" ? null : e.currentTarget.value)}
                maxlength={FLOW_NAME_MAX}
                autocomplete="off"
              />
            </div>
          {:else if t.type === "trigger.itemStatus"}
            {@const c = t.config}
            <Select
              size="sm"
              aria-label="Schedule"
              value={c.sourceId}
              options={withCurrent(scheduleOptions, c.sourceId, "Choose a schedule")}
              onchange={(v) => {
                c.sourceId = v;
                c.itemId = null;
              }}
            />
            <Select size="sm" aria-label="Status" value={c.status} options={STATUS_OPTIONS} onchange={(v) => (c.status = v)} />
            <Select
              size="sm"
              aria-label="Item"
              value={c.itemId ?? ""}
              options={[{ value: "", label: "Any item" }, ...withCurrent(scheduleItems[c.sourceId] ?? [], c.itemId ?? "", "").filter((o) => o.value !== "")]}
              onchange={(v) => (c.itemId = v === "" ? null : v)}
            />
          {:else if t.type === "trigger.beforeNext"}
            {@const c = t.config}
            <NumberInput
              aria-label="Minutes before"
              value={c.minutes}
              min={BEFORE_MINUTES.min}
              max={BEFORE_MINUTES.max}
              digits={4}
              suffix="min"
              onchange={(v) => (c.minutes = v)}
            />
            <span class="muted">before the next item of</span>
            <Select
              size="sm"
              aria-label="Schedule"
              value={c.sourceId}
              options={withCurrent(scheduleOptions, c.sourceId, "Choose a schedule")}
              onchange={(v) => (c.sourceId = v)}
            />
          {:else if t.type === "trigger.timeOfDay"}
            {@const c = t.config}
            <TimeInput aria-label="Time of day" value={c.at} onchange={(v) => (c.at = v)} oninvalid={(r) => (timeInvalid = r !== null)} />
            <span class="days" role="group" aria-label="Days (none = every day)">
              {#each DAYS as d, i (d)}
                <button type="button" class="day vf" aria-pressed={c.days.includes(i)} onclick={() => (c.days = toggleDay(c.days, i))}>
                  {d}
                </button>
              {/each}
            </span>
          {:else if t.type === "trigger.timerZero"}
            {@const c = t.config}
            {@render layerPickers(c, "Trigger")}
            <Select
              size="sm"
              aria-label="Timer field"
              value={c.key}
              options={withCurrent(timerKeys(c.screenId, c.layerId), c.key, "Choose a timer")}
              onchange={(v) => (c.key = v)}
            />
          {:else if t.type === "trigger.layer"}
            {@const c = t.config}
            {@render layerPickers(c, "Trigger")}
            <Select
              size="sm"
              aria-label="On or off air"
              value={c.onAir ? "on" : "off"}
              options={ON_AIR}
              onchange={(v) => (c.onAir = v === "on")}
            />
          {:else if t.type === "trigger.mediaEnded"}
            {@render layerPickers(t.config, "Trigger")}
          {:else if t.type === "trigger.midi"}
            <MidiTriggerFields config={t.config} onchange={(c) => (t.config = c)} />
          {:else if t.type === "trigger.osc"}
            {@const c = t.config}
            <div class="input sm grow vf vf-within">
              <input
                class="mono"
                type="text"
                aria-label="OSC address"
                placeholder="/kinetra/go"
                bind:value={c.address}
                maxlength={200}
                autocomplete="off"
                spellcheck="false"
              />
            </div>
          {:else if t.type === "trigger.apiChanged"}
            {@const c = t.config}
            <Select
              size="sm"
              aria-label="Trigger API source"
              value={c.sourceId}
              options={withCurrent(P.apiOptions(lookups), c.sourceId, "Choose an API source")}
              onchange={(v) => {
                c.sourceId = v;
                c.column = null;
              }}
            />
            <Select
              size="sm"
              aria-label="Trigger column"
              value={c.column ?? ""}
              options={P.apiColumnOptions(lookups, c.sourceId, c.column ?? null)}
              onchange={(v) => (c.column = v === "" ? null : v)}
            />
          {:else if t.type === "trigger.slideshowWrap" || t.type === "trigger.sourceRow"}
            {@const c = t.config}
            <Select
              size="sm"
              aria-label="Trigger source"
              value={c.sourceId}
              options={withCurrent(textOptions, c.sourceId, "Choose a source")}
              onchange={(v) => (c.sourceId = v)}
            />
          {/if}
        </div>
        {#if t.type === "trigger.cue"}
          <p class="help" id="{uid}-label-help">Empty shows the name. Set the hotkey in Control.</p>
        {:else if t.type === "trigger.runByFlow"}
          <p class="help">Started by another flow's Run flow step.</p>
        {/if}
        {@render issueLine(triggerIssue(t, lookups))}
      </div>

      <div class="steps-head">
        <span class="section-label" id="{uid}-then">Then</span>
        <span class="help">Alt+↑/↓ moves a step.</span>
      </div>
      {#if draft.steps.length > 0}
        <ol class="steps" aria-labelledby="{uid}-then">
          {#each draft.steps as s, i (s.id)}
            {@const n = i + 1}
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <li class="step" data-step={i} onkeydown={(e) => onStepKey(e, i)}>
              <div class="row">
                <span class="num">{n}</span>
                <Select size="sm" aria-label="Step {n} type" value={s.type} options={STEP_OPTIONS} onchange={(v) => setStepType(i, v)} />
                {#if s.type === "action.take"}
                  {@const c = s.config}
                  {@render layerPickers(c, `Step ${n}`)}
                  <span class="muted">for</span>
                  <NumberInput
                    aria-label="Step {n} seconds on air (0 = stays)"
                    value={c.forMs ? c.forMs / 1000 : 0}
                    min={0}
                    max={TAKE_FOR_MS.max / 1000}
                    digits={4}
                    suffix="s"
                    onchange={(v) => (c.forMs = v > 0 ? Math.round(v * 1000) : null)}
                  />
                {:else if s.type === "action.clear" || s.type === "action.setFields"}
                  {@render layerPickers(s.config, `Step ${n}`)}
                {:else if s.type === "action.clearScreen"}
                  {@const c = s.config}
                  <Select
                    size="sm"
                    aria-label="Step {n} screen"
                    value={c.screenId}
                    options={withCurrent(screenOptions, c.screenId, "Choose a screen")}
                    onchange={(v) => (c.screenId = v)}
                  />
                {:else if s.type === "action.setTimer"}
                  {@const c = s.config}
                  {@render layerPickers(c, `Step ${n}`)}
                  <Select
                    size="sm"
                    aria-label="Step {n} timer field"
                    value={c.key}
                    options={withCurrent(timerKeys(c.screenId, c.layerId), c.key, "Choose a timer")}
                    onchange={(v) => (c.key = v)}
                  />
                  <Select
                    size="sm"
                    aria-label="Step {n} timer action"
                    value={c.action.action}
                    options={c.action.action === "configure" ? [...TIMER_ACTIONS, { value: "configure", label: "Set up" }] : TIMER_ACTIONS}
                    onchange={(v) => (c.action = timerFor(v, c.action))}
                  />
                  {#if c.action.action === "set"}
                    {@const a = c.action}
                    <NumberInput
                      aria-label="Step {n} timer seconds"
                      value={a.ms / 1000}
                      min={0}
                      max={86400}
                      digits={5}
                      suffix="s"
                      onchange={(v) => (a.ms = Math.round(v * 1000))}
                    />
                  {/if}
                {:else if s.type === "action.stepSource"}
                  {@const c = s.config}
                  <Select
                    size="sm"
                    aria-label="Step {n} source"
                    value={c.sourceId}
                    options={withCurrent(textOptions, c.sourceId, "Choose a source")}
                    onchange={(v) => (c.sourceId = v)}
                  />
                  <Select size="sm" aria-label="Step {n} direction" value={modeOf(c.step)} options={STEP_MODES} onchange={(m) => (c.step = stepFor(m))} />
                  {#if c.step.to === "index"}
                    <NumberInput
                      aria-label="Step {n} row number"
                      value={c.step.index + 1}
                      min={1}
                      max={100000}
                      digits={5}
                      onchange={(v) => (c.step = { to: "index", index: v - 1 })}
                    />
                  {/if}
                {:else if s.type === "action.pauseSource"}
                  {@const c = s.config}
                  <Select size="sm" aria-label="Step {n} pause or resume" value={c.mode} options={PAUSE} onchange={(v) => (c.mode = v)} />
                  <Select
                    size="sm"
                    aria-label="Step {n} source"
                    value={c.sourceId}
                    options={withCurrent(textOptions, c.sourceId, "Choose a source")}
                    onchange={(v) => (c.sourceId = v)}
                  />
                {:else if s.type === "action.schedule"}
                  {@const c = s.config}
                  <Select
                    size="sm"
                    aria-label="Step {n} schedule"
                    value={c.sourceId}
                    options={withCurrent(scheduleOptions, c.sourceId, "Choose a schedule")}
                    onchange={(v) => {
                      c.sourceId = v;
                      if (c.item.pick === "item") c.item = { pick: "now" };
                    }}
                  />
                  <Select
                    size="sm"
                    aria-label="Step {n} item"
                    value={pickValue(c.item)}
                    options={[
                      { value: "now", label: "The NOW item" },
                      { value: "next", label: "The UP NEXT item" },
                      ...(c.item.pick === "input" ? [{ value: "input", label: "The given item" }] : []),
                      ...withCurrent(scheduleItems[c.sourceId] ?? [], c.item.pick === "item" ? c.item.itemId : "", "").filter((o) => o.value !== "").map((o) => ({ ...o, value: `item:${o.value}` })),
                    ]}
                    onchange={(v) => (c.item = pickFor(v))}
                  />
                  <Select
                    size="sm"
                    aria-label="Step {n} schedule action"
                    value={c.op.op}
                    options={OPS}
                    onchange={(v) => (c.op = opFor(v, c.op))}
                  />
                  {#if c.op.op === "delay"}
                    {@const op = c.op}
                    <NumberInput
                      aria-label="Step {n} delay minutes"
                      value={op.minutes}
                      min={DELAY_MINUTES.min}
                      max={DELAY_MINUTES.max}
                      digits={5}
                      suffix="min"
                      onchange={(v) => (op.minutes = v)}
                    />
                  {:else if c.op.op === "setStart"}
                    {@const op = c.op}
                    <span class="tin vf vf-within">
                      <input
                        type="datetime-local"
                        class="mono"
                        aria-label="Step {n} new start"
                        value={op.start}
                        oninput={(e) => (op.start = e.currentTarget.value)}
                      />
                    </span>
                  {:else if c.op.op === "setNext"}
                    {@const op = c.op}
                    <span class="muted">at</span>
                    <span class="tin vf vf-within">
                      <input
                        type="datetime-local"
                        class="mono"
                        aria-label="Step {n} start (empty keeps its time)"
                        value={op.start ?? ""}
                        oninput={(e) => (op.start = e.currentTarget.value === "" ? null : e.currentTarget.value)}
                      />
                    </span>
                  {/if}
                {:else if s.type === "logic.delay"}
                  {@const c = s.config}
                  <NumberInput
                    aria-label="Step {n} seconds to wait"
                    value={c.ms / 1000}
                    min={0}
                    max={86400}
                    step={0.1}
                    digits={6}
                    suffix="s"
                    onchange={(v) => (c.ms = Math.round(v * 1000))}
                  />
                {:else if s.type === "action.setVariant"}
                  {@const c = s.config}
                  {@render layerPickers(c, `Step ${n}`)}
                  <Select
                    size="sm"
                    aria-label="Step {n} variant"
                    value={c.variant}
                    options={variantCache.pick(c.screenId, c.layerId, c.variant)}
                    onchange={(v) => (c.variant = v)}
                  />
                  <Select
                    size="sm"
                    aria-label="Step {n} transition while on air"
                    value={c.transition}
                    options={TRANSITIONS}
                    onchange={(v) => (c.transition = v)}
                  />
                {:else if s.type === "action.takePreview"}
                  {@const c = s.config}
                  <Select
                    size="sm"
                    aria-label="Step {n} screen"
                    value={c.screenId}
                    options={withCurrent(screenOptions, c.screenId, "Choose a screen")}
                    onchange={(v) => (c.screenId = v)}
                  />
                  <Select
                    size="sm"
                    aria-label="Step {n} transition"
                    value={pickOf(c.transition)}
                    options={TAKE_PICK}
                    onchange={(v) => (c.transition = transitionFor(v, c.transition))}
                  />
                  {#if c.transition?.kind === "crossfade"}
                    {@const t = c.transition}
                    <NumberInput
                      aria-label="Step {n} crossfade duration"
                      value={t.ms}
                      min={TAKE_MS.min}
                      max={TAKE_MS.max}
                      step={50}
                      digits={4}
                      suffix="ms"
                      onchange={(v) => (t.ms = v)}
                    />
                  {/if}
                {:else if s.type === "action.runFlow"}
                  {@const c = s.config}
                  <Select
                    size="sm"
                    aria-label="Step {n} flow"
                    value={c.flowId}
                    options={P.runFlowPick(lookups, selfId, c.flowId)}
                    onchange={(v) => (c.flowId = v)}
                  />
                  <Switch label="Wait until it finishes" checked={c.wait} onchange={(on) => (c.wait = on)} />
                {:else if s.type === "action.stopFlow"}
                  {@const c = s.config}
                  <Select
                    size="sm"
                    aria-label="Step {n} flow to stop"
                    value={c.flowId}
                    options={P.stopFlowPick(lookups, selfId, c.flowId)}
                    onchange={(v) => (c.flowId = v)}
                  />
                {:else if s.type === "action.media"}
                  {@const c = s.config}
                  {@render layerPickers(c, `Step ${n}`)}
                  <Select
                    size="sm"
                    aria-label="Step {n} media action"
                    value={c.action}
                    options={P.MEDIA_ACTIONS}
                    onchange={(v) => {
                      c.action = v;
                      c.ms = v === "seek" ? (c.ms ?? 0) : null;
                    }}
                  />
                  {#if c.action === "seek"}
                    <NumberInput
                      aria-label="Step {n} seconds into the file"
                      value={(c.ms ?? 0) / 1000}
                      min={0}
                      step={0.1}
                      digits={6}
                      suffix="s"
                      onchange={(v) => (c.ms = Math.round(v * 1000))}
                    />
                  {/if}
                {:else if s.type === "action.log"}
                  {@const c = s.config}
                  <div class="input sm grow vf vf-within">
                    <input type="text" aria-label="Step {n} message" bind:value={c.message} maxlength={500} autocomplete="off" />
                  </div>
                  <Switch label="Toast" checked={c.toast} onchange={(on) => (c.toast = on)} />
                {/if}
                <span class="tools">
                  <IconButton size="sm" label="Move step {n} up" tooltip="top" disabled={i === 0} onclick={() => moveStep(i, -1)}>
                    <ArrowUp size={14} />
                  </IconButton>
                  <IconButton
                    size="sm"
                    label="Move step {n} down"
                    tooltip="top"
                    disabled={i === draft.steps.length - 1}
                    onclick={() => moveStep(i, 1)}
                  >
                    <ArrowDown size={14} />
                  </IconButton>
                  <IconButton variant="danger" size="sm" label="Delete step {n}" tooltip="left" onclick={() => draft?.steps.splice(i, 1)}>
                    <Trash2 size={14} />
                  </IconButton>
                </span>
              </div>

              {#if s.type === "action.setFields"}
                {@const c = s.config}
                {@const defs = fieldDefsOf(c.screenId, c.layerId)}
                {@const used = Object.keys(c.fields)}
                <div class="pairs">
                  {#each Object.entries(c.fields) as [k, v] (k)}
                    {@const opts = withCurrent(
                      defs.filter((f) => f.key === k || !used.includes(f.key)).map((f) => ({ value: f.key, label: f.label || f.key })),
                      k,
                      "Choose a field",
                    )}
                    <div class="pair">
                      <Select size="sm" aria-label="Step {n} field" value={k} options={opts} onchange={(to) => renameField(c, k, to)} />
                      <div class="input sm grow vf vf-within">
                        <input
                          type="text"
                          aria-label="Step {n} value of {k}"
                          value={v}
                          oninput={(e) => (c.fields = { ...c.fields, [k]: e.currentTarget.value })}
                          autocomplete="off"
                        />
                      </div>
                      <IconButton variant="danger" size="sm" label="Remove field {k} from step {n}" tooltip="left" onclick={() => removeField(c, k)}>
                        <Trash2 size={14} />
                      </IconButton>
                    </div>
                  {/each}
                  <Button size="sm" variant="ghost" disabled={defs.every((f) => used.includes(f.key))} onclick={() => addField(c)}>
                    {#snippet icon()}<Plus size={14} />{/snippet}
                    Add field
                  </Button>
                </div>
              {/if}
              {@render issueLine(stepIssue(s, lookups, selfId))}
            </li>
          {/each}
        </ol>
      {:else}
        <p class="muted">Add steps: take, clear, wait, step a source, run a flow.</p>
      {/if}
      <div>
        <Button size="sm" onclick={addStep}>
          {#snippet icon()}<Plus size={14} />{/snippet}
          Add step
        </Button>
      </div>
    {/if}

    {#if error}<p class="error" role="alert">{error}</p>{/if}

    <div class="actions">
      <span class="summary" role="status">
        {#if draft && result.errors > 0}
          <span class="error">{result.errors} {result.errors === 1 ? "problem" : "problems"} to fix</span>
        {:else if result.warnings > 0}
          <span class="warning">{result.warnings} {result.warnings === 1 ? "target is" : "targets are"} missing and will be skipped</span>
        {/if}
      </span>
      <Button variant="ghost" onclick={cancel} disabled={saving}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={result.errors > 0 && !saving} loading={saving}>Save</Button>
    </div>
  </form>
</dialog>

<style>
  .dialog {
    width: min(960px, calc(100vw - 2 * var(--s-6)));
    max-height: calc(100vh - 2 * var(--s-6));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: auto;
  }
  .dialog[open] {
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
    animation: fade var(--dur) var(--ease);
  }
  .inner {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-5);
  }
  .title {
    margin-bottom: var(--s-1);
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .name-row {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  label {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    flex: 1;
    min-width: 0;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input.sm {
    height: var(--control-h-sm);
  }
  .input.grow {
    min-width: 10rem;
  }
  .input input {
    flex: 1;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
  }
  .block .input {
    background: var(--surface-2);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-2);
  }
  .tin {
    --vf-radius: var(--r-control);
    display: inline-flex;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .tin input {
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    color: var(--text);
  }
  .days {
    display: inline-flex;
    gap: 2px;
  }
  .day {
    --vf-radius: var(--r-control);
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .day[aria-pressed="true"] {
    background: var(--accent-soft);
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .steps-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding-top: var(--s-2);
  }
  .steps {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .step {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding: var(--s-1) var(--s-1) var(--s-1) 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
  }
  .step .input {
    background: var(--surface-2);
  }
  .num {
    width: 3ch;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  /* Move / delete stay together at the end of the row, even when the step wraps. */
  .tools {
    display: inline-flex;
    flex: none;
    margin-left: auto;
  }
  .pairs {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-1);
    padding-left: calc(3ch + var(--s-2));
  }
  .pair {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    width: 100%;
  }
  .issue {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    margin: 0;
    padding-left: calc(3ch + var(--s-2));
    font-size: var(--fs-xs);
  }
  .block .issue {
    padding-left: 0;
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .issue.error {
    font-size: var(--fs-xs);
  }
  .warning {
    color: var(--warning);
  }
  .muted,
  .help {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .help {
    font-size: var(--fs-xs);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding-top: var(--s-4);
    border-top: 1px solid var(--line);
  }
  .summary {
    flex: 1;
    font-size: var(--fs-sm);
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dialog[open],
    .dialog::backdrop {
      animation: none;
    }
  }
</style>
