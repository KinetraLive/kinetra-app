<!--
  Third-party licenses: modal on native <dialog> + showModal(), loaded lazily from
  Preferences > About (its own chunk; the notices JSON is another, loaded on mount via
  $lib/licenses). Search narrows by name, package, version or license; groups App (npm),
  Core (Rust) and Fonts list name, mono version and a license badge; each row is a disclosure
  that shows the repository and the license files as text.
  Esc, the backdrop and the close button call `onclose`; the parent unmounts it and returns focus.

  Props:
    onclose: () => void
-->
<script lang="ts">
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import Search from "lucide-svelte/icons/search";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import Badge from "$lib/components/Badge.svelte";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { filterLicenses, loadLicenses, type Licenses } from "$lib/licenses";

  let { onclose }: { onclose: () => void } = $props();

  const uid = $props.id();
  let dialog: HTMLDialogElement | undefined = $state();
  let searchEl: HTMLInputElement | undefined = $state();
  let data = $state.raw<Licenses | null>(null);
  let error = $state<string | null>(null);
  let query = $state("");
  let expanded = $state<Record<string, boolean>>({});

  const groups = $derived(data ? filterLicenses(data.groups, query) : []);
  const total = $derived(data ? data.groups.reduce((n, g) => n + g.items.length, 0) : 0);
  const shown = $derived(groups.reduce((n, g) => n + g.items.length, 0));

  function load() {
    error = null;
    loadLicenses().then(
      (d) => (data = d),
      (e: unknown) => (error = `Couldn't load the license list: ${toAppError(e).message}`),
    );
  }

  $effect(() => {
    const d = dialog;
    if (!d || d.open) return;
    if (typeof d.showModal === "function") d.showModal();
    else d.setAttribute("open", "");
    searchEl?.focus();
  });

  $effect(load);

  function onCancelEvent(e: Event) {
    e.preventDefault();
    onclose();
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === dialog) onclose();
  }

  function toggle(key: string) {
    expanded[key] = !expanded[key];
  }

  /** DOM id of a row's panel (keys hold `/`, `@` and dots). */
  const panelId = (key: string) => `${uid}-p-${key.replace(/[^a-zA-Z0-9_-]/g, "_")}`;
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
  bind:this={dialog}
  class="dialog"
  aria-labelledby="{uid}-title"
  aria-describedby="{uid}-sum"
  oncancel={onCancelEvent}
  onclick={onBackdropClick}
