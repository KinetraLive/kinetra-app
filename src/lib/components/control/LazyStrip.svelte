<!--
  LazyStrip: a section of Control's side panel whose content is a lazy chunk (./strips.ts: the
  Automation strip), loaded when the strip mounts; until then only the section
  label shows (same height, no shift). It hands Control's Menu to the chunk, so the chunk never
  imports it (that would split Menu out of the route's chunk).

  Props:
    label: string                                   the section's name
    load: () => Promise<Component<StripProps>>
    ctl: ControlCtl
-->
<script lang="ts">
  import type { Component } from "svelte";
  import type { ControlCtl } from "./control.svelte";
  import Menu from "./Menu.svelte";
  import type { StripProps } from "./strips";

  let { label, load, ctl }: { label: string; load: () => Promise<Component<StripProps>>; ctl: ControlCtl } = $props();

  let Body = $state.raw<Component<StripProps> | null>(null);

  $effect(() => {
    let alive = true;
    load().then(
      (c) => alive && (Body = c),
      () => {},
    );
    return () => {
      alive = false;
    };
  });
</script>

<section class="strip" aria-label={label}>
  {#if Body}
    <Body {ctl} {Menu} />
  {:else}
    <h2 class="section-label head">{label}</h2>
  {/if}
</section>

<style>
  .strip {
    border-top: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: center;
    min-height: calc(var(--row-h) + var(--s-2));
    margin: 0;
    padding: var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
  }
</style>
