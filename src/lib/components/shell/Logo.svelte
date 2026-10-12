<!--
  Kinetra logo for the top bar (brand/BrandLogo: the Stagger mark and the "kinetra" wordmark,
  following the theme; canary builds add the CANARY badge). A link to the dashboard; its
  accessible name is "Kinetra".

  Props:
    href?: string   link target (default "/dashboard")
-->
<script lang="ts">
  import BrandLogo from "$lib/components/brand/BrandLogo.svelte";

  let { href = "/dashboard" }: { href?: string } = $props();
</script>

<a class="logo vf" {href} aria-label="Kinetra">
  <BrandLogo height={32} />
</a>

<style>
  .logo {
    --vf-radius: var(--r-control);
    display: inline-flex;
    flex: none;
    align-items: center;
    height: var(--control-h);
    padding: 0 var(--s-2) 0 var(--s-1);
    border-radius: var(--r-control);
    color: var(--text);
    text-decoration: none;
  }
</style>
