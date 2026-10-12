// The whole CodeMirror 6 setup for the template editor. This is the ONLY module that imports
// @codemirror/* and it is itself only loaded with a dynamic `import("./codemirror")` from
// CodeEditor.svelte, so CodeMirror lives in a lazy chunk of the editor route and never enters any
// other route's bundle.
//
//   const cm = await import("./codemirror");
//   const handle = cm.createEditor(parent, { lang: "html", doc, label, onChange, completions });
//   handle.getValue();   // read the text only when needed (debounced), never per keystroke
//   handle.destroy();    // on unmount
//
// HTML: a slim language on @lezer/html with CSS nested in <style> and style="" (what templates
// use). @codemirror/lang-html is not used: it statically bundles the JavaScript grammar for
// <script> and on* attributes (~35 KB gz), and scripts in templates never run in the renderer.
// CSS: the same language as @codemirror/lang-css, defined here on @lezer/css; lang-css itself (its
// keyword lists) is loaded only on the first CSS completion request, in a small chunk of its own.
//
// Theme: built from the app's CSS custom properties (var(--surface-1), var(--text), ...), so it
// follows the light/dark theme switch without reconfiguring the view.

import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
} from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  bracketMatchingHandle,
  continuedIndent,
  foldInside,
  foldNodeProp,
  HighlightStyle,
  indentNodeProp,
  indentOnInput,
  indentUnit,
  LanguageSupport,
  LRLanguage,
  syntaxHighlighting,
  syntaxTree,
} from "@codemirror/language";
import {
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  highlightSelectionMatches,
  replaceAll,
  replaceNext,
  search,
  searchKeymap,
  SearchQuery,
  setSearchQuery,
} from "@codemirror/search";
import { EditorSelection, EditorState, type Extension, type Text } from "@codemirror/state";
import {
  Decoration,
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  MatchDecorator,
  ViewPlugin,
  type DecorationSet,
  type Panel,
  type ViewUpdate,
} from "@codemirror/view";
import type { SyntaxNode } from "@lezer/common";
import { parser as cssParser } from "@lezer/css";
import { tags as t } from "@lezer/highlight";
import { configureNesting, parser as htmlParser } from "@lezer/html";

export type EditorLang = "html" | "css";

export interface CompletionItem {
  /** Field or variable key. */
  key: string;
  /** Human label, shown as the completion detail. */
  label: string;
}

export interface EditorOptions {
  lang: EditorLang;
  /** Initial text. */
  doc: string;
  /** Accessible name of the editable area. */
  label: string;
  /** Called after every document change (no arguments: read the text lazily). */
  onChange: () => void;
  /** Current field keys (HTML: `{{key}}`) or variable keys (CSS: `var(--key)`), read on demand. */
  completions: () => ReadonlyArray<CompletionItem>;
}

export interface EditorHandle {
  getValue(): string;
  /** Document length in characters (O(1)). */
  length(): number;
  focus(): void;
  destroy(): void;
}

// ---- Theme ------------------------------------------------------------------------------------

