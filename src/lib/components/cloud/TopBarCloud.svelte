<!--
  Cloud backup's part of the top bar (only in builds with cloud backup: `$cloud`, a lazy chunk):
  watches the cloud mirror, shows the chip (a lazy chunk of its own) while the open project backs
  up or a restore runs, and hosts the version history dialog that the chip and the palette open
  (`openHistory`). No props.
-->
<script lang="ts">
  import { cloud } from "./state.svelte";
  import { loadHistory, setHistoryOpener, setHistoryReady } from "./history/host";

  $effect(() => cloud.watch());
  /** The version history dialog (opened from the chip and the palette through `openHistory`). */
  let historyOpen = $state(false);
  /** A version to offer restoring at once ("Open the other version"). */
  let historyRestore = $state<string | null>(null);
  $effect(() => {
    setHistoryOpener((restore) => {
      historyRestore = restore ?? null;
      historyOpen = true;
    });
    return () => setHistoryOpener(null);
  });
  const loadChip = () => import("./CloudChip.svelte").then((m) => m.default);
  $effect(() => {
    setHistoryReady(!!cloud.view?.project?.registered);
    return () => setHistoryReady(false);
  });
  const view = $derived(cloud.view);
  const restoring = $derived(!!view?.restore && !view.restore.result);
</script>

{#if view?.access === "available" && view.project && (view.project.backup || restoring)}
  {#await loadChip() then CloudChip}<CloudChip />{/await}
{/if}
{#if historyOpen}
  {#await loadHistory() then History}<History
      open={historyOpen}
      restore={historyRestore}
      openable
      onclose={() => (historyOpen = false)}
    />{/await}
{/if}
