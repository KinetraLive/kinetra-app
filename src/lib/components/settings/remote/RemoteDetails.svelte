<!--
  Preferences > Network & API > Remote control > Connection details (a lazy chunk): for support
  and troubleshooting. A counted diagnostics watch while mounted (`remote_diag_watch`), updated by
  `remote-diag` (≤ 2/s, only while open). IDs, sizes, timings and codes only: never values, names
  or tokens.
    Test connection  §4a probe while off, a ping on the live connection while on; the result in
                     plain words with its step times (rate limited: the button waits)
    numbers          state, server version, the 32-screen note (when the project has more), run / show IDs (Copy run ID), round trip (last /
                     median / max) and clock offset, snapshot, messages per type, commands,
                     connections and the last close, recent refused commands
    Export diagnostics…  a JSON file (save dialog); nothing is uploaded
  No props.
-->
<script lang="ts">
  import Button from "$lib/components/Button.svelte";
  import SettingsRow from "$lib/components/SettingsRow.svelte";
  import SettingsSection from "$lib/components/SettingsSection.svelte";
  import TallyDot from "$lib/components/TallyDot.svelte";
  import { on, toAppError } from "$lib/api";
  import { remoteDiagWatch, remoteExportDiagnostics, remoteTest } from "$lib/api/remote";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { RemoteDiag, RemoteTestResult } from "$lib/types";
  import { remote } from "$lib/components/remote/state.svelte";

  let diag = $state.raw<RemoteDiag | null>(null);
  $effect(() => {
    let alive = true;
    const off = on("remote-diag", (d) => (diag = d));
    remoteDiagWatch(true).then((d) => alive && (diag = d), () => {});
    return () => {
      alive = false;
      off();
      void remoteDiagWatch(false).catch(() => {});
    };
  });

  let testing = $state(false);
  let result = $state.raw<RemoteTestResult | null>(null);
  /** Rate limited: Test connection waits until then. */
  let waitUntil = $state(0);
  let now = $state(Date.now());
  $effect(() => {
    if (waitUntil <= now) return;
    const t = setTimeout(() => (now = Date.now()), Math.min(1000, waitUntil - now));
    return () => clearTimeout(t);
  });
  const waitS = $derived(Math.max(0, Math.ceil((waitUntil - now) / 1000)));

  async function test() {
    testing = true;
    try {
      result = await remoteTest();
      if (result.retryAfterS) {
        waitUntil = Date.now() + result.retryAfterS * 1000;
        now = Date.now();
      }
    } catch (e) {
      toasts.push({ kind: "error", message: toAppError(e).message });
    } finally {
      testing = false;
    }
  }

  async function exportDiag() {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const path = await save({
      title: "Export diagnostics",
      defaultPath: "kinetra-remote-diagnostics.json",
      filters: [{ name: "Diagnostics", extensions: ["json"] }],
    });
    if (!path) return;
    try {
      await remoteExportDiagnostics(path);
      toasts.push({ kind: "success", message: `Diagnostics saved to ${path}` });
    } catch (e) {
      toasts.push({ kind: "error", message: `Couldn't export: ${toAppError(e).message}` });
    }
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      toasts.push({ kind: "info", message: "Copied" });
    } catch {
      toasts.push({ kind: "error", message: "Couldn't copy" });
    }
  }

  const ms = (v: number | null | undefined) => (v == null ? "–" : `${v} ms`);
  const kb = (b: number | null | undefined) => (b == null ? "–" : b < 1024 ? `${b} B` : `${(b / 1024).toFixed(1)} KB`);
  const clock = (at: number) => new Date(at).toLocaleTimeString();
  const types = $derived(diag ? Object.entries(diag.counters) : []);
  const STATES: Record<string, string> = {
    live: "Live", connecting: "Connecting", reconnecting: "Reconnecting", showInUse: "Waiting: Move it here?",
    ended: "Ended", stopped: "Stopped", off: "Off",
  };
  const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
</script>