const theme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--text)",
    backgroundColor: "var(--surface-1)",
    fontSize: "var(--fs-md)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.6",
    overflow: "auto",
  },
  ".cm-content": {
    caretColor: "var(--accent)",
    padding: "var(--s-2) 0",
  },
  ".cm-line": { padding: "0 var(--s-3)" },
  ".cm-cursor, .cm-dropCursor": { borderLeft: "2px solid var(--accent)" },
  ".cm-selectionBackground": { backgroundColor: "var(--accent-soft)" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground": {
    backgroundColor: "color-mix(in srgb, var(--accent) 30%, transparent)",
  },
  ".cm-activeLine": { backgroundColor: "color-mix(in srgb, var(--text) 4%, transparent)" },
  ".cm-gutters": {
    backgroundColor: "var(--surface-1)",
    color: "var(--text-muted)",
    border: "none",
    borderRight: "1px solid var(--line)",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 var(--s-2) 0 var(--s-3)", minWidth: "4ch" },
  ".cm-activeLineGutter": {
    backgroundColor: "color-mix(in srgb, var(--text) 4%, transparent)",
    color: "var(--text)",
  },
  "&.cm-focused .cm-matchingBracket": {
    backgroundColor: "var(--accent-soft)",
    outline: "1px solid var(--accent-text)",
  },
  "&.cm-focused .cm-nonmatchingBracket": { outline: "1px solid var(--danger)" },
  ".cm-selectionMatch": { backgroundColor: "color-mix(in srgb, var(--accent) 12%, transparent)" },
  ".cm-searchMatch": {
    backgroundColor: "color-mix(in srgb, var(--warning) 24%, transparent)",
    outline: "1px solid color-mix(in srgb, var(--warning) 60%, transparent)",
  },
  ".cm-searchMatch.cm-searchMatch-selected": {
    backgroundColor: "color-mix(in srgb, var(--accent) 35%, transparent)",
  },
  ".cm-kfield": {
    backgroundColor: "var(--accent-soft)",
    borderRadius: "3px",
  },
  // Autocomplete / tooltips
  ".cm-tooltip": {
    backgroundColor: "var(--surface-3)",
    color: "var(--text)",
    border: "1px solid var(--line-strong)",
    borderRadius: "var(--r-control)",
    overflow: "hidden",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--font-mono)",
    fontSize: "var(--fs-sm)",
    maxHeight: "16em",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li": { padding: "2px var(--s-2)" },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li[aria-selected]": {
    backgroundColor: "var(--accent-soft)",
    color: "var(--text)",
  },
  ".cm-completionDetail": {
    marginLeft: "var(--s-2)",
    color: "var(--text-muted)",
    fontFamily: "var(--font-body)",
    fontStyle: "normal",
  },
  ".cm-completionIcon": { display: "none" },
  ".cm-completionMatchedText": { textDecoration: "none", color: "var(--accent-text)" },
  // Search panel (ours, see searchPanel below)
  ".cm-panels": {
    backgroundColor: "var(--surface-2)",
    color: "var(--text)",
  },
  ".cm-panels.cm-panels-bottom": { borderTop: "1px solid var(--line)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--line)" },
  ".cm-k-search": {
    display: "flex",
    flexWrap: "wrap",
    alignItems: "center",
    gap: "var(--s-2)",
    padding: "var(--s-2) var(--s-3)",
    fontFamily: "var(--font-body)",
    fontSize: "var(--fs-sm)",
  },
  ".cm-k-search label": {
    display: "inline-flex",
    alignItems: "center",
    gap: "var(--s-1)",
    color: "var(--text-muted)",
  },
  ".cm-k-search input[type=text]": {
    width: "14em",
    height: "var(--control-h-sm)",
    padding: "0 var(--s-2)",
    border: "1px solid var(--line-strong)",
    borderRadius: "var(--r-control)",
    backgroundColor: "var(--surface-1)",
    color: "var(--text)",
    fontFamily: "var(--font-mono)",
    fontSize: "var(--fs-sm)",
  },
  ".cm-k-search input[type=text]:focus-visible": {
    outline: "none",
    borderColor: "var(--focus)",
  },
  ".cm-k-search input[type=checkbox]": { accentColor: "var(--accent)" },
  ".cm-k-search button": {
    height: "var(--control-h-sm)",
    padding: "0 var(--s-2)",
    borderRadius: "var(--r-control)",
    backgroundColor: "var(--surface-3)",
    boxShadow: "inset 0 0 0 1px var(--line-strong)",
    color: "var(--text)",
    fontSize: "var(--fs-sm)",
  },
  ".cm-k-search button:hover": {
    backgroundColor: "color-mix(in srgb, var(--text) 6%, var(--surface-3))",
  },
  ".cm-k-search .cm-k-close": { marginLeft: "auto" },
});

