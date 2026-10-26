<!--
  Preferences › Outputs (a lazy chunk; per machine, works without a project): the display map
  with "Identify displays", then the outputs with "Add output". While this section is shown the
  core watches for display changes (`watchDisplays`, counted; released on leave), so plugging a
  display in or out updates the map and the Display choices at once. Outputs are saved through
  the prefs store (optimistic; the core clamps and reconciles the windows). Status follows
  `outputs-changed`. Screens come from the open project (none: "Open a project to choose a
  screen"). On macOS (no display outputs yet) only a notice, and nothing is watched. No props.
-->
<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import { onMount } from "svelte";
  import { on, toAppError } from "$lib/api";
  import { identifyDisplays, listOutputs, ndiInfo, outputTestPattern, watchDisplays } from "$lib/api/outputs";
  import { listScreenDocs } from "$lib/api/screens";
  import Button from "$lib/components/Button.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import AdvancedHint from "$lib/components/AdvancedHint.svelte";
  import { canAddNdi, newNdiOutput, newOutput } from "$lib/components/outputs/model";
  import { NDI_TRADEMARK, NDI_URL, openExternal } from "$lib/components/outputs/ndi-links";
  import { isAdvanced } from "$lib/stores/mode.svelte";
  import { isMac } from "$lib/platform";
  import { prefs } from "$lib/stores/prefs.svelte";
  import { project } from "$lib/stores/project.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { OUTPUTS_MAX, type DisplayInfo, type NdiInfo, type OutputConfig, type OutputStatus, type TestPattern } from "$lib/types";
  import DisplayMap from "../outputs/DisplayMap.svelte";
  import NdiOutputRow from "../outputs/NdiOutputRow.svelte";
  import OutputRow from "../outputs/OutputRow.svelte";
  import NotOnMac from "../NotOnMac.svelte";

  const mac = isMac();

  let displays = $state.raw<readonly DisplayInfo[]>([]);
  let loaded = $state(false);
  let status = $state.raw<readonly OutputStatus[]>([]);
  let screens = $state.raw<readonly { id: string; name: string }[] | null>(null);
  let identifying = $state(false);
  let ndi = $state.raw<NdiInfo | null>(null);

  const outputs = $derived<readonly OutputConfig[]>(prefs.value?.outputs ?? []);

  onMount(() => {
    if (mac) return;
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
    const askNdi = () =>
      ndiInfo().then(
        (i) => (ndi = i),
        () => {},
      );
    void askNdi();
    // Back from installing the NDI® Runtime: asking again also starts outputs that waited for it.
    const onFocus = () => {
      if (ndi && !ndi.installed) void askNdi();
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
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

  function addNdi() {
    if (!ndi) return;
    const o = newNdiOutput(outputs, screens?.[0] ?? null, ndi.defaultRate);
    if (o) void save([...outputs, o]);
  }

  const hasNdi = $derived(outputs.some((o) => o.kind === "ndi"));

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

{#if mac}
<SettingsSection title="Outputs">
  <NotOnMac feature="Display outputs" />
</SettingsSection>
{:else}
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
    {#if ndi?.available}
      <Button size="sm" disabled={!canAddNdi(outputs)} onclick={addNdi}>
        {#snippet icon()}<Plus size={14} />{/snippet}
        Add NDI® output
      </Button>
    {/if}
  {/snippet}
  {#if outputs.length === 0}
    <p class="empty">Show a screen on a display, as fill or key, or send it over NDI®.</p>
  {:else}
    <ul class="list" aria-label="Outputs">
      {#each outputs as o (o.id)}
        {#if o.kind === "ndi"}
          <NdiOutputRow
            output={o}
            {outputs}
            status={status.find((s) => s.id === o.id)}
            {screens}
            downloadUrl={ndi?.downloadUrl ?? NDI_URL}
            onsave={(next) => void save(next)}
          />
        {:else}
          <OutputRow
            output={o}
            {outputs}
            status={status.find((s) => s.id === o.id)}
            {displays}
            {screens}
            onsave={(next) => void save(next)}
            onpattern={(p) => void pattern(o.id, p)}
          />
        {/if}
      {/each}
    </ul>
  {/if}
  {#if hasNdi || ndi?.available}
    <div class="ndi-foot">
      {#if hasNdi && !isAdvanced()}<AdvancedHint feature="More NDI® settings" />{/if}
      {#if ndi && !ndi.installed}
        <p class="note">
          NDI® outputs need the free NDI® Runtime.
          <a class="link" href={ndi.downloadUrl} onclick={(e) => void openExternal(e, ndi!.downloadUrl)}>Download</a>
        </p>
      {/if}
      <p class="note">{NDI_TRADEMARK}</p>
    </div>
  {/if}
</SettingsSection>
{/if}

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
  .ndi-foot {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    padding-top: var(--s-2);
    border-top: 1px solid var(--line);
  }
  .note {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .link {
    color: var(--accent-text);
  }
</style>
