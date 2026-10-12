<!--
  VariantMenu: the active variant's actions (lazy chunk, opened from VariantTabs' ⋯ button):
  Duplicate as variant…, Rename…, Set as default, Delete…. Duplicate and Rename ask for a name in
  a dialog; Delete asks for confirmation and is disabled for the last variant.

  Keyboard: opens with focus on the first item; ArrowUp/ArrowDown/Home/End move; Escape closes
  and refocuses the ⋯ button; Tab or a press outside closes it.

  Props:
    variant: VariantDoc        the variant the actions apply to
    isDefault: boolean
    names: string[]            every variant's name (duplicate names are refused)
    count: number              number of variants (limits: add ≤ TEMPLATE_VARIANTS_MAX, delete ≥ 2)
    actions: VariantActions    each resolves true on success (the dialog then closes)
    onclose: (refocus: boolean) => void
-->
<script lang="ts">
  import { onMount, tick } from "svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import { TEMPLATE_VARIANTS_MAX, VARIANT_NAME_MAX, type VariantDoc } from "$lib/types";
  import type { VariantActions } from "./VariantTabs.svelte";
  import { copyName, variantNameIssue } from "./variants";

  let {
    variant,
    isDefault,
    names,
    count,
    actions,
    onclose,
  }: {
    variant: VariantDoc;
    isDefault: boolean;
    names: readonly string[];
    count: number;
    actions: VariantActions;
    onclose: (refocus: boolean) => void;
  } = $props();

  const uid = $props.id();
  type Mode = "menu" | "duplicate" | "rename" | "delete";
  let mode = $state<Mode>("menu");
  let menu: HTMLDivElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let name = $state("");
  let tried = $state(false);
  let busy = $state(false);

  const full = $derived(count >= TEMPLATE_VARIANTS_MAX);
  const others = $derived(mode === "rename" ? names.filter((n) => n !== variant.name) : names);
  const issue = $derived(variantNameIssue(name, others));

  const items = $derived([
    {
      id: "duplicate",
      label: "Duplicate as variant…",
      disabled: full,
      hint: full ? `Up to ${TEMPLATE_VARIANTS_MAX} variants` : undefined,
    },
    { id: "rename", label: "Rename…", disabled: false, hint: undefined },
    { id: "default", label: "Set as default", disabled: isDefault, hint: isDefault ? "Default" : undefined },
    {
      id: "delete",
      label: "Delete…",
      disabled: count <= 1,
      hint: count <= 1 ? "Last variant" : undefined,
    },
  ]);

  const all = () => [...(menu?.querySelectorAll<HTMLButtonElement>("[role=menuitem]:not(:disabled)") ?? [])];

  onMount(() => {
    void tick().then(() => all()[0]?.focus());
    const onDown = (e: PointerEvent) => {
      if (mode === "menu" && menu && !menu.contains(e.target as Node) && !(e.target as Element)?.closest?.(".more-wrap"))
        onclose(false);
    };
    window.addEventListener("pointerdown", onDown, true);
    return () => window.removeEventListener("pointerdown", onDown, true);
  });

  function onMenuKey(e: KeyboardEvent) {
    const list = all();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (e.key === "ArrowDown") next = (i + 1) % list.length;
    else if (e.key === "ArrowUp") next = (i - 1 + list.length) % list.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = list.length - 1;
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onclose(true);
      return;
    } else if (e.key === "Tab") {
      onclose(false);
      return;
    }
    if (next < 0) return;
    e.preventDefault();
    list[next]?.focus();
  }

  async function pick(id: string) {
    if (id === "default") {
      onclose(true);
      await actions.setDefault(variant.id);
      return;
    }
    tried = false;
    name = id === "duplicate" ? copyName(variant.name, names) : id === "rename" ? variant.name : "";
    mode = id as Mode;
    // After the dialog has opened (it focuses its own button first).
    if (id !== "delete")
      requestAnimationFrame(() => {
        input?.focus();
        input?.select();
      });
  }

  async function confirm() {
    if (busy) return;
    if (mode !== "delete") {
      tried = true;
      if (issue) return;
    }
    busy = true;
    let ok = false;
    try {
      if (mode === "duplicate") ok = await actions.duplicate(name.trim());
      else if (mode === "rename") ok = name.trim() === variant.name || (await actions.rename(variant.id, name.trim()));
      else if (mode === "delete") ok = await actions.remove(variant.id);
    } finally {
      busy = false;
    }
    if (ok) onclose(mode !== "delete" && mode !== "duplicate");
  }

  function cancel() {
    onclose(true);
  }
</script>

{#if mode === "menu"}
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div class="menu" role="menu" aria-label="Variant {variant.name}" bind:this={menu} onkeydown={onMenuKey}>
    {#each items as item (item.id)}
      <button
        type="button"
        class="item vf"
        class:danger={item.id === "delete"}
        role="menuitem"
        tabindex="-1"
        data-action={item.id}
        disabled={item.disabled}
        onclick={() => void pick(item.id)}
      >
        <span class="label">{item.label}</span>
        {#if item.hint}<span class="hint">{item.hint}</span>{/if}
      </button>
    {/each}
  </div>
{:else if mode === "delete"}
  <ConfirmDialog
    title="Delete variant?"
    message="“{variant.name}” will be deleted. Layers showing it switch to the default variant."
    confirmLabel="Delete"
    danger
    {busy}
    onconfirm={() => void confirm()}
    oncancel={cancel}
  />
{:else}
  <ConfirmDialog
    title={mode === "duplicate" ? `Duplicate “${variant.name}” as a variant` : "Rename variant"}
    confirmLabel={mode === "duplicate" ? "Duplicate" : "Rename"}
    {busy}
    onconfirm={() => void confirm()}
    oncancel={cancel}
  >
    <label class="field" for="{uid}-name">
      <span class="field-label">Name</span>
      <span class="vf vf-within input-wrap">
        <input
          bind:this={input}
          id="{uid}-name"
          type="text"
          autocomplete="off"
          maxlength={VARIANT_NAME_MAX + 10}
          bind:value={name}
          aria-invalid={tried && issue ? "true" : undefined}
          aria-describedby={tried && issue ? `${uid}-err` : undefined}
          onkeydown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void confirm();
            }
          }}
        />
      </span>
    </label>
    {#if tried && issue}<p class="err" id="{uid}-err">{issue}</p>{/if}
  </ConfirmDialog>
{/if}

<style>
  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + var(--s-1));
    right: 0;
    display: flex;
    flex-direction: column;
    width: max-content;
    min-width: 14rem;
    padding: var(--s-1);
    border-radius: var(--r-panel);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-height: var(--row-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    color: var(--text);
    text-align: left;
  }
  .item:hover:not(:disabled),
  .item:focus-visible {
    background: var(--surface-2);
  }
  .item.danger {
    color: var(--danger);
  }
  .item:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .label {
    flex: 1;
  }
  .hint {
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .field-label {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .input-wrap {
    display: flex;
    border-radius: var(--r-control);
  }
  input {
    width: 100%;
    height: var(--control-h);
    padding: 0 var(--s-2);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .err {
    margin-top: var(--s-1);
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
