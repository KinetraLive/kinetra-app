<!--
  CodeEditor: one CodeMirror 6 view. CodeMirror is loaded with a dynamic import of ./codemirror
  (the only module that touches @codemirror/*), so it stays out of every other bundle.

  Props:
    lang: "html" | "css"
    doc: string            initial text (read once; the view owns the text afterwards)
    label: string          accessible name of the editable area
    onchange: () => void   after every edit (read the text lazily through `onready`'s handle)
    onready: (handle) => void   the handle (getValue, length, focus, destroy) once created
    completions: () => { key, label }[]   field/variable keys for autocompletion, read on demand

  The view is created on mount and destroyed on unmount.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import type { CompletionItem, EditorHandle, EditorLang } from "./codemirror";

  let {
    lang,
    doc,
    label,
    onchange,
    onready,
    completions,
  }: {
    lang: EditorLang;
    doc: string;
    label: string;
    onchange: () => void;
    onready: (handle: EditorHandle) => void;
    completions: () => ReadonlyArray<CompletionItem>;
  } = $props();

  let host: HTMLDivElement | undefined = $state();
  let failed = $state<string | null>(null);

  onMount(() => {
    let handle: EditorHandle | null = null;
    let alive = true;
    const initial = untrack(() => doc);
    import("./codemirror").then(
      (cm) => {
        if (!alive || !host) return;
        handle = cm.createEditor(host, {
          lang,
          doc: initial,
          label,
          onChange: () => onchange(),
          completions: () => completions(),
        });
        onready(handle);
      },
      (e: unknown) => {
        if (alive) failed = e instanceof Error ? e.message : String(e);
      },
    );
    return () => {
      alive = false;
      handle?.destroy();
      handle = null;
    };
  });
</script>

<div class="code" bind:this={host} data-lang={lang}>
  {#if failed}<p class="failed">Couldn't load the code editor: {failed}</p>{/if}
</div>

<style>
  .code {
    height: 100%;
    min-height: 0;
    background: var(--surface-1);
  }
  .failed {
    padding: var(--s-4);
    color: var(--danger);
  }
</style>
