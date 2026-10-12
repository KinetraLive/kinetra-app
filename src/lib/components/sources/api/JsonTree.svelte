<!--
  JsonTree: a test response as a collapsible tree (role=tree, one tab stop). Rendered as a flat list
  of the visible nodes with aria-level, so a big response costs only what is expanded; a container
  shows its first 50 children and a "Show more" row for the rest.

  Keyboard: ↑/↓ move, → expands (or goes to the first child), ← collapses (or goes to the parent),
  Home/End, Enter runs `onactivate` (the editor's default action: rows for a list, a column for a
  value inside a row). Clicking a node selects it.

  Props:
    json: unknown
    label: string                        the tree's accessible name
    selected: string | null              the selected node's JSON Pointer (bindable)
    reveal?: string | null               expanded to (with its first child) whenever it changes
    mark: (ptr) => Mark | null           rows / mapped column keys, highlighted
    onactivate?: (ptr) => void           Enter / double-click
-->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import { pointerOf, tokensOf, type Mark } from "./model";

  const PAGE = 50;

  let {
    json,
    label,
    selected = $bindable(null),
    reveal = null,
    mark,
    onactivate,
  }: {
    json: unknown;
    label: string;
    selected?: string | null;
    reveal?: string | null;
    mark: (ptr: string) => Mark | null;
    onactivate?: (ptr: string) => void;
  } = $props();

  type Row =
    | { kind: "node"; ptr: string; name: string | null; depth: number; value: unknown; size: number | null; open: boolean }
    | { kind: "more"; ptr: string; depth: number; rest: number };

  let expanded = $state.raw<ReadonlySet<string>>(new Set([""]));
  let shown = $state.raw<Readonly<Record<string, number>>>({});

  const isContainer = (v: unknown) => v !== null && typeof v === "object";
  const sizeOf = (v: unknown) => (Array.isArray(v) ? v.length : isContainer(v) ? Object.keys(v as object).length : null);

  const rows = $derived.by(() => {
    const out: Row[] = [];
    const visit = (value: unknown, ptr: string, name: string | null, depth: number, path: (string | number)[]) => {
      const size = sizeOf(value);
      const open = size !== null && expanded.has(ptr);
      out.push({ kind: "node", ptr, name, depth, value, size, open });
      if (!open) return;
      const limit = shown[ptr] ?? PAGE;
      const entries: [string | number, unknown][] = Array.isArray(value)
        ? value.slice(0, limit).map((v, i) => [i, v])
        : Object.entries(value as object).slice(0, limit);
      for (const [k, v] of entries) {
        const p = [...path, k];
        visit(v, pointerOf(p), String(k), depth + 1, p);
      }
      if ((size ?? 0) > limit) out.push({ kind: "more", ptr: `${ptr}#more`, depth: depth + 1, rest: (size ?? 0) - limit });
    };
    visit(json, "", null, 0, []);
    return out;
  });

  const nodes = $derived(rows.filter((r): r is Extract<Row, { kind: "node" }> => r.kind === "node"));
  const focusPtr = $derived(nodes.some((n) => n.ptr === selected) ? selected : (nodes[0]?.ptr ?? ""));

  // Expands the path to `reveal` (and it) when it changes.
  $effect(() => {
    const target = reveal;
    if (target === null || target === undefined) return;
    untrack(() => {
      const tokens = tokensOf(target);
      if (!tokens) return;
      const next = new Set(expanded);
      for (let i = 0; i <= tokens.length; i++) next.add(pointerOf(tokens.slice(0, i)));
      // The first row of a list, so the mapped values show.
      next.add(pointerOf([...tokens, 0]));
      expanded = next;
    });
  });

  let root: HTMLDivElement | undefined = $state();

  function toggle(ptr: string, open?: boolean) {
    const next = new Set(expanded);
    if (open ?? !next.has(ptr)) next.add(ptr);
    else next.delete(ptr);
    expanded = next;
  }

  async function focusOn(ptr: string) {
    selected = ptr;
    await tick();
    root?.querySelector<HTMLElement>(`[data-ptr="${CSS.escape(ptr)}"]`)?.focus();
  }

  const parentOf = (ptr: string) => {
    const t = tokensOf(ptr);
    return t && t.length > 0 ? pointerOf(t.slice(0, -1)) : null;
  };

  function onkeydown(e: KeyboardEvent) {
    const i = nodes.findIndex((n) => n.ptr === focusPtr);
    const n = nodes[i];
    if (!n) return;
    let to: string | null | undefined;
    switch (e.key) {
      case "ArrowDown":
        to = nodes[i + 1]?.ptr;
        break;
      case "ArrowUp":
        to = nodes[i - 1]?.ptr;
        break;
      case "Home":
        to = nodes[0]?.ptr;
        break;
      case "End":
        to = nodes.at(-1)?.ptr;
        break;
      case "ArrowRight":
        if (n.size === null) return;
        if (!n.open) toggle(n.ptr, true);
        else to = nodes[i + 1]?.depth === n.depth + 1 ? nodes[i + 1].ptr : undefined;
        break;
      case "ArrowLeft":
        if (n.open) toggle(n.ptr, false);
        else to = parentOf(n.ptr);
        break;
      case "Enter":
        onactivate?.(n.ptr);
        break;
      case " ":
        selected = n.ptr;
        break;
      default:
        return;
    }
    e.preventDefault();
    if (to !== undefined && to !== null) void focusOn(to);
  }

  function preview(v: unknown): string {
    if (typeof v === "string") return `"${v.length > 120 ? `${v.slice(0, 120)}…` : v}"`;
    return v === undefined ? "" : String(v);
  }
