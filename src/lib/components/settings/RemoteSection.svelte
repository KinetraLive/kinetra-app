<!--
  Preferences > Network & API > Remote control (`#remote`): people control this show from a
  browser at control.kinetra.live. Project-only; a Pro feature (ProLock "control").
    Remote control   the switch (this project, until Kinetra quits: off at every start)
    Status           TallyDot + words; Start again after it ended or stopped
    Lock remote take while on
    Separate show    "Make this a separate show…" once the project has a show (a copied project
                     shares it with the original; this gives it its own link and crew)
    Connection details  a lazy chunk (./remote/RemoteDetails.svelte): Test connection, numbers,
                     Export diagnostics…, Copy run ID
  The core decides everything; this mirrors `remote` (watched while mounted). No props.
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import ProLock from "$lib/components/license/ProLock.svelte";
  import { toAppError } from "$lib/api";
  import { remoteLock, remoteNewShow, remoteSet, remoteStartAgain, remoteTakeover } from "$lib/api/remote";
  import { chipOf } from "$lib/components/remote/chip.svelte";
  import { remote } from "$lib/components/remote/state.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";

  type Details = typeof import("./remote/RemoteDetails.svelte").default;
  type Confirm = typeof import("$lib/components/ConfirmDialog.svelte").default;

  $effect(() => remote.watch());
  const view = $derived(remote.view);
  const chip = $derived(chipOf(view));

  let busy = $state(false);
  async function run(f: () => Promise<unknown>) {
    if (busy) return;
    busy = true;
    try {
      await f();
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = false;
    }
  }

  const status = $derived.by(() => {
    if (!view?.on) return "Off";
    switch (view.state) {
      case "live":
        return view.crew.length === 0 ? "On · no one connected yet" : `On · ${view.crew.length} connected`;
      case "connecting":
        return "Connecting…";
      case "reconnecting":
        return "Reconnecting…";
      case "showInUse":
        return `Remote control is running on ${view.machine}. Move it here?`;
      case "ended":
        return "Ended";
      case "stopped":
        return view.message;
      default:
        return "Off";
    }
  });

  let Details = $state.raw<Details | null>(null);
  $effect(() => {
    void import("./remote/RemoteDetails.svelte").then((m) => (Details = m.default), () => {});
  });

  let Confirm = $state.raw<Confirm | null>(null);
  let confirming = $state(false);
  async function askSeparate() {
    Confirm ??= (await import("$lib/components/ConfirmDialog.svelte")).default;
    confirming = true;
  }
</script>

<SettingsSection title="Remote control">
  <ProLock feature="control" label="Remote control">
    <SettingsRow label="Remote control" description="Control this show from a browser." for="pref-remote">
      <Switch
        id="pref-remote"
        label="Remote control"
        hideLabel
        checked={view?.on ?? false}
        disabled={busy}
        onchange={(on) => run(() => remoteSet(on))}
      />
    </SettingsRow>
    <SettingsRow label="Status">
      <p class="status" role="status">
        <TallyDot state={chip?.tally ?? "idle"} />
        <span>{status}</span>
        {#if view?.on && (view.state === "ended" || view.state === "stopped")}
          <Button size="sm" disabled={busy} onclick={() => run(remoteStartAgain)}>Start again</Button>
        {:else if view?.state === "showInUse"}
          <Button size="sm" variant="primary" disabled={busy} onclick={() => run(remoteTakeover)}>Move it here</Button>
          <Button size="sm" disabled={busy} onclick={() => run(remoteNewShow)}>Make this a separate show</Button>
        {/if}
      </p>
    </SettingsRow>
    {#if view?.on}
      <SettingsRow label="Lock remote take" description="Remote users can't change what's on air." for="pref-remote-lock">
        <Switch
          id="pref-remote-lock"
          label="Lock remote take"
          hideLabel
          checked={view.takeLocked}
          disabled={busy}
          onchange={(on) => run(() => remoteLock(on))}
        />
      </SettingsRow>
    {/if}
    {#if view?.hasShowId}
      <SettingsRow label="Separate show" description="A copied project shares its show and crew.">
        <Button disabled={busy} onclick={askSeparate}>Make this a separate show…</Button>
      </SettingsRow>
    {/if}
  </ProLock>
</SettingsSection>

{#if Details}
  <Details />
{/if}

{#if Confirm && confirming}
  <Confirm
    title="Make this a separate show?"
    message="This project gets its own remote control link and crew. The other copy keeps the current ones."
    confirmLabel="Make a separate show"
    {busy}
    onconfirm={() =>
      run(async () => {
        await remoteNewShow();
        confirming = false;
      })}
    oncancel={() => (confirming = false)}
  />
{/if}

<style>
  .status {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    margin: 0;
    min-width: 0;
  }
</style>