/** Calm syntax colours from the tokens (all pass contrast on --surface-1 in both themes). */
const highlight = HighlightStyle.define([
  { tag: [t.tagName, t.keyword, t.modifier, t.controlKeyword], color: "var(--accent-text)" },
  { tag: [t.propertyName, t.definition(t.propertyName)], color: "var(--accent-text)" },
  { tag: [t.attributeName], color: "var(--text-muted)" },
  { tag: [t.string, t.attributeValue, t.special(t.string)], color: "var(--warning)" },
  // Not --danger: that is for errors only (t.invalid below).
  { tag: [t.number, t.unit, t.color, t.integer, t.float], color: "var(--text)" },
  { tag: [t.variableName, t.special(t.variableName)], color: "var(--accent-text)", fontStyle: "italic" },
  { tag: [t.className, t.labelName, t.typeName], color: "var(--text)", fontWeight: "600" },
  { tag: [t.atom, t.bool, t.constant(t.name)], color: "var(--text)" },
  { tag: t.comment, color: "var(--text-muted)", fontStyle: "italic" },
  {
    tag: [t.punctuation, t.angleBracket, t.bracket, t.separator, t.operator],
    color: "var(--text-muted)",
  },
  { tag: [t.processingInstruction, t.documentMeta, t.meta], color: "var(--text-muted)" },
  { tag: t.invalid, color: "var(--danger)", textDecoration: "underline wavy" },
]);

// ---- CSS language -------------------------------------------------------------------------------
// As @codemirror/lang-css defines it.

const cssLanguage = LRLanguage.define({
  name: "css",
  parser: cssParser.configure({
    props: [
      indentNodeProp.add({ Declaration: continuedIndent() }),
      foldNodeProp.add({ "Block KeyframeList": foldInside }),
    ],
  }),
  languageData: {
    commentTokens: { block: { open: "/*", close: "*/" } },
    indentOnInput: /^\s*\}$/,
    wordChars: "-",
  },
});

let cssKeywords: Promise<CompletionSource> | null = null;
/** lang-css's completions (properties, values, pseudo-classes, tags, variables), loaded on first use. */
const cssCompletions: CompletionSource = (ctx) => {
  cssKeywords ??= import("@codemirror/lang-css").then((m) => m.cssCompletionSource);
  return cssKeywords.then((source) => source(ctx));
};

function css(): LanguageSupport {
  return new LanguageSupport(cssLanguage, cssLanguage.data.of({ autocomplete: cssCompletions }));
}

// ---- HTML language ------------------------------------------------------------------------------
// Indentation, folding and tag bracket matching follow @codemirror/lang-html.

const htmlLanguage = LRLanguage.define({
  name: "html",
  parser: htmlParser.configure({
    props: [
      indentNodeProp.add({
        Element(cx) {
          const after = /^(\s*)(<\/)?/.exec(cx.textAfter)!;
          if (cx.node.to <= cx.pos + after[0].length) return cx.continue();
          return cx.lineIndent(cx.node.from) + (after[2] ? 0 : cx.unit);
        },
        "OpenTag CloseTag SelfClosingTag"(cx) {
          return cx.column(cx.node.from) + cx.unit;
        },
        Document(cx) {
          if (cx.pos + /\s*/.exec(cx.textAfter)![0].length < cx.node.to) return cx.continue();
          let end: SyntaxNode | null = null;
          for (let cur: SyntaxNode = cx.node; ; ) {
            const last = cur.lastChild;
            if (!last || last.name !== "Element" || last.to !== cur.to) break;
            end = cur = last;
          }
          const close = end?.lastChild;
          if (end && !(close && (close.name === "CloseTag" || close.name === "SelfClosingTag")))
            return cx.lineIndent(end.from) + cx.unit;
          return null;
        },
      }),
      foldNodeProp.add({
        Element(node) {
          const first = node.firstChild;
          const last = node.lastChild;
          if (!first || first.name !== "OpenTag" || !last) return null;
          return { from: first.to, to: last.name === "CloseTag" ? last.from : node.to };
        },
      }),
      bracketMatchingHandle.add({
        "OpenTag CloseTag": (node) => node.getChild("TagName"),
      }),
    ],
    wrap: configureNesting(
      [{ tag: "style", parser: cssLanguage.parser }],
      [{ name: "style", parser: cssLanguage.parser.configure({ top: "Styles" }) }],
    ),
  }),
  languageData: {
    commentTokens: { block: { open: "<!--", close: "-->" } },
    indentOnInput: /^\s*<\/\w+\W$/,
    wordChars: "-_",
  },
});

const selfClosers = new Set(
  "area base br col embed hr img input link meta param source track wbr".split(" "),
);

function elementName(doc: Text, tree: SyntaxNode | null, max = doc.length): string {
  if (!tree) return "";
  const tag = tree.firstChild;
  const name = tag && tag.getChild("TagName");
  return name ? doc.sliceString(name.from, Math.min(name.to, max)) : "";
}

