<!--
  Settings group: an uppercase section label (+ optional description, + optional actions on the
  right) above its rows. No card or panel: rows sit on the page background and are separated by
  full-width 1px --line dividers (one above every row, one below the last).

  The section is a size container: SettingsRow lays out as two columns (label + description left,
  control right; columns from --settings-cols) and stacks below 560px.

  Props:
    title: string
    description?: string
    actions?: Snippet    right side of the header (e.g. a "Create snapshot now" button)
    headingLevel?: 2 | 3                                     (default 2)
    children: Snippet    usually <SettingsRow>s; other full-width content (lists, inline panels)
                         can sit between them

  Example:
    <SettingsSection title="Backups" description="Snapshots of the project folder.">
      <SettingsRow label="Automatic backups" for="bk-on">
        <Switch id="bk-on" label="Automatic backups" hideLabel bind:checked={on} />
      </SettingsRow>
    </SettingsSection>
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  type Props = {
    title: string;
    description?: string;
    actions?: Snippet;
    headingLevel?: 2 | 3;
    children: Snippet;
  };

  let { title, description, actions, headingLevel = 2, children }: Props = $props();
  const id = $props.id();
</script>

<section class="section" aria-labelledby="{id}-h">
  <header class="head">
    <div class="text">
      <svelte:element this={`h${headingLevel}`} id="{id}-h" class="section-label title">{title}</svelte:element>
      {#if description}<p class="desc">{description}</p>{/if}
    </div>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
  </header>
  <div class="body">
    {@render children()}
  </div>
</section>

<style>
  .section {
    /* Label column : control column. The label column never gets narrower than a readable
       description; the control column starts at the same x in every section. */
    --settings-cols: minmax(220px, 5fr) minmax(0, 7fr);
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: var(--s-4);
    min-height: var(--control-h);
    padding-bottom: var(--s-2);
  }
  .text {
    min-width: 0;
  }
  .desc {
    margin-top: var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    gap: var(--s-2);
    flex: none;
  }
  .body {
    min-width: 0;
    border-bottom: 1px solid var(--line);
  }
</style>
