<!--
  SheetEditor: the spreadsheet editor for list values and data tables (lazy chunk; hosts import()
  it). Contract: ./types.ts (SheetEditorProps).

    toolbar   title + note (unless hideTitle), Add row, Add column (editable columns), Paste,
              Undo / Redo, the selection's Move up / Move down / Delete, the host's actions
              (`toolbar` snippet, given the active row), and the mono count against the limit
    grid      one focusable role=grid element (aria-activedescendant names the active cell);
              sticky header row and row-number column; rows are virtualised past VIRTUAL_FROM
              (fixed row height, the visible window ± BUFFER rows, spacers above and below)

  Keyboard (on the grid): arrows / Home / End / PageUp / PageDown move (Ctrl to the edge); Tab /
  Shift+Tab along the row with wrap (at either end Tab leaves the grid); Enter or F2 edits, typing
  starts an edit with that character; while editing Enter commits and moves down (on the last row
  it adds a row), Tab commits and moves along, Esc cancels. Esc on the grid calls `onclose`.
  Alt+↑/↓ move the selected rows (or the active one); Shift+↑/↓ and Shift+Space select rows;
  Ctrl+A selects all; Delete removes the selected rows (undo toast) or clears the active cell;
  Ctrl+Z / Ctrl+Shift+Z (Ctrl+Y) undo / redo. Image cells open the asset picker on Enter.

  Mouse: click a cell to make it active (Shift: select the row range, Ctrl: toggle the row); click
  a row number to select rows; double-click edits; drag a row's grip to reorder (the selection
  moves when the grip's row is in it).

  Paste (Ctrl+V on the grid, or Paste): one value goes into the active cell; several cells open
  the column-mapping dialog (PasteDialog, its own lazy chunk). Apply is one undo step.

  Every change calls onchange(rows, columns, origin) at once (the host debounces). Changes past a
  limit are refused with a message. New rows/columns given by the host (a reload) replace the
  working copy and clear undo, unless they hold the same data.

  `readonly` (a preview, e.g. an API source's mapped rows): no edit tools, typing, paste, reorder,
  delete or undo; moving around and selecting rows still work.
-->
<script module lang="ts">
  const VIRTUAL_FROM = 60;
  const BUFFER = 16;
  const QUANTUM = 8;
  const UNDO_CAP = 100;

  let pasteChunk: Promise<typeof import("./PasteDialog.svelte")> | null = null;
  function loadPasteDialog() {
    pasteChunk ??= import("./PasteDialog.svelte");
    pasteChunk.catch(() => (pasteChunk = null));
    return pasteChunk;
  }
</script>

<script lang="ts">
  import { tick, untrack } from "svelte";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import ArrowLeft from "lucide-svelte/icons/arrow-left";
  import ArrowRight from "lucide-svelte/icons/arrow-right";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import ClipboardPaste from "lucide-svelte/icons/clipboard-paste";
  import Columns3 from "lucide-svelte/icons/columns-3";
  import ImageIcon from "lucide-svelte/icons/image";
  import Plus from "lucide-svelte/icons/plus";
  import Redo2 from "lucide-svelte/icons/redo-2";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Undo2 from "lucide-svelte/icons/undo-2";
  import X from "lucide-svelte/icons/x";
  import Button from "$lib/components/Button.svelte";
  import IconButton from "$lib/components/IconButton.svelte";
  import { assetName, thumbUrl } from "$lib/components/assets/assets";
  import { isMultiCell } from "$lib/components/sources/paste";
  import { server } from "$lib/stores/server.svelte";
  import { toasts } from "$lib/stores/toasts.svelte";
  import type { SourceColumn } from "$lib/types";
  import { History } from "./history";
  import { clampCell, enterDown, isMoveKey, isTypingKey, move, tab, type Cell } from "./keys";
  import { cellIssue, columnsMax, countText, rowsIssue } from "./limits";
  import { planPaste, type PastePlan } from "./paste";
  import {
    addColumn,
    identity,
    insertOrder,
    isIdentity,
    keyIssue,
    moveOrder,
    nudgeOrder,
    permute,
    removeColumn,
    removeOrder,
    sameTable,
    setCell,
    setKey,
    setKind,
    setLabel,
    shiftColumn,
    type Table,
  } from "./rows";
  import { NONE, all, range, remap, selectOne, sorted, toggle, type RowSelection } from "./selection";
  import type { SheetEditorProps, SheetRow } from "./types";

  let {
    title,
    note = null,
    columns,
    rows,
    limits,
    editableColumns = false,
    onchange,
    onclose,
    hideTitle = false,
    currentRow = null,
    lockedKeys = [],
    toolbar,
    readonly = false,
  }: SheetEditorProps = $props();

  const uid = $props.id();

  // ---- Working copy ---------------------------------------------------------------------------------

  type Snap = { t: Table; ids: number[] };

  let data = $state.raw<Table>(untrack(() => ({ rows, columns })));
  /** A stable identity per row (follows moves; undo restores it). */
  let ids: number[] = untrack(() => identity(rows.length));
  let nextId = untrack(() => rows.length);
  /** What the host last passed (or we last emitted). */
  let seen: Table = untrack(() => ({ rows, columns }));
  /** Keys of columns added here whose key still follows the label. */
  const autoKeys = new Set<string>();

  const history = new History<Snap>(UNDO_CAP);
  let histV = $state(0);
  const canUndo = $derived(histV >= 0 && history.canUndo);
  const canRedo = $derived(histV >= 0 && history.canRedo);

  let active = $state<Cell>({ row: 0, col: 0 });
  let sel = $state.raw<RowSelection>(NONE);
  let editing = $state<{ row: number; col: number; original: string; pushed: boolean } | null>(null);
  let draft = $state("");
  let message = $state<{ text: string; bad: boolean } | null>(null);

  const n = $derived(data.rows.length);
  const cols = $derived(data.columns.length);
  const locked = $derived(new Set(lockedKeys));
  const hasImages = $derived(data.columns.some((c) => c.kind === "image"));
  // Thumbnails load from the output server: follow its status while an image column shows.
  $effect(() => {
    if (hasImages) return server.watch();
  });

  // Rows / columns from the host: a different table replaces the working copy.
  $effect.pre(() => {
    const t: Table = { rows, columns };
    untrack(() => {
      if (t.rows === seen.rows && t.columns === seen.columns) return;
      seen = t;
      if (sameTable(t, data)) return;
      data = t;
      ids = identity(t.rows.length);
      nextId = t.rows.length;
      history.clear();
      histV++;
      sel = NONE;
      editing = null;
      active = clampCell(active, { rows: t.rows.length, cols: t.columns.length }) ?? { row: 0, col: 0 };
    });
  });

  function say(text: string, bad = true) {
    message = { text, bad };
  }

  const rowsText = (k: number) => `${k} ${k === 1 ? "row" : "rows"}`;

  /** Takes `next` (order: new row i is old row order[i]; null = same rows). False when refused. */
  function apply(next: Table, order: readonly number[] | null, opts: { push?: boolean; check?: boolean } = {}): boolean {
    const { push = true, check = true } = opts;
    if (check) {
      const issue = rowsIssue(next.rows, next.columns, limits, data.rows.length);
      if (issue) {
        say(issue);
        return false;
      }
    }
    if (push) {
      history.push({ t: data, ids });
      histV++;
    }
    const origin = order ?? identity(next.rows.length);
    ids = origin.map((o) => (o < 0 ? nextId++ : ids[o]));
    finish(next, origin);
    return true;
  }

  function finish(next: Table, origin: readonly number[]) {
    const before = data.rows.length;
    data = next;
    seen = next;
    message = null;
    if (origin.length !== before || !isIdentity(origin)) {
      sel = remap(sel, origin);
      const to = origin.indexOf(active.row);
      if (to >= 0) active = { row: to, col: active.col };
    }
    active = clampCell(active, { rows: next.rows.length, cols: next.columns.length }) ?? { row: 0, col: 0 };
    onchange(next.rows, next.columns, origin);
  }

  function restore(s: Snap) {
    const pos = new Map(ids.map((id, i) => [id, i]));
    const origin = s.ids.map((id) => pos.get(id) ?? -1);
    ids = s.ids;
    finish(s.t, origin);
  }

  function undo() {
    endEdit(true);
    const s = history.undo({ t: data, ids });
    histV++;
    if (s) restore(s);
  }

  function redo() {
    endEdit(true);
    const s = history.redo({ t: data, ids });
    histV++;
    if (s) restore(s);
  }

  // ---- Rows -------------------------------------------------------------------------------------

  function addRowAt(at: number): boolean {
    const order = insertOrder(n, at);
    return apply({ rows: permute(data.rows, order, () => ({})), columns: data.columns }, order);
  }

  function addRow() {
    endEdit(true);
    if (addRowAt(n)) void goTo({ row: n - 1, col: 0 }, true);
  }

  function reorder(order: readonly number[]) {
    if (isIdentity(order)) return;
    apply({ rows: permute(data.rows, order, () => ({})), columns: data.columns }, order, { check: false });
  }

  const picked = () => (sel.rows.size > 0 ? sorted(sel) : n > 0 ? [active.row] : []);

  function nudge(dir: -1 | 1) {
    const order = nudgeOrder(n, picked(), dir);
    if (order) reorder(order);
  }

  function deleteRows(list: readonly number[]) {
    if (list.length === 0) return;
    endEdit(true);
    const before: Snap = { t: data, ids };
    const order = removeOrder(n, list);
    apply({ rows: permute(data.rows, order, () => ({})), columns: data.columns }, order, { check: false });
    sel = NONE;
    toasts.push({
      kind: "info",
      message: `Deleted ${rowsText(list.length)}.`,
      action: {
        label: "Undo",
        run: () => {
          history.push({ t: data, ids });
          histV++;
          restore(before);
        },
      },
    });
  }

  function setValue(row: number, col: number, value: string, push = true): boolean {
    const key = data.columns[col]?.key;
    if (key === undefined || row >= n || (data.rows[row]?.[key] ?? "") === value) return false;
    const issue = cellIssue(value, limits);
    if (issue) {
      say(issue);
      return false;
    }
    return apply({ rows: setCell(data.rows, row, key, value), columns: data.columns }, null, {
      push,
      check: limits.bytes !== null,
    });
  }

  // ---- Editing ----------------------------------------------------------------------------------

  let editInput: HTMLInputElement | undefined = $state();

  function startEdit(initial?: string) {
    if (readonly) return;
    const col = data.columns[active.col];
    if (!col || n === 0) return;
    if (col.kind === "image") {
      void chooseImage(active.row, active.col);
      return;
    }
    const original = data.rows[active.row]?.[col.key] ?? "";
    editing = { row: active.row, col: active.col, original, pushed: false };
    draft = initial ?? original;
    if (initial !== undefined) onDraft(initial);
  }

  function onDraft(v: string) {
    const e = editing;
    if (!e) return;
    const before = data.rows[e.row]?.[data.columns[e.col]?.key ?? ""] ?? "";
    if (v === before) {
      draft = v;
      return;
    }
    if (setValue(e.row, e.col, v, !e.pushed)) {
      e.pushed = true;
      draft = v;
    } else if (editInput) editInput.value = before; // refused: keep the last good value
  }

  /** Ends the edit: `keep` keeps what was typed, otherwise the cell goes back. */
  function endEdit(keep: boolean) {
    const e = editing;
    if (!e) return;
    editing = null;
    if (!keep && e.pushed) {
      setValue(e.row, e.col, e.original, false);
      history.pop();
      histV++;
    }
    if (grid && grid.contains(document.activeElement)) grid.focus();
  }

  function onEditKey(e: KeyboardEvent) {
    e.stopPropagation();
    const size = { rows: n, cols };
    if (e.key === "Escape") {
      e.preventDefault();
      endEdit(false);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const d = enterDown(active, size);
      endEdit(true);
      if (d.addRow && !addRowAt(n)) return;
      void goTo(d.cell);
    } else if (e.key === "Tab") {
      const to = tab(active, e.shiftKey, size);
      e.preventDefault();
      endEdit(true);
      void goTo(to);
    }
  }

  let picking = false;
  async function chooseImage(row: number, col: number) {
    const column = data.columns[col];
    if (!column || picking) return;
    picking = true;
    try {
      const { pickAsset } = await import("$lib/components/assets/pick");
      const v = await pickAsset({ current: data.rows[row]?.[column.key] ?? "", label: `${column.label || column.key}, row ${row + 1}` });
      if (v !== null) setValue(row, col, v);
    } catch {
      say("Couldn't open the images.");
    } finally {
      picking = false;
    }
  }

  // ---- Columns ------------------------------------------------------------------------------------

  const maxCols = $derived(columnsMax(limits));

  function onAddColumn() {
    if (cols >= maxCols) {
      say(`Up to ${maxCols} columns.`);
      return;
    }
    const next = addColumn(data);
    autoKeys.add(next.columns[next.columns.length - 1].key);
    apply(next, null, { check: false });
    void tick().then(() => grid?.querySelector<HTMLInputElement>(`input[data-label="${cols - 1}"]`)?.focus());
  }

  /** Column label typing: one undo step per focus. */
  let labelPushed = false;
  function onLabel(c: number, label: string) {
    const col = data.columns[c];
    if (!col) return;
    const rekey = autoKeys.has(col.key) && !locked.has(col.key);
    const next = setLabel(data, c, label, rekey);
    const key = next.columns[c].key;
    if (key !== col.key) {
      autoKeys.delete(col.key);
      autoKeys.add(key);
    }
    apply(next, null, { push: !labelPushed, check: false });
    labelPushed = true;
  }

  function onKey(c: number, input: HTMLInputElement) {
    const col = data.columns[c];
    const key = input.value.trim();
    if (!col || key === col.key) return;
    const issue = keyIssue(data, c, key);
    if (issue) {
      say(`${col.label || col.key}: ${issue}`);
      input.value = col.key;
      return;
    }
    autoKeys.delete(col.key);
    apply(setKey(data, c, key), null, { check: false });
  }

  function columnOp(next: Table) {
    if (next !== data) apply(next, null, { check: false });
  }

  // ---- Navigation and virtualisation ---------------------------------------------------------------

  let grid: HTMLDivElement | undefined = $state();
  let head: HTMLDivElement | undefined = $state();
  let rowH = $state(32);
  let headH = 0;
  let viewH = $state(640);
  let top = $state(0);

  const virtual = $derived(n > VIRTUAL_FROM);
  const start = $derived(virtual ? Math.max(0, top - BUFFER) : 0);
  const end = $derived(virtual ? Math.min(n, top + Math.ceil(viewH / rowH) + BUFFER) : n);
  const visible = $derived.by(() => {
    const out: { r: SheetRow; i: number; id: number }[] = [];
    for (let i = start; i < end; i++) out.push({ r: data.rows[i], i, id: ids[i] });
    return out;
  });

  function syncWindow() {
    if (!grid) return;
    const f = Math.floor(grid.scrollTop / rowH);
    const q = f - (f % QUANTUM);
    if (q !== top) top = q;
  }

  let scrollRaf = 0;
  function onScroll() {
    if (!scrollRaf) {
      scrollRaf = requestAnimationFrame(() => {
        scrollRaf = 0;
        syncWindow();
        // The edited row scrolled out of the rendered window: keep what was typed.
        if (editing && (editing.row < start || editing.row >= end)) {
          endEdit(true);
          grid?.focus({ preventScroll: true });
        }
      });
    }
  }

  $effect(() => {
    const g = grid;
    if (!g) return;
    const measure = () => {
      const h = parseFloat(getComputedStyle(g).getPropertyValue("--row-h"));
      if (h > 0) rowH = h;
      headH = head?.offsetHeight ?? 0;
      if (g.clientHeight > 0) viewH = g.clientHeight;
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(g);
    if (head) ro.observe(head);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(scrollRaf);
      scrollRaf = 0;
    };
  });

  /** Makes `cell` active and scrolls it into view (rendering its row first). */
  async function goTo(cell: Cell, focus = false) {
    const c = clampCell(cell, { rows: n, cols });
    if (!c) return;
    active = c;
    if (grid) {
      const y = c.row * rowH;
      const room = grid.clientHeight - headH;
      if (y < grid.scrollTop) grid.scrollTop = y;
      else if (room > 0 && y + rowH > grid.scrollTop + room) grid.scrollTop = y + rowH - room;
      syncWindow();
    }
    if (focus) grid?.focus();
    await tick();
    const el = grid?.querySelector<HTMLElement>(`#${CSS.escape(cellId(c.row, c.col))}`);
    el?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }

  const cellId = (r: number, c: number) => `${uid}-c${r}-${c}`;
  const activeId = $derived(n > 0 && cols > 0 && active.row >= start && active.row < end ? cellId(active.row, active.col) : undefined);

  function onGridKey(e: KeyboardEvent) {
    if (editing || e.target !== grid) return;
    if (readonly && e.altKey) return;
    if (readonly && !isMoveKey(e.key) && e.key !== "Tab" && e.key !== "Escape" && !(e.key === " " && e.shiftKey) && !((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a")) return;
    const size = { rows: n, cols };
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key;
    if (mod && !e.altKey) {
      const l = k.toLowerCase();
      if (l === "z" || l === "y") {
        e.preventDefault();
        if (l === "y" || e.shiftKey) redo();
        else undo();
      } else if (l === "a") {
        e.preventDefault();
        sel = all(n);
      } else if (isMoveKey(k)) {
        e.preventDefault();
        void goTo(move(active, k, size, true));
      }
      return;
    }
    if (e.altKey) {
      if (k === "ArrowUp" || k === "ArrowDown") {
        e.preventDefault();
        nudge(k === "ArrowUp" ? -1 : 1);
      }
      return;
    }
    if (k === "Escape") {
      if (onclose) {
        e.preventDefault();
        e.stopPropagation();
        onclose();
      }
      return;
    }
    if (n === 0 || cols === 0) {
      if (k === "Enter") {
        e.preventDefault();
        addRow();
      }
      return;
    }
    if (isMoveKey(k)) {
      e.preventDefault();
      const to = move(active, k, size, false, Math.max(1, Math.floor(viewH / rowH) - 1));
      if (e.shiftKey && to.row !== active.row) {
        const base = sel.anchor === null ? selectOne(active.row) : sel;
        sel = range(base, to.row);
      }
      void goTo(to);
    } else if (k === "Tab") {
      const to = tab(active, e.shiftKey, size);
      if (to.row === active.row && to.col === active.col) return; // leave the grid
      e.preventDefault();
      void goTo(to);
    } else if (k === "Enter" || k === "F2") {
      e.preventDefault();
      startEdit();
    } else if (k === " " && e.shiftKey) {
      e.preventDefault();
      sel = selectOne(active.row);
    } else if (k === "Delete" || k === "Backspace") {
      e.preventDefault();
      if (sel.rows.size > 0) deleteRows(sorted(sel));
      else setValue(active.row, active.col, "");
    } else if (isTypingKey(e) && data.columns[active.col]?.kind !== "image") {
      e.preventDefault();
      startEdit(k);
    }
  }

  // ---- Mouse --------------------------------------------------------------------------------------

  function hit(t: EventTarget | null): { row: number; col: number | null } | null {
    const el = (t as Element | null)?.closest?.<HTMLElement>("[data-r]");
    if (!el) return null;
    return { row: Number(el.dataset.r), col: el.dataset.c === undefined ? null : Number(el.dataset.c) };
  }

  function onBodyPointer(e: PointerEvent) {
    if (e.button > 0 || (e.target as Element).closest(".grip, .edit")) return;
    const h = hit(e.target);
    if (!h) return;
    if (editing) endEdit(true);
    if (e.shiftKey) {
      sel = range(sel.anchor === null ? selectOne(active.row) : sel, h.row);
    } else if (e.ctrlKey || e.metaKey) sel = toggle(sel, h.row);
    else if (h.col === null) sel = selectOne(h.row);
    else sel = NONE;
    active = { row: h.row, col: h.col ?? active.col };
  }

  function onBodyDbl(e: MouseEvent) {
    if (readonly) return;
    const h = hit(e.target);
    if (h && h.col !== null && !editing) startEdit();
  }

  let drag = $state<{ picked: number[]; before: number } | null>(null);
  let dragY = 0;
  let dragRaf = 0;

  function onGripDown(e: PointerEvent, row: number) {
    if (e.button > 0) return;
    e.preventDefault();
    endEdit(true);
    if (!sel.rows.has(row)) sel = selectOne(row);
    active = { row, col: active.col };
    grid?.focus();
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    dragY = e.clientY;
    drag = { picked: sorted(sel), before: row };
  }

  function dragFrame() {
    dragRaf = 0;
    if (!drag || !grid) return;
    const box = grid.getBoundingClientRect();
    let scrolled = false;
    if (dragY < box.top + headH + rowH) {
      grid.scrollTop -= rowH / 2;
      scrolled = true;
    } else if (dragY > box.bottom - rowH) {
      grid.scrollTop += rowH / 2;
      scrolled = true;
    }
    if (scrolled) syncWindow();
    const y0 = box.top + headH - grid.scrollTop;
    const before = Math.min(Math.max(0, Math.round((dragY - y0) / rowH)), n);
    if (before !== drag.before) drag = { ...drag, before };
    if (scrolled) dragRaf = requestAnimationFrame(dragFrame);
  }

  function onGripMove(e: PointerEvent) {
    if (!drag) return;
    dragY = e.clientY;
    if (!dragRaf) dragRaf = requestAnimationFrame(dragFrame);
  }

  function onGripUp() {
    if (!drag) return;
    cancelAnimationFrame(dragRaf);
    dragRaf = 0;
    const d = drag;
    drag = null;
    reorder(moveOrder(n, d.picked, d.before));
  }

  // ---- Paste ------------------------------------------------------------------------------------------

  type PasteDialogT = typeof import("./PasteDialog.svelte").default;
  let PasteDialog = $state.raw<PasteDialogT | null>(null);
  let pasting = $state.raw<{ plan: PastePlan; at: number } | null>(null);

  async function openPaste(text: string) {
    if (!isMultiCell(text)) {
      const v = text.replace(/(\r\n|\r|\n)$/, "");
      if (n === 0) {
        if (!addRowAt(0)) return;
        setValue(0, 0, v, false);
      } else setValue(active.row, active.col, v);
      return;
    }
    const plan = planPaste(text, data.columns, editableColumns, maxCols);
    if (!plan) {
      say("The clipboard holds no cells to paste.");
      return;
    }
    try {
      PasteDialog ??= (await loadPasteDialog()).default;
    } catch {
      say("Couldn't open the paste preview.");
      return;
    }
    pasting = { plan, at: n === 0 ? 0 : active.row };
  }

  function onPaste(e: ClipboardEvent) {
    const t = document.activeElement;
    if (readonly || !grid || !t || !grid.contains(t) || pasting) return;
    if (t instanceof HTMLInputElement && !t.classList.contains("edit")) return; // column labels and keys
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (text === "" || (editing && !isMultiCell(text))) return; // one value into the input as usual
    e.preventDefault();
    endEdit(true);
    void openPaste(text);
  }

  async function pasteButton() {
    let text: string;
    try {
      text = await navigator.clipboard.readText();
    } catch {
      say("Couldn't read the clipboard. Click the grid and press Ctrl+V.");
      return;
    }
    if (text.trim() === "") say("The clipboard has no text to paste.");
    else void openPaste(text);
  }

  function applyPasted(next: Table, order: number[], added: number, replaced: number) {
    pasting = null;
    if (apply(next, order, { check: false })) {
      const parts = [];
      if (added > 0) parts.push(`added ${rowsText(added)}`);
      if (replaced > 0) parts.push(`replaced ${rowsText(replaced)}`);
      say(`Pasted: ${parts.join(", ")}.`, false);
    }
  }

  const gridCols = $derived(`var(--num-w) repeat(${Math.max(1, cols)}, minmax(var(--col-min), 1fr))`);
  // Every row is as wide as the grid (content never widens a row, so columns line up).
  const rowW = $derived(`calc(var(--num-w) + ${Math.max(1, cols)} * var(--col-min))`);
  const activeRow = $derived(n > 0 ? active.row : null);
  const selCount = $derived(sel.rows.size);
</script>

<svelte:document onpaste={onPaste} />

<div class="sheet">
  <div class="bar" role="group" aria-label="{title}: table tools">
    {#if !hideTitle}<h2 class="title">{title}</h2>{/if}
    {#if note}<span class="note">{note}</span>{/if}
    {#if !readonly}
    <Button size="sm" onclick={addRow} disabled={n >= limits.rows} title={n >= limits.rows ? `Up to ${limits.rows} rows.` : undefined}>
      {#snippet icon()}<Plus size={14} />{/snippet}
      Add row
    </Button>
    {#if editableColumns}
      <Button size="sm" onclick={onAddColumn} disabled={cols >= maxCols} title={cols >= maxCols ? `Up to ${maxCols} columns.` : undefined}>
        {#snippet icon()}<Columns3 size={14} />{/snippet}
        Add column
      </Button>
    {/if}
    <Button size="sm" variant="ghost" onclick={() => void pasteButton()}>
      {#snippet icon()}<ClipboardPaste size={14} />{/snippet}
      Paste
    </Button>
    <IconButton size="sm" label="Undo" disabled={!canUndo} onclick={undo}><Undo2 size={14} /></IconButton>
    <IconButton size="sm" label="Redo" disabled={!canRedo} onclick={redo}><Redo2 size={14} /></IconButton>
    {/if}
    {#if selCount > 0}
      <span class="sep" aria-hidden="true"></span>
      <span class="small">{selCount} selected</span>
      {#if !readonly}
      <IconButton size="sm" label="Move selected rows up" onclick={() => nudge(-1)}><ArrowUp size={14} /></IconButton>
      <IconButton size="sm" label="Move selected rows down" onclick={() => nudge(1)}><ArrowDown size={14} /></IconButton>
      <Button size="sm" variant="danger" onclick={() => deleteRows(sorted(sel))}>
        {#snippet icon()}<Trash2 size={14} />{/snippet}
        Delete
      </Button>
      {/if}
    {/if}
    {#if toolbar}
      <span class="sep" aria-hidden="true"></span>
      {@render toolbar(activeRow)}
    {/if}
    <span class="count" class:full={n >= limits.rows}>{countText(data.rows, data.columns, limits)}</span>
  </div>
  <p class="msg" class:bad={message?.bad} role="status">{message?.text ?? ""}</p>

  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="grid"
    role="grid"
    tabindex="0"
    aria-label={title}
    aria-rowcount={n + 1}
    aria-colcount={cols + 1}
    aria-multiselectable="true"
    aria-readonly={readonly ? "true" : undefined}
    aria-activedescendant={activeId}
    style:--grid={gridCols}
    style:--row-w={rowW}
    bind:this={grid}
    onkeydown={onGridKey}
    onscroll={onScroll}
  >
    <div class="hrow" role="row" aria-rowindex={1} bind:this={head}>
      <div class="corner" role="columnheader" aria-colindex={1}>#</div>
      {#each data.columns as col, c (c)}
        <div class="hcell" role="columnheader" aria-colindex={c + 2}>
          {#if editableColumns}
            {@const keyLocked = locked.has(col.key)}
            <div class="htop">
              <span class="label vf vf-within">
                <input
                  type="text"
                  autocomplete="off"
                  spellcheck="false"
                  data-label={c}
                  aria-label="Column {c + 1} label"
                  aria-invalid={col.label.trim() === "" ? "true" : undefined}
                  value={col.label}
                  onfocus={() => (labelPushed = false)}
                  oninput={(e) => onLabel(c, e.currentTarget.value)}
                  onkeydown={(e) => e.stopPropagation()}
                />
              </span>
              <IconButton
                size="sm"
                label="Image column"
                tooltip="top"
                pressed={col.kind === "image"}
                pressedFill={false}
                onclick={() => columnOp(setKind(data, c, col.kind === "image" ? "text" : "image"))}
              >
                <ImageIcon size={12} />
              </IconButton>
              <IconButton variant="danger" size="sm" label="Remove column {col.label || c + 1}" tooltip="top" disabled={cols <= 1} onclick={() => columnOp(removeColumn(data, c))}>
                <X size={12} />
              </IconButton>
            </div>
            <span class="keyrow">
              <input
                class="key mono"
                type="text"
                autocomplete="off"
                spellcheck="false"
                aria-label="Column {c + 1} key"
                title={keyLocked ? "Locked: screens use this column." : "Key used by templates and bindings"}
                readonly={keyLocked}
                value={col.key}
                onchange={(e) => onKey(c, e.currentTarget)}
                onkeydown={(e) => {
                  e.stopPropagation();
                  if (e.key === "Enter") e.currentTarget.blur();
                }}
              />
              <IconButton size="sm" label="Move column left" tooltip="top" disabled={c === 0} onclick={() => columnOp(shiftColumn(data, c, -1))}>
                <ArrowLeft size={12} />
              </IconButton>
              <IconButton size="sm" label="Move column right" tooltip="top" disabled={c === cols - 1} onclick={() => columnOp(shiftColumn(data, c, 1))}>
                <ArrowRight size={12} />
              </IconButton>
            </span>
          {:else}
            <span class="hlabel">{col.label || col.key}</span>
            <span class="key mono">{col.key}{#if col.kind === "image"}{" · image"}{/if}</span>
          {/if}
        </div>
      {/each}
    </div>

    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="body" style:height="{n * rowH}px" onpointerdown={onBodyPointer} ondblclick={onBodyDbl}>
      <div aria-hidden="true" style:height="{start * rowH}px"></div>
      {#each visible as v (v.id)}
        {@const i = v.i}
        <div
          class="row"
          class:sel={sel.rows.has(i)}
          class:current={i === currentRow}
          role="row"
          aria-rowindex={i + 2}
          aria-selected={sel.rows.has(i)}
        >
          <div class="num" role="rowheader" aria-colindex={1} data-r={i}>
            {#if !readonly}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <span
              class="grip"
              aria-hidden="true"
              title="Drag to reorder (Alt+↑/↓)"
              onpointerdown={(e) => onGripDown(e, i)}
              onpointermove={onGripMove}
              onpointerup={onGripUp}
              onpointercancel={onGripUp}
            ><svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"
                ><circle cx="9" cy="5" r="1.6" /><circle cx="15" cy="5" r="1.6" /><circle cx="9" cy="12" r="1.6" /><circle
                  cx="15"
                  cy="12"
                  r="1.6"
                /><circle cx="9" cy="19" r="1.6" /><circle cx="15" cy="19" r="1.6" /></svg
              ></span>
            {/if}
            <span class="n">{i + 1}</span>{#if i === currentRow}<span class="visually-hidden">, current row</span>{/if}
          </div>
          {#each data.columns as col, c (c)}
            {@const value = v.r[col.key] ?? ""}
            {@const isActive = i === active.row && c === active.col}
            <div
              class="cell"
              class:active={isActive}
              role="gridcell"
              id={isActive ? cellId(i, c) : undefined}
              aria-colindex={c + 2}
              data-r={i}
              data-c={c}
            >
              {#if editing && editing.row === i && editing.col === c}
                <input
                  class="edit"
                  type="text"
                  autocomplete="off"
                  aria-label="{col.label || col.key}, row {i + 1}"
                  value={draft}
                  bind:this={editInput}
                  {@attach (el) => {
                    el.focus();
                    el.setSelectionRange(el.value.length, el.value.length);
                  }}
                  oninput={(e) => onDraft(e.currentTarget.value)}
                  onkeydown={onEditKey}
                  onblur={() => endEdit(true)}
                />
              {:else if col.kind === "image"}
                {@const src = thumbUrl(server.status, value)}
                {#if src}<img {src} alt="" width="32" height="22" loading="lazy" decoding="async" />{/if}
                <span class="text" class:muted={value === ""}>{value === "" ? "No image" : assetName(value)}</span>
              {:else}
                <span class="text">{value}</span>
              {/if}
            </div>
          {/each}
        </div>
      {/each}
      {#if drag}
        <div class="drop" aria-hidden="true" style:transform="translateY({drag.before * rowH}px)"></div>
      {/if}
    </div>
  </div>

  {#if n === 0 && readonly}
    <div class="empty"><p>No rows.</p></div>
  {:else if n === 0}
    <div class="empty">
      <p>No rows yet. Add one, or paste rows copied from a spreadsheet.</p>
      <Button size="sm" variant="primary" onclick={addRow}>
        {#snippet icon()}<Plus size={14} />{/snippet}
        Add row
      </Button>
    </div>
  {/if}
</div>

{#if PasteDialog && pasting}
  <PasteDialog
    plan={pasting.plan}
    rows={data.rows}
    columns={data.columns}
    {limits}
    at={pasting.at}
    {editableColumns}
    onapply={applyPasted}
    oncancel={() => (pasting = null)}
  />
{/if}

<style>
  .sheet {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
  }
  .bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-2);
    min-height: calc(var(--row-h) + var(--s-2));
    padding: var(--s-1) var(--s-4);
    border-bottom: 1px solid var(--line);
  }
  .title {
    margin-right: var(--s-2);
    font-family: var(--font-heading);
    font-size: var(--fs-lg);
    font-weight: 600;
  }
  .note,
  .small {
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .note {
    margin-right: var(--s-2);
  }
  .sep {
    width: 1px;
    height: var(--s-4);
    margin: 0 var(--s-1);
    background: var(--line);
  }
  .count {
    font-variant-numeric: tabular-nums;
    margin-left: auto;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .count.full {
    color: var(--warning);
  }
  .msg {
    padding: 0 var(--s-4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }
  .msg:not(:empty) {
    padding-block: var(--s-1);
    border-bottom: 1px solid var(--line);
  }
  .msg.bad {
    color: var(--danger);
  }

  /* ---- Grid ---- */
  .grid {
    --num-w: 4.5rem;
    --col-min: 12rem;
    flex: 1;
    min-height: 0;
    overflow: auto;
    outline: none;
    user-select: none;
  }
  .hrow,
  .row {
    display: grid;
    grid-template-columns: var(--grid);
    min-width: var(--row-w);
  }
  .hrow {
    position: sticky;
    top: 0;
    z-index: 2;
    border-bottom: 1px solid var(--line);
    background: var(--surface-1);
  }
  .corner,
  .num {
    position: sticky;
    left: 0;
    z-index: 1;
    background: var(--surface-1);
  }
  .corner {
    display: grid;
    place-items: center;
    color: var(--text-muted);
    font-size: var(--fs-xs);
  }
  .hcell {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 2px;
    min-width: 0;
    min-height: var(--row-h);
    padding: var(--s-1) var(--s-2);
    border-left: 1px solid var(--line);
  }
  .hlabel {
    overflow: hidden;
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .htop {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .label {
    display: flex;
    flex: 1;
    min-width: 0;
    border-radius: var(--r-control);
  }
  .label input {
    width: 100%;
    min-width: 0;
    height: var(--control-h-sm);
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: var(--surface-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
    font-weight: 600;
  }
  .label input[aria-invalid="true"] {
    box-shadow: inset 0 0 0 1px var(--danger);
  }
  .keyrow {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .key {
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    font-size: var(--fs-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  input.key {
    flex: 1;
    height: 20px;
    padding: 0 var(--s-2);
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
  }
  input.key:not([readonly]):hover,
  input.key:focus {
    background: var(--surface-2);
    color: var(--text);
  }
  input.key:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 1px var(--focus);
  }

  .body {
    position: relative;
    min-width: var(--row-w);
  }
  .row {
    height: var(--row-h);
    box-sizing: border-box;
    border-bottom: 1px solid var(--line);
  }
  .row.sel,
  .row.sel .num {
    background: var(--accent-soft);
  }
  .row.sel .num {
    background: color-mix(in srgb, var(--accent) 14%, var(--surface-1));
  }
  .num {
    display: flex;
    align-items: center;
    gap: var(--s-1);
    padding: 0 var(--s-3) 0 var(--s-1);
    color: var(--text-muted);
    font-size: var(--fs-xs);
    cursor: default;
  }
  .num .n {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }
  .row.current .num {
    color: var(--accent-text);
    font-weight: 600;
  }
  .row.current .num::before {
    content: "";
    position: absolute;
    top: 3px;
    bottom: 3px;
    left: 0;
    width: 3px;
    border-radius: var(--r-full);
    background: var(--accent);
  }
  .grip {
    display: inline-flex;
    padding: 2px;
    border-radius: 2px;
    color: var(--text-muted);
    cursor: grab;
    opacity: 0.5;
    touch-action: none;
  }
  .row:hover .grip,
  .grip:active {
    opacity: 1;
  }
  .cell {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    padding: 0 var(--s-2);
    border-left: 1px solid var(--line);
    font-size: var(--fs-md);
  }
  .cell .text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell .muted {
    color: var(--text-muted);
  }
  .cell img {
    flex: none;
    width: 32px;
    height: 22px;
    border-radius: 2px;
    background: var(--surface-2);
    object-fit: contain;
  }
  .cell.active {
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .grid:focus .cell.active,
  .cell.active:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: -2px;
  }
  .edit {
    position: absolute;
    inset: 0;
    width: 100%;
    padding: 0 var(--s-2);
    border: 0;
    background: var(--surface-2);
    outline: 2px solid var(--focus);
    outline-offset: -2px;
    font-size: var(--fs-md);
    user-select: text;
  }
  .drop {
    position: absolute;
    top: -1px;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent);
    pointer-events: none;
  }

  .empty {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--s-3);
    padding: var(--s-4);
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  @media (prefers-reduced-motion: no-preference) {
    .grip {
      transition: opacity var(--dur-fast) var(--ease);
    }
  }
</style>
