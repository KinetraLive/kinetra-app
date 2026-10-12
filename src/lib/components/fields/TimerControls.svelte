<!--
  TimerControls: a `timer` field (lazy chunk; FieldInputs loads it on demand). A mono display
  computed locally from the TimerState and the wall clock, Start / Pause, Reset, Set (mm:ss),
  direction (count up / down) and, counting down, what happens at zero (stop / hide).

  Ticking: only while the timer runs and the document is visible, one timeout aligned to the
  timer's next whole second (a countdown at zero stops). Nothing runs at rest: no timer, no
  listener while paused.

  With `ontimer` each control sends one TimerAction (the server applies it, and the new value
  comes back through `value`). Without it the action is applied locally and the new state is sent
  as JSON through `onvalue` (template preview).

  Props:
    label: string                   the field's label (accessible names)
    def: FieldDef                   the field (its default is what Reset returns to locally)
    value: string                   TimerState as JSON
    disabled?: boolean
    ontimer?: (action: TimerAction) => void
    onvalue?: (value: string) => void
-->
<script lang="ts">
  import Pause from "lucide-svelte/icons/pause";
  import Play from "lucide-svelte/icons/play";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import Button from "$lib/components/Button.svelte";
  import Select from "$lib/components/Select.svelte";
  import type { FieldDef, TimerAction, TimerDir, TimerEnd } from "$lib/types";
  import { applyTimerAction, formatTimer, nextTickIn, parseDuration, parseTimer, timerValue } from "./timer";

  let {
    label,
    def,
    value,
    disabled = false,
    ontimer,
    onvalue,
  }: {
    label: string;
    def: FieldDef;
    value: string;
    disabled?: boolean;
    ontimer?: (action: TimerAction) => void;
    onvalue?: (value: string) => void;
  } = $props();

  const uid = $props.id();
  const st = $derived(parseTimer(value));

  /** Bumped by the tick timeout and on visibility changes; re-reads the clock. */
  let tick = $state(0);
  let hidden = $state(typeof document !== "undefined" && document.hidden);
  const shown = $derived.by(() => {
    void tick;
    return formatTimer(st, Date.now());
  });

  // Visibility is only watched while running.
  $effect(() => {
    if (!st.run) return;
    const onVis = () => {
      hidden = document.hidden;
      tick++;
    };
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  });

  // One timeout to the next displayed second, only while running and visible.
  $effect(() => {
    void tick;
    if (!st.run || hidden) return;
    const delay = nextTickIn(st, Date.now());
    if (delay === null) return;
    const t = setTimeout(() => tick++, delay);
    return () => clearTimeout(t);
  });

  function send(action: TimerAction) {
    if (ontimer) ontimer(action);
    else onvalue?.(timerValue(applyTimerAction(st, action, Date.now(), def)));
  }

  let setText = $state("");
  const setMs = $derived(parseDuration(setText));
  const setInvalid = $derived(setText.trim() !== "" && setMs === null);

  function applySet() {
    if (setMs === null) return;
    send({ action: "set", ms: setMs });
    setText = "";
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

<div class="timer">
  <div class="row">
    <span class="display mono" role="timer" aria-label="{label} time" id="{uid}-time">{shown}</span>
    {#if st.run}
      <Button size="sm" {disabled} onclick={() => send({ action: "pause" })} aria-label="Pause {label}">
        {#snippet icon()}<Pause size={14} />{/snippet}
        Pause
      </Button>
    {:else}
      <Button size="sm" variant="primary" {disabled} onclick={() => send({ action: "start" })} aria-label="Start {label}">
        {#snippet icon()}<Play size={14} />{/snippet}
        Start
      </Button>
    {/if}
    <Button size="sm" variant="ghost" {disabled} onclick={() => send({ action: "reset" })} aria-label="Reset {label}">
      {#snippet icon()}<RotateCcw size={14} />{/snippet}
      Reset
    </Button>
  </div>
  <div class="row">
    <form
      class="set"
      onsubmit={(e) => {
        e.preventDefault();
        applySet();
      }}
    >
      <span class="input vf vf-within">
        <input
          class="mono"
          type="text"
          inputmode="numeric"
          autocomplete="off"
          spellcheck="false"
          placeholder="mm:ss"
          aria-label="Set {label} to (mm:ss)"
          aria-invalid={setInvalid ? "true" : undefined}
          bind:value={setText}
          {disabled}
        />
      </span>
      <Button size="sm" type="submit" disabled={disabled || setMs === null} aria-label="Set {label}">Set</Button>
    </form>
    <Select
      size="sm"
      aria-label="{label} direction"
      value={st.dir}
      options={DIRS}
      {disabled}
      onchange={(dir: TimerDir) => send({ action: "configure", dir, end: st.end })}
    />
    {#if st.dir === "down"}
      <Select
        size="sm"
        aria-label="{label} at zero"
        value={st.end}
        options={ENDS}
        {disabled}
        onchange={(end: TimerEnd) => send({ action: "configure", dir: st.dir, end })}
      />
    {/if}
  </div>
</div>

<style>
  .timer {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  .display {
    min-width: 8ch;
    font-size: var(--fs-lg);
    font-variant-numeric: tabular-nums;
  }
  .set {
    display: inline-flex;
    gap: var(--s-1);
  }
  .input {
    display: inline-flex;
    border-radius: var(--r-control);
  }
  .input input {
    width: 9ch;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .input input:disabled {
    opacity: 0.6;
  }
</style>