>
  <div class="inner">
    <header class="head">
      <div class="heading">
        <h2 id="{uid}-title" class="title">Third-party licenses</h2>
        <p id="{uid}-sum" class="sum">
          Kinetra is built on open-source software.
          {#if data}<span>{query.trim() ? `${shown} of ${total}` : total}</span> packages.{/if}
        </p>
      </div>
      <span class="search vf vf-within">
        <span class="search-icon" aria-hidden="true"><Search size={14} /></span>
        <input
          bind:this={searchEl}
          bind:value={query}
          type="search"
          aria-label="Search licenses"
          autocomplete="off"
          spellcheck="false"
        />
      </span>
      <IconButton label="Close" onclick={onclose}><X size={16} /></IconButton>
    </header>

    <div class="body">
      {#if error}
        <div class="state">
          <p class="err">{error}</p>
          <Button size="sm" onclick={load}>Try again</Button>
        </div>
      {:else if data}
        {#if shown === 0}
          <div class="state">
            <p class="muted">No package matches “{query.trim()}”.</p>
            <Button size="sm" variant="ghost" onclick={() => (query = "")}>Clear search</Button>
          </div>
        {/if}
        {#each groups as g (g.id)}
          {#if g.items.length}
            <section class="group" aria-labelledby="{uid}-g-{g.id}">
              <h3 id="{uid}-g-{g.id}" class="section-label">
                {g.label} <span class="count">{g.items.length}</span>
              </h3>
              <ul class="list">
                {#each g.items as it (`${g.id}/${it.name}@${it.version}`)}
                  {@const key = `${g.id}/${it.name}@${it.version}`}
                  {@const open = !!expanded[key]}
                  <li class="item">
                    <button
                      type="button"
                      class="row"
                      aria-expanded={open}
                      aria-controls={panelId(key)}
                      onclick={() => toggle(key)}
                    >
                      <span class="chev" class:open aria-hidden="true"><ChevronRight size={14} /></span>
                      <span class="name">{it.name}</span>
                      <span class="version mono">{it.version}</span>
                      <Badge>{it.license}</Badge>
                    </button>
                    {#if open}
                      <div class="panel" id={panelId(key)}>
                        {#if it.package || it.url || it.copyright}
                          <dl class="meta">
                            {#if it.copyright}<div><dt>Copyright</dt><dd>{it.copyright}</dd></div>{/if}
                            {#if it.package}<div><dt>Package</dt><dd class="mono">{it.package}</dd></div>{/if}
                            {#if it.url}<div><dt>Source</dt><dd class="mono">{it.url}</dd></div>{/if}
                          </dl>
                        {/if}
                        {#each it.files ?? [] as f (f.name)}
                          <p class="file mono">{f.name}</p>
                          <!-- A scrollable text box is focusable so the keyboard can scroll it. -->
                          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                          <pre class="text mono" tabindex="0" aria-label="{f.name} of {it.name}">{data.texts[f.text]}</pre>
                        {:else}
                          <p class="muted">No license file in the package. Licensed under <span class="mono">{it.license}</span>.</p>
                        {/each}
                      </div>
                    {/if}
                  </li>
                {/each}
              </ul>
            </section>
          {/if}
        {/each}
      {/if}
    </div>
  </div>
</dialog>

<style>
  .dialog {
    width: min(880px, calc(100vw - 2 * var(--s-6)));
    height: min(760px, calc(100vh - 2 * var(--s-6)));
    margin: auto;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-panel);
    background: var(--surface-2);
    color: var(--text);
    overflow: hidden;
  }
  .dialog[open] {
    animation: enter var(--dur) var(--ease);
  }
  .dialog::backdrop {
    background: var(--overlay);
    animation: fade var(--dur) var(--ease);
  }
  .inner {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    padding: var(--s-4) var(--s-5);
    border-bottom: 1px solid var(--line);
  }
  .heading {
    flex: 1 1 16rem;
    min-width: 0;
  }
  .title {
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .sum {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .search {
    display: flex;
    position: relative;
    flex: 0 1 16rem;
    border-radius: var(--r-control);
  }
  /* A magnifier inside the field marks it as search (no placeholder text). */
  .search-icon {
    position: absolute;
    top: 50%;
    left: var(--s-2);
    display: inline-flex;
    color: var(--text-muted);
    transform: translateY(-50%);
    pointer-events: none;
    z-index: 1;
  }
  .search input {
    width: 100%;
    min-width: 0;
    height: var(--control-h-sm);
    padding: 0 var(--s-2) 0 calc(var(--s-2) + 14px + var(--s-1));
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--s-2) var(--s-5) var(--s-5);
  }
  .state {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-3);
    padding: var(--s-5) 0;
  }
  .group {
    padding-top: var(--s-4);
  }
  .section-label {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    margin-bottom: var(--s-2);
  }
  .count {
    color: var(--text-muted);
    letter-spacing: 0;
  }
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .item {
    border-top: 1px solid var(--line);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    width: 100%;
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .row:hover {
    background: var(--surface-3);
  }
  /* The global focus outline, drawn inside the full-width row. */
  .row:focus-visible {
    outline-offset: -2px;
  }
  .chev {
    display: inline-flex;
    color: var(--text-muted);
    transition: transform var(--dur) var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .version {
    flex: 1;
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    padding: var(--s-1) var(--s-2) var(--s-4) calc(var(--s-2) + 14px + var(--s-2));
  }
  .meta {
    display: grid;
    gap: var(--s-1);
    margin: 0;
    font-size: var(--fs-sm);
  }
  .meta div {
    display: flex;
    gap: var(--s-3);
    min-width: 0;
  }
  .meta dt {
    flex: none;
    width: 4.5rem;
    color: var(--text-muted);
  }
  .meta dd {
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .file {
    margin-top: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }
  .text {
    max-height: 20rem;
    margin: 0;
    padding: var(--s-3);
    overflow: auto;
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line);
    font-size: var(--fs-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .muted {
    color: var(--text-muted);
  }
  .err {
    color: var(--danger);
  }

  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .chev {
      transition: none;
    }
  }
</style>
