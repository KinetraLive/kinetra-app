<!--
  PanelEditor: the project's button panel for remote control (protocol 2 §4.3; a lazy chunk, a
  native modal <dialog>; Simple and Advanced, also with remote control off). Collaborators with
  the button panel permission press these on the control site; the actions never leave the app.
    list     the buttons in order (swatch, label, a mark when something is missing); Add button,
             move the selected one up / down
    button   label (1–40), color, Ask to confirm, 1–8 actions in order (kind, target, details;
             move, remove), Delete button
  Saved ~0.6 s after a change once every button is complete (a label and at least one action);
  a step whose target is gone shows the core's words (`PanelView.issues`). Props: open, onclose.
-->
<script lang="ts">
  import { backdropClose } from "$lib/components/backdrop";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Trash from "lucide-svelte/icons/trash-2";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import { getPanel, onPanelChanged, panelTargets, savePanel } from "$lib/api/panel";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Select from "$lib/components/Select.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import FieldInputs from "$lib/components/fields/FieldInputs.svelte";
  import type { PanelAction, PanelActionKind, PanelButton, PanelIssue, PanelTargets } from "$lib/types";
  import { PANEL_ACTIONS_MAX, PANEL_BUTTONS_MAX, PANEL_COLORS, PANEL_LABEL_MAX } from "$lib/types";
  import {
    available,
    COLOR_LABELS,
    KIND_LABELS,
    layersFor,
    move,
    newAction,
    newButton,
    problem,
    retarget,
    settableFields,
    timerFields,
  } from "./model";

  let { open, onclose }: { open: boolean; onclose: () => void } = $props();

  const id = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let targets = $state.raw<PanelTargets | null>(null);
  let buttons = $state<PanelButton[]>([]);
  let issues = $state.raw<PanelIssue[]>([]);
  let selected = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  const current = $derived(buttons.find((b) => b.id === selected) ?? null);
  const currentProblem = $derived(current ? problem(current) : null);
  const kinds = $derived(
    targets ? (Object.keys(KIND_LABELS) as PanelActionKind[]).filter((k) => available(targets!, k)) : [],
  );

  async function load() {
    loadError = null;
    try {
      const [view, t] = await Promise.all([getPanel(), panelTargets()]);
      targets = t;
      buttons = view.doc.buttons;
      issues = view.issues;
      if (!buttons.some((b) => b.id === selected)) selected = buttons[0]?.id ?? null;
    } catch (e) {
      loadError = toAppError(e).message;
    }
  }

  $effect(() => {
    const d = dialog;
    if (!d) return;
    if (open && !d.open) {
      if (typeof d.showModal === "function") d.showModal();
      else d.setAttribute("open", "");
      void load();
    } else if (!open && d.open) {
      if (typeof d.close === "function") d.close();
      else d.removeAttribute("open");
    }
  });

  // An outside edit, a restore or a project switch while open: take it, unless a save is waiting.
  $effect(() => {
    if (!open) return;
    return onPanelChanged((v) => {
      if (timer) return;
      buttons = v.doc.buttons;
      issues = v.issues;
      if (!buttons.some((b) => b.id === selected)) selected = buttons[0]?.id ?? null;
    });
  });

  async function save() {
    timer = null;
    if (buttons.some((b) => problem(b) !== null)) return;
    try {
      const v = await savePanel({ buttons: $state.snapshot(buttons) });
      issues = v.issues;
      saveError = null;
    } catch (e) {
      saveError = toAppError(e).message;
    }
  }

  function changed() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void save(), 600);
  }

  function close() {
    if (timer) {
      clearTimeout(timer);
      void save();
    }
    onclose();
  }

  function onCancelEvent(e: Event) {
    e.preventDefault();
    close();
  }

  function add() {
    if (!targets) return;
    const b = newButton(targets, buttons);
    if (!b) return;
    buttons.push(b);
    selected = b.id;
    changed();
  }

  function remove(b: PanelButton) {
    const i = buttons.findIndex((x) => x.id === b.id);
    buttons.splice(i, 1);
    selected = buttons[Math.min(i, buttons.length - 1)]?.id ?? null;
    changed();
  }

  function shift(b: PanelButton, by: number) {
    const i = buttons.findIndex((x) => x.id === b.id);
    buttons = move(buttons, i, i + by);
    changed();
  }

  function setKind(b: PanelButton, i: number, kind: PanelActionKind) {
    const a = targets && newAction(targets, kind);
    if (!a) return;
    b.actions[i] = a;
    changed();
  }

  function addAction(b: PanelButton) {
    const a = targets && kinds[0] ? newAction(targets, kinds[0]) : null;
    if (!a || b.actions.length >= PANEL_ACTIONS_MAX) return;
    b.actions.push(a);
    changed();
  }

  const issuesOf = (b: PanelButton) => issues.filter((x) => x.button === b.id);
  const marked = (b: PanelButton) => problem(b) !== null || issuesOf(b).length > 0;

  const layerValue = (a: PanelAction) => ("layer" in a ? `${a.screen}/${a.layer}` : "");
  function layerOptions(a: PanelAction) {
    const opts = targets
      ? layersFor(targets, a.kind).map((r) => ({ value: `${r.screen}/${r.layer.id}`, label: `${r.screenName} · ${r.layer.name}` }))
      : [];
    const v = layerValue(a);
    return opts.some((o) => o.value === v) ? opts : [{ value: v, label: "Missing layer", disabled: true }, ...opts];
  }
  const layerOf = (a: PanelAction) =>
    "layer" in a ? targets?.screens.find((s) => s.id === a.screen)?.layers.find((l) => l.id === a.layer) ?? null : null;

  function withMissing(opts: { value: string; label: string }[], v: string, missing: string) {
    return opts.some((o) => o.value === v) ? opts : [{ value: v, label: missing, disabled: true }, ...opts];
  }

  const OPS = [
    { value: "start", label: "Start" },
    { value: "pause", label: "Pause" },
    { value: "reset", label: "Reset" },
  ] as const;
  const MODES = [
    { value: "take", label: "Take (the project's transition)" },
    { value: "cut", label: "Cut" },
  ] as const;
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{id}-title"
  oncancel={onCancelEvent}
  {@attach backdropClose(() => close())}
