<!--
  Preferences > About (D3): version, build channel and commit, Check for updates (./updates.svelte:
  up to date / "Version X is available" with its notes and Install and restart, disabled with the
  reason while anything is on air; install progress), links to kinetra.live, the docs and the Discord community (opened
  in the browser), the runtime versions and folders (mono values; long paths ellipsize with the
  full path in the tooltip; the Log folder row carries "Open log folder") and Third-party licenses
  (LicensesDialog, a lazy chunk; focus returns to the button on close).
  Help › Check for updates opens this section with a check requested (`updateRequest`). No props.
-->
<script lang="ts">
  import { tick } from "svelte";
  import Download from "lucide-svelte/icons/download";
  import ExternalLink from "lucide-svelte/icons/external-link";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import MessagesSquare from "lucide-svelte/icons/messages-square";
  import RefreshCw from "lucide-svelte/icons/refresh-cw";
  import ScrollText from "lucide-svelte/icons/scroll-text";
  import { toAppError } from "$lib/api";
  import { appInfo } from "$lib/api/app";
  import BrandLogo from "$lib/components/brand/BrandLogo.svelte";
  import Button from "$lib/components/Button.svelte";
  import ProgressBar from "$lib/components/ProgressBar.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import { revealInExplorer } from "$lib/projectFlow";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { AppInfo } from "$lib/types";
  import { formatBytePair } from "./format";
  import { firstVisible } from "./lazy.svelte";
  import { updateRequest } from "./state.svelte";
  import { CHECK_FAILED, UP_TO_DATE, updater } from "./updates.svelte";

  const WEBSITE = "https://kinetra.live";
  const DOCS = "https://kinetra.live/docs";
  const DISCORD = "https://discord.gg/nK4JaEh3";
  const CHANNELS = { stable: "Stable", canary: "Canary" } as const;

  let info = $state.raw<AppInfo | null>(null);
  let error = $state<string | null>(null);

  const build = $derived([
    { label: "Version", value: info?.version, mono: true },
    { label: "Channel", value: info ? (CHANNELS[info.channel] ?? info.channel) : undefined, mono: false },
    { label: "Commit", value: info?.commit, mono: true },
  ]);
  const runtime = $derived([
    { label: "Tauri", value: info?.tauriVersion },
    { label: "WebView", value: info?.webviewVersion },
    { label: "Config folder", value: info?.configDir },
    { label: "Log folder", value: info?.logDir, logs: true },
  ]);

  const st = $derived(updater.state);
  $effect(() => updater.answerRequest(updateRequest.seq));

  const progressText = $derived.by(() => {
    if (st.kind !== "installing") return "";
    const p = st.progress;
    if (p?.phase === "install") return "Installing…";
    if (p && p.total) return `Downloading ${formatBytePair(p.downloaded, p.total)}`;
    return "Downloading…";
  });

  async function load() {
    try {
      info = await appInfo();
    } catch (e) {
      error = `Couldn't read app info: ${toAppError(e).message}`;
    }
  }

  async function openLink(e: MouseEvent, url: string) {
    e.preventDefault();
    try {
      const { openUrl } = await import("@tauri-apps/plugin-opener");
      await openUrl(url);
    } catch (err) {
      toasts.push({ kind: "error", message: `Couldn't open the browser: ${toAppError(err).message}` });
    }
  }

  type LicensesDialogType = typeof import("./LicensesDialog.svelte").default;
  let Licenses = $state.raw<LicensesDialogType | null>(null);
  let licensesBtn: HTMLElement | null = null;

  async function openLicenses(e: MouseEvent) {
    licensesBtn = e.currentTarget as HTMLElement;
    try {
      Licenses = (await import("./LicensesDialog.svelte")).default;
    } catch (err) {
      toasts.push({ kind: "error", message: `Couldn't open the licenses: ${toAppError(err).message}` });
    }
  }

  function closeLicenses() {
    Licenses = null;
    const b = licensesBtn;
    void tick().then(() => b?.isConnected && b.focus());
  }

  async function openLogs() {
    if (!info) return;
    try {
      await revealInExplorer(info.logDir);
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't show the folder: ${toAppError(e).message}` });
    }
  }
</script>

