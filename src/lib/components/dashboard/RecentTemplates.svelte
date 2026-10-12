<!--
  Dashboard "Recent templates": the most recently changed templates as rows (name linking to the
  editor, origin "From <preset>" / "Blank" muted, mono relative time). Shown only once the project
  has content; with no templates it offers "Create a template" (opens the gallery).

  Props:
    templates: readonly TemplateSummary[]   newest first, already capped (see ./data recentTemplates)
    presets: readonly PresetInfo[]          for the origin label (falls back to the preset id)
    oncreate: () => void                    "Create a template"
-->
<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import { relativeTime } from "$lib/components/picker/format";
  import { absoluteTime, originLabel } from "$lib/components/templates/list/format";
  import type { PresetInfo, TemplateSummary } from "$lib/types";

  let {
    templates,
    presets,
    oncreate,
  }: {
    templates: readonly TemplateSummary[];
    presets: readonly PresetInfo[];
    oncreate: () => void;
  } = $props();
  const uid = $props.id();

  // Computed once per list change; relative times don't tick (no timers at rest).
  const rows = $derived.by(() => {
    const now = Date.now();
    return templates.map((t) => ({
      id: t.id,
      name: t.name,
      updatedAt: t.updatedAt,
      origin: originLabel(t.preset, presets),
      when: relativeTime(t.updatedAt, now),
      title: absoluteTime(t.updatedAt),
    }));
  });
</script>

<section class="panel" aria-labelledby="{uid}-h">
  <h2 id="{uid}-h" class="section-label">Recent templates</h2>
  {#if rows.length > 0}
    <ul class="list">
      {#each rows as t (t.id)}
        <li>
          <a class="row vf" href="/templates/{t.id}">
            <span class="name">{t.name}</span>
            <time class="when" datetime={t.updatedAt} title={t.title || undefined}>{t.when}</time>
            <span class="origin">{t.origin}</span>
          </a>
        </li>
      {/each}
    </ul>
  {:else}
    <div class="none">
      <p class="muted">No templates yet.</p>
      <button type="button" class="more vf" onclick={oncreate}>
        <Plus size={14} aria-hidden="true" />Create a template
      </button>
    </div>
  {/if}
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
    border-bottom: 1px solid var(--line);
  }
  /* Two lines per row: name + time, then the origin. */
  .row {
    --vf-radius: var(--r-control);
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    column-gap: var(--s-3);
    align-items: baseline;
    padding: var(--s-2) 0;
    border-top: 1px solid var(--line);
    color: var(--text);
    text-decoration: none;
  }
  .row:hover .name {
    text-decoration: underline;
  }
  .name,
  .origin {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .origin {
    grid-column: 1 / -1;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .none {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-1);
    padding-top: var(--s-2);
    border-top: 1px solid var(--line);
  }
  .muted {
    color: var(--text-muted);
  }
  .more {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .more:hover {
    text-decoration: underline;
  }
</style>
