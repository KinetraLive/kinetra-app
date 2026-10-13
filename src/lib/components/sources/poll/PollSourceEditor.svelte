<!--
  PollSourceEditor: a poll source on the Sources page. Its name, which poll of the account it
  shows, the results as templates get them, and where to run it: the Polls page. Bind it like any source; its columns are the answers (label, votes, pct,
  pct_round) plus the poll's own values on every row (question, total, status, leader, url,
  short_url, code, qr).

  Props (the Sources page's editor contract): id, usedBy, poll (the summary's status), onsaved,
  onduplicated, ondeleted.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import Button from "$lib/components/Button.svelte";
  import { on, toAppError } from "$lib/api";
  import { getPollSource } from "$lib/api/polls";
  import { deleteSource, renameSource } from "$lib/api/sources";
  import PollResults from "$lib/components/polls/PollResults.svelte";
  import { shortUrl, STATE_LABEL, stateOf } from "$lib/components/polls/model";
  import { toasts } from "$lib/stores/toasts.svelte";
  import { SOURCE_NAME_MAX, type PollSourceDoc, type SourceSummary } from "$lib/types";
  import { usedByLabel } from "../model";

  let {
    id,
    usedBy,
    poll = null,
    onsaved,
    ondeleted,
  }: {
    id: string;
    usedBy: string[];
    poll?: SourceSummary["poll"];
    onsaved: (summary: SourceSummary) => void;
    onduplicated?: (doc: { id: string }) => void;
    ondeleted: () => void;
  } = $props();

  let doc = $state.raw<PollSourceDoc | null>(null);
  // The summary's poll status, then whatever source-changed says.
  let latest = $state.raw<SourceSummary["poll"] | undefined>(undefined);
  const view = $derived(latest === undefined ? poll : latest);
  let name = $state("");

  async function load() {
    try {
      doc = await getPollSource(id);
      name = doc.name;
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't open the source: ${toAppError(e).message}` });
    }
  }

  onMount(() => {
    void load();
    return on("source-changed", (s) => {
      if (s.id !== id) return;
      latest = s.poll ?? null;
      void load();
    });
  });

  const data = $derived(doc?.cache ?? null);

  async function rename() {
    const next = name.trim();
    if (!doc || !next || next === doc.name) {
      name = doc?.name ?? name;
      return;
    }
    try {
      onsaved(await renameSource(id, next.slice(0, SOURCE_NAME_MAX)));
      doc = { ...doc, name: next };
    } catch (e) {
      name = doc.name;
      toasts.push({ kind: "error", message: `Couldn't rename: ${toAppError(e).message}` });
    }
  }

  let askDelete = $state(false);
  async function remove() {
    askDelete = false;
    try {
      await deleteSource(id);
      ondeleted();
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't delete: ${toAppError(e).message}` });
    }
  }
</script>

{#if doc}
  <div class="editor">
    <header class="bar">
      <span class="name vf vf-within">
        <input
          type="text"
          aria-label="Source name"
          autocomplete="off"
          maxlength={SOURCE_NAME_MAX + 20}
          bind:value={name}
          onblur={rename}
          onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        />
      </span>
      <span class="muted">Poll</span>
      <span class="muted">{usedByLabel(usedBy.length)}</span>
      <span class="spacer"></span>
      <Button href="/polls">Open in Polls</Button>
      <Button variant="danger" onclick={() => (askDelete = true)}>Delete</Button>
    </header>

    <div class="body">
      {#if doc.gone}
        <p class="note">This poll was deleted on the server. Its last results stay on screen.</p>
      {/if}
      {#if data}
        <section class="panel">
          <h2>{data.question || "Untitled"}</h2>
          <p class="muted">
            {STATE_LABEL[stateOf(data)]}
            · <span class="mono">{shortUrl(data.url)}</span>
          </p>
          <PollResults poll={data} conn={view?.conn} />
          {#if (view?.conn ?? "idle") === "idle"}
            <p class="muted">Results update live while a screen uses this source or the Polls page shows the poll.</p>
          {/if}
        </section>
      {:else}
        <p class="muted">No results yet.</p>
      {/if}
    </div>
  </div>

  {#if askDelete}
    {#await import("$lib/components/ConfirmDialog.svelte") then { default: ConfirmDialog }}
      <ConfirmDialog
        title="Delete {doc.name}?"
        message="Only this source is deleted; the poll keeps running on the Polls page."
        confirmLabel="Delete source"
        danger
        onconfirm={remove}
        oncancel={() => (askDelete = false)}
      />
    {/await}
  {/if}
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-3) var(--s-5);
    border-bottom: 1px solid var(--line);
  }
  .name {
    display: flex;
    border-radius: var(--r-control);
  }
  .name input {
    width: 18rem;
    max-width: 100%;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .muted {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    max-width: 44rem;
    padding: var(--s-5);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-3);
    padding: var(--s-4);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-lg);
  }
  .note {
    margin: 0;
    color: var(--warning);
    font-size: var(--fs-sm);
  }
</style>
