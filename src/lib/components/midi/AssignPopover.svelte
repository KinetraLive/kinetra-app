<!--
  AssignPopover: "Assign MIDI…" from a layer's or cue's context menu. Opens a learn session at once
  ("Press a key, pad or fader…"); the first message becomes the action's source (a layer: Take or
  clear; a cue: GO), replacing an earlier one, with lights on. Then: the source in mono, a warning
  when another action uses it (neither runs until one changes; links to Preferences > MIDI & OSC),
  Learn again and Done. Esc cancels / closes; focus returns to the row.

  Props:
    target: ControlTarget    what the source will run
    name: string             the layer's or cue's name
    kind: "layer" | "cue"
    onclose: () => void
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Button from "$lib/components/Button.svelte";
  import { preferencesLink } from "$lib/components/settings/open";
  import type { ControlTarget, MidiSource } from "$lib/types";
  import { controls } from "./controls.svelte";
  import { Learn, LEARN_HINT } from "./learn.svelte";
  import { loadLookups } from "./lookups";
  import { assign, overlaps, sourceOf, sourceText, targetText, targetValue, type Lookups } from "./model";

  let { target, name, kind, onclose }: { target: ControlTarget; name: string; kind: "layer" | "cue"; onclose: () => void } = $props();

  const uid = $props.id();
  const learn = new Learn();
  let learned = $state.raw<MidiSource | null>(null);
  let lookups = $state.raw<Lookups | null>(null);
  let root: HTMLElement | undefined = $state();

  function start() {
    learned = null;
    learn.start(
      (l) => {
        const source = sourceOf(l);
        learned = source;
        void controls.update((d) => assign(d, target, source));
        void tick().then(() => root?.querySelector<HTMLElement>("[data-done]")?.focus());
      },
      onclose,
    );
    void tick().then(() => root?.querySelector<HTMLElement>("[data-cancel]")?.focus());
  }

  onMount(() => {
    start();
    return () => learn.stop();
  });

  /** Other actions on an overlapping source (a conflict: none of them runs). */
  const others = $derived.by(() => {
    const s = learned;
    if (!s) return [];
    const v = targetValue(target);
    return (controls.doc?.mappings ?? []).filter((m) => targetValue(m.target) !== v && overlaps(m.source, s));
  });
  $effect(() => {
    if (others.length > 0 && !lookups) void loadLookups().then((k) => (lookups = k));
  });

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape" && !learn.active) {
      e.preventDefault();
      e.stopPropagation();
      onclose();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="assign" role="dialog" aria-labelledby="{uid}-t" tabindex="-1" bind:this={root} onkeydown={onkey}>
  <p class="title" id="{uid}-t">Assign MIDI to {name}</p>
  <p class="what">{kind === "layer" ? "Takes or clears it" : "Runs the cue"}</p>
  {#if learn.active}
    <p class="hint" role="status">{LEARN_HINT}</p>
    <div class="acts">
      <Button size="sm" variant="ghost" data-cancel onclick={onclose}>Cancel</Button>
    </div>
  {:else if learned}
    <p class="done" role="status">Assigned: <span class="mono src">{sourceText(learned)}</span></p>
    {#if others.length > 0}
      <p class="warn" role="alert">
        <TriangleAlert size={12} aria-hidden="true" />
        <span>
          Also assigned to {lookups ? others.map((m) => targetText(m.target, lookups as Lookups)).join(", ") : "another action"}. Neither
          runs until one changes. <a href="/settings#midi" onclick={preferencesLink("midi", onclose)}>MIDI & OSC</a>
        </span>
      </p>
    {/if}
    <div class="acts">
      <Button size="sm" variant="ghost" onclick={start}>Learn again</Button>
      <Button size="sm" variant="primary" data-done onclick={onclose}>Done</Button>
    </div>
  {/if}
</div>

<style>
  .assign {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    width: 18rem;
    padding: var(--s-3);
  }
  .assign:focus {
    outline: none;
  }
  p {
    margin: 0;
  }
  .title {
    font-weight: 600;
  }
  .what {
    margin-top: calc(var(--s-2) * -1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .hint {
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
  .src {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .warn {
    display: flex;
    gap: var(--s-1);
    color: var(--warning);
    font-size: var(--fs-sm);
  }
  .warn :global(svg) {
    flex: none;
    margin-top: 3px;
  }
  .warn a {
    color: var(--accent-text);
  }
  .acts {
    display: flex;
    justify-content: flex-end;
    gap: var(--s-1);
  }
</style>