>
  <header class="head">
    <h2 id="{id}-title" class="title">Button panel</h2>
    <IconButton label="Close" onclick={close}><X size={16} /></IconButton>
  </header>

  {#if loadError}
    <div class="body">
      <p class="error" role="alert">{loadError}</p>
      <Button onclick={load}>Try again</Button>
    </div>
  {:else if targets}
    <div class="body split">
      <section class="list" aria-label="Buttons">
        {#if buttons.length === 0}
          <p class="muted">No buttons yet.</p>
        {:else}
          <ul class="items">
            {#each buttons as b (b.id)}
              <li>
                <button
                  type="button"
                  class="item vf"
                  data-selected={b.id === selected ? "" : undefined}
                  aria-current={b.id === selected ? "true" : undefined}
                  onclick={() => (selected = b.id)}
                >
                  <span class="swatch" data-color={b.color} aria-hidden="true"></span>
                  <span class="label">{b.label.trim() || "Untitled"}</span>
                  {#if marked(b)}
                    <span class="warn" title="Needs attention"><TriangleAlert size={14} aria-label="Needs attention" /></span>
                  {/if}
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="row">
          <Button onclick={add} disabled={buttons.length >= PANEL_BUTTONS_MAX || kinds.length === 0}>Add button</Button>
          {#if current}
            <IconButton label="Move up" disabled={buttons[0]?.id === current.id} onclick={() => shift(current!, -1)}><ArrowUp size={16} /></IconButton>
            <IconButton label="Move down" disabled={buttons.at(-1)?.id === current.id} onclick={() => shift(current!, 1)}><ArrowDown size={16} /></IconButton>
          {/if}
        </div>
        {#if kinds.length === 0}
          <p class="muted">Add a screen with a layer first.</p>
        {:else}
          <p class="muted tnum">{buttons.length} of {PANEL_BUTTONS_MAX}</p>
        {/if}
      </section>

      {#if current}
        {@const b = current}
        <section class="form" aria-label="Button">
          <div class="field">
            <label class="section-label" for="{id}-label">Label</label>
            <div class="input-wrap vf vf-within">
              <input
                id="{id}-label"
                class="input"
                maxlength={PANEL_LABEL_MAX}
                bind:value={b.label}
                oninput={changed}
                autocomplete="off"
              />
            </div>
          </div>

          <div class="field">
            <span class="section-label" id="{id}-color">Color</span>
            <div class="swatches" role="radiogroup" aria-labelledby="{id}-color">
              {#each PANEL_COLORS as c (c)}
                <button
                  type="button"
                  role="radio"
                  class="pick vf"
                  aria-checked={b.color === c}
                  aria-label={COLOR_LABELS[c]}
                  title={COLOR_LABELS[c]}
                  onclick={() => {
                    b.color = c;
                    changed();
                  }}
                >
                  <span class="swatch" data-color={c}></span>
                </button>
              {/each}
            </div>
          </div>

          <div class="inline">
            <Switch
              label="Ask to confirm"
              checked={b.confirm}
              onchange={(on) => {
                b.confirm = on;
                changed();
              }}
            />
          </div>

          <div class="field">
            <span class="section-label">Actions</span>
            <ol class="steps">
              {#each b.actions as a, i (i)}
                {@const missing = issuesOf(b).find((x) => x.step === i + 1)}
                <li class="step">
                  <span class="num mono">{i + 1}</span>
                  <div class="what">
                    <Select
                      aria-label="Action {i + 1}"
                      value={a.kind}
                      options={kinds.map((k) => ({ value: k, label: KIND_LABELS[k] }))}
                      onchange={(k) => setKind(b, i, k)}
                    />
                    {#if a.kind === "take" || a.kind === "clear" || a.kind === "setFields" || a.kind === "setVariant" || a.kind === "timer"}
                      <Select
                        aria-label="Layer"
                        value={layerValue(a)}
                        options={layerOptions(a)}
                        onchange={(v) => {
                          const [s, l] = v.split("/");
                          b.actions[i] = retarget(targets!, a, s!, l!);
                          changed();
                        }}
                      />
                    {/if}
                    {#if a.kind === "setVariant"}
                      {@const l = layerOf(a)}
                      <Select
                        aria-label="Look"
                        value={a.variant}
                        options={withMissing((l?.variants ?? []).map((v) => ({ value: v.id, label: v.label })), a.variant, "Missing look")}
                        onchange={(v) => {
                          a.variant = v;
                          changed();
                        }}
                      />
                    {:else if a.kind === "timer"}
                      {@const l = layerOf(a)}
                      <Select
                        aria-label="Timer"
                        value={a.field}
                        options={withMissing((l ? timerFields(l) : []).map((f) => ({ value: f.key, label: f.label })), a.field, "Missing timer")}
                        onchange={(v) => {
                          a.field = v;
                          changed();
                        }}
                      />
                      <Select
                        aria-label="Timer action"
                        value={a.op}
                        options={[...OPS]}
                        onchange={(v) => {
                          a.op = v;
                          changed();
                        }}
                      />
                    {:else if a.kind === "runCue"}
                      <Select
                        aria-label="Cue"
                        value={a.flow}
                        options={withMissing(targets.cues.map((c) => ({ value: c.id, label: c.label })), a.flow, "Missing cue")}
                        onchange={(v) => {
                          a.flow = v;
                          changed();
                        }}
                      />
                    {:else if a.kind === "selectRow"}
                      {@const src = targets.sources.find((s) => s.id === a.source)}
                      <Select
                        aria-label="Data source"
                        value={a.source}
                        options={withMissing(
                          targets.sources.filter((s) => s.rows.length > 0).map((s) => ({ value: s.id, label: s.name })),
                          a.source,
                          "Missing data source",
                        )}
                        onchange={(v) => {
                          const s = targets!.sources.find((x) => x.id === v);
                          b.actions[i] = { kind: "selectRow", source: v, row: s?.rows[0]?.id ?? "" };
                          changed();
                        }}
                      />
                      <Select
                        aria-label="Row"
                        value={a.row}
                        options={withMissing((src?.rows ?? []).map((r) => ({ value: r.id, label: r.label })), a.row, "Missing row")}
                        onchange={(v) => {
                          a.row = v;
                          changed();
                        }}
                      />
                    {:else if a.kind === "studioTake"}
                      <Select
                        aria-label="Screen"
                        value={a.screen}
                        options={withMissing(targets.screens.map((s) => ({ value: s.id, label: s.name })), a.screen, "Missing screen")}
                        onchange={(v) => {
                          a.screen = v;
                          changed();
                        }}
                      />
                      <Select
                        aria-label="Transition"
                        value={a.mode}
                        options={[...MODES]}
                        onchange={(v) => {
                          a.mode = v;
                          changed();
                        }}
                      />
                    {/if}
                    {#if a.kind === "setFields"}
                      {@const l = layerOf(a)}
                      {#if l}
                        <div class="values">
                          <p class="muted">Only the fields you change are set.</p>
                          <FieldInputs
                            defs={settableFields(l)}
                            values={a.values}
                            onfield={(k, v) => {
                              a.values[k] = v;
                              changed();
                            }}
                          />
                        </div>
                      {/if}
                    {/if}
                    {#if missing}
                      <p class="issue" role="note"><TriangleAlert size={14} aria-hidden="true" /> {missing.message}</p>
                    {/if}
                  </div>
                  <div class="tools">
                    <IconButton
                      label="Move action {i + 1} up"
                      size="sm"
                      disabled={i === 0}
                      onclick={() => {
                        b.actions = move(b.actions, i, i - 1);
                        changed();
                      }}><ArrowUp size={14} /></IconButton
                    >
                    <IconButton
                      label="Move action {i + 1} down"
                      size="sm"
                      disabled={i === b.actions.length - 1}
                      onclick={() => {
                        b.actions = move(b.actions, i, i + 1);
                        changed();
                      }}><ArrowDown size={14} /></IconButton
                    >
                    <IconButton
                      label="Remove action {i + 1}"
                      size="sm"
                      onclick={() => {
                        b.actions.splice(i, 1);
                        changed();
                      }}><Trash size={14} /></IconButton
                    >
                  </div>
                </li>
              {/each}
            </ol>
            <div class="row">
              <Button size="sm" onclick={() => addAction(b)} disabled={b.actions.length >= PANEL_ACTIONS_MAX}>Add action</Button>
              <span class="muted tnum">{b.actions.length} of {PANEL_ACTIONS_MAX}</span>
            </div>
          </div>

          {#if currentProblem}
            <p class="issue" role="status">{currentProblem}</p>
          {/if}
          {#if saveError}
            <p class="error" role="alert">{saveError}</p>
          {/if}

          <div class="row end">
            <Button variant="danger" onclick={() => remove(b)}>Delete button</Button>
          </div>
        </section>
      {:else}
        <section class="form empty">
          <p class="muted">Buttons appear on the control site for collaborators allowed to press them.</p>
        </section>
      {/if}
    </div>
  {/if}
</dialog>

<style>
  .dialog {
    width: min(960px, calc(100vw - 2 * var(--s-6)));
    height: min(680px, calc(100vh - 2 * var(--s-6)));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: hidden;
    display: none;
    flex-direction: column;
  }
  .dialog[open] {
    display: flex;
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--s-3) var(--s-3) var(--s-3) var(--s-5);
    border-bottom: 1px solid var(--line);
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4) var(--s-5);
    min-height: 0;
    flex: 1;
  }
  .split {
    display: grid;
    grid-template-columns: minmax(200px, 260px) 1fr;
    gap: 0;
    padding: 0;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-3);
    border-right: 1px solid var(--line);
    background: var(--surface-1);
    overflow: auto;
    min-height: 0;
  }
  .items {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 100%;
    height: var(--row-h);
    padding: 0 var(--s-2);
    border: 1px solid transparent;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .item:hover {
    background: var(--surface-2);
  }
  .item[data-selected] {
    background: var(--selected-bg);
    border-color: var(--accent);
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warn {
    display: inline-flex;
    color: var(--warning);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    padding: var(--s-4) var(--s-5);
    overflow: auto;
    min-height: 0;
  }
  .form.empty {
    justify-content: center;
    align-items: center;
  }
  .field {
    display: grid;
    gap: var(--s-2);
  }
  .input-wrap {
    display: flex;
    border-radius: var(--r-control);
    max-width: 360px;
  }
  .input {
    flex: 1;
    outline: none;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text);
    font: inherit;
  }
  .swatches {
    display: flex;
    gap: var(--s-1);
  }
  .pick {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: 1px solid transparent;
    border-radius: var(--r-control);
    background: transparent;
    cursor: pointer;
  }
  .pick[aria-checked="true"] {
    border-color: var(--accent);
    background: var(--selected-bg);
  }
  .swatch {
    flex: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px var(--line);
  }
  /* The button colors as the control site shows them (user content, not status). */
  .swatch[data-color="grey"] {
    background: #8a8f98;
  }
  .swatch[data-color="red"] {
    background: #d9534f;
  }
  .swatch[data-color="orange"] {
    background: #e08a3c;
  }
  .swatch[data-color="yellow"] {
    background: #d8b93a;
  }
  .swatch[data-color="green"] {
    background: #4caf72;
  }
  .swatch[data-color="blue"] {
    background: #4a8fd9;
  }
  .swatch[data-color="purple"] {
    background: #9a6ad6;
  }
  .inline {
    display: flex;
  }
  .steps {
    display: grid;
    gap: var(--s-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .step {
    display: grid;
    grid-template-columns: 24px 1fr auto;
    gap: var(--s-2);
    align-items: start;
    padding: var(--s-2);
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background: var(--surface-1);
  }
  .num {
    line-height: var(--control-h);
    color: var(--text-muted);
    text-align: center;
  }
  .what {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
    min-width: 0;
  }
  .values {
    flex-basis: 100%;
    display: grid;
    gap: var(--s-2);
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .row.end {
    justify-content: flex-end;
    margin-top: auto;
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
  }
  .issue {
    display: flex;
    flex-basis: 100%;
    align-items: center;
    gap: var(--s-1);
    margin: 0;
    color: var(--warning);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  @media (max-width: 720px) {
    .split {
      grid-template-columns: 1fr;
    }
    .list {
      border-right: 0;
      border-bottom: 1px solid var(--line);
    }
  }
</style>
