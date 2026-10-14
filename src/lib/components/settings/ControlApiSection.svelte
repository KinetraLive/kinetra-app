<!--
  Preferences > Network & API > Control API (`#control-api`; Simple and Advanced): the switch (per
  computer, off by default), the address Companion and scripts connect to (the output server's,
  so only while a project is open; the network address with LAN access on), the API token (never
  shown: Copy, New token… after a confirm, which disconnects everyone) and the connected programs
  (name, address, when; Disconnect). Follows `control-api` while mounted. No props.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Copy from "lucide-svelte/icons/copy";
  import {
    controlApiDisconnect,
    controlApiRegenerateToken,
    controlApiSet,
    controlApiToken,
    controlApiView,
    onControlApi,
  } from "$lib/api/control-api";
  import Button from "$lib/components/Button.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { relativeTime } from "$lib/components/picker/format";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { ControlApiView } from "$lib/types";
  import { toAppError } from "$lib/api";

  let view = $state.raw<ControlApiView | null>(null);
  // Writable derived: follows the view, snaps back when switching fails.
  let enabled = $derived(view?.enabled ?? false);
  let confirming = $state(false);
  let busy = $state(false);

  onMount(() => {
    void controlApiView().then(
      (v) => (view = v),
      () => {},
    );
    return onControlApi((v) => (view = v));
  });

  $effect(() => server.watch());
  const status = $derived(server.status);
  /** host:port, as Companion asks for it. */
  const hostPort = (url: string) => url.replace(/^https?:\/\//, "");
  const addresses = $derived(
    status?.state === "running"
      ? [
          { label: "This computer", value: hostPort(status.url) },
          ...(status.lanUrl ? [{ label: "Other computers", value: hostPort(status.lanUrl) }] : []),
        ]
      : [],
  );

  async function copy(text: string, what: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.push({ kind: "info", message: `Copied ${what}` });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy to the clipboard." });
    }
  }

  async function setOn(on: boolean) {
    try {
      view = await controlApiSet(on);
    } catch (e) {
      enabled = view?.enabled ?? false;
      toasts.push({ kind: "error", message: toAppError(e).message });
    }
  }

  async function copyToken() {
    try {
      await copy(await controlApiToken(), "the API token");
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    }
  }

  async function regenerate() {
    busy = true;
    try {
      await controlApiRegenerateToken();
      confirming = false;
      toasts.push({ kind: "info", message: "New token made. Copy it into Companion." });
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = false;
    }
  }

  async function disconnect(id: number) {
    try {
      await controlApiDisconnect(id);
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    }
  }
</script>

<SettingsSection title="Control API">
  <SettingsRow label="Control API" description="For Bitfocus Companion, Stream Deck and scripts." for="pref-control-api">
    <Switch id="pref-control-api" label="Control API" hideLabel bind:checked={enabled} onchange={(on) => void setOn(on)} />
  </SettingsRow>
  {#if enabled}
    <SettingsRow label="Address">
      {#if addresses.length > 0}
        <ul class="list">
          {#each addresses as a (a.value)}
            <li>
              <span class="what">{a.label}</span>
              <code class="mono">{a.value}</code>
              <IconButton size="sm" label="Copy {a.value}" tooltip="left" onclick={() => void copy(a.value, a.value)}>
                <Copy size={12} />
              </IconButton>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="muted">Open a project to start the server.</p>
      {/if}
    </SettingsRow>
    <SettingsRow label="API token" description="Companion and scripts need it.">
      <div class="actions">
        <Button size="sm" onclick={() => void copyToken()}>
          {#snippet icon()}<Copy size={14} />{/snippet}
          Copy token
        </Button>
        <Button size="sm" variant="ghost" onclick={() => (confirming = true)}>New token…</Button>
      </div>
    </SettingsRow>
    <SettingsRow label="Connected" stacked={(view?.clients.length ?? 0) > 0}>
      {#if view && view.clients.length > 0}
        <ul class="list">
          {#each view.clients as c (c.id)}
            <li>
              <span class="what">{c.name}</span>
              <code class="mono">{c.addr}</code>
              <span class="muted">{relativeTime(new Date(c.since).toISOString())}</span>
              <Button size="sm" variant="ghost" onclick={() => void disconnect(c.id)}>Disconnect</Button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="muted">Nothing connected. Add the Kinetra module in Companion.</p>
      {/if}
    </SettingsRow>
  {/if}
</SettingsSection>

{#if confirming}
  <ConfirmDialog
    title="Make a new token?"
    message="Connected programs disconnect and need the new token."
    confirmLabel="Make new token"
    {busy}
    onconfirm={() => void regenerate()}
    oncancel={() => (confirming = false)}
  />
{/if}

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
    min-width: 0;
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: var(--row-h);
    min-width: 0;
  }
  .what {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono {
    user-select: text;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-2);
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
  }
</style>
