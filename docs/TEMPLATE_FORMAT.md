# Kinetra template format

The on-disk format of a Kinetra graphics template (schema version 2), and the rules the renderer
applies when it draws one. Written for anyone writing or editing a template by hand, in Kinetra's
code editor or in an external editor (Kinetra watches the project folder and reloads on save).

Sources of truth, if this document and the code ever disagree: `src-tauri/src/core/templates.rs`
(validation), `core/model.rs` (`TemplateMeta`, `VariableDef`), `core/live.rs` (`FieldDef`),
`core/fieldvals.rs` (list / clock / timer values), `core/vars.rs` (variable values) and
`renderer/src/layer.ts` (compile, placeholders, animation contract).

---

## 1. File structure

A template is one folder under the project's `templates/` folder:

```
<project>/templates/<id>/
  template.json                 # the manifest: name, fields, variables, variants, …
  variants/
    <variantId>/
      template.html             # this look's markup; its first element is the root
      template.css              # this look's styles; goes into the layer's shadow root
    <variantId>/
      …
```

- **`<id>`** is the template's id: `[A-Za-z0-9_-]{1,64}`. Kinetra creates 10-character ids over
  `[a-z0-9]`. Folders with any other name are ignored.
- Each entry in `template.json`'s `variants` has a folder `variants/<variantId>/`. A missing
  `template.html` or `template.css` reads as empty.
- Files are UTF-8. Kinetra writes them atomically (temp file, then rename).
- Assets (images, video) don't live in the template; they live in the project's `assets/` folder
  and are referenced as `/assets/<file>` in field values.

**Schema 1 (legacy).** A v1 template has `template.html` and `template.css` directly in its
folder and no `variants`. On project open it is migrated (after a `pre-migrate` backup) into a
single variant `default` named "Default"; the migration is idempotent. Kinetra never writes v1.

**Newer schema.** A `template.json` with a higher `schemaVersion` than this build understands is
never overwritten; the template fails to open with an error.

---

## 2. The manifest: `template.json`

```jsonc
{
  "schemaVersion": 2,
  "name": "Lower third",                     // 1–100 characters (trimmed)
  "description": "A name strap.",            // ≤ 1000 characters; optional, default ""
  "preset": { "id": "lower-third", "version": 3 },  // or null: where it was copied from
  "fields": [ /* §3 */ ],                    // ≤ 64
  "variables": [ /* §4 */ ],                 // ≤ 32
  "autoOutMs": null,                         // or 500–600000
  "design": { "w": 960, "h": 200, "x": 115, "y": 783, "sizing": "fit" },  // optional
  "defaultVariant": "default",               // one of the variant ids
  "variants": [ /* §5 */ ],                  // 1–12
  "updatedAt": "2026-10-10T12:00:00Z"        // RFC 3339, set by Kinetra on every save
}
```

| Key | Type | Rules |
|---|---|---|
| `schemaVersion` | number | `2`. |
| `name` | string | 1–100 characters after trimming. |
| `description` | string | ≤ 1000 characters. |
| `preset` | `{id, version}` \| `null` | Set when copied from a built-in preset; drives "A newer look is available". Leave `null` for your own templates. |
| `fields` | array | The content an operator fills in (§3). ≤ 64. |
| `variables` | array | Styling knobs exposed as CSS custom properties (§4). ≤ 32. |
| `autoOutMs` | number \| `null` | Take the graphic off air this many ms after it goes in, 500–600000. The server runs the timer, so every output agrees. `null` = stays on air. |
| `design` | object | Optional design box (§2.1). Omit for a full-screen template. |
| `defaultVariant` | string | Id of the variant a layer shows unless it picks another. Must be in `variants`. |
| `variants` | array | The looks (§5). 1–12. |
| `updatedAt` | string | Written by Kinetra. |

**Keys** of fields, variables and list columns all follow `[a-z][a-z0-9_-]{0,31}`: a lowercase
letter, then up to 31 lowercase letters, digits, `_` or `-`. Field keys and variable keys share
one namespace: a key may not be both.

