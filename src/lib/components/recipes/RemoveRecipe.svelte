<!--
  RemoveRecipe: confirms removing an installed recipe (lazy chunk). "Also delete what it created"
  (its source, templates and layers) is off by default: removing the recipe alone keeps the
  graphics and data and only drops its panel and rules. A failed remove shows the error in a toast
  and keeps the dialog open.

  Props: RemoveProps (./lazy.ts).
-->
<script lang="ts">
  import { toAppError } from "$lib/api";
  import { removeRecipe } from "$lib/api/recipes";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { RemoveProps } from "./lazy";

  let { recipe, onclose, onremoved }: RemoveProps = $props();

  let deleteCreated = $state(false);
  let busy = $state(false);

  const parts = $derived(
    [
      recipe.sourceId ? "its source" : null,
      recipe.templateIds.length > 0 ? `${recipe.templateIds.length} ${recipe.templateIds.length === 1 ? "template" : "templates"}` : null,
      recipe.layerIds.length > 0 ? `${recipe.layerIds.length} ${recipe.layerIds.length === 1 ? "layer" : "layers"}` : null,
    ].filter((p) => p !== null),
  );

  async function confirm() {
    busy = true;
    try {
      await removeRecipe(recipe.id, deleteCreated);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't remove ${recipe.name}: ${toAppError(e).message}` });
      busy = false;
      return;
    }
    busy = false;
    onremoved(deleteCreated);
  }
</script>

<ConfirmDialog
  title="Remove {recipe.name}?"
  message="Its panel and rules go away. What it created stays in the project unless you delete it too."
  confirmLabel="Remove"
  danger
  {busy}
  onconfirm={confirm}
  oncancel={onclose}
>
  <label class="check">
    <input type="checkbox" bind:checked={deleteCreated} disabled={busy} />
    Also delete what it created{parts.length > 0 ? ` (${parts.join(", ")})` : ""}
  </label>
  {#if deleteCreated}
    <p class="warn">Layers go off air and are deleted; this can't be undone.</p>
  {/if}
</ConfirmDialog>

<style>
  .check {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    margin-top: var(--s-1);
  }
  .check input {
    accent-color: var(--accent);
  }
  .warn {
    margin: var(--s-1) 0 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
