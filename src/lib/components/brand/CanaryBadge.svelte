<!--
  CANARY badge after the wordmark, canary builds only: BrandLogo imports it as `$channel-badge`,
  which svelte.config.js points here only when KINETRA_CHANNEL=canary (stable builds get
  NoBadge.svelte, so they carry none of this). Sits exactly on the wordmark's x-height band
  (the artwork's x-height runs from 59.4 to 119 of its 160 units).
-->
<script lang="ts">
  import Tooltip from "$lib/components/Tooltip.svelte";
  import { t } from "$lib/i18n";
</script>

<span class="slot">
  <Tooltip text={t("Experimental build. Don't use for live shows.")} placement="bottom">
    <span class="canary-badge">{t("Canary")}</span>
  </Tooltip>
</span>

<style>
  .slot {
    display: inline-flex;
    flex: none;
    margin-top: calc(var(--logo-h, 32px) * 59.4 / 160);
    margin-left: var(--s-1);
  }
  /* The canary app icon's yellow with near-black text (contrast ≈ 13:1 in both themes). */
  .canary-badge {
    display: inline-flex;
    align-items: center;
    height: calc(var(--logo-h, 32px) * 59.6 / 160);
    padding: 0 6px;
    border-radius: 999px;
    background: #f5c400;
    color: #111111;
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    white-space: nowrap;
  }
</style>