**Labels** (fields, variables, options, columns) are 1–100 characters after trimming. They are
what operators see.

### 2.1 Design box (`design`)

The size the graphic is designed at, and where a new layer goes.

| Key | Rules |
|---|---|
| `w`, `h` | Integers 16–8192. |
| `x`, `y` | Optional, finite numbers: a new layer's top-left on a 1920×1080 screen (scaled to the screen's size and clamped inside it). Absent = centered. |
| `sizing` | `"fit"` or `"reflow"`: what new layers from this template get. Default `"reflow"`. |

- **`fit` (Scale to fit):** the template is laid out in a box of exactly `w`×`h` px, then scaled
  uniformly (never stretched) and centered into the layer's box. Use it for designed graphics
  (lower thirds, bugs, titles). That box is the template's containing block, and container queries
  resolve against the design size.
- **`reflow`:** the template fills the layer's box and lays itself out (grids, tickers, schedules,
  sponsor walls). Use container units and container queries to respond to the box.
- Without `design`, a new layer fills the screen, and `fit` uses the screen size as the design size.
- The design box should hug the graphic (built-in presets cover ≥ 90 % on both axes at the design
  size), so that resizing the layer box resizes the visible graphic.

---

## 3. Fields

Fields are the content: names, titles, images, rows, timers. Declared once in `template.json`,
shared by every variant, and placed in HTML with `{{key}}` (§6).

```jsonc
{ "key": "name", "label": "Name", "type": "text", "default": "" }
```

| Key | Rules |
|---|---|
| `key` | See key rules above. |
| `label` | 1–100 characters. |
| `type` | One of the types below. |
| `default` | String. The value a new layer starts with (format per type). |
| `columns` | `list` only: 1–16 `{key, label, kind?}`; `kind` is `"text"` (default, omitted) or `"image"`. Column keys follow the key rules and are unique within the field. |
| `format` | `clock` only: optional display format (below). |

### Field types

All values are strings. Every type except `html` is inserted **as text** (escaped).

| Type | Value | Notes |
|---|---|---|
| `text` | Plain text. | One line in the operator UI. |
| `multiline` | Plain text with line breaks. | Line breaks only show if your CSS sets `white-space: pre-line` (or similar). |
| `number` | Text of a number. | Not parsed by the renderer; it's just shown. |
| `color` | A color string. | Usually bound into a `style` attribute (§6.3). |
| `image` | A URL, usually `/assets/<file>`. | Bind into `src`. Only http(s), relative and `data:image/…` URLs are accepted. |
| `video` | A URL, usually `/assets/<file>`. | Bind into a `<video src>`. Same URL rules. |
| `html` | Markup. | Inserted as markup; `<script>`, `on*` and `srcdoc` attributes and unsafe URLs are stripped. Markup only in text position; in an attribute it is plain text. |
| `list` | JSON array of row objects (below). | Repeated with `data-each` (§6.4). |
| `clock` | — (no value to set). | Shows the time of day, rendered by the output from the server-corrected clock. |
| `timer` | JSON `TimerState` (below). | Counts up or down in the output; never ticks over the network. |

**`list` value.** A compact JSON array of objects, column key → text, every column present:
`[{"text":"Doors open 18:00"},{"text":"Bar closes 01:00"}]`. At most 100 rows, cells ≤ 4 KB,
the whole value ≤ 64 KB. Image-column cells hold `/assets/<file>`. The `default` must be a valid
list (`"[]"` for none).

Rows may carry reserved **row marks**, keys starting with `_` that the server sets when a list is
bound to a source: `_status` (`previous | now | next | delayed | upcoming | cancelled`, on
schedules), `_current` (`true` on a slideshow's current row) and `_duration` (seconds). They
become `data-status`, `data-current`, `data-duration` attributes on the row element (§6.4).

**`clock` format.** Tokens `HH` (00–23), `mm`, `ss`, plus `:`, `.` and spaces; ≤ 32 characters.
Absent or `""` = `HH:mm:ss`.

**`timer` value.** `{"run": bool, "at": epochMs, "base": ms, "dir": "up" | "down", "end": "stop" | "hide"}`.
Displayed time = `base` ± time since `at` while running, `base` while paused. Shown as `m:ss`
under an hour and `h:mm:ss` from an hour. A countdown stops at zero; with `end: "hide"` the
server takes the layer off air at zero. The `default` is `""` (a stopped count-up at 0:00) or a
`TimerState` (`run` is ignored: defaults never run). Operators start, pause and reset timers from
Control.

**Field values and layers.** Each screen layer stores its own field values; a missing value uses
the field's `default`. A field may instead be **bound** to a data source (a column of its current
row, or a whole source for a `list`); that is screen data, not part of the template.

