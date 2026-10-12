<!--
  Preferences › License: plan and status, valid until / updates until, the account, the last
  successful check and how long it works offline, this computer (name, Main / Backup) and the
  other computers on the license (Deactivate each). Buttons: Deactivate this computer, Sign out,
  Manage account, Upgrade to Pro (Free only); Sign in when signed out. Pro features are listed
  with their lock. Display only: the core decides; errors use the API.md messages. No props.
-->
<svelte:options runes />

<script lang="ts">
  import { onMount } from "svelte";
  import ExternalLink from "lucide-svelte/icons/external-link";
  import Button from "$lib/components/Button.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import {
    cloudLinks,
    licenseCheckNow,
    licenseDeactivate,
    licenseList,
    signInCancel,
    signOut,
  } from "$lib/api/license";
  import { cloudMessage } from "$lib/components/license/messages";
  import { openInBrowser } from "$lib/components/license/open";
  import { agoText, dateText, planName, statusIsProblem, statusText } from "$lib/components/license/plans";
  import ProLock from "$lib/components/license/ProLock.svelte";
  import SignInCode from "$lib/components/license/SignInCode.svelte";
  import { license } from "$lib/components/license/state.svelte";
  import type { CloudActivation } from "$lib/types";

  license.start();
  const v = $derived(license.view);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let confirmOwn = $state(false);

  /** This computer's activation and the others, from the account's licenses. */
  const computers = $derived.by(() => {
    const own: CloudActivation[] = [];
    const others: (CloudActivation & { plan: string })[] = [];
    for (const l of v?.licenses ?? []) {
      for (const a of l.activations) {
        if (a.this_machine) own.push(a);
        else others.push({ ...a, plan: l.plan });
      }
    }
    return { own: own[0] ?? null, others };
  });

  onMount(() => {
    // Fresh computer list when signed in (failures keep the cached one).
    if (license.view?.signedIn) void licenseList().catch(() => {});
  });

  async function run(f: () => Promise<unknown>) {
    busy = true;
    error = null;
    try {
      await f();
    } catch (e) {
      error = cloudMessage(e);
    } finally {
      busy = false;
    }
  }
  const open = (k: "account" | "pricing") => void cloudLinks().then((l) => openInBrowser(l[k]));
</script>

<SettingsSection title="License">
  {#if !v}
    <p class="muted">Loading…</p>
  {:else}
    <SettingsRow label="Plan">
      <span>{planName(v.plan)}</span>
      {#if !v.features.ai}
        <Button size="sm" onclick={() => open("pricing")}>Upgrade to Pro</Button>
      {/if}
    </SettingsRow>
    <SettingsRow label="Status">
      <span class:warn={statusIsProblem(v.status)}>{statusText(v.status)}</span>
    </SettingsRow>
    {#if v.pending}
      <SettingsRow label="Pending">
        <span class="warn">A license change applies when the show ends.</span>
      </SettingsRow>
    {/if}
    {#if v.endsAt}
      <SettingsRow label="Valid until"><span class="tnum">{dateText(v.endsAt)}</span></SettingsRow>
    {/if}
    {#if v.updatesUntil}
      <SettingsRow label="Updates until"><span class="tnum">{dateText(v.updatesUntil)}</span></SettingsRow>
    {/if}
    {#if v.status !== "none"}
      <SettingsRow label="Offline">
        <span class="tnum">
          {#if v.access === "full"}Works for {v.offlineDaysLeft} more {v.offlineDaysLeft === 1 ? "day" : "days"} without a connection
          {:else if v.status === "reduced"}Limited now; connect to check the license
          {:else}Connect to check the license{/if}
        </span>
      </SettingsRow>
    {/if}
    <SettingsRow label="Last check">
      <span class="tnum">{v.lastCheckAt ? agoText(v.lastCheckAt) : "Never"}{v.online === false ? " · offline now" : ""}</span>
      <Button size="sm" variant="ghost" disabled={busy} onclick={() => void run(licenseCheckNow)}>Check now</Button>
    </SettingsRow>
    <SettingsRow label="Pro features">
      <ProLock feature="ai" label="AI templates"><span>AI templates</span></ProLock>
    </SettingsRow>
  {/if}
</SettingsSection>

{#if v}
  <SettingsSection title="Account">
    {#if v.signedIn && v.account}
      <SettingsRow label="Signed in as">
        <span>{v.account.name ? `${v.account.name} · ` : ""}{v.account.email}</span>
      </SettingsRow>
      <SettingsRow label="Account">
        <Button size="sm" onclick={() => open("account")}>
          {#snippet icon()}<ExternalLink size={14} />{/snippet}
          Manage account
        </Button>
        <Button size="sm" variant="ghost" disabled={busy} onclick={() => void run(signOut)}>Sign out</Button>
      </SettingsRow>
    {:else if v.signin.state !== "idle"}
      <SettingsRow label="Sign in" stacked>
        <SignInCode signin={v.signin} onretry={() => void license.signIn()} oncancel={() => void signInCancel()} />
      </SettingsRow>
    {:else}
      <SettingsRow label="Not signed in" description="Your license keeps working offline.">
        <Button size="sm" variant="primary" onclick={() => void license.signIn()}>Sign in</Button>
      </SettingsRow>
    {/if}
  </SettingsSection>

  <SettingsSection title="Computers">
    <SettingsRow label="This computer">
      <span>{v.machineName}{v.slot ? ` · ${v.slot === "backup" ? "Backup" : "Main"}` : ""}</span>
      {#if computers.own && v.signedIn}
        {#if confirmOwn}
          <Button size="sm" variant="danger" disabled={busy} onclick={() => void run(() => licenseDeactivate(computers.own!.id)).then(() => (confirmOwn = false))}>
            Deactivate now
          </Button>
          <Button size="sm" variant="ghost" onclick={() => (confirmOwn = false)}>Cancel</Button>
        {:else}
          <Button size="sm" variant="danger" onclick={() => (confirmOwn = true)}>Deactivate this computer</Button>
        {/if}
      {/if}
    </SettingsRow>
    {#if confirmOwn}
      <p class="muted note">Pro stops here after the show; you can activate again any time.</p>
    {/if}
    {#each computers.others as a (a.id)}
      <SettingsRow label={a.machine_name ?? "Another computer"} description="{planName(a.plan)} · {a.slot === 'backup' ? 'Backup' : 'Main'}">
        <Button size="sm" variant="danger" disabled={busy || !v.signedIn} onclick={() => void run(() => licenseDeactivate(a.id))}>Deactivate</Button>
      </SettingsRow>
    {/each}
  </SettingsSection>
{/if}

{#if error}<p class="error" role="alert">{error}</p>{/if}

<style>
  .muted {
    color: var(--text-muted);
  }
  .warn {
    color: var(--warning);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .note {
    margin: 0;
    font-size: var(--fs-sm);
  }
</style>
