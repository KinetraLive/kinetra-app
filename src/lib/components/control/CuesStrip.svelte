<!--
  CuesStrip: Control's cue buttons, one per flow with a Cue trigger (replaces the Macros strip).
  Flows run server-side; the running state follows `flow-state` (ctl.flows). Scoped to the selected
  screen: only cues with a node aimed at it (aimsAt), unless "Show all cues" is on (kept for the
  session). None here: "No cues for this screen" with New cue and Show all cues.

  Each cue: its label (the cue's label, else the flow's name), the mono hotkey badge, its state
  (Running, Failed: …, or when it last fired, mono and relative), the hotkey button and Run (Stop
  while running; Run again restarts or is ignored, per the flow). "Set hotkey" waits for the next
  F-key (Shift allowed, Esc cancels), like layers; keys are "flow/<id>". A disarmed cue is disabled
  with the text "Disarmed" (arm it in Flows or under Automation). "New cue" opens the lazy linear editor.
  A "MIDI" chip shows when a MIDI control is assigned (the source in its tooltip). Right-click (or
  Shift+F10 / the menu key on the cue; the page's side panel handles it) opens the lazy context menu: Assign MIDI…, Remove MIDI,
  Set hotkey.

  Props:
    ctl: ControlCtl
-->
<script lang="ts" module>
  /** "Show all cues" (session). */
  let all = $state(false);
</script>

<script lang="ts">
  import Keyboard from "lucide-svelte/icons/keyboard";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { aimsAt, cueLabel, relTime } from "$lib/components/flows/flows.svelte";
  import type { ControlCtl } from "./control.svelte";
  import { flowHotkey, flowHotkeyKey } from "./hotkeys";
  import { lazy } from "./LayerRow.svelte";

  let { ctl }: { ctl: ControlCtl } = $props();

  const uid = $props.id();
  const newCue = () => ctl.flowEditor.show({ mode: "new", kind: "cue" });
  const toggleAll = () => (all = !all);
  const cues = $derived(all ? ctl.flows.cues : ctl.flows.cues.filter((f) => ctl.selectedId !== null && aimsAt(f, ctl.selectedId)));
</script>

<section class="strip" aria-labelledby="{uid}-title">
  <header class="head">
    <h2 class="section-label" id="{uid}-title">Cues</h2>
    {#if ctl.flows.cues.length > 0}
      <span class="acts">
        <Button size="sm" variant="ghost" aria-pressed={all} onclick={toggleAll}>Show all cues</Button>
        <Button size="sm" variant="ghost" onclick={newCue}>New cue</Button>
      </span>
    {/if}
  </header>
  {#if ctl.flows.loaded && ctl.flows.cues.length === 0}
    <div class="empty">
      <p>Run several steps from one button or F-key.</p>
      <Button size="sm" variant="primary" onclick={newCue}>New cue</Button>
    </div>
  {:else if ctl.flows.loaded && cues.length === 0}
    <div class="empty">
      <p>No cues for this screen.</p>
      <span class="acts">
        <Button size="sm" variant="primary" onclick={newCue}>New cue</Button>
        <Button size="sm" variant="ghost" onclick={toggleAll}>Show all cues</Button>
      </span>
    </div>
  {:else}
    <ul class="list">
      {#each cues as f (f.id)}
        {@const key = flowHotkeyKey(f.id)}
        {@const label = cueLabel(f)}
        {@const hotkey = flowHotkey(ctl.ui.hotkeys, f.id)}
        {@const capturing = ctl.capturing === key}
        {@const failed = ctl.flows.failed.get(f.id)}
        <li class="cue" data-cue={f.id}>
          <span class="text">
            <span class="name" class:off={!f.armed}>{label}</span>
            {#if capturing}
              <span class="capture" role="status">Press F1–F12 (Shift optional). Esc cancels.</span>
            {:else if !f.armed}
              <span class="muted">Disarmed</span>
            {:else if f.running}
              <span class="state" role="status">Running</span>
            {:else if failed}
              <span class="failed" role="status">Failed{failed.message ? `: ${failed.message}` : ""}</span>
            {:else if f.lastFiredAt}
              <span class="muted" title={f.lastFiredAt}>{relTime(f.lastFiredAt)}</span>
            {/if}
          </span>
          {#if hotkey}<span class="badge mono" title="Hotkey">{hotkey}</span>{/if}
          {#if lazy.body}<lazy.body.MidiChip k={key} />{/if}
          {#if capturing}
            {#if hotkey}
              <Button size="sm" variant="ghost" onclick={() => ctl.removeHotkey(key)} aria-label="Remove hotkey of {label}">
                Remove
              </Button>
            {/if}
            <Button size="sm" variant="ghost" onclick={() => ctl.startCapture(key)}>Cancel</Button>
          {:else}
            <IconButton size="sm" label="Set hotkey for {label}" tooltip="left" onclick={() => ctl.startCapture(key)}>
              <Keyboard size={14} />
            </IconButton>
          {/if}
          {#if f.running}
            <Button size="sm" onclick={() => ctl.flows.stop(f.id)} aria-label="Stop {label}">Stop</Button>
          {:else}
            <Button size="sm" variant="primary" disabled={!f.armed} onclick={() => ctl.flows.run(f.id)} aria-label="Run {label}">
              Run
            </Button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .strip {
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: calc(var(--row-h) + var(--s-2));
    padding: var(--s-1) var(--s-2) var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
  .head h2 {
    margin: 0;
  }
  .acts {
    display: flex;
    gap: var(--s-1);
  }
  .acts :global([aria-pressed="true"]) {
    box-shadow: inset 0 0 0 1px var(--accent);
    color: var(--accent-text);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: var(--s-2);
    list-style: none;
  }
  .cue {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    min-height: var(--row-h);
    padding: var(--s-1) var(--s-1) var(--s-1) var(--s-2);
    border-radius: var(--r-control);
  }
  .cue:hover {
    background: var(--surface-2);
  }
  .text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
    line-height: 1.25;
  }
  .text > * {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name.off,
  .muted {
    color: var(--text-muted);
  }
  .state,
  .capture,
  .failed,
  .muted {
    font-size: var(--fs-xs);
  }
  .state,
  .capture {
    color: var(--accent-text);
  }
  .failed {
    color: var(--danger);
  }
  .badge {
    flex: none;
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
    padding: var(--s-3);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .empty p {
    margin: 0;
  }
</style>