---

## 4. Variables

Variables are styling knobs (colors, fonts, sizes, choices) that an operator can override per
layer without touching code. Each becomes the CSS custom property `--<key>` on the layer's host
element, so `var(--<key>)` works everywhere in `template.css`, including `:host`.

```jsonc
{ "key": "accent", "label": "Accent", "type": "color", "default": "#33d6ff" }
{ "key": "font-heading", "label": "Heading font", "type": "font", "default": "Space Grotesk" }
{ "key": "radius", "label": "Corner radius", "type": "range",
  "min": 0, "max": 24, "step": 1, "unit": "px", "default": "2" }
{ "key": "align", "label": "Alignment", "type": "select",
  "options": [{ "value": "left", "label": "Left" }, { "value": "center", "label": "Center" }],
  "default": "left" }
```

Every value, the `default` included, is validated against a strict grammar and **re-emitted**
from its parsed parts; raw text never reaches CSS.

| Type | Extra keys | Valid values | CSS value of `--<key>` |
|---|---|---|---|
| `color` | — | `#rgb`, `#rrggbb`, `#rrggbbaa` or `transparent`, any case | The value in lowercase. |
| `font` | — | Exactly one curated family name (list below) | The family's full stack, e.g. `"Barlow", system-ui, sans-serif`. |
| `range` | `min`, `max`, `step` (finite, `min < max`, `step > 0`), `unit` (`[a-z%]{0,8}`, `""` = unitless) | A plain decimal number within `min..max` (no exponent) | The number (≤ 6 decimals) plus `unit`, e.g. `12px`, `1.5`. |
| `select` | `options`: 1–64 `{value, label}`, values `[A-Za-z0-9_-]{1,64}`, unique | One of the option values | The option value as a bare token. |

**Resolution order** for a layer's value: layer override → project brand kit → the variant's
`vars` → the definition's `default`. The brand kit only fills the shared keys `accent`,
`secondary`, `text`, `plate`, `font-heading` and `font-body` (and fills an empty `logo` image
field with the brand logo), so use those keys if a template should follow the project's brand.

**Curated fonts** (the only families a template can rely on; served by Kinetra at `/fonts/`, so
every OBS/vMix machine renders the same): Barlow, Barlow Condensed, Oswald, Montserrat,
IBM Plex Sans, IBM Plex Sans Condensed, Source Serif 4, Space Grotesk, Manrope, Archivo,
Playfair Display, Jost, Instrument Sans, Plus Jakarta Sans, Outfit, Roboto Mono, IBM Plex Mono.
Weights 400, 600 and 700 ship for every family (some ship more); italics only for some. **Inter is
not allowed**: a stored "Inter" reads as IBM Plex Sans and is rewritten on open and save.

---

## 5. Variants

A variant is one **look** of the template: its own HTML and CSS and its own variable defaults,
sharing the template's fields and variable definitions. Typical uses: Default / Compact /
Sponsor, or one look per design style.

```jsonc
"defaultVariant": "default",
"variants": [
  { "id": "default", "name": "Default", "vars": {} },
  { "id": "compact", "name": "Compact", "vars": { "radius": "0", "accent": "#e63946" } }
]
```

| Key | Rules |
|---|---|
| `id` | `[a-z][a-z0-9-]{0,31}` (no `_`: it is also the folder name), unique. |
| `name` | 1–40 characters after trimming, unique ignoring case. |
| `vars` | This look's variable defaults: only declared variable keys, each a valid value. Missing keys use the definition's `default`. |

