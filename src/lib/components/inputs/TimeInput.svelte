<!--
  TimeInput: a forgiving time-of-day box (mono). Typing "12", "12.00", "1200", "930" or "9" all
  mean a time (parseTime in ./time.ts); the box shows what was typed and is normalised to HH:mm on
  blur. A valid time is committed (`onchange`) on blur and on Enter (before the form submits), never
  per keystroke, so a host that re-sorts on change doesn't move rows mid-typing.

  An invalid time stays in the box as typed (never dropped): aria-invalid, --danger text and the
  reason under the box (`compact`: in a tooltip and for screen readers only, for table cells).
  `oninvalid(reason | null)` reports every change of validity so the host can block its Save.

  Props:
    value: string                    "HH:mm", or "" for none
    onchange: (value: string) => void  a valid time (or "" when `optional`) on blur / Enter
    oninvalid?: (reason: string | null) => void
    optional?: boolean               empty is allowed (commits "")
    variant?: "box" | "cell"         box: a bordered control-h-sm box; cell: a borderless table cell
    compact?: boolean                the reason as a tooltip instead of a line under the box
    error?: string | null            the host's own problem with a valid time (e.g. ends before it
                                     starts): marked the same way while the box itself parses
    errorId?: string                 the element that already shows `error` (described by it)
    id?, aria-label?, disabled?, and any data-* attributes (on the <input>)
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { parseTime } from "./time";

  let {
    value,
    onchange,
    oninvalid,
    optional = false,
    variant = "box",
    compact = false,
    error = null,
    errorId,
    id,
    disabled = false,
    ...rest
  }: {
    value: string;
    onchange: (value: string) => void;
    oninvalid?: (reason: string | null) => void;
    optional?: boolean;
    variant?: "box" | "cell";
    compact?: boolean;
    error?: string | null;
    errorId?: string;
    id?: string;
    disabled?: boolean;
    "aria-label"?: string;
    [attr: `data-${string}`]: string | undefined;
  } = $props();

  const uid = $props.id();
  const inputId = $derived(id ?? `${uid}-time`);

  // What the box shows: the value, until the user types; then what they typed.
  let text = $state("");
  let focused = false;
  let reason = $state<string | null>(null);

  // Follow the value from outside while the box isn't being edited (a new value replaces a
  // held error too). Only `value` is tracked.
  $effect.pre(() => {
    const v = value;
    untrack(() => {
      if (focused) return;
      text = v;
      setReason(null);
    });
  });

  // Leaving (e.g. the host switched to another trigger type) withdraws an error it reported.
  $effect(() => () => {
    if (reason !== null) oninvalid?.(null);
  });

  /** What is wrong, if anything: the typed text first, then the host's problem. */
  const shown = $derived(reason ?? error);
  const describedBy = $derived(reason !== null ? `${uid}-why` : error ? (errorId ?? `${uid}-why`) : undefined);

  function check(t: string): { value: string } | { reason: string } {
    if (optional && t.trim() === "") return { value: "" };
    const p = parseTime(t);
    return p.ok ? { value: p.value } : { reason: p.reason };
  }

  function setReason(r: string | null) {
    if (r === reason) return;
    reason = r;
    oninvalid?.(r);
  }

  function oninput(e: Event & { currentTarget: HTMLInputElement }) {
    text = e.currentTarget.value;
    const c = check(text);
    setReason("reason" in c ? c.reason : null);
  }

  /** Normalises the box and commits a valid time (if it changed). */
  function commit() {
    const c = check(text);
    if ("reason" in c) {
      setReason(c.reason);
      return;
    }
    setReason(null);
    text = c.value;
    if (c.value !== value) onchange(c.value);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter") commit();
  }
</script>

<span class="time {variant}" class:vf={variant === "box"} class:vf-within={variant === "box"} class:bad={shown !== null}>
  <input
    {...rest}
    id={inputId}
    class="mono"
    type="text"
    inputmode="numeric"
    autocomplete="off"
    spellcheck="false"
    maxlength="8"
    size="5"
    value={text}
    {disabled}
    aria-invalid={shown !== null ? "true" : undefined}
    aria-describedby={describedBy}
    title={compact && shown ? shown : undefined}
    {oninput}
    {onkeydown}
    onfocus={() => (focused = true)}
    onblur={() => {
      focused = false;
      commit();
    }}
  />
  {#if describedBy === `${uid}-why`}
    <span class="why" class:sr={compact} id="{uid}-why">{shown}</span>
  {/if}
</span>

<style>
  .time {
    position: relative;
    display: inline-flex;
    flex-direction: column;
    min-width: 0;
  }
  .box {
    --vf-radius: var(--r-control);
    border-radius: var(--r-control);
  }
  input {
    width: 100%;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    color: var(--text);
  }
  .box input {
    width: 7ch;
    min-width: 7ch;
    box-sizing: content-box;
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    background: var(--time-bg, var(--surface-2));
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .cell {
    width: 100%;
  }
  .cell input {
    height: var(--row-h);
    background: transparent;
  }
  .cell input:hover,
  .cell input:focus {
    outline: none;
    background: var(--surface-2);
  }
  .cell input:focus {
    box-shadow: inset 0 0 0 1px var(--focus);
  }
  .bad input {
    color: var(--danger);
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  input:disabled {
    opacity: 0.6;
  }
  .why {
    margin-top: var(--s-1);
    color: var(--danger);
    font-size: var(--fs-xs);
    line-height: 1.3;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
