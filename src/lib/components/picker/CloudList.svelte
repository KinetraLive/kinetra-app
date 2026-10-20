<!--
  The picker's "In the cloud" (a lazy chunk, loaded when the panel opens): the account's cloud
  projects, each with Download (the shared RestorePanel: a new folder that continues the same cloud
  backup). Signed out: a way to sign in; without Pro: "Check for backups" (asks the server once, so
  a lapsed plan can still download, and is told until when). The core decides access; this only
  shows what it says.
  Props: id, onopened(folder) (after Open project).
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { toAppError } from "$lib/api";
  import { cloudRefresh, cloudView } from "$lib/api/cloud";
  import { cloudProjectsList } from "$lib/api/cloud-history";
  import Button from "$lib/components/Button.svelte";
  import { projectLine } from "$lib/components/cloud/history/words";
  import { stateWords } from "$lib/components/cloud/words";
  import { preferencesLink } from "$lib/components/settings/open";
  import type { CloudAccess, CloudProjectSummary } from "$lib/types";

  let { id, onopened }: { id: string; onopened: (folder: string) => void } = $props();

  let access = $state<CloudAccess | null>(null);
  /** Lapsed: until when the backups can be downloaded. */
  let note = $state<string | null>(null);
  let projects = $state.raw<CloudProjectSummary[] | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  /** The project whose Download panel is open. */
  let downloading = $state<string | null>(null);
  const loadPanel = () => import("$lib/components/cloud/restore/RestorePanel.svelte").then((m) => m.default);

  async function load(check: boolean) {
    busy = true;
    error = null;
    try {
      const v = check ? await cloudRefresh(true) : await cloudView();
      // No cloud backup in the plan and nothing stored: the same offer as without Pro.
      access = v.access === "available" && v.state === "notInPlan" ? "free" : v.access;
      note = v.state === "lapsed" ? (stateWords(v)?.text ?? null) : null;
      projects = access === "available" ? await cloudProjectsList(false) : null;
    } catch (e) {
      error = toAppError(e).message;
    } finally {
      busy = false;
    }
  }

  onMount(() => void load(false));
</script>

<section class="cloud" {id} aria-label="In the cloud" aria-busy={busy}>
  {#if error}
    <p class="msg" role="alert">{error}</p>
    <div><Button size="sm" onclick={() => load(false)}>Try again</Button></div>
  {:else if access === "signedOut"}
    <p class="msg">Sign in to download projects from your cloud backup.</p>
    <div><a class="link" href="/settings#license" onclick={preferencesLink("license")}>Sign in…</a></div>
  {:else if access === "free"}
    <p class="msg">Cloud backup is part of Kinetra Pro.</p>
    <div><Button size="sm" disabled={busy} onclick={() => load(true)}>Check for backups</Button></div>
  {:else if note}
    <p class="msg">{note}</p>
  {/if}
  {#if error || access !== "available"}
    <!-- said above -->
  {:else if projects && projects.length === 0}
    <p class="msg">No projects are backed up to the cloud yet.</p>
  {:else if projects}
    <ul class="list">
      {#each projects as p (p.id)}
        <li class="row">
          <div class="line">
            <span class="text">
              <span class="name">{p.name}</span>
              <span class="facts">{projectLine(p)}{p.here ? " · Also on this computer" : ""}</span>
            </span>
            {#if downloading !== p.id}
              <Button size="sm" disabled={!p.lastAt} onclick={() => (downloading = p.id)}>Download</Button>
            {/if}
          </div>
          {#if downloading === p.id}
            {#await loadPanel() then Panel}
              <Panel
                projectId={p.id}
                versionId={null}
                name={p.name}
                at={p.lastAt}
                modes={["download"]}
                onopen={onopened}
                onclose={() => (downloading = null)}
              />
            {/await}
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .cloud {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-3);
    border-radius: var(--r-panel);
    box-shadow: inset 0 0 0 1px var(--line);
    background: var(--surface-1);
  }
  .msg {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .link {
    color: var(--accent-text);
    font-size: var(--fs-sm);
  }
  .list {
    display: grid;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    padding: var(--s-2) 0;
  }
  .row + .row {
    border-top: 1px solid var(--line);
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .text {
    display: grid;
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .facts {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
</style>
