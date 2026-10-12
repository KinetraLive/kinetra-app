<!--
  Sun/moon toggle for the top bar. Flips to the opposite of the theme currently shown and persists
  it (see $lib/stores/theme.svelte). aria-pressed = dark theme active.

  Props:
    size?: "sm" | "md"                                       (default "md")
    tooltip?: "top" | "bottom" | "left" | "right" | "none"  (default "bottom")
-->
<script lang="ts">
  import Moon from "lucide-svelte/icons/moon";
  import Sun from "lucide-svelte/icons/sun";
  import { theme } from "$lib/stores/theme.svelte";
  import IconButton from "./IconButton.svelte";

  type Props = {
    size?: "sm" | "md";
    tooltip?: "top" | "bottom" | "left" | "right" | "none";
  };

  let { size = "md", tooltip = "bottom" }: Props = $props();

  const dark = $derived(theme.resolved === "dark");
</script>

<IconButton
  {size}
  {tooltip}
  label={dark ? "Switch to light theme" : "Switch to dark theme"}
  pressed={dark}
  pressedFill={false}
  onclick={() => theme.toggle()}
>
  <span class="stack" class:dark>
    <span class="sun"><Sun size={16} /></span>
    <span class="moon"><Moon size={16} /></span>
  </span>
</IconButton>

<style>
  .stack {
    display: grid;
    place-items: center;
  }
  .sun,
  .moon {
    grid-area: 1 / 1;
    display: inline-flex;
    transition:
      transform var(--dur-slow) var(--ease),
      opacity var(--dur-slow) var(--ease);
  }
  /* Show the theme you would switch TO: moon while light, sun while dark. */
  .sun {
    opacity: 0;
    transform: rotate(-90deg) scale(0.6);
  }
  .dark .sun {
    opacity: 1;
    transform: none;
  }
  .dark .moon {
    opacity: 0;
    transform: rotate(90deg) scale(0.6);
  }
</style>
