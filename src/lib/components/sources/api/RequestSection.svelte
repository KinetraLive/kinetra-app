<!--
  RequestSection: the API editor's Request: URL, method (GET / POST), headers and, for POST, the
  body (`linkOnly`: the URL alone, titled "Link", for a CSV link in Simple mode). A header holds a plain value, or with Secret on, a value kept in the OS keychain: typed into
  a write-only password box and stored with Store (setSourceSecret, after the pending edits are
  saved so the header exists). A stored secret reads "Stored in Windows Credential Manager" with
  Replace / Remove; a secret id without a value on this machine (`stored: false`) asks for it again.
  Secret values never enter the document.

  Props:
    sourceId: string
    doc: ApiDoc
    issues: ApiIssues
    onedit: (doc) => void          a working-copy change (autosaved)
    flush: () => Promise<void>      saves pending edits now
    onsecret: (reply) => void       the document a secret command returned
-->
<script lang="ts">
  import KeyRound from "lucide-svelte/icons/key-round";
  import Plus from "lucide-svelte/icons/plus";
  import X from "lucide-svelte/icons/x";
  import { toAppError } from "$lib/api";
  import { clearSourceSecret, setSourceSecret } from "$lib/api/api-sources";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import Select from "$lib/components/Select.svelte";
  import type { ApiHeader, ApiMethod, SourceDoc } from "$lib/types";
  import { secretState, type ApiDoc, type ApiIssues } from "./model";

  let {
    sourceId,
    doc,
    issues,
    onedit,
    flush,
    onsecret,
    linkOnly = false,
  }: {
    sourceId: string;
    doc: ApiDoc;
    issues: ApiIssues;
    onedit: (doc: ApiDoc) => void;
    flush: () => Promise<void>;
    onsecret: (reply: SourceDoc) => void;
    /** A CSV link in Simple mode: "Link" with the URL only (method, headers and body hidden). */
    linkOnly?: boolean;
  } = $props();

  const uid = $props.id();
  const METHODS: { value: ApiMethod; label: string }[] = [
    { value: "GET", label: "GET" },
    { value: "POST", label: "POST" },
  ];

  /** Per header index: the write-only value being typed (shown while entering a secret). */
  let entering = $state<Record<number, string>>({});
  let busy = $state<number | null>(null);
  let errors = $state<Record<number, string>>({});

  const headers = $derived(doc.request.headers);

  function setRequest(patch: Partial<ApiDoc["request"]>) {
    onedit({ ...doc, request: { ...doc.request, ...patch } });
  }

  function setHeader(i: number, patch: Partial<ApiHeader>) {
    setRequest({ headers: headers.map((h, j) => (j === i ? { ...h, ...patch } : h)) });
  }

  function addHeader() {
    setRequest({ headers: [...headers, { name: "", value: "" }] });
  }

  /** Index shifts after removing header `i`. */
  function dropIndex<T>(m: Record<number, T>, i: number): Record<number, T> {
    const out: Record<number, T> = {};
    for (const [k, v] of Object.entries(m)) {
      const n = Number(k);
      if (n !== i) out[n > i ? n - 1 : n] = v;
    }
    return out;
  }

  async function removeHeader(i: number) {
    const h = headers[i];
    if (h.secret && !(await clear(i))) return;
    entering = dropIndex(entering, i);
    errors = dropIndex(errors, i);
    setRequest({ headers: doc.request.headers.filter((_, j) => j !== i) });
  }

  /** Secret on: the plain value moves into the write-only box (not saved until Store). */
  function secretOn(i: number) {
    entering = { ...entering, [i]: headers[i].value ?? "" };
    setHeader(i, { value: null });
  }

  /** Secret off: a stored value is removed from the keychain; an unsaved one is dropped. */
  async function secretOff(i: number) {
    if (headers[i].secret) {
      await clear(i);
      return;
    }
    const { [i]: typed = "", ...rest } = entering;
    entering = rest;
    setHeader(i, { value: typed });
  }

  function fail(i: number, e: unknown) {
    errors = { ...errors, [i]: toAppError(e).message };
  }

  async function store(i: number) {
    const name = headers[i]?.name.trim() ?? "";
    const value = entering[i] ?? "";
    if (!name || !value || busy !== null) return;
    busy = i;
    const { [i]: _old, ...others } = errors;
    errors = others;
    try {
      await flush();
      const reply = await setSourceSecret(sourceId, name, value);
      const { [i]: _done, ...rest } = entering;
      entering = rest;
      onsecret(reply);
    } catch (e) {
      fail(i, e);
    } finally {
      busy = null;
    }
  }

  async function clear(i: number): Promise<boolean> {
    const name = headers[i]?.name.trim() ?? "";
    if (busy !== null) return false;
    busy = i;
    try {
      await flush();
      onsecret(await clearSourceSecret(sourceId, name));
      const { [i]: _gone, ...rest } = entering;
      entering = rest;
      return true;
    } catch (e) {
      fail(i, e);
      return false;
    } finally {
      busy = null;
    }
  }

  function cancelReplace(i: number) {
    const { [i]: _gone, ...rest } = entering;
    entering = rest;
  }
