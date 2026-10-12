<!--
  Preferences > MIDI & OSC > Mappings of this project (controls.json; part of the section's lazy
  chunk). Grouped like Keyboard shortcuts (Layers, Cues, Slides and rows, Show, Faders and knobs).
  A row: the source in mono ("Note 36 · Ch 1 · APC40"), what it does, Lights (LED feedback; layers
  and cues only), Learn (the next MIDI message becomes its source; Esc cancels) and Remove.
  "Add" row: pick an action (the same layers, cues, slides, Clear all and Panic as the shortcuts
  list, plus faders: a layer's opacity, range variables and number fields), then Learn.
  Conflicts are shown, never resolved: a panel names each source with the actions sharing it
  ("Note 36 · Ch 1 runs nothing: it is assigned to A, B.") and offers "Change …" per mapping.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import X from "lucide-svelte/icons/x";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { controls } from "$lib/components/midi/controls.svelte";
  import { Learn, LEARN_HINT } from "$lib/components/midi/learn.svelte";
  import { loadLookups } from "$lib/components/midi/lookups";
  import {
    GROUP_LABEL,
    GROUP_ORDER,
    addMapping,
    conflictById,
    conflicts,
    groupOf,
    hasLights,
    relearn,
    removeMapping,
    setLights,
    sourceOf,
    sourceText,
    targetOptions,
    targetText,
    type Lookups,
  } from "$lib/components/midi/model";
  import TargetSelect from "$lib/components/midi/TargetSelect.svelte";

  const uid = $props.id();
  let lookups = $state.raw<Lookups>({ screens: [], flows: [], sources: [] });
  /** The mapping waiting for a message ("new": the Add row). */
  let learning = $state<string | null>(null);
  let newTarget = $state("");
  let root: HTMLElement | undefined = $state();
  const learn = new Learn();

  onMount(() => {
    const off = controls.watch();
    loadLookups(true).then((k) => (lookups = k));
    return () => {
      learn.stop();
      off();
    };
  });

  const mappings = $derived(controls.doc?.mappings ?? []);
  const options = $derived(targetOptions(lookups));
  const clashes = $derived(conflicts(mappings));
  const clashOf = $derived(conflictById(clashes));
  const groups = $derived(
    GROUP_ORDER.map((g) => ({ g, rows: mappings.filter((m) => groupOf(m.target) === g) })).filter((x) => x.rows.length > 0),
  );
  const nameOf = (id: string) => {
    const m = mappings.find((x) => x.id === id);
    return m ? targetText(m.target, lookups) : "a removed mapping";
  };

  async function start(id: string) {
    learning = id;
    learn.start(
      (l) => {
        learning = null;
        const source = sourceOf(l);
        if (id === "new") {
          const opt = options.find((o) => o.value === newTarget);
          if (!opt) return;
          newTarget = "";
          void controls.update((d) => addMapping(d, opt.target, source));
        } else void controls.update((d) => relearn(d, id, source));
      },
      () => (learning = null),
    );
    await tick();
    root?.querySelector<HTMLElement>(`[data-mapping="${CSS.escape(id)}"] [data-cancel]`)?.focus();
  }

  function cancel() {
    learn.stop();
    learning = null;
  }
</script>

{#snippet hint()}
  <span class="hint" role="status">{LEARN_HINT}</span>
{/snippet}

<div class="mappings" bind:this={root}>
  {#if clashes.length > 0}
    <div class="conflicts" role="region" aria-labelledby="{uid}-ch">
      <p class="conflicts-head" id="{uid}-ch">
        <TriangleAlert size={14} aria-hidden="true" />
        {clashes.length === 1 ? "1 conflict" : `${clashes.length} conflicts`}
      </p>
      <ul>
        {#each clashes as c (c.mappingIds.join())}
          <li>
            <span>
              <span class="mono src">{sourceText(c.source, false)}</span>
              runs nothing: it is assigned to {c.mappingIds.map(nameOf).join(", ")}.
            </span>
            <span class="fix">
              {#each c.mappingIds as id (id)}
                <Button size="sm" onclick={() => void start(id)}>Change {nameOf(id)}</Button>
              {/each}
            </span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#each groups as { g, rows } (g)}
    <section class="group" aria-labelledby="{uid}-{g}">
      <h4 class="section-label" id="{uid}-{g}">{GROUP_LABEL[g]}</h4>
      <ul class="rows">
        {#each rows as m (m.id)}
          {@const what = targetText(m.target, lookups)}
          {@const clash = clashOf.get(m.id)}
          <li class="row" data-mapping={m.id} class:clash={!!clash}>
            <span class="src-cell">
              {#if learning === m.id}
                {@render hint()}
              {:else}
                <span class="mono src">{sourceText(m.source)}</span>
                {#if clash}
                  <span class="badge" title="Also assigned to {clash.mappingIds.filter((x) => x !== m.id).map(nameOf).join(', ')}">
                    <TriangleAlert size={12} aria-hidden="true" />Conflict
                  </span>
                {/if}
              {/if}
            </span>
            <span class="what">{what}</span>
            <span class="lights">
              {#if hasLights(m.target)}
                <Switch label="Lights" checked={m.feedback} onchange={(on) => void controls.update((d) => setLights(d, m.id, on))} />
              {/if}
            </span>
            <span class="acts">
              {#if learning === m.id}
                <Button size="sm" variant="ghost" data-cancel onclick={cancel}>Cancel</Button>
              {:else}
                <Button size="sm" aria-label="Learn a new control for {what}" onclick={() => void start(m.id)}>Learn</Button>
                <IconButton size="sm" variant="danger" label="Remove {sourceText(m.source)} from {what}" tooltip="left" onclick={() => void controls.update((d) => removeMapping(d, m.id))}>
                  <X size={14} />
                </IconButton>
              {/if}
            </span>
          </li>
        {/each}
      </ul>
    </section>
  {/each}

  <div class="add" data-mapping="new">
    <span class="section-label" id="{uid}-add">Add</span>
    <TargetSelect aria-labelledby="{uid}-add" value={newTarget} {options} onchange={(v) => (newTarget = v)} />
    {#if learning === "new"}
      {@render hint()}
      <Button size="sm" variant="ghost" data-cancel onclick={cancel}>Cancel</Button>
    {:else}
      <Button size="sm" disabled={newTarget === ""} onclick={() => void start("new")}>Learn</Button>
    {/if}
  </div>
</div>

<style>
  .mappings {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
  }
  .conflicts {
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--warning);
  }
  .conflicts-head {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    margin: 0 0 var(--s-1);
    color: var(--warning);
    font-weight: 600;
  }
  .conflicts ul {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .conflicts li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
    min-height: var(--row-h);
  }
  .fix {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--s-1);
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .group h4 {
    margin: 0;
  }
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    /* Same columns in every row (each row is its own grid). */
    grid-template-columns: 19rem minmax(0, 1fr) 6.5rem 6.5rem;
    align-items: center;
    gap: var(--s-1) var(--s-3);
    min-height: var(--row-h);
    padding: var(--s-1) 0;
    border-top: 1px solid var(--line);
  }
  .rows .row:last-child {
    border-bottom: 1px solid var(--line);
  }
  .src-cell {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .src {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-xs);
    line-height: 18px;
    white-space: nowrap;
  }
  .row.clash .src,
  .conflicts .src {
    box-shadow: inset 0 0 0 1px var(--warning);
  }
  .what {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lights {
    justify-self: end;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--warning);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .hint {
    color: var(--accent-text);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .acts {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-1);
  }
  .add {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
  }
  @container (max-width: 560px) {
    .row {
      grid-template-columns: minmax(0, 1fr) auto;
    }
  }
</style>
