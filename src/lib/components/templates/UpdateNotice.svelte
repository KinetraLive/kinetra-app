<!--
  UpdateNotice: one quiet line for a template whose built-in preset ships a newer look
  ("A newer look is available · Update"). Update runs the lazy update flow (./update.ts: confirm,
  backup, update, toast with Undo). A lazy chunk itself: the templates list pane, the template
  editor and the layout editor's Inspector load it only when `presetUpdate` is set.

  Props:
    target: { id, name }         the template
    message?: string             the notice text (default "A newer look is available")
    label?: string               the action (default "Update")
    before?: () => Promise<unknown>   runs before the backend writes (the editor saves)
    after?: (doc: TemplateDoc) => void   after the update and after an undo
-->
<script lang="ts">
  import type { TemplateDoc } from "$lib/types";
  import type { UpdateTarget } from "./update";

  let {
    target,
    message = "A newer look is available",
    label = "Update",
    before,
    after,
  }: {
    target: UpdateTarget;
    message?: string;
    label?: string;
    before?: () => Promise<unknown>;
    after?: (doc: TemplateDoc) => void;
  } = $props();

  let busy = $state(false);

  async function run() {
    busy = true;
    try {
      const { updateLook } = await import("./update");
      await updateLook({ id: target.id, name: target.name }, { before, after });
    } finally {
      busy = false;
    }
  }
</script>

<p class="update-notice">
  <span class="text">{message}</span>
  <span class="sep" aria-hidden="true">·</span>
  <button
    type="button"
    class="link vf"
    disabled={busy}
    aria-label={label === "Update" ? `Update “${target.name}” to the latest look` : undefined}
    onclick={run}>{label}</button
  >
</p>

<style>
  .update-notice {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0 var(--s-2);
    min-width: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .text {
    min-width: 0;
  }
  .link {
    height: var(--control-h-sm);
    margin: 0 calc(var(--s-2) * -1);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    font-weight: 500;
  }
  .link:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .link:disabled {
    opacity: 0.6;
  }
</style>