</script>

<div class="tree" role="tree" aria-label={label} tabindex="-1" bind:this={root} {onkeydown}>
  {#each rows as r (r.ptr)}
    {#if r.kind === "more"}
      <div class="more" role="none" style:--depth={r.depth}>
        <button
          type="button"
          class="more-btn"
          onclick={() => {
            const p = r.ptr.slice(0, -"#more".length);
            shown = { ...shown, [p]: (shown[p] ?? PAGE) + PAGE * 4 };
          }}
        >
          Show {Math.min(r.rest, PAGE * 4)} more of {r.rest}
        </button>
      </div>
    {:else}
      {@const m = mark(r.ptr)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="node vf"
        class:mapped={m !== null && m.keys.length > 0}
        class:rows={m?.rows}
        role="treeitem"
        data-ptr={r.ptr}
        aria-level={r.depth + 1}
        aria-expanded={r.size !== null ? r.open : undefined}
        aria-selected={r.ptr === selected}
        tabindex={r.ptr === focusPtr ? 0 : -1}
        style:--depth={r.depth}
        onclick={() => (selected = r.ptr)}
        ondblclick={() => onactivate?.(r.ptr)}
      >
        {#if r.size !== null}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            class="twist"
            class:open={r.open}
            aria-hidden="true"
            onclick={(e) => {
              e.stopPropagation();
              selected = r.ptr;
              toggle(r.ptr);
            }}
          ><ChevronRight size={12} /></span>
        {:else}
          <span class="twist" aria-hidden="true"></span>
        {/if}
        <span class="key mono">{r.name ?? "root"}</span>
        {#if r.size !== null}
          <span class="size mono">{Array.isArray(r.value) ? `[${r.size}]` : `{${r.size}}`}</span>
        {:else}
          <span class="val mono" class:str={typeof r.value === "string"} class:nil={r.value === null}>{preview(r.value)}</span>
        {/if}
        {#if m?.rows}<span class="tag rows-tag">rows</span>{/if}
        {#each m?.keys ?? [] as k (k)}<span class="tag mono">{k}</span>{/each}
      </div>
    {/if}
  {/each}
</div>

<style>
  .tree {
    display: flex;
    flex-direction: column;
    padding: var(--s-1);
    font-size: var(--fs-sm);
  }
  .tree:focus-visible {
    outline: none;
  }
  .node,
  .more {
    --vf-radius: var(--r-control);
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-height: 26px;
    padding: 0 var(--s-2) 0 calc(var(--s-2) + var(--depth) * 16px);
    border-radius: var(--r-control);
    white-space: nowrap;
  }
  .node {
    position: relative;
    cursor: pointer;
  }
  .node:hover {
    background: var(--surface-2);
  }
  .node[aria-selected="true"] {
    background: var(--selected-bg);
  }
  .node.mapped {
    background: var(--accent-soft);
  }
  /* The rows node: a 2px accent bar on its left edge (::after is the focus ring / selection border). */
  .node.rows::before {
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 0;
    width: 2px;
    border-radius: var(--r-full);
    background: var(--accent);
    content: "";
  }
  .twist {
    display: inline-grid;
    flex: none;
    place-items: center;
    width: 16px;
    height: 16px;
    color: var(--text-muted);
  }
  .twist :global(svg) {
    transition: transform var(--dur-fast) var(--ease);
  }
  .twist.open :global(svg) {
    transform: rotate(90deg);
  }
  .key {
    color: var(--text);
  }
  .size,
  .nil {
    color: var(--text-muted);
  }
  .val {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-muted);
  }
  .val.str {
    color: var(--accent-text);
  }
  .tag {
    flex: none;
    padding: 0 var(--s-1);
    border-radius: var(--r-control);
    background: var(--accent);
    color: var(--accent-ink);
    font-size: var(--fs-xs);
    line-height: 18px;
  }
  .rows-tag {
    font-family: var(--font-heading);
    font-weight: 600;
    letter-spacing: var(--tracking-label);
    text-transform: uppercase;
  }
  .more-btn {
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .more-btn:hover {
    background: var(--surface-2);
  }
  @media (prefers-reduced-motion: reduce) {
    .twist :global(svg) {
      transition: none;
    }
  }
</style>
