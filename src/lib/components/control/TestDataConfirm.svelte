<!--
  TestDataConfirm: test poll data and Program, in a project that also shows a real poll. Mounted by
  the Control page under the toolbar only while both exist (a lazy chunk).
  - The question before an app take: Control's `confirmTestData` shows it; "Allow test data on air"
    allows it for this session in the core, for every take path (MIDI, OSC, the API, flows too).
  - The notice when the core refused a remote take (`ctl.testDataBlocked`): which layer, from
    where, and the same "Allow test data on air".

  Props:
    ctl: ControlCtl
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import type { TakeVia } from "$lib/types";
  import type { ControlCtl } from "./control.svelte";

  let { ctl }: { ctl: ControlCtl } = $props();

  let asking = $state<{ names: string[]; done: (ok: boolean) => void } | null>(null);

  $effect(() => {
    const ask = (names: string[]) =>
      new Promise<boolean>((done) => {
        asking = { names, done };
      });
    ctl.confirmTestData = ask;
    return () => {
      if (ctl.confirmTestData === ask) ctl.confirmTestData = null;
      asking?.done(false);
      asking = null;
    };
  });

  function answer(ok: boolean) {
    asking?.done(ok);
    asking = null;
  }

  const and = (names: readonly string[]) =>
    names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;

  const VIA: Record<TakeVia, string> = {
    app: "Kinetra",
    midi: "A MIDI control",
    osc: "An OSC message",
    api: "Companion or the API",
    flow: "A flow",
  };
  const blocked = $derived(ctl.testDataBlocked);
</script>

{#if blocked && !ctl.testDataAllowed}
  <div class="notice" role="status">
    <span class="text">
      <strong>{VIA[blocked.via]} tried to take {and(blocked.layers)} to Program.</strong>
      It shows test poll data and this project also runs a real poll, so it stayed off air.
    </span>
    <Button size="sm" onclick={() => void ctl.allowTestData()}>Allow test data on air</Button>
    <Button size="sm" variant="ghost" onclick={() => (ctl.testDataBlocked = null)}>Dismiss</Button>
  </div>
{/if}

{#if asking}
  <ConfirmDialog
    title="Allow test data on air?"
    message="{and(asking.names)} {asking.names.length === 1 ? 'shows' : 'show'} a test poll with simulated votes, and this project also runs real polls. Allowing it lets every take put test data on Program until Kinetra restarts, also from MIDI, OSC and Companion."
    confirmLabel="Allow test data on air"
    onconfirm={() => answer(true)}
    oncancel={() => answer(false)}
  />
{/if}

<style>
  .notice {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-2) var(--s-4);
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
    border-left: 3px solid var(--warning);
    font-size: var(--fs-sm);
  }
  .text {
    flex: 1;
    min-width: 16rem;
  }
</style>
