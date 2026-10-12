<!--
  FixRecipe: "Fix…" for an automation that needs attention (lazy chunk; Control's Automation strip).

  Its source is gone (and its layers are there): pick another source of the kind it runs on (text
  for slideshows, a schedule for schedules and countdowns) and `relinkRecipeSource` points it and
  its rules at that one. The server's refusal (e.g. the wrong kind) shows inline and the dialog
  stays. No source of that kind in the project: it says so and offers Delete.
  One of its layers (or its screen) is gone: a layer can't be brought back from here, so it says
  which and offers Delete (the parent's remove dialog, with its keep / delete-created choice).

  Props: FixProps (./lazy.ts).
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import { relinkRecipeSource } from "$lib/api/recipes";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import Select from "$lib/components/Select.svelte";
  import { sourceKindOf } from "./automation";
  import type { FixProps } from "./lazy";

  let { broken, sources, screenName, onclose, onfixed, ondelete }: FixProps = $props();

  const recipe = $derived(broken.recipe);
  const kind = $derived(sourceKindOf(recipe.panel));
  const kindText = $derived(kind === "text" ? "text source" : "schedule");
  const options = $derived(sources.filter((s) => s.kind === kind).map((s) => ({ value: s.id, label: s.name })));
  const layersGone = $derived(broken.missingLayers.length > 0);

  let picked = $state("");
  let busy = $state(false);
  let error = $state<string | null>(null);

  const value = $derived(picked || options[0]?.value || "");

  const message = $derived.by(() => {
    if (broken.screenMissing) {
      return "Its screen was deleted, so its rules can't run. Delete this automation, then add it again.";
    }
    if (layersGone) {
      const n = broken.missingLayers.length;
      return `${n === 1 ? "Its layer was" : `${n} of its layers were`} deleted from “${screenName}”. Its rules can't run without ${n === 1 ? "it" : "them"}. Delete this automation, then add it again.`;
    }
    if (options.length === 0) {
      return `Its source was deleted, and this project has no other ${kindText}. Add one in Sources, or delete this automation.`;
    }
    return `Its source was deleted. Pick another ${kindText}: its rules follow.`;
  });

  const relinks = $derived(!layersGone && !broken.screenMissing && options.length > 0);

  async function confirm() {
    if (!relinks) {
      ondelete();
      return;
    }
    busy = true;
    error = null;
    try {
      const inst = await relinkRecipeSource(recipe.id, value);
      busy = false;
      onfixed(inst);
    } catch (e) {
      busy = false;
      error = toAppError(e).message;
    }
  }
</script>

<ConfirmDialog
  title="Fix {recipe.name}"
  {message}
  confirmLabel={relinks ? "Use this source" : "Delete…"}
  danger={!relinks}
  {busy}
  onconfirm={confirm}
  oncancel={onclose}
>
  {#if relinks}
    <label class="pick">
      <span class="section-label">Source</span>
      <Select size="sm" aria-label="New source for {recipe.name}" {value} {options} onchange={(v) => (picked = v)} disabled={busy} />
    </label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  {/if}
</ConfirmDialog>

<style>
  .pick {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin-top: var(--s-1);
  }
  .error {
    margin: var(--s-1) 0 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