</script>

<section class="section" aria-labelledby="{uid}-title">
  <h3 class="section-label" id="{uid}-title">{linkOnly ? "Link" : "Request"}</h3>

  <div class="url-row">
    {#if !linkOnly}<Select
      size="md"
      aria-label="Method"
      value={doc.request.method}
      options={METHODS}
      onchange={(v) => setRequest({ method: v, body: v === "POST" ? (doc.request.body ?? "") : doc.request.body })}
    />{/if}
    <span class="input grow vf vf-within">
      <input
        type="url"
        class="mono"
        aria-label="URL"
        autocomplete="off"
        spellcheck="false"
        value={doc.request.url}
        aria-invalid={issues.url ? "true" : undefined}
        aria-describedby={issues.url ? `${uid}-url-err` : undefined}
        oninput={(e) => setRequest({ url: e.currentTarget.value })}
      />
    </span>
  </div>
  {#if issues.url}<p class="bad small" id="{uid}-url-err">{issues.url}</p>{/if}

  {#if !linkOnly}
  <div class="headers" role="group" aria-labelledby="{uid}-headers">
    <div class="sub-head">
      <span class="small muted" id="{uid}-headers">Headers</span>
      <Button size="sm" variant="ghost" onclick={addHeader}>
        {#snippet icon()}<Plus size={14} />{/snippet}
        Add header
      </Button>
    </div>
    {#each headers as h, i (i)}
      {@const st = secretState(h)}
      {@const secretUi = st !== "none" || i in entering}
      {@const typing = i in entering || st === "missing"}
      <div class="header" data-header={i}>
        <span class="input name vf vf-within" class:grow={secretUi}>
          <input
            type="text"
            class="mono"
            aria-label="Header {i + 1} name"
            autocomplete="off"
            spellcheck="false"
            value={h.name}
            readonly={st !== "none"}
            title={st !== "none" ? "Remove the secret to rename this header." : undefined}
            aria-invalid={issues.headers[i] ? "true" : undefined}
            oninput={(e) => setHeader(i, { name: e.currentTarget.value })}
          />
        </span>
        {#if !secretUi}
          <span class="input grow vf vf-within">
            <input
              type="text"
              class="mono"
              aria-label="Header {i + 1} value"
              autocomplete="off"
              spellcheck="false"
              value={h.value ?? ""}
              oninput={(e) => setHeader(i, { value: e.currentTarget.value })}
            />
          </span>
        {/if}
        <IconButton
          size="sm"
          label="Secret: keep {h.name || `header ${i + 1}`} in the OS keychain"
          tooltip="top"
          pressed={secretUi}
          pressedFill={false}
          disabled={busy !== null}
          onclick={() => (secretUi ? void secretOff(i) : secretOn(i))}
        >
          <KeyRound size={14} />
        </IconButton>
        <IconButton variant="danger" size="sm" label="Remove header {h.name || i + 1}" tooltip="top" disabled={busy !== null} onclick={() => void removeHeader(i)}>
          <X size={14} />
        </IconButton>
      </div>
      {#if secretUi}
        <div class="secret-line">
          {#if st === "missing"}
            <p class="warn small">Secret missing on this machine — enter it again</p>
          {/if}
          {#if typing}
            <form class="secret-entry" onsubmit={(e) => (e.preventDefault(), void store(i))}>
              <span class="input grow vf vf-within">
                <input
                  type="password"
                  aria-label="Secret value of {h.name || `header ${i + 1}`}"
                  autocomplete="new-password"
                  spellcheck="false"
                  value={entering[i] ?? ""}
                  oninput={(e) => (entering = { ...entering, [i]: e.currentTarget.value })}
                />
              </span>
              <Button size="sm" variant="primary" type="submit" loading={busy === i} disabled={!h.name.trim() || !(entering[i] ?? "")}>
                Store
              </Button>
              {#if st === "stored"}
                <Button size="sm" variant="ghost" onclick={() => cancelReplace(i)}>Cancel</Button>
              {/if}
            </form>
          {:else}
            <div class="secret-entry">
              <span class="stored grow small">
                <KeyRound size={12} aria-hidden="true" />
                Stored in Windows Credential Manager
              </span>
              <Button size="sm" variant="ghost" onclick={() => (entering = { ...entering, [i]: "" })}>Replace</Button>
              <Button size="sm" variant="danger" loading={busy === i} onclick={() => void clear(i)}>Remove</Button>
            </div>
          {/if}
        </div>
      {/if}
      {#if issues.headers[i]}<p class="bad small">{issues.headers[i]}</p>{/if}
      {#if errors[i]}<p class="bad small" role="alert">{errors[i]}</p>{/if}
    {/each}
  </div>

  {#if doc.request.method === "POST"}
    <label class="small muted" for="{uid}-body">Body</label>
    <span class="input area vf vf-within">
      <textarea
        id="{uid}-body"
        class="mono"
        rows="4"
        spellcheck="false"
        value={doc.request.body ?? ""}
        oninput={(e) => setRequest({ body: e.currentTarget.value })}
      ></textarea>
    </span>
    <p class="small muted">Sent as JSON when valid, otherwise as text.</p>
  {/if}
  {/if}
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
  }
  .url-row,
  .header,
  .secret-entry,
  .sub-head {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
  }
  .sub-head {
    justify-content: space-between;
  }
  .headers {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .input {
    --vf-radius: var(--r-control);
    display: flex;
    height: var(--control-h);
    border-radius: var(--r-control);
    background: var(--surface-1);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .input.name {
    flex: 0 1 12rem;
    min-width: 6rem;
  }
  .input.name.grow {
    flex: 1;
  }
  .input.area {
    height: auto;
  }
  .input input,
  .input textarea {
    flex: 1;
    width: 100%;
    min-width: 0;
    padding: 0 var(--s-2);
    border: 0;
    background: transparent;
    font-size: var(--fs-sm);
  }
  .input textarea {
    padding: var(--s-2);
    resize: vertical;
  }
  .input input[readonly] {
    color: var(--text-muted);
  }
  .input:has([aria-invalid="true"]) {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .secret-line {
    display: flex;
    flex-direction: column;
    gap: var(--s-1);
    margin: 0 0 var(--s-1) var(--s-4);
    padding-left: var(--s-2);
    border-left: 1px solid var(--line-strong);
  }
  .secret-line p {
    margin: 0;
  }
  .stored {
    display: inline-flex;
    align-items: center;
    gap: var(--s-1);
    color: var(--text-muted);
  }
  .small {
    font-size: var(--fs-sm);
  }
  .muted {
    color: var(--text-muted);
  }
  .bad {
    color: var(--danger);
  }
  .warn {
    color: var(--warning);
  }
</style>
