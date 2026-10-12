<!--
  Preferences > Keyboard shortcuts, the body (a lazy chunk, loaded by ShortcutsSection when it
  first scrolls near the viewport or the page opens on #shortcuts).

  Every command with its keys, grouped by palette section, with a search filter. A row: title,
  where its keys work (Control, the layout editor, …), its keys (mono; each removable), Record (the
  next key pressed is added; a layer or cue key is replaced; Esc cancels; no command runs
  meanwhile) and Reset (back to the default keys) when it was changed.
  - Layer and cue keys (saved with the project, F1–F12 with or without Shift) are listed for every
    layer of every screen and every cue, by name, also while Control isn't open.
  - Commands of pages that aren't open (layout editor, flow editor, Control) come from their own
    command factories (./catalog), registered as inert copies while this is mounted, so their
    defaults and conflicts count.
  - Conflicts are shown, never resolved: a warning panel at the top names each key with the
    commands that share it ("runs nothing") and offers "Change …" for each; the rows say
    "Conflict" next to the key. Changing one of them is the fix.
  All writes go through the binding table (`setKeys`); errors show under the row.
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import Search from "lucide-svelte/icons/search";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import { listFlows } from "$lib/api/flows";
  import { listScreens } from "$lib/api/live";
  import { bindingOf, CUE_COMMAND, LAYER_COMMAND, projectKeyOf, setKeys } from "$lib/commands/bindings.svelte";
  import { conflicts } from "$lib/commands/conflicts";
  import { ariaKeys } from "$lib/commands/keys";
  import { commands, pauseShortcuts, registerCommands, titleOf } from "$lib/commands/registry.svelte";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { cueText } from "$lib/components/flows/cue";
  import { t } from "$lib/i18n";
  import { TEST_SCREEN_ID, type FlowInfo, type LiveScreenView } from "$lib/types";
  import { inert, pageCommands } from "./catalog";
  import {
    SCOPE_LABEL,
    SECTION_LABEL,
    conflictsById,
    groupRows,
    keysAfterRecording,
    keysAfterRemoving,
    recordedKey,
    type ShortcutRow,
  } from "./model";

  const uid = $props.id();
  let query = $state("");
  let flows = $state.raw<readonly FlowInfo[]>([]);
  let screens = $state.raw<readonly LiveScreenView[]>([]);
  /** The row waiting for a key (and the key it replaces: a conflict's fix). */
  let recording = $state<string | null>(null);
  let replacing: string | undefined;
  let rowError = $state<{ id: string; message: string } | null>(null);
  let root: HTMLElement | undefined = $state();

  // Pages' commands that aren't registered now, as inert copies; screen and layer names; cue names.
  onMount(() => {
    const now = new Set(commands().map((c) => c.id));
    const offCatalog = registerCommands(pageCommands().filter((c) => !now.has(c.id)).map(inert));
    listScreens().then(
      (l) => (screens = l),
      () => {},
    );
    listFlows().then(
      (l) => (flows = l),
      () => {},
    );
    return offCatalog;
  });

  const scopeText = (s: ShortcutRow["scope"]) => (SCOPE_LABEL[s] ? t(SCOPE_LABEL[s]) : "");

  const rows = $derived.by((): ShortcutRow[] => {
    const out: ShortcutRow[] = [];
    for (const c of commands()) {
      if (projectKeyOf(c.id) !== null) continue; // listed below, by name, for every layer and cue
      let title: string;
      try {
        title = titleOf(c);
      } catch {
        continue;
      }
      out.push({ id: c.id, title, section: c.section, scope: c.keyScope ?? "global", binding: bindingOf(c.id), project: false });
    }
    for (const v of screens) {
      const s = v.screen;
      if (s.id === TEST_SCREEN_ID || s.id.startsWith("__")) continue;
      for (const l of v.layers) {
        const id = `${LAYER_COMMAND}${s.id}/${l.id}`;
        const title = t("Take or clear {layer} ({screen})", { layer: l.name, screen: s.name });
        out.push({ id, title, section: "live", scope: "control", binding: bindingOf(id), project: true });
      }
    }
    for (const f of flows) {
      const cue = cueText(f);
      if (cue === null) continue;
      const id = CUE_COMMAND + f.id;
      out.push({ id, title: t("GO {cue}", { cue }), section: "live", scope: "control", binding: bindingOf(id), project: true });
    }
    return out;
  });

  const byId = $derived(new Map(rows.map((r) => [r.id, r])));
  const groups = $derived(groupRows(rows, query, scopeText));
  const clashes = $derived(conflicts());
  const clashById = $derived(conflictsById(clashes));
  const nameOf = (id: string) => byId.get(id)?.title ?? t("A removed layer or cue");

  // ---- Recording -----------------------------------------------------------------------------

  async function record(id: string, replace?: string) {
    rowError = null;
    replacing = replace;
    recording = id;
    await tick();
    root?.querySelector<HTMLElement>(`[data-row="${CSS.escape(id)}"] [data-cancel]`)?.focus();
  }

  // While recording: no command runs (paused), the window's next key is the new binding.
  $effect(() => {
    const id = recording;
    if (!id) return;
    const resume = pauseShortcuts();
    const onkeydown = (e: KeyboardEvent) => {
      const r = recordedKey(e);
      if (!r) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      recording = null;
      if (r.kind === "cancel") return;
      const row = byId.get(id);
      if (row) void write(row, keysAfterRecording(row, r.keys, replacing));
    };
    window.addEventListener("keydown", onkeydown, true);
    return () => {
      window.removeEventListener("keydown", onkeydown, true);
      resume();
    };
  });

  async function write(row: ShortcutRow, keys: readonly string[] | null) {
    rowError = null;
    try {
      await setKeys(row.id, keys);
    } catch (e) {
      rowError = { id: row.id, message: e instanceof Error ? e.message : toAppError(e).message };
    }
  }

  const changed = (r: ShortcutRow) => r.binding.source !== "default";
  const recordLabel = (r: ShortcutRow) =>
    r.project ? (r.binding.keys.length > 0 ? t("Change") : t("Set key")) : t("Add key");
  const recordHint = (r: ShortcutRow) =>
    r.project ? t("Press F1–F12 (Shift optional). Esc cancels.") : t("Press the new keys. Esc cancels.");
</script>

<div class="shortcuts" bind:this={root}>
  <div class="tools">
    <span class="search vf vf-within">
      <span class="search-icon" aria-hidden="true"><Search size={14} /></span>
      <input type="search" aria-label={t("Search shortcuts")} autocomplete="off" spellcheck="false" bind:value={query} />
    </span>
  </div>

  {#if clashes.length > 0}
    <div class="conflicts" role="region" aria-labelledby="{uid}-ch">
      <p class="conflicts-head" id="{uid}-ch">
        <TriangleAlert size={14} aria-hidden="true" />
        {clashes.length === 1 ? t("1 conflict") : t("{count} conflicts", { count: clashes.length })}
      </p>
      <ul>
        {#each clashes as c (c.keys)}
          <li>
            <span class="clash-text">
              <kbd class="mono">{c.keys}</kbd>
              {t("runs nothing: it is assigned to {commands}.", { commands: c.commandIds.map(nameOf).join(", ") })}
            </span>
            <span class="clash-fix">
              {#each c.commandIds as id (id)}
                {#if byId.has(id)}
                  <Button size="sm" onclick={() => void record(id, c.keys)}>{t("Change {command}", { command: nameOf(id) })}</Button>
                {/if}
              {/each}
            </span>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  {#each groups as g (g.section)}
    <section class="group" aria-labelledby="{uid}-{g.section}">
      <h3 class="section-label" id="{uid}-{g.section}">{t(SECTION_LABEL[g.section])}</h3>
      <ul class="rows">
        {#each g.rows as r (r.id)}
          {@const clash = clashById.get(r.id)}
          {@const where = scopeText(r.scope)}
          {@const isRecording = recording === r.id}
          <li class="row" data-row={r.id} class:clash={!!clash}>
            <span class="name">
              <span class="title">{r.title}</span>
              {#if where}<span class="where">{where}</span>{/if}
            </span>
            <span class="keys">
              {#if isRecording}
                <span class="hint" role="status">{recordHint(r)}</span>
              {:else if r.binding.keys.length === 0}
                <span class="none">{t("None")}</span>
              {:else}
                {#each r.binding.keys as k (k)}
                  <span class="key">
                    <kbd class="mono">{k}</kbd>
                    <IconButton
                      size="sm"
                      label={t("Remove {keys} from {command}", { keys: k, command: r.title })}
                      tooltip="top"
                      onclick={() => void write(r, keysAfterRemoving(r, k))}
                    >
                      <X size={12} />
                    </IconButton>
                  </span>
                {/each}
              {/if}
              {#if clash && !isRecording}
                <span class="badge" title={t("{keys} is also assigned to {commands}", {
                  keys: clash[0].keys,
                  commands: clash[0].commandIds.filter((x) => x !== r.id).map(nameOf).join(", "),
                })}>
                  <TriangleAlert size={12} aria-hidden="true" />{t("Conflict")}
                </span>
              {/if}
            </span>
            <span class="acts">
              {#if isRecording}
                <Button size="sm" variant="ghost" data-cancel onclick={() => (recording = null)}>{t("Cancel")}</Button>
              {:else}
                <Button
                  size="sm"
                  aria-label={t("{action} for {command}", { action: recordLabel(r), command: r.title })}
                  aria-keyshortcuts={r.binding.keys.map(ariaKeys).join(" ") || undefined}
                  onclick={() => void record(r.id)}
                >
                  {recordLabel(r)}
                </Button>
                {#if changed(r) && !r.project}
                  <Button size="sm" variant="ghost" onclick={() => void write(r, null)}>{t("Reset")}</Button>
                {/if}
              {/if}
            </span>
            {#if rowError?.id === r.id}<p class="error" role="alert">{rowError.message}</p>{/if}
          </li>
        {/each}
      </ul>
    </section>
  {:else}
    <p class="empty">{t("No shortcuts match.")}</p>
  {/each}
</div>

<style>
  .shortcuts {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    padding-top: var(--s-3);
  }
  .tools {
    display: flex;
  }
  .search {
    display: flex;
    position: relative;
    flex: 0 1 18rem;
    border-radius: var(--r-control);
  }
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
    color: var(--text);
  }

  .conflicts {
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--warning);
  }
  .conflicts-head {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    margin: 0 0 var(--s-1);
    color: var(--warning);
    font-weight: 600;
  }
  .conflicts ul {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .conflicts li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-2);
    min-height: var(--row-h);
  }
  .clash-fix {
    display: inline-flex;
    flex-wrap: wrap;
    gap: var(--s-1);
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: center;
    gap: var(--s-1) var(--s-3);
    min-height: var(--row-h);
    padding: var(--s-1) 0;
    border-top: 1px solid var(--line);
  }
  .rows .row:last-child {
    border-bottom: 1px solid var(--line);
  }
  .name {
    display: flex;
    align-items: baseline;
    gap: var(--s-2);
    min-width: 0;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .where,
  .none,
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .hint {
    color: var(--accent-text);
  }
  .keys {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--s-1);
  }
  .key {
    display: inline-flex;
    align-items: center;
  }
  kbd {
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    color: var(--text);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .row.clash kbd,
  .conflicts kbd {
    box-shadow: inset 0 0 0 1px var(--warning);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--warning);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .acts {
    display: inline-flex;
    gap: var(--s-1);
  }
  .error {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .empty {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  @container (max-width: 560px) {
    .row {
      grid-template-columns: minmax(0, 1fr) auto;
    }
    .keys {
      grid-column: 1 / -1;
      justify-content: flex-start;
    }
  }
</style>