function isClosed(doc: Text, elt: SyntaxNode, name: string): boolean {
  for (;;) {
    if (elt.lastChild?.name !== "CloseTag") return false;
    const next = elt.parent;
    if (!next || elementName(doc, next) !== name) return true;
    elt = next;
  }
}

/** Inserts the close tag when `>` ends an open tag or `</` starts one (as lang-html does). */
const autoCloseTags = EditorView.inputHandler.of((view, from, to, text, insert) => {
  if (view.composing || view.state.readOnly || from !== to || (text !== ">" && text !== "/"))
    return false;
  if (!htmlLanguage.isActiveAt(view.state, from, -1)) return false;
  const base = insert();
  const { state } = base;
  const closeTags = state.changeByRange((range) => {
    const didType = state.doc.sliceString(range.from - 1, range.to) === text;
    const { head } = range;
    const after = syntaxTree(state).resolveInner(head, -1);
    let name: string;
    if (didType && text === ">" && after.name === "EndTag") {
      const tag = after.parent;
      if (
        tag?.parent &&
        (name = elementName(state.doc, tag.parent, head)) &&
        !selfClosers.has(name) &&
        !isClosed(state.doc, tag.parent, name)
      ) {
        const end = head + (state.doc.sliceString(head, head + 1) === ">" ? 1 : 0);
        return { range, changes: { from: head, to: end, insert: `</${name}>` } };
      }
    } else if (didType && text === "/" && after.name === "IncompleteCloseTag") {
      const tag = after.parent;
      if (
        tag &&
        after.from === head - 2 &&
        tag.lastChild?.name !== "CloseTag" &&
        (name = elementName(state.doc, tag, head)) &&
        !selfClosers.has(name)
      ) {
        const end = head + (state.doc.sliceString(head, head + 1) === ">" ? 1 : 0);
        const ins = `${name}>`;
        return {
          range: EditorSelection.cursor(head + ins.length, -1),
          changes: { from: head, to: end, insert: ins },
        };
      }
    }
    return { range };
  });
  if (closeTags.changes.empty) return false;
  view.dispatch([base, state.update(closeTags, { userEvent: "input.complete", scrollIntoView: true })]);
  return true;
});

/** HTML with CSS in <style> and style="" (CSS completions there, as in the CSS tab). */
function html(): LanguageSupport {
  return new LanguageSupport(htmlLanguage, [autoCloseTags, css().support]);
}

// ---- `{{key}}` marks in HTML --------------------------------------------------------------------

const fieldMark = Decoration.mark({ class: "cm-kfield" });
const fieldMatcher = new MatchDecorator({
  regexp: /\{\{\s*[a-z][a-z0-9_-]*\s*\}\}/g,
  decoration: () => fieldMark,
});
const fieldMarks = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = fieldMatcher.createDeco(view);
    }
    update(u: ViewUpdate) {
      this.decorations = fieldMatcher.updateDeco(u, this.decorations);
    }
  },
  { decorations: (v) => v.decorations },
);

// ---- Completions ----------------------------------------------------------------------------------

function fieldCompletions(items: () => ReadonlyArray<CompletionItem>) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const m = ctx.matchBefore(/\{\{\s*[a-z0-9_-]*$/);
    if (!m) return null;
    const from = m.from + m.text.length - (m.text.match(/[a-z0-9_-]*$/)?.[0].length ?? 0);
    const closed = ctx.state.sliceDoc(ctx.pos, ctx.pos + 2) === "}}";
    const options: Completion[] = items().map((f) => ({
      label: f.key,
      detail: f.label,
      type: "variable",
      apply: closed ? f.key : `${f.key}}}`,
    }));
    return { from, options, validFor: /^[a-z0-9_-]*$/ };
  };
}

function variableCompletions(items: () => ReadonlyArray<CompletionItem>) {
  return (ctx: CompletionContext): CompletionResult | null => {
    const m = ctx.matchBefore(/var\(\s*-{0,2}[a-z0-9_-]*$/);
    if (!m) return null;
    const from = m.from + m.text.search(/-|[a-z0-9_]*$/);
    const closed = ctx.state.sliceDoc(ctx.pos, ctx.pos + 1) === ")";
    const options: Completion[] = items().map((v) => ({
      label: `--${v.key}`,
      detail: v.label,
      type: "variable",
      apply: closed ? `--${v.key}` : `--${v.key})`,
    }));
    return { from, options, validFor: /^-{0,2}[a-z0-9_-]*$/ };
  };
}

// ---- Search panel (visible labels, no placeholders) -----------------------------------------------

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Partial<HTMLElementTagNameMap[K]> & Record<string, unknown> = {},
  ...children: (Node | string)[]
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  Object.assign(e, props);
  for (const c of children) e.append(c);
  return e;
}