- 1–12 variants; the last one can't be deleted. `defaultVariant` must name one of them.
- Each variant's look is `variants/<id>/template.html` + `template.css`.
- A variant doesn't have to use every field. Field values stay on the layer, so switching to a
  look that ignores a field and back again keeps its value.
- A layer picks a variant by id (absent = the template's default). A layer naming a variant that
  no longer exists falls back to the default with a warning.

---

## 6. HTML

### 6.1 Structure

- The **first element** of `template.html` is the **root**. The renderer sets
  `data-state="in"` on it when the layer goes on air and `data-state="out"` when it goes off
  (§8). Write exactly one top-level element; anything after it is rendered but gets no state.
- Each layer renders in its own **open shadow root**: the template's CSS can't leak out and the
  page's styles can't leak in.
- Every `<img>` gets `decoding="async"` unless the template sets `decoding` itself.
- `template.html` ≤ 256 KB.

### 6.2 Placeholders: `{{key}}`

`{{key}}` (whitespace inside the braces allowed: `{{ key }}`) is replaced by the field's value.
Placeholders work in **text** and in **attribute values**, mixed with literal text:

```html
<div class="name">{{name}}</div>
<div class="role">{{title}} · {{company}}</div>
<img class="photo" src="{{photo}}" alt="">
<div class="chip" data-status="{{status}}"></div>
```

- Text placeholders are inserted as text nodes (escaped), except `html` fields (§3).
- A placeholder naming no field renders as empty.
- `clock` and `timer` placeholders render the formatted time and update once per second while on
  air (and the page is visible).
- A value change patches only the bound text node or attribute: no re-render, and running
  animations don't restart.

### 6.3 Attribute rules

| Attribute | Rule |
|---|---|
| `on*` (event handlers), `srcdoc` | Placeholders are **never** bound; the attribute keeps its literal text. |
| `src`, `href`, `srcset`, `poster`, `xlink:href`, `action`, `formaction` | The value must be http(s), relative or `data:image/…`; anything else (e.g. `javascript:`) becomes `""`. |
| `style` | Each bound value must match `[#\w.%+\- ]{0,64}`: colors, numbers, lengths, percentages, plain words. No `( ) ; : { } \ / @ <` or quotes, so no `url()` and no escaping the declaration. Anything else becomes `""`. Example: `style="--team: {{color}}"`. |
| everything else | Value inserted as given. |

### 6.4 Lists: `data-each`

An element with `data-each="<listField>"` is a **row template**, cloned once per row of that
`list` field, in order, where it stands.

```html
<ul class="rows">
  <li class="row" data-each="items" data-limit="8">
    <span class="n">{{#}}</span>
    <span class="t">{{.title}}</span>
    <img src="{{.logo}}" alt="">
    <span class="when">{{start}}</span>   <!-- a layer field works inside rows too -->
  </li>
</ul>
```

| Inside a row | Meaning |
|---|---|
| `{{.col}}` | The cell of column `col` of this row. |
| `{{#}}` | The row number, starting at 1. |
| `{{key}}` | A normal layer field, same value in every row. |

- `data-limit="<n>"` shows at most `n` rows.
- Each row element gets `style="--i: <index>"` (0-based) for staggered animations, and
  `data-row="in"` when it is added. A row removed while on air gets `data-row="out"` and is
  detached when its animation ends (off air: at once).
- Row marks (§3) become `data-<name>` attributes on the row: `data-status="now"`,
  `data-current="true"`, `data-duration="8"`. Only tokens `[a-z0-9-]{1,32}` are written.
- The row's parent element gets `data-count="<n>"` and `--n: <n>` (rows on screen, leaving rows
  excluded), and `data-empty` when there are no rows, for count-aware layouts.
