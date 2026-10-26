<!--
  Control's outputs strip (a lazy chunk, mounted only while this machine has outputs): one chip
  per output on air (a red TallyDot, its name and the screen it shows, or "Test pattern"; an NDI®
  output only while a receiver has it), and amber warnings for the ones that can't show ("Output 2:
  display disconnected", "Output 2: screen not found", "Kinetra Main: NDI® Runtime not installed").
  Follows `outputs-changed`; renders nothing while no output is on.
  "Outputs" opens Preferences › Outputs.

  Props:
    ctl: ControlCtl   (screen names)
-->
<script lang="ts">
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { onMount } from "svelte";
  import { on, openPreferences } from "$lib/api";
  import { listOutputs } from "$lib/api/outputs";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { isOnAir, screenOf, warningText } from "$lib/components/outputs/labels";
  import { prefs } from "$lib/stores/prefs.svelte";
  import type { OutputStatus } from "$lib/types";
  import type { ControlCtl } from "./control.svelte";

  let { ctl }: { ctl: ControlCtl } = $props();

  let status = $state.raw<readonly OutputStatus[]>([]);

  onMount(() => {
    const off = on("outputs-changed", (s) => (status = s));
    listOutputs().then(
      (s) => (status = s),
      () => {},
    );
    return off;
  });

  const configs = $derived(prefs.value?.outputs ?? []);
  const nameOf = (id: string | null) => (id ? (ctl.screens.find((s) => s.screen.id === id)?.screen.name ?? null) : null);
  const items = $derived(
    configs.flatMap((o): { id: string; warn: string | null; text: string }[] => {
      const s = status.find((x) => x.id === o.id);
      const warn = warningText(o, s);
      if (warn) return [{ id: o.id, warn, text: warn }];
      // An NDI output is on air only while a receiver has it.
      if (!s || !isOnAir(s)) return [];
      const what = s.pattern ? "Test pattern" : nameOf(screenOf(configs, o));
      const name = o.kind === "ndi" ? `${o.name} (NDI®)` : o.name;
      return [{ id: o.id, warn: null, text: what ? `${name} · ${what}` : name }];
    }),
  );
</script>

{#if items.length > 0}
  <div class="strip" role="group" aria-label="Outputs">
    <button type="button" class="label section-label vf" onclick={() => void openPreferences("outputs")}>Outputs</button>
    {#each items as it (it.id)}
      <span class="chip" class:warn={it.warn}>
        {#if it.warn}
          <TriangleAlert size={12} aria-hidden="true" />
        {:else}
          <TallyDot state="program" pulse={false} size={6} label="Showing" />
        {/if}
        {it.text}
      </span>
    {/each}
  </div>
{/if}

<style>
  .strip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-1) var(--s-2);
    padding: var(--s-1) var(--s-3);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .label {
    --vf-radius: var(--r-control);
    padding: 2px var(--s-1);
    border-radius: var(--r-control);
    color: var(--text-muted);
  }
  .label:hover {
    color: var(--text);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: 22px;
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .chip.warn {
    color: var(--warning);
    box-shadow: inset 0 0 0 1px var(--warning);
  }
</style>