function searchPanel(view: EditorView): Panel {
  let query = getSearchQuery(view.state);
  const find = el("input", { type: "text", value: query.search, spellcheck: false });
  find.setAttribute("main-field", "true");
  const replace = el("input", { type: "text", value: query.replace, spellcheck: false });
  const matchCase = el("input", { type: "checkbox", checked: query.caseSensitive });
  const regexp = el("input", { type: "checkbox", checked: query.regexp });

  function commit() {
    const next = new SearchQuery({
      search: find.value,
      replace: replace.value,
      caseSensitive: matchCase.checked,
      regexp: regexp.checked,
    });
    if (!next.eq(query)) {
      query = next;
      view.dispatch({ effects: setSearchQuery.of(next) });
    }
  }
  function button(label: string, run: () => void, cls = "") {
    const b = el("button", { type: "button", className: cls, textContent: label });
    b.addEventListener("click", () => {
      commit();
      run();
    });
    return b;
  }

  const dom = el(
    "div",
    { className: "cm-k-search" },
    el("label", {}, "Find", find),
    el("label", {}, "Replace", replace),
    el("label", {}, matchCase, "Match case"),
    el("label", {}, regexp, "Regex"),
    button("Previous", () => findPrevious(view)),
    button("Next", () => findNext(view)),
    button("Replace", () => replaceNext(view)),
    button("Replace all", () => replaceAll(view)),
    button("Close", () => closeSearchPanel(view), "cm-k-close"),
  );
  dom.addEventListener("input", commit);
  dom.addEventListener("change", commit);
  dom.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && e.target instanceof HTMLInputElement && e.target.type === "text") {
      e.preventDefault();
      commit();
      if (e.target === replace) replaceNext(view);
      else if (e.shiftKey) findPrevious(view);
      else findNext(view);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeSearchPanel(view);
      view.focus();
    }
  });

  return {
    dom,
    top: false,
    update(u) {
      const q = getSearchQuery(u.state);
      if (!q.eq(query)) {
        query = q;
        find.value = q.search;
        replace.value = q.replace;
        matchCase.checked = q.caseSensitive;
        regexp.checked = q.regexp;
      }
    },
  };
}

// ---- Editor -----------------------------------------------------------------------------------

export function createEditor(parent: HTMLElement, opts: EditorOptions): EditorHandle {
  const language: Extension =
    opts.lang === "html"
      ? [
          html(),
          fieldMarks,
          EditorState.languageData.of(() => [{ autocomplete: fieldCompletions(opts.completions) }]),
        ]
      : [
          css(),
          EditorState.languageData.of(() => [
            { autocomplete: variableCompletions(opts.completions) },
          ]),
        ];

  const view = new EditorView({
    parent,
    state: EditorState.create({
      doc: opts.doc,
      extensions: [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        drawSelection(),
        history(),
        indentOnInput(),
        bracketMatching(),
        closeBrackets(),
        autocompletion({ icons: false }),
        highlightSelectionMatches(),
        search({ createPanel: searchPanel }),
        EditorState.tabSize.of(2),
        indentUnit.of("  "),
        keymap.of([
          ...closeBracketsKeymap,
          ...defaultKeymap,
          ...searchKeymap,
          ...historyKeymap,
          ...completionKeymap,
          indentWithTab,
        ]),
        language,
        theme,
        syntaxHighlighting(highlight),
        EditorView.contentAttributes.of({ "aria-label": opts.label, spellcheck: "false" }),
        EditorView.updateListener.of((u) => {
          if (u.docChanged) opts.onChange();
        }),
      ],
    }),
  });

  return {
    getValue: () => view.state.doc.toString(),
    length: () => view.state.doc.length,
    focus: () => view.focus(),
    destroy: () => view.destroy(),
  };
}
