<!--
  Preferences › Outputs (a lazy chunk; per machine, works without a project): the display map
  with "Identify displays", then the outputs with "Add output". While this section is shown the
  core watches for display changes (`watchDisplays`, counted; released on leave), so plugging a
  display in or out updates the map and the Display choices at once. Outputs are saved through
  the prefs store (optimistic; the core clamps and reconciles the windows). Status follows
  `outputs-changed`. Screens come from the open project (none: "Open a project to choose a
  screen"). No props.
-->
<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import { onMount } from "svelte";
  import { on, toAppError } from "$lib/api";
  import { identifyDisplays, listOutputs, outputTestPattern, watchDisplays } from "$lib/api/outputs";
  import { listScreenDocs } from "$lib/api/screens";
  import Button from "$lib/components/Button.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { newOutput } from "$lib/components/outputs/model";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { OUTPUTS_MAX, type DisplayInfo, type OutputConfig, type OutputStatus, type TestPattern } from "$lib/types";
  import DisplayMap from "../outputs/DisplayMap.svelte";
  import OutputRow from "../outputs/OutputRow.svelte";

  let displays = $state.raw<readonly DisplayInfo[]>([]);
  let loaded = $state(false);
  let status = $state.raw<readonly OutputStatus[]>([]);
  let screens = $state.raw<readonly { id: string; name: string }[] | null>(null);
  let identifying = $state(false);

  const outputs = $derived<readonly OutputConfig[]>(prefs.value?.outputs ?? []);

  onMount(() => {
    const offDisplays = on("displays-changed", (d) => {
      displays = d;
      loaded = true;
    });
    const offStatus = on("outputs-changed", (s) => (status = s));
    watchDisplays(true).then(
      (d) => {
        displays = d;
        loaded = true;
      },
      () => (loaded = true),
    );
    listOutputs().then(
      (s) => (status = s),
      () => {},
    );
    return () => {
      offDisplays();
      offStatus();
      void watchDisplays(false).catch(() => {});
    };
  });

  // The open project's screens (none without a project).
  $effect(() => {
    const open = !!project.current;
    let alive = true;
    if (!open) screens = null;
    else
      listScreenDocs().then(
        (list) => alive && (screens = list.map((s) => ({ id: s.id, name: s.name }))),
        () => alive && (screens = []),
      );
    return () => {
      alive = false;
    };
  });

  async function save(next: OutputConfig[]) {
    const p = prefs.value;
    if (!p) return;
    try {
      await prefs.update({ ...p, outputs: next.length > 0 ? next : undefined });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't save the output: ${toAppError(e).message}` });
    }
  }

  function add() {
    const o = newOutput(outputs, displays, screens?.[0]?.id ?? null);
    if (o) void save([...outputs, o]);
  }

  async function pattern(id: string, p: TestPattern | null) {
    // Pressed at once; `outputs-changed` confirms.
    status = status.some((s) => s.id === id)
      ? status.map((s) => (s.id === id ? { ...s, pattern: p } : s))
      : [...status, { id, state: "off", displayId: null, frameLocked: null, pattern: p, message: null }];
    try {
      await outputTestPattern(id, p);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't show the test pattern: ${toAppError(e).message}` });
      listOutputs().then(
        (s) => (status = s),
        () => {},
      );
    }
  }

  async function identify() {
    identifying = true;
    try {
      await identifyDisplays();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't identify the displays: ${toAppError(e).message}` });
    } finally {
      identifying = false;
    }
  }
</script>

<SettingsSection title="Displays">
  {#snippet actions()}
    <Button size="sm" disabled={displays.length === 0} loading={identifying} onclick={() => void identify()}>Identify displays</Button>
  {/snippet}
  {#if loaded && displays.length === 0}
    <p class="empty">No displays found.</p>
  {:else}
    <DisplayMap {displays} {outputs} {status} />
  {/if}
</SettingsSection>

<SettingsSection title="Outputs">
  {#snippet actions()}
    <Button size="sm" disabled={outputs.length >= OUTPUTS_MAX || displays.length === 0} onclick={add}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Add output
    </Button>
  {/snippet}
  {#if outputs.length === 0}
    <p class="empty">Show a screen on a display, as fill or key.</p>
  {:else}
    <ul class="list" aria-label="Outputs">
      {#each outputs as o (o.id)}
        <OutputRow
          output={o}
          {outputs}
          status={status.find((s) => s.id === o.id)}
          {displays}
          {screens}
          onsave={(next) => void save(next)}
          onpattern={(p) => void pattern(o.id, p)}
        />
      {/each}
    </ul>
  {/if}
</SettingsSection>

<style>
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .empty {
    margin: 0;
    padding: var(--s-3) 0;
    border-top: 1px solid var(--line);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>