<SettingsSection title="Connection details">
  <SettingsRow label="Test connection" description="Checks sign-in, plan, clock and network.">
    <div class="test">
      <Button disabled={testing || waitS > 0} onclick={test}>
        {testing ? "Testing…" : waitS > 0 ? `Test again in ${waitS} s` : "Test connection"}
      </Button>
      {#if result}
        <p class="result" role="status">
          <TallyDot state={result.ok ? "preview" : "warning"} />
          <span>{result.message}{result.detail ? `: ${result.detail}` : ""}</span>
        </p>
        {#if result.ok}
          <p class="muted tnum">
            {[
              result.connectMs !== null ? `Connect ${ms(result.connectMs)}` : null,
              result.welcomeMs !== null ? `Handshake ${ms(result.welcomeMs)}` : null,
              `Round trip ${ms(result.rttMs)}`,
            ]
              .filter(Boolean)
              .join(" · ")}
          </p>
        {/if}
      {/if}
    </div>
  </SettingsRow>

  {#if diag}
    <SettingsRow label="Connection">
      <div class="lines">
        <span>{STATES[diag.state] ?? diag.state} · protocol <span class="mono">{diag.protocol}</span></span>
        {#if diag.serverVersion}<span class="mono">{diag.serverVersion}</span>{/if}
        {#if diag.runId}
          <span class="id">Run <span class="mono">{diag.runId}</span> <Button size="sm" variant="ghost" onclick={() => diag?.runId && copy(diag.runId)}>Copy run ID</Button></span>
        {/if}
        {#if diag.showId}<span class="id">Show <span class="mono">{diag.showId}</span></span>{/if}
      </div>
    </SettingsRow>
    {#if remote.view?.on && remote.view.screensHidden > 0}
      <SettingsRow label="Screens">
        <span>Remote users see the first 32 screens ({remote.view.screensHidden} more in this project).</span>
      </SettingsRow>
    {/if}
    <SettingsRow label="Round trip">
      <span class="tnum">
        {`Last ${ms(diag.rtt.last)} · median ${ms(diag.rtt.median)} · max ${ms(diag.rtt.max)}`}{diag.clockOffsetMs !== null ? ` · clock offset ${ms(diag.clockOffsetMs)}` : ""}
      </span>
    </SettingsRow>
    <SettingsRow label="State sent">
      <span class="tnum">
        {`Update ${diag.snapshot.seq ?? "–"} · full state ${kb(diag.snapshot.bytes)}, sent ${diag.snapshot.sent}× · ${plural(diag.snapshot.resyncs, "resync", "resyncs")}`}{#if diag.snapshot.lastResync}{" (last: "}<span class="mono">{diag.snapshot.lastResync.reason}</span>){/if}
      </span>
    </SettingsRow>
    <SettingsRow label="Commands">
      <span class="tnum">
        {`${diag.commands.received} received · ${diag.commands.accepted} done`}{diag.commands.resultMsMedian !== null ? ` · answered in ${ms(diag.commands.resultMsMedian)} (max ${ms(diag.commands.resultMsMax)})` : ""}
      </span>
    </SettingsRow>
    <SettingsRow label="Connections">
      <span class="tnum">
        {`${plural(diag.connects, "connect", "connects")} · ${plural(diag.reconnects.length, "reconnect", "reconnects")}`}{#if diag.lastClose}{" · last close "}<span class="mono">{diag.lastClose.code} {diag.lastClose.reason}</span>{/if}{#if diag.errors.length}{" · last error "}<span class="mono">{diag.errors[diag.errors.length - 1].code}</span>{/if}
      </span>
    </SettingsRow>
    {#if types.length > 0}
      <SettingsRow label="Messages" stacked>
        <table class="tbl tnum">
          <thead><tr><th>Type</th><th>In</th><th>Out</th><th>Bytes in</th><th>Bytes out</th></tr></thead>
          <tbody>
            {#each types as [t, c] (t)}
              <tr><td class="mono">{t}</td><td>{c.in}</td><td>{c.out}</td><td>{kb(c.inBytes)}</td><td>{kb(c.outBytes)}</td></tr>
            {/each}
          </tbody>
        </table>
      </SettingsRow>
    {/if}
    {#if diag.refused.length > 0}
      <SettingsRow label="Refused commands" stacked>
        <table class="tbl tnum">
          <thead><tr><th>Time</th><th>Action</th><th>Role</th><th>Code</th><th>Target</th></tr></thead>
          <tbody>
            {#each [...diag.refused].reverse() as r (r.corr + r.at)}
              <tr>
                <td>{clock(r.at)}</td><td class="mono">{r.action}</td><td>{r.role}</td><td class="mono">{r.code}</td>
                <td class="mono">{r.layer ?? r.cue ?? ""}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </SettingsRow>
    {/if}
  {/if}

  <SettingsRow label="Diagnostics" description="For support. No content or names.">
    <Button onclick={exportDiag}>Export diagnostics…</Button>
  </SettingsRow>
</SettingsSection>

<style>
  .test,
  .lines {
    display: grid;
    justify-items: start;
    gap: var(--s-1);
    min-width: 0;
  }
  .result {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    margin: 0;
  }
  p {
    margin: 0;
  }
  .id {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .mono {
    user-select: text;
  }
  .muted {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-sm);
  }
  .tbl th {
    color: var(--text-muted);
    font-weight: 500;
    text-align: left;
  }
  .tbl th,
  .tbl td {
    padding: var(--s-1) var(--s-2) var(--s-1) 0;
    border-bottom: 1px solid var(--line);
  }
</style>
