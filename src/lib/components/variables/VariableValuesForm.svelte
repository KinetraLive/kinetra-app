<!--
  VariableValuesForm: the no-code values of a template's variables, one row per variable (colour
  picker + hex + "Transparent" toggle, font list, slider + number, option list). Shared by the template editor (where a
  value is the template default) and the screen editor's inspector (where a value is a per-layer
  override of that default).

  Props:
    variables: VariableDef[]
    values?: Record<string, string>     effective value per key; a key that is absent shows the
                                        variable's own `default` (the template editor passes none)
    overrides?: Record<string, string>  when given, every key present here is marked "Overrides
                                        template" with a Reset button (screen editor)
    errors?: (string | null)[]          value error per variable index, shown under the row
    onvalue: (key: string, value: string, index: number) => void
    onreset?: (key: string) => void     Reset of an overridden key

  Colours: "Transparent" sets `transparent` (the swatch shows the checkerboard); picking or typing a
  colour turns it off, and so does the toggle (back to the last colour shown, else the definition
  default, else black).

  Renders only the rows (no heading); an empty list renders an empty container.
-->
<script lang="ts">
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import Button from "$lib/components/Button.svelte";
  import NumberInput from "$lib/components/NumberInput.svelte";
  import Select from "$lib/components/Select.svelte";
  import { TEMPLATE_FONTS, type VariableDef } from "$lib/types";
  import { TRANSPARENT, alphaOf, isHex, isTransparent, normalizeColor, pickerHex } from "./color";
  import TransparentToggle from "./TransparentToggle.svelte";

  let {
    variables,
    values,
    overrides,
    errors,
    onvalue,
    onreset,
  }: {
    variables: readonly VariableDef[];
    values?: Readonly<Record<string, string>>;
    overrides?: Readonly<Record<string, string>>;
    errors?: ReadonlyArray<string | null>;
    onvalue: (key: string, value: string, index: number) => void;
    onreset?: (key: string) => void;
  } = $props();

  const uid = $props.id();
  const FONT_OPTIONS = TEMPLATE_FONTS.map((f) => ({ value: f.name, label: f.name }));

  const valueOf = (v: VariableDef): string => values?.[v.key] ?? v.default;

  /** The last hex colour per key before it was made transparent (what turning it off restores). */
  const lastHex = new Map<string, string>();

  function toggleTransparent(v: VariableDef, value: string, i: number) {
    if (isTransparent(value)) {
      onvalue(v.key, lastHex.get(v.key) ?? (isHex(v.default) ? v.default : "#000000"), i);
    } else {
      if (isHex(value)) lastHex.set(v.key, value);
      onvalue(v.key, TRANSPARENT, i);
    }
  }

  function rangeNumber(v: VariableDef & { type: "range" }, raw: string): number {
    const n = raw.trim() === "" ? NaN : Number(raw);
    return Number.isFinite(n) ? n : v.min;
  }
</script>

<div class="values">
  {#each variables as v, i}
    {@const value = valueOf(v)}
    {@const err = errors?.[i] ?? null}
    {@const overridden = overrides !== undefined && Object.hasOwn(overrides, v.key)}
    {@const id = `${uid}-v${i}`}
    <div class="value-row" data-key={v.key}>
      <label class="vlabel" for={id}>{v.label || v.key}</label>
      <div class="control">
        {#if v.type === "color"}
          {@const clear = isTransparent(value)}
          <span class="swatch vf vf-within" class:checker={clear} class:clear>
            <input
              type="color"
              aria-label="{v.label || v.key} color picker"
              value={pickerHex(value)}
              oninput={(e) => {
                // Keep an alpha suffix (#rrggbbaa) when present.
                onvalue(v.key, e.currentTarget.value + alphaOf(value), i);
              }}
            />
          </span>
          <span class="text vf vf-within">
            <input
              {id}
              class="mono hex"
              type="text"
              spellcheck="false"
              autocomplete="off"
              maxlength="11"
              {value}
              aria-invalid={err ? "true" : undefined}
              aria-describedby={err ? `${id}-err` : undefined}
              oninput={(e) => onvalue(v.key, normalizeColor(e.currentTarget.value.trim()), i)}
            />
          </span>
          <TransparentToggle pressed={clear} label={v.label || v.key} ontoggle={() => toggleTransparent(v, value, i)} />
        {:else if v.type === "font"}
          <Select {id} class="grow" {value} options={FONT_OPTIONS} onchange={(x: string) => onvalue(v.key, x, i)} />
        {:else if v.type === "range"}
          <span class="slider vf vf-within">
            <input
              type="range"
              aria-label="{v.label || v.key} slider"
              min={v.min}
              max={v.max}
              step={v.step}
              value={rangeNumber(v, value)}
              oninput={(e) => onvalue(v.key, e.currentTarget.value, i)}
            />
          </span>
          <NumberInput
            {id}
            value={rangeNumber(v, value)}
            min={v.min}
            max={v.max}
            step={v.step}
            digits={Math.max(3, String(v.max).length + 1)}
            suffix={v.unit || undefined}
            onchange={(n) => onvalue(v.key, String(n), i)}
          />
        {:else if v.type === "select"}
          <Select
            {id}
            class="grow"
            {value}
            options={v.options.filter((o) => o.value !== "")}
            onchange={(x: string) => onvalue(v.key, x, i)}
          />
        {/if}
      </div>
      {#if overridden}
        <div class="override">
          <span class="marker">Overrides template</span>
          {#if onreset}
            <Button
              size="sm"
              variant="ghost"
              aria-label="Reset {v.label || v.key} to the template value"
              onclick={() => onreset(v.key)}
            >
              {#snippet icon()}<RotateCcw size={12} />{/snippet}
              Reset
            </Button>
          {/if}
        </div>
      {/if}
      {#if err}<p class="err" id="{id}-err">{err}</p>{/if}
    </div>
  {/each}
</div>

<style>
  .values {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
  }
  .value-row {
    display: grid;
    grid-template-columns: minmax(8rem, 12rem) minmax(0, 1fr);
    gap: var(--s-1) var(--s-3);
    align-items: center;
    min-height: var(--row-h);
  }
  .vlabel {
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .control {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .control :global(.grow) {
    flex: 1;
    max-width: 20rem;
  }
  .value-row .err,
  .override {
    grid-column: 2;
  }
  .override {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .marker {
    color: var(--accent-text);
    font-size: var(--fs-xs);
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
  .text {
    display: flex;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .text input {
    width: 100%;
    min-width: 0;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .text input.hex {
    width: calc(11ch + 2 * var(--s-2));
  }
  /* Transparent: the checkerboard shows through the (still clickable) picker. */
  .swatch.clear {
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .swatch.clear input[type="color"] {
    opacity: 0;
  }
  input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .slider {
    display: flex;
    flex: 1;
    max-width: 20rem;
    border-radius: var(--r-control);
  }
  input[type="range"] {
    width: 100%;
    accent-color: var(--accent);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-sm);
  }

  /* The inspector column is narrow: stack label over control there (a container query keeps the
     template editor's wide two-column rows unchanged). */
  .values {
    container-type: inline-size;
  }
  @container (max-width: 22rem) {
    .value-row {
      grid-template-columns: minmax(0, 1fr);
    }
    .value-row .err,
    .override {
      grid-column: 1;
    }
  }
  @media (max-width: 720px) {
    .value-row {
      grid-template-columns: minmax(0, 1fr);
    }
    .value-row .err,
    .override {
      grid-column: 1;
    }
  }
</style>
