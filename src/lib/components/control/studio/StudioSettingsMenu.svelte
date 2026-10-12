<!--
  StudioSettingsMenu: the toolbar's Studio mode settings button and its popover (lazy studio
  chunk; the toolbar shows it once the chunk is loaded). Esc in the popover closes it and
  refocuses the button.

  Props:
    studio: StudioCtl
-->
<script lang="ts">
  import Settings2 from "lucide-svelte/icons/settings-2";
  import type { StudioCtl } from "./studio.svelte";
  import StudioSettingsPop from "./StudioSettingsPop.svelte";

  let { studio }: { studio: StudioCtl } = $props();

  let open = $state(false);
  let button: HTMLButtonElement | undefined = $state();
</script>

<span class="anchor">
  <button
    type="button"
    class="icon vf"
    aria-label="Studio mode settings"
    aria-expanded={open}
    title="Studio mode settings"
    data-studio-trigger
    bind:this={button}
    onclick={() => (open = !open)}
  >
    <Settings2 size={14} />
  </button>
  {#if open}
    <StudioSettingsPop
      settings={studio.settings}
      onchange={studio.set}
      onclose={(refocus) => {
        open = false;
        if (refocus) button?.focus();
      }}
    />
  {/if}
</span>

<style>
  .anchor {
    position: relative;
    display: inline-flex;
  }
  .icon {
    --vf-radius: var(--r-control);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-h-sm);
    height: var(--control-h-sm);
    border-radius: var(--r-control);
    color: var(--text-muted);
  }
  .icon:hover,
  .icon[aria-expanded="true"] {
    background: var(--surface-3);
    color: var(--text);
  }
</style>
