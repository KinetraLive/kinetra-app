<!--
  FieldInputs: one input per template field, by type, in a two-column label/input grid. Shared by
  the layout editor's Inspector, Control and the template editor's preview values.

  Inputs by type: multiline / html → textarea (html in mono); color → colour picker + mono hex;
  text → text input; number → mono text input with a decimal keypad; image / video → mono text
  input (a URL) plus ImageChoose (a lazy chunk): thumbnail (images), "Choose…" (the asset
  picker; the media library for video) and Clear;
  list → ListField (a lazy chunk): one line, "6 items · Edit" (the count in mono), which opens the
  spreadsheet drawer on the rows; linked to a source (`linked`): "Linked to: <name> · Edit source",
  which opens the drawer on that data table; clock → "Time of day" and the format in mono, read-only (the
  template sets it); timer → TimerControls (a lazy chunk): live display, Start / Pause, Reset,
  Set (mm:ss), direction and, counting down, what happens at zero.

  Presentational: values in, `onfield(key, value)` / `ontimer(key, action)` out.

  Props:
    defs: FieldDef[]
    values: Record<string, string>     current value per key (missing → the field's default)
    bound?: Record<string, string>     key → what it is bound to (e.g. "Guests · Name"): shown
                                       read-only instead of an input (a focusable read-only
                                       textbox: the value, then the binding in mono). A bound
                                       list shows only the binding ("Guests · all rows, sorted
                                       by Score ↓, top 10"); a bound timer too ("Schedule · Up
                                       next · Countdown").
    linked?: Record<string, LinkedList>  list key → the source it is linked to (a bound list with
                                       an entry shows "Linked to: …" instead of the binding)
    disabled?: boolean
    onfield: (key, value) => void      list values arrive as JSON; timers without `ontimer` too
    onfocus?: (key) => void
    onblur?: (key) => void
    extra?: Snippet<[FieldDef]>        rendered after each field's input (e.g. Bind, Prev/Next)
    ontimer?: (key, action: TimerAction) => void
                                       timer fields send one action per control (Control and the
                                       Inspector call setTimer). Without it a timer applies the
                                       action locally and sends the new state via onfield.
-->
<script module lang="ts">
  // List and timer fields are their own chunks, loaded only when such a field shows: this
  // component stays small in every route that uses it.
  let listChunk: Promise<typeof import("./ListField.svelte")> | null = null;
  let timerChunk: Promise<typeof import("./TimerControls.svelte")> | null = null;
  const loadList = () => (listChunk ??= import("./ListField.svelte"));
  const loadTimer = () => (timerChunk ??= import("./TimerControls.svelte"));
  let imageChunk: Promise<typeof import("./ImageChoose.svelte")> | null = null;
  const loadImage = () => (imageChunk ??= import("./ImageChoose.svelte"));

  /** `#rrggbb` part of a colour for `<input type="color">` (black when invalid). Kept here, not
   * imported from the template editor's model, so Control doesn't load that module. */
  const pickerHex = (v: string) => (/^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$/.test(v) ? v.slice(0, 7).toLowerCase() : "#000000");
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import type { FieldDef, TimerAction } from "$lib/types";
  import type { LinkedList } from "./drawer";

  let {
    defs,
    values,
    bound = {},
    linked = {},
    disabled = false,
    onfield,
    onfocus,
    onblur,
    extra,
    ontimer,
  }: {
    defs: readonly FieldDef[];
    values: Readonly<Record<string, string>>;
    bound?: Readonly<Record<string, string>>;
    linked?: Readonly<Record<string, LinkedList>>;
    disabled?: boolean;
    onfield: (key: string, value: string) => void;
    onfocus?: (key: string) => void;
    onblur?: (key: string) => void;
    extra?: Snippet<[FieldDef]>;
    ontimer?: (key: string, action: TimerAction) => void;
  } = $props();

  const uid = $props.id();
  const valueOf = (f: FieldDef) => values[f.key] ?? f.default;

  const timerSender = (key: string) => (ontimer ? (a: TimerAction) => ontimer(key, a) : undefined);
</script>

<div class="grid">
  {#each defs as f, i (f.key)}
    {@const id = `${uid}-f${i}`}
    {@const value = valueOf(f)}
    {@const link = f.type === "list" ? linked[f.key] : undefined}
    {@const isBound = bound[f.key] !== undefined && !link}
    {@const composite = f.type === "list" || f.type === "clock" || f.type === "timer"}
    <label id="{id}-label" for={isBound || composite ? undefined : id} class:top={composite && !isBound}>
      {f.label || f.key}
    </label>
    <div class="cell" class:wrap={f.type === "list"}>
      {#if isBound}
        <!-- A bound list or timer shows only its binding. -->
        {@const list = f.type === "list" || f.type === "timer"}
        <span
          {id}
          class="bound vf"
          role="textbox"
          aria-readonly="true"
          aria-labelledby="{id}-label"
          aria-describedby={list ? undefined : `${id}-to`}
          tabindex="0"
          title={list ? bound[f.key] : value}
        >
          <span class="bound-value">{list ? bound[f.key] : value}</span>
          {#if !list}<span class="bound-to mono" id="{id}-to">{bound[f.key]}</span>{/if}
        </span>
      {:else if f.type === "list"}
        {#await loadList() then { default: ListField }}
          <ListField
            {id}
            label={f.label || f.key}
            columns={f.columns ?? []}
            {value}
            linked={link ?? null}
            {disabled}
            onchange={(v) => onfield(f.key, v)}
          />
        {/await}
      {:else if f.type === "clock"}
        <span class="readonly" {id} role="group" aria-labelledby="{id}-label">
          <span>Time of day</span>
          <span class="mono">{f.format || "HH:mm:ss"}</span>
        </span>
      {:else if f.type === "timer"}
        <span class="timer" {id} role="group" aria-labelledby="{id}-label">
          {#await loadTimer() then { default: TimerControls }}
            <TimerControls
              label={f.label || f.key}
              def={f}
              {value}
              {disabled}
              ontimer={timerSender(f.key)}
              onvalue={(v) => onfield(f.key, v)}
            />
          {/await}
        </span>
      {:else if f.type === "multiline" || f.type === "html"}
        <span class="input vf vf-within">
          <textarea
            {id}
            rows="2"
            class:mono={f.type === "html"}
            {value}
            {disabled}
            oninput={(e) => onfield(f.key, e.currentTarget.value)}
            onfocus={() => onfocus?.(f.key)}
            onblur={() => onblur?.(f.key)}
          ></textarea>
        </span>
      {:else if f.type === "color"}
        <span class="color">
          <span class="swatch vf vf-within">
            <input
              type="color"
              aria-label="{f.label || f.key} color picker"
              value={pickerHex(value)}
              {disabled}
              oninput={(e) => onfield(f.key, e.currentTarget.value)}
              onfocus={() => onfocus?.(f.key)}
              onblur={() => onblur?.(f.key)}
            />
          </span>
          <span class="input vf vf-within">
            <input
              {id}
              class="mono"
              type="text"
              spellcheck="false"
              autocomplete="off"
              maxlength="9"
              {value}
              {disabled}
              oninput={(e) => onfield(f.key, e.currentTarget.value.trim())}
              onfocus={() => onfocus?.(f.key)}
              onblur={() => onblur?.(f.key)}
            />
          </span>
        </span>
      {:else}
        <span class="input vf vf-within">
          <input
            {id}
            type="text"
            autocomplete="off"
            spellcheck={f.type === "text"}
            class:mono={f.type !== "text"}
            inputmode={f.type === "number" ? "decimal" : undefined}
            {value}
            {disabled}
            oninput={(e) => onfield(f.key, e.currentTarget.value)}
            onfocus={() => onfocus?.(f.key)}
            onblur={() => onblur?.(f.key)}
          />
        </span>
      {/if}
      {#if (f.type === "image" || f.type === "video") && !isBound}
        {#await loadImage() then { default: ImageChoose }}
          <ImageChoose label={f.label || f.key} kind={f.type} {value} {disabled} onchange={(v) => onfield(f.key, v)} />
        {/await}
      {/if}
      {#if extra}{@render extra(f)}{/if}
    </div>
  {/each}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: minmax(5rem, auto) minmax(0, 1fr);
    gap: var(--s-2) var(--s-3);
    align-items: center;
  }
  .grid > label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .cell > :first-child {
    flex: 1;
  }
  .grid > label.top {
    align-self: start;
    line-height: var(--control-h);
  }
  .cell.wrap {
    flex-wrap: wrap;
  }
  .readonly {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    min-height: var(--control-h);
    color: var(--text-muted);
  }
  .readonly .mono {
    color: var(--text);
  }
  .timer {
    min-width: 0;
    padding: var(--s-1) 0;
  }
  .input {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .input input,
  .input textarea {
    width: 100%;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input {
    height: var(--control-h);
  }
  .input textarea {
    padding: var(--s-1) var(--s-2);
    resize: vertical;
  }
  .color {
    display: flex;
    gap: var(--s-2);
    min-width: 0;
  }
  .color .input {
    flex: 1;
  }
  .swatch {
    display: inline-flex;
    flex: none;
    border-radius: var(--r-control);
  }
  input[type="color"] {
    width: var(--control-h);
    height: var(--control-h);
    padding: 2px;
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    cursor: pointer;
  }
  input:disabled,
  textarea:disabled {
    opacity: 0.6;
  }
  .bound {
    display: flex;
    flex-direction: column;
    justify-content: center;
    min-width: 0;
    min-height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  .bound-value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bound-to {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
</style>
