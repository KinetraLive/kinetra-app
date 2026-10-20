<!--
  Preferences › Cloud backup › Cloud projects: every project this account backs up (from any
  computer), most recently backed up first. Per project: Rename (inline; the local project keeps
  its own name) and Move to trash (inline confirm; deleted for good after 7 days, still counting
  toward storage until then). "Show the trash" lists trashed ones with Restore. Downloading a
  project comes with restore. Loads when shown; reloads after every change.
  Props: none.
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import { cloudProjectRename, cloudProjectsList, cloudProjectTrash, cloudProjectUntrash } from "$lib/api/cloud-history";
  import Badge from "$lib/components/Badge.svelte";
  import Button from "$lib/components/Button.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import { projectLine } from "$lib/components/cloud/history/words";
  import { cloud } from "$lib/components/cloud/state.svelte";
  import { readOnly } from "$lib/components/cloud/words";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { CloudProjectSummary } from "$lib/types";

  let projects = $state.raw<CloudProjectSummary[] | null>(null);
  let failed = $state<string | null>(null);
  let showTrash = $state(false);
  let busy = $state<string | null>(null);
  let renaming = $state<string | null>(null);
  let nameText = $state("");
  let trashing = $state<string | null>(null);
  /** Lapsed: renaming is refused (§10.2); deleting and downloading still work. */
  const locked = $derived(readOnly(cloud.view));

  const visible = $derived((projects ?? []).filter((p) => showTrash || !p.trashedAt));

  async function load() {
    failed = null;
    try {
      projects = await cloudProjectsList(showTrash);
    } catch (e) {
      failed = toAppError(e).message;
    }
  }

  $effect(() => {
    void showTrash;
    void load();
  });

  async function act(id: string, f: () => Promise<unknown>) {
    if (busy) return;
    busy = id;
    try {
      await f();
      await load();
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      busy = null;
    }
  }

  const rename = (p: CloudProjectSummary) =>
    act(p.id, async () => {
      await cloudProjectRename(p.id, nameText);
      renaming = null;
    });

  function onKey(e: KeyboardEvent, p: CloudProjectSummary) {
    if (e.key === "Enter") void rename(p);
    if (e.key === "Escape") renaming = null;
  }
</script>

<div class="wrap">
  <div class="top">
    <span class="hint">{projects ? (visible.length === 1 ? "1 project" : `${visible.length} projects`) : ""}</span>
    <label class="trash-switch">
      <span>Show the trash</span>
      <Switch label="Show the trash" hideLabel checked={showTrash} onchange={(on) => (showTrash = on)} />
    </label>
  </div>
  {#if failed}
    <p class="msg" role="alert">{failed}</p>
    <Button size="sm" onclick={load}>Try again</Button>
  {:else if projects && visible.length === 0}
    <p class="msg">No projects are backed up to the cloud yet.</p>
  {/if}
  <ul class="list">
    {#each visible as p (p.id)}
      <li class="row" aria-busy={busy === p.id}>
        <div class="main">
          {#if renaming === p.id}
            <span class="vf vf-within input-wrap">
              <!-- svelte-ignore a11y_autofocus -->
              <input class="input" aria-label="Name" maxlength="200" autofocus bind:value={nameText} onkeydown={(e) => onKey(e, p)} />
            </span>
          {:else}
            <div class="name">
              <span>{p.name}</span>
              {#if p.open}<Badge tone="accent">This project</Badge>{/if}
              {#if p.conflict}<Badge>Backed up on two computers</Badge>{/if}
            </div>
          {/if}
          <div class="line">{projectLine(p)}</div>
          {#if trashing === p.id}
            <div class="confirm" role="group" aria-label="Move to the trash?">
              <span>Move to the trash? It's deleted for good after 7 days.</span>
              <Button size="sm" variant="danger" disabled={busy === p.id} onclick={() => act(p.id, async () => { await cloudProjectTrash(p.id); trashing = null; })}>Move to trash</Button>
              <Button size="sm" variant="ghost" onclick={() => (trashing = null)}>Cancel</Button>
            </div>
          {/if}
        </div>
        <div class="actions">
          {#if p.trashedAt}
            <Button size="sm" disabled={busy === p.id} onclick={() => act(p.id, () => cloudProjectUntrash(p.id))}>Restore</Button>
          {:else if renaming === p.id}
            <Button size="sm" variant="primary" disabled={busy === p.id} onclick={() => rename(p)}>Save</Button>
            <Button size="sm" variant="ghost" onclick={() => (renaming = null)}>Cancel</Button>
          {:else}
            {#if !locked}
              <Button
                size="sm"
                variant="ghost"
                disabled={busy === p.id}
                onclick={() => {
                  renaming = p.id;
                  nameText = p.name;
                  trashing = null;
                }}>Rename</Button
              >
            {/if}
            <Button
              size="sm"
              variant="danger"
              disabled={busy === p.id}
              onclick={() => {
                trashing = p.id;
                renaming = null;
              }}>Move to trash</Button
            >
          {/if}
        </div>
      </li>
    {/each}
  </ul>
</div>

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
  }
  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-3);
  }
  .hint,
  .line,
  .msg {
    color: var(--text-muted);
  }
  .line {
    font-size: var(--fs-sm);
  }
  .trash-switch {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
  }
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: flex;
    gap: var(--s-3);
    align-items: flex-start;
    padding: var(--s-2) 0;
    border-top: 1px solid var(--line);
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .name {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    overflow-wrap: anywhere;
  }
  .confirm {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    margin-top: var(--s-2);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-1);
  }
  .input-wrap {
    display: flex;
    border-radius: var(--r-control);
  }
  .input {
    flex: 1;
    min-width: 160px;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background: var(--surface-1);
    color: var(--text);
    font: inherit;
    outline: none;
  }
</style>
