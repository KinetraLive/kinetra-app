<!--
  The Kinetra logo from the brand kit (src/assets/brand/): the Stagger mark (three rounded bars,
  the top one in the brand accent) and the "kinetra" wordmark (Syne 700, outlined: no font). Two
  images, one per theme ("for dark bg" on the dark theme, "for light bg" on the light one), chosen
  by CSS from <html data-theme>, so a theme switch never reloads anything. Canary builds add the
  CANARY badge after the wordmark (`$channel-badge`, chosen at build time in svelte.config.js;
  stable builds get an empty component and carry no canary text or styles).

  Props:
    height?: number   image height in px (default 32); the visible artwork is ~60 % of it
    badge?: boolean   show the channel badge (default true; canary builds only)

  (The kit's mark-only SVGs are in src/assets/brand/ too; nothing uses them yet, and importing
  them here would inline them into every route's JS.)
-->
<script lang="ts">
  import logoDark from "../../../assets/brand/kinetra-logo-for-dark-bg.svg";
  import logoLight from "../../../assets/brand/kinetra-logo-for-light-bg.svg";
  import ChannelBadge from "$channel-badge";

  let { height = 32, badge = true }: { height?: number; badge?: boolean } = $props();
  // The artwork's viewBox is 639.9 × 160.
  const width = $derived(Math.round((639.9 / 160) * height));
</script>

<span class="brand" style:--logo-h="{height}px">
  <!-- One accessible name whichever theme image shows; the badge stays outside it. -->
  <span class="art" role="img" aria-label="Kinetra">
    <img class="for-dark" src={logoDark} alt="" {width} {height} draggable="false" />
    <img class="for-light" src={logoLight} alt="" {width} {height} draggable="false" />
  </span>
  {#if badge}<ChannelBadge />{/if}
</span>

<style>
  .brand {
    display: inline-flex;
    flex: none;
    align-items: flex-start;
    height: var(--logo-h);
  }
  .art {
    display: inline-flex;
    flex: none;
  }
  img {
    flex: none;
    user-select: none;
  }
  .for-light {
    display: none;
  }
  :global([data-theme="light"]) .for-dark {
    display: none;
  }
  :global([data-theme="light"]) .for-light {
    display: block;
  }
</style>