- Updates patch rows in place by index; only changed cells are written.
- Not supported: `data-each` on the root element (it is removed), and nested `data-each`
  (an inner row template inside another is ignored). Two separate row templates for the same
  list (e.g. a ticker's duplicated run) are fine.

### 6.5 Helper attributes

| Attribute | Set by | Meaning |
|---|---|---|
| `data-state="in" \| "out"` | renderer, on the root | On air / going off air (§8). |
| `data-empty` | renderer | On an element whose bound text and URL attributes are all empty, and on a list's parent with no rows. Use it to collapse missing parts: `.photo[data-empty] { display: none }`. |
| `data-fit` | you | Shrinks the element's font size until its text fits its width (strictly: no sub-pixel overhang) and at most `data-fit-lines` lines (default 1), down to `data-fit-min` × the CSS size (default 0.6). Runs before the root goes `in` and in the same frame as a text change. The element's height is not a limit; use the lines. |
| `data-row`, `data-count`, `--i`, `--n`, `data-<mark>` | renderer | Lists (§6.4). |

The renderer owns `data-state`, `data-row`, `data-each`, `data-limit`; don't set them yourself
except `data-each` and `data-limit` as described.

---

## 7. CSS

`template.css` goes into the layer's shadow root as one `<style>`; ≤ 256 KB.

**Available to the CSS**

| What | Where |
|---|---|
| `--<key>` for every variable | Set on the host; inherited by everything. |
| `--auto-out-ms` (e.g. `5000ms`) | On the host when the template has `autoOutMs`, e.g. to drain a progress bar over the time on air. |
| `:host` | The layer host: positioned at the layer's box, `overflow: visible` so out-animations can leave it. |
| Container queries | In `fit`, the design box is the containing block; put `container-type: size` on your root to query it. In `reflow`, the root fills the layer box. |
| Curated fonts | Name them in `font-family`/`font` (directly, or through a `font` variable). The output loads only the families and weights the CSS names, before the graphic plays in (waiting at most 1.5 s), so a fallback font never shows on air. |

**Allowed:** any CSS the output's Chromium-based browser source supports: layout, `color-mix()`,
container units and queries, `@keyframes`, transitions, `:has()`, custom properties.

**Forbidden** (not supported; built-in presets are tested against each):

| Don't | Why |
|---|---|
| `url(…)` and `@import` | Templates load nothing external; graphics must work offline and on every machine. Images come in through `image` fields (`/assets/…`). |
| `@font-face` | Ignored inside a shadow root; only the curated fonts are available. |
| `@media (prefers-reduced-motion)` | On-air graphics always animate as designed. Windows "Animation effects: off" turns this on in OBS/vMix and would freeze the graphics. |
| Fonts outside the curated list, or Inter | They aren't bundled; the output would show a fallback. |
| A weight or italic the family doesn't ship | The browser synthesizes it (fake bold, slanted roman). |

**Recommended:** animate only `transform` and `opacity` (smooth at any size); keep the
interesting part of a `fit` graphic inside its design box; collapse missing parts with
`data-empty` or `:empty`.

---

## 8. Animation contract

1. **In.** When the layer goes on air the root gets `data-state="in"`. Put the in-animation on
   `.root[data-state="in"]` (use `animation-fill-mode: both` so it starts hidden).
2. **Out.** When it goes off air the root gets `data-state="out"`. Put the out-animation on
   `.root[data-state="out"]` (`forwards` so it stays hidden). The layer is removed when the
   root's animations and transitions (subtree included) finish, or after the longest computed
   duration + delay, capped at **3 s**.
3. **No out-animation** = removed at once.
4. **Updates while on air** never replay the in-animation: field changes patch in place; a look
   change or staged content taken to air shows the new look settled (finite animations
   finished). Infinite animations (ticker scrolls, loops) keep running.
5. **Variant swaps** on air run as a ~300 ms crossfade or the old look's out then the new look's
   in; both are driven by the renderer (host opacity and `data-state`).
6. **PANIC (cut)** removes the layer at once with no out-animation.
7. The output's preview bus shows layers settled: no in/out animation.

Clocks, timers and swaps are the renderer's only timers; a template needs no JavaScript.

---

## 9. What is never allowed in a template

- **JavaScript.** No `<script>` (it is never executed) and no `on*` handler attributes. Templates
  are HTML + CSS only; data comes from fields, behavior from Kinetra.
- **External resources.** No `http(s)://` URLs in HTML or CSS, no `url()`, `@import` or
  `@font-face`. Network data (APIs, chat) is fetched by Kinetra and arrives as field values.
- **Secrets.** Nothing private goes into a template; templates are copied into exports and
  backups.

---

## 10. Limits at a glance

| Limit | Value |
|---|---|
| Template id | `[A-Za-z0-9_-]{1,64}` |
| Name / description | 1–100 / ≤ 1000 characters |
| Field, variable, column key | `[a-z][a-z0-9_-]{0,31}`, unique across fields and variables |
| Labels | 1–100 characters |
| Fields / variables / variants | ≤ 64 / ≤ 32 / 1–12 |
| Variant id / name | `[a-z][a-z0-9-]{0,31}` / 1–40 characters, unique ignoring case |
| `template.html`, `template.css` | ≤ 256 KB each, per variant |
| Select options | 1–64, values `[A-Za-z0-9_-]{1,64}` |
| Range unit | `[a-z%]{0,8}` |
| List | 1–16 columns, ≤ 100 rows, cells ≤ 4 KB, value ≤ 64 KB |
| Clock format | `HH` `mm` `ss` `:` `.` space, ≤ 32 characters |
| Timer | `base` ≤ 1000 hours |
| `autoOutMs` | 500–600000 |
| Design box | 16–8192 px per side |
| Out-animation | removed after ≤ 3 s |
| Font wait before play-in | ≤ 1.5 s |

Errors in `template.json` stop Kinetra from saving (the editor says what to fix); an external
edit that doesn't validate keeps the last good version on air.

---

## 11. Built-in presets

Presets in `presets/templates/<id>/` use the same format, plus:

- `preset: {id, version}` in their `template.json`. Bump `version` whenever the HTML, CSS,
  variables or design change; record changed defaults and removed keys in `history.json`
  (`previousDefaults`, `removed`).
- `sample.json`: realistic, neutral field values for the gallery (thumbnails, hover preview), the
  brand preview and the sizing checks (not copied into projects). The template editor shows empty
  text fields as placeholders instead (`[Name]`).
- `variants/<style>/vars.json`: a style look's variable values, registered into `template.json`
  with `node scripts/register-variant.mjs <id> <style>`.
- Each look stays within 12 KB of HTML + CSS.

---

## 12. Example

A lower third with a name, a role, an optional photo, a list of tags, an accent color, a heading
font and two looks. It auto-outs after 8 s and scales to fit a 960×200 box at the bottom left.

### `templates/k3m9x2p7qa/template.json`

```json
{
  "schemaVersion": 2,
  "name": "Speaker strap",
  "description": "Name, role, photo and topic tags.",
  "preset": null,
  "fields": [
    { "key": "name", "label": "Name", "type": "text", "default": "" },
    { "key": "role", "label": "Role", "type": "text", "default": "" },
    { "key": "photo", "label": "Photo", "type": "image", "default": "" },
    {
      "key": "tags",
      "label": "Topics",
      "type": "list",
      "columns": [{ "key": "tag", "label": "Topic" }],
      "default": "[]"
    }
  ],
  "variables": [
    { "key": "accent", "label": "Accent", "type": "color", "default": "#33d6ff" },
    { "key": "plate", "label": "Plate", "type": "color", "default": "#0c0d11d9" },
    { "key": "text", "label": "Text", "type": "color", "default": "#f5f5f2" },
    { "key": "font-heading", "label": "Heading font", "type": "font", "default": "Barlow" },
    {
      "key": "radius",
      "label": "Corner radius",
      "type": "range",
      "min": 0, "max": 24, "step": 1, "unit": "px",
      "default": "6"
    },
    {
      "key": "speed",
      "label": "Animation speed",
      "type": "range",
      "min": 0.5, "max": 2, "step": 0.1, "unit": "",
      "default": "1"
    }
  ],
  "autoOutMs": 8000,
  "design": { "w": 960, "h": 200, "x": 115, "y": 783, "sizing": "fit" },
  "defaultVariant": "default",
  "variants": [
    { "id": "default", "name": "Default", "vars": {} },
    {
      "id": "light",
      "name": "Light",
      "vars": { "plate": "#ffffff", "text": "#16202e", "accent": "#2563eb", "radius": "0" }
    }
  ],
  "updatedAt": "2026-10-10T12:00:00Z"
}
```

### `variants/default/template.html`

(The `light` variant can use the same markup with its own CSS, or different markup entirely.)

```html
<div class="strap">
  <div class="plate">
    <img class="photo" src="{{photo}}" alt="">
    <div class="text">
      <div class="name" data-fit data-fit-min="0.6">{{name}}</div>
      <div class="role" data-fit data-fit-lines="2">{{role}}</div>
      <ul class="tags">
        <li class="tag" data-each="tags" data-limit="4">{{.tag}}</li>
      </ul>
    </div>
    <span class="timer-bar" aria-hidden="true"></span>
  </div>
</div>
```

### `variants/default/template.css`

```css
/* The renderer sets data-state="in" | "out" on .strap. */
:host {
  --ease: cubic-bezier(0.2, 0, 0, 1);
  --ease-in: cubic-bezier(0.4, 0, 1, 1);
}

.strap {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: flex-end;
  container-type: size;
}

.plate {
  position: relative;
  display: flex;
  gap: 24px;
  align-items: center;
  max-width: 100%;
  padding: 20px 32px 20px 20px;
  background: var(--plate);
  color: var(--text);
  border-left: 6px solid var(--accent);
  border-radius: var(--radius);
  overflow: hidden;
}

.photo {
  width: 120px;
  height: 120px;
  object-fit: cover;
  border-radius: var(--radius);
}
.photo[data-empty] {
  display: none;
}

.text {
  min-width: 0;
}

.name {
  font: 700 56px/1.1 var(--font-heading);
  white-space: nowrap;
}

.role {
  margin-top: 4px;
  font: 400 28px/1.25 var(--font-heading);
  opacity: 0.85;
}
.role[data-empty] {
  display: none;
}

.tags {
  display: flex;
  gap: 8px;
  margin: 10px 0 0;
  padding: 0;
  list-style: none;
}
.tags[data-empty] {
  display: none;
}

.tag {
  padding: 2px 10px;
  border: 1px solid var(--accent);
  border-radius: 999px;
  font: 600 18px/1.4 var(--font-heading);
}
.tag[data-row="in"] {
  animation: tag-in calc(200ms * var(--speed)) var(--ease) both;
  animation-delay: calc(var(--i) * 60ms + 200ms);
}
.tag[data-row="out"] {
  animation: fade-out 160ms var(--ease-in) forwards;
}

/* Drains over the time on air (autoOutMs). */
.timer-bar {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 3px;
  background: var(--accent);
  transform-origin: 0 50%;
}
.strap[data-state="in"] .timer-bar {
  animation: drain var(--auto-out-ms, 0ms) linear both;
}

.strap[data-state="in"] .plate {
  animation: plate-in calc(320ms * var(--speed)) var(--ease) both;
}
.strap[data-state="out"] .plate {
  animation: plate-out calc(220ms * var(--speed)) var(--ease-in) forwards;
}

@keyframes plate-in {
  from {
    opacity: 0;
    transform: translateX(-24px);
  }
}
@keyframes plate-out {
  to {
    opacity: 0;
    transform: translateX(-24px);
  }
}
@keyframes tag-in {
  from {
    opacity: 0;
    transform: translateY(8px);
  }
}
@keyframes fade-out {
  to {
    opacity: 0;
  }
}
@keyframes drain {
  to {
    transform: scaleX(0);
  }
}
```

### `variants/light/`

Same two files; `light` differs only in its `vars` above, so it can be a copy of `default`.

### A layer's field values (in the screen file, for reference)

```json
{
  "name": "Alex Morgan",
  "role": "Producer, Northwind Studios",
  "photo": "/assets/ingrid.jpg",
  "tags": "[{\"tag\":\"Live\"},{\"tag\":\"Remote production\"}]"
}
```