<div {@attach firstVisible(load)}>
  <SettingsSection title="About">
    <div class="brand"><BrandLogo height={40} /></div>
    {#if error}
      <SettingsRow label="App info"><span class="muted">{error}</span></SettingsRow>
    {:else}
      <dl class="list">
        {#each build as it (it.label)}
          <div class="item">
            <dt>{it.label}</dt>
            <dd><span class="value" class:mono={it.mono} title={it.value}>{it.value ?? "…"}</span></dd>
          </div>
        {/each}
      </dl>
    {/if}

    <SettingsRow label="Updates">
      <div class="updates">
        <div class="line">
          <Button
            id="pref-check-updates"
            size="sm"
            loading={st.kind === "checking"}
            disabled={st.kind === "installing"}
            onclick={() => updater.check()}
          >
            {#snippet icon()}<RefreshCw size={14} />{/snippet}
            Check for updates
          </Button>
          <p class="result" role="status">
            {#if st.kind === "current"}
              {UP_TO_DATE}
            {:else if st.kind === "failed"}
              <span class="warn">{CHECK_FAILED}</span>
            {:else if st.kind === "available" || st.kind === "installing"}
              <span class="strong">Version <span class="mono">{st.check.version}</span> is available</span>
            {/if}
          </p>
        </div>
        {#if st.kind === "available" || st.kind === "installing"}
          {#if st.check.notes}<p class="notes">{st.check.notes}</p>{/if}
          {#if st.kind === "installing"}
            <div class="progress">
              <ProgressBar
                label="Installing the update"
                value={st.progress?.phase === "download" && st.progress.total ? st.progress.downloaded : undefined}
                max={st.progress?.total ?? 100}
                valueText={progressText}
              />
              <span class="muted tnum">{progressText}</span>
            </div>
          {:else}
            <div class="line">
              <Button
                id="pref-install-update"
                size="sm"
                variant="primary"
                disabled={!!st.check.blocked}
                aria-describedby={st.check.blocked ? "pref-update-blocked" : undefined}
                onclick={() => updater.install()}
              >
                {#snippet icon()}<Download size={14} />{/snippet}
                Install and restart
              </Button>
              {#if st.check.blocked}
                <span id="pref-update-blocked" class="muted">{st.check.blocked}</span>
              {/if}
            </div>
            {#if st.installError}<p class="error" role="alert">{st.installError}</p>{/if}
          {/if}
        {/if}
      </div>
    </SettingsRow>

    <SettingsRow label="Website">
      <a class="link mono" href={WEBSITE} onclick={(e) => openLink(e, WEBSITE)}>
        kinetra.live <ExternalLink size={12} aria-hidden="true" />
      </a>
    </SettingsRow>
    <SettingsRow label="Documentation">
      <a class="link mono" href={DOCS} onclick={(e) => openLink(e, DOCS)}>
        kinetra.live/docs <ExternalLink size={12} aria-hidden="true" />
      </a>
    </SettingsRow>
    <SettingsRow label="Community">
      <Button id="pref-discord" size="sm" onclick={(e: MouseEvent) => openLink(e, DISCORD)}>
        {#snippet icon()}<MessagesSquare size={14} />{/snippet}
        Join our Discord
      </Button>
    </SettingsRow>

    {#if !error}
      <dl class="list">
        {#each runtime as it (it.label)}
          <div class="item">
            <dt>{it.label}</dt>
            <dd>
              <span class="value mono" title={it.value}>{it.value ?? "…"}</span>
              {#if it.logs}
                <Button id="pref-logs" size="sm" variant="ghost" disabled={!info} onclick={openLogs}>
                  {#snippet icon()}<FolderOpen size={14} />{/snippet}
                  Open log folder
                </Button>
              {/if}
            </dd>
          </div>
        {/each}
      </dl>
    {/if}
    <SettingsRow label="Third-party licenses">
      <Button id="pref-licenses" size="sm" onclick={openLicenses}>
        {#snippet icon()}<ScrollText size={14} />{/snippet}
        View licenses
      </Button>
    </SettingsRow>
  </SettingsSection>
</div>

{#if Licenses}<Licenses onclose={closeLicenses} />{/if}

<style>
  /* The artwork's left margin: line the bars up with the rows. */
  .brand {
    margin: 0 0 var(--s-3) -5px;
    line-height: 0;
  }
  .list {
    margin: 0;
  }
  .item {
    display: grid;
    grid-template-columns: var(--settings-cols);
    align-items: center;
    gap: var(--s-2) var(--s-6);
    min-height: calc(var(--row-h) + var(--s-4));
    padding: var(--s-2) 0;
    border-top: 1px solid var(--line);
  }
  dt {
    font-weight: 500;
  }
  dd {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    min-width: 0;
    margin: 0;
  }
  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-sm);
    color: var(--text-muted);
    user-select: text;
  }
  .updates {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    min-width: 0;
    width: 100%;
    padding: var(--s-1) 0;
  }
  .line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2) var(--s-3);
    min-width: 0;
  }
  .result {
    margin: 0;
    font-size: var(--fs-sm);
  }
  .strong {
    font-weight: 600;
  }
  .notes {
    max-width: 70ch;
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm);
    line-height: 1.45;
    white-space: pre-wrap;
    user-select: text;
  }
  .progress {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    max-width: 360px;
    font-size: var(--fs-sm);
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    border-radius: var(--r-control);
    color: var(--accent-text);
    font-size: var(--fs-sm);
    text-decoration: none;
  }
  .link:hover {
    text-decoration: underline;
  }
  .link:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  @container (max-width: 560px) {
    .item {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .muted {
    color: var(--text-muted);
  }
  .warn {
    color: var(--warning);
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
</style>
