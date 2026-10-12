<!--
  TimerDefault: a `timer` field's default state in the Fields tab (what Reset returns to):
  direction, start time (mm:ss, mono) and, counting down, what happens at zero. Always paused.

  Props:
    field: string                       the field's label (accessible names)
    value: string                       the field's default (TimerState JSON)
    onchange: (value: string) => void
-->
<script lang="ts">
  import Select from "$lib/components/Select.svelte";
  import type { TimerDir, TimerEnd } from "$lib/types";
  import { formatSeconds, parseDuration, parseTimer, timerValue } from "$lib/components/fields/timer";

  let { field, value, onchange }: { field: string; value: string; onchange: (value: string) => void } = $props();

  const st = $derived(parseTimer(value));
  /** What is typed in Start while it doesn't parse; null shows the saved start. */
  let draft = $state<string | null>(null);
  const startText = $derived(draft ?? formatSeconds(st.base / 1000));
  const invalid = $derived(draft !== null && parseDuration(draft) === null);

  function emit(p: { base?: number; dir?: TimerDir; end?: TimerEnd }) {
    onchange(timerValue({ ...st, run: false, at: 0, ...p }));
  }

  function onStart(text: string) {
    const ms = parseDuration(text);
    if (ms === null) {
      draft = text;
      return;
    }
    draft = text;
    emit({ base: ms });
  }

  const DIRS: { value: TimerDir; label: string }[] = [
    { value: "up", label: "Count up" },
    { value: "down", label: "Count down" },
  ];
  const ENDS: { value: TimerEnd; label: string }[] = [
    { value: "stop", label: "Stop at zero" },
    { value: "hide", label: "Play out at zero" },
  ];
</script>

<div class="timer" role="group" aria-label="Default of {field}">
  <Select
    size="sm"
    aria-label="{field} direction"
    value={st.dir}
    options={DIRS}
    onchange={(dir: TimerDir) => emit({ dir })}
  />
  <label class="start">
    <span>Start</span>
    <span class="input vf vf-within">
      <input
        class="mono"
        type="text"
        inputmode="numeric"
        autocomplete="off"
        spellcheck="false"
        aria-label="{field} start time (mm:ss)"
        aria-invalid={invalid ? "true" : undefined}
        value={startText}
        oninput={(e) => onStart(e.currentTarget.value)}
        onblur={() => (draft = null)}
      />
    </span>
  </label>
  {#if st.dir === "down"}
    <Select
      size="sm"
      aria-label="{field} at zero"
      value={st.end}
      options={ENDS}
      onchange={(end: TimerEnd) => emit({ end })}
    />
  {/if}
  {#if invalid}<p class="err">Use mm:ss, up to 99:59:59.</p>{/if}
</div>

<style>
  .timer {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
  }
  .start {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .input {
    display: inline-flex;
    border-radius: var(--r-control);
  }
  .input input {
    width: 10ch;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
  }
  .input input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .err {
    flex-basis: 100%;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
