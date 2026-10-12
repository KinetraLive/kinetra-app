//! Built-in template presets, embedded from `presets/templates/<id>/`, plus the style library
//! index `presets/library.json`. Copied into a project when a template is created from one.
//!
//! - A preset folder has the layout of a project template: schema 2 (`template.json` with
//!   `variants` and `defaultVariant`, looks in `variants/<variantId>/template.html|css`) or
//!   schema 1 (one look, `template.html` / `template.css` next to `template.json`, read as the
//!   variant `default`). Only the looks `variants` lists are read; other files in a variant
//!   folder (`vars.json`, the source of its entry for `scripts/register-variant.mjs`) and folders
//!   not listed yet are ignored.
//! - `presets/library.json` = `{ styles: [{id, name, description}], presets: [{id, kind, legacy}] }`:
//!   the design styles in order (a style id is a variant id of the library presets; `classic` is
//!   a variant, not a style entry) and the presets in gallery order with their type (`kind`) or
//!   `legacy: true`. It is compiled in (through `OUT_DIR`, see `build.rs`); a missing file is an
//!   empty library.
//! - rust-embed reads the folders from disk in debug builds and embeds them in release builds.
//! - `test-lower-third` is the dev test template: embedded (the live state uses it) but never listed
//!   or offered.
//! - Gallery thumbnails (`thumb.png`) ship with the app UI; they are ignored here.
//! - A preset's version is `preset.version` from its template.json, else 1.

use std::borrow::Cow;
use std::sync::OnceLock;

use rust_embed::RustEmbed;
use serde::Deserialize;

use crate::core::model::{
    PresetInfo, PresetVariantInfo, StyleInfo, TemplateMeta, TemplateVariant, VariantDoc,
};
use crate::core::persist;
use crate::core::templates::{
    self, TemplateError, TemplateFile, TemplateResult, CSS_FILE, DEFAULT_VARIANT, HTML_FILE,
    VARIANTS_DIR,
};

#[derive(RustEmbed)]
#[folder = "../presets/templates/"]
struct PresetFiles;

const LIBRARY_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/library.json"));

/// Embedded but never listed (dev only).
pub const DEV_ONLY: &[&str] = &["test-lower-third"];

/// Gallery order of the presets that predate the library index (after the library's own order).
/// Unknown extra presets follow, sorted by id.
pub const ORDER: &[&str] = &[
    "classic-bar",
    "minimal-line",
    "slide-in-accent",
    "news-strap",
    "corner-bug",
    "fullscreen-title",
    "leaderboard",
    "scoreboard",
    "ticker",
    "schedule-board",
    "now-next",
    "next-up",
    "countdown",
    "sponsor-slideshow",
    "sponsor-wall",
];

/// `presets/library.json`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Library {
    pub styles: Vec<StyleInfo>,
    pub presets: Vec<LibraryPreset>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryPreset {
    pub id: String,
    pub kind: Option<String>,
    pub legacy: bool,
}

impl Library {
    pub fn parse(text: &str) -> Library {
        match serde_json::from_str(text) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("presets/library.json is not valid: {e}");
                Library::default()
            }
        }
    }

    fn entry(&self, id: &str) -> Option<&LibraryPreset> {
        self.presets.iter().find(|p| p.id == id)
    }
}

/// Where preset files come from: the embedded folders, or (tests) a fixture set installed for
/// the current thread with [`testing::install`].
fn file(path: &str) -> Option<Cow<'static, [u8]>> {
    #[cfg(test)]
    if let Some(f) = testing::current() {
        return f.files.get(path).map(|b| Cow::Owned(b.clone()));
    }
    PresetFiles::get(path).map(|f| f.data)
}

fn paths() -> Vec<String> {
    #[cfg(test)]
    if let Some(f) = testing::current() {
        return f.files.keys().cloned().collect();
    }
    PresetFiles::iter().map(|p| p.replace('\\', "/")).collect()
}

/// The style library index.
pub fn library() -> Cow<'static, Library> {
    #[cfg(test)]
    if let Some(f) = testing::current() {
        return Cow::Owned(f.library.clone());
    }
    static LIB: OnceLock<Library> = OnceLock::new();
    Cow::Borrowed(LIB.get_or_init(|| Library::parse(LIBRARY_JSON)))
}

/// The design styles, in library order.
pub fn styles() -> Vec<StyleInfo> {
    library().styles.clone()
}

/// A preset ready to be copied into a project.
#[derive(Debug, Clone)]
pub struct Preset {
    pub info: PresetInfo,
    /// As in the preset's template.json (its `preset` ref and `updatedAt` are not meaningful);
    /// `defaultVariant` is one of `variants`.
    pub meta: TemplateMeta,
    /// Every look the preset ships, in its own order.
    pub variants: Vec<VariantDoc>,
}

impl Preset {
    pub fn variant(&self, id: &str) -> Option<&VariantDoc> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// The default look (the first one if `defaultVariant` names none).
    pub fn default_look(&self) -> &VariantDoc {
        self.variant(&self.meta.default_variant)
            .unwrap_or(&self.variants[0])
    }
}

fn text(path: &str) -> Option<String> {
    file(path).map(|f| String::from_utf8_lossy(&f).into_owned())
}

/// The version of preset `id` (`preset.version` in its template.json, else 1) without loading its
/// looks; `None` for an unknown or dev-only id. Cheap: one small JSON parse of an embedded file.
pub fn version(id: &str) -> Option<u32> {
    #[derive(Deserialize)]
    struct Head {
        preset: Option<Ref>,
    }
    #[derive(Deserialize)]
    struct Ref {
        version: Option<u32>,
    }
    if !templates::is_id(id) || DEV_ONLY.contains(&id) {
        return None;
    }
    let json = file(&format!("{id}/template.json"))?;
    let head: Head = serde_json::from_slice(&json).ok()?;
    Some(head.preset.and_then(|p| p.version).unwrap_or(1))
}

/// What older versions of a preset shipped that the current one doesn't (`<id>/history.json`,
/// optional): defaults a field or variable used to have, and keys it no longer has. Updating a
/// project template to the latest look ([`crate::core::template_update`]) treats those as the
/// preset's own, not the user's. Add to it whenever a version bump changes a default or drops a
/// key.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct History {
    /// Key (field or variable) → defaults it had in older versions.
    pub previous_defaults: std::collections::BTreeMap<String, Vec<String>>,
    /// Field and variable keys older versions had.
    pub removed: Vec<String>,
}

/// [`History`] of preset `id` (empty when it has none or it is invalid).
pub fn history(id: &str) -> History {
    let Some(json) = file(&format!("{id}/history.json")) else {
        return History::default();
    };
    serde_json::from_slice(&json).unwrap_or_else(|e| {
        tracing::warn!("presets/templates/{id}/history.json is not valid: {e}");
        History::default()
    })
}

/// Every embedded preset id (folders with at least one file), excluding dev-only ones, in gallery
/// order: the library's order first, then [`ORDER`], then the rest by id. Includes presets whose
/// files are invalid; [`load`] reports those.
pub fn ids() -> Vec<String> {
    let lib = library();
    let mut ids: Vec<String> = paths()
        .into_iter()
        .filter_map(|path| {
            let (id, _) = path.split_once('/')?;
            Some(id.to_string())
        })
        .filter(|id| !DEV_ONLY.contains(&id.as_str()))
        .collect();
    ids.sort_by_key(|id| {
        let rank = match lib.presets.iter().position(|p| &p.id == id) {
            Some(i) => i,
            None => lib.presets.len() + ORDER.iter().position(|o| o == id).unwrap_or(ORDER.len()),
        };
        (rank, id.clone())
    });
    ids.dedup();
    ids
}

/// Loads and fully validates a preset. `UnknownPreset` for a missing or dev-only id; `BadPreset`
/// (naming the preset) when its files are missing or invalid.
pub fn load(id: &str) -> TemplateResult<Preset> {
    if !templates::is_id(id) || DEV_ONLY.contains(&id) {
        return Err(TemplateError::UnknownPreset(id.to_string()));
    }
    let bad = |msg: String| TemplateError::BadPreset {
        id: id.to_string(),
        msg,
    };
    let json_path = format!("{id}/template.json");
    let Some(json) = file(&json_path) else {
        return Err(if ids().iter().any(|i| i == id) {
            bad("template.json is missing".into())
        } else {
            TemplateError::UnknownPreset(id.to_string())
        });
    };
    let parsed: TemplateFile =
        persist::decode::<TemplateFile>(std::path::Path::new(&json_path), &json)
            .map_err(|e| bad(e.to_string()))?;
    let version = serde_json::from_slice::<serde_json::Value>(&json)
        .ok()
        .and_then(|v| v.pointer("/preset/version").and_then(|n| n.as_u64()))
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(1);
    let TemplateFile {
        mut meta,
        mut variants,
    } = parsed;
    templates::validate_name(&meta.name).map_err(|e| bad(e.to_string()))?;
    templates::validate_shared(&meta).map_err(|e| bad(e.to_string()))?;
    if meta.default_variant.is_empty() {
        meta.default_variant = variants
            .first()
            .map(|v| v.id.clone())
            .unwrap_or_else(|| DEFAULT_VARIANT.to_string());
    }
    templates::validate_variants(&meta, &mut variants).map_err(|e| bad(e.to_string()))?;
    let mut looks = Vec::with_capacity(variants.len());
    for TemplateVariant {
        id: vid,
        name,
        vars,
    } in variants
    {
        let (html, css) = read_look(id, &vid).ok_or_else(|| {
            bad(format!(
                "variant \u{201c}{vid}\u{201d} has no {VARIANTS_DIR}/{vid}/{HTML_FILE} and {CSS_FILE}"
            ))
        })?;
        templates::validate_html(&html).map_err(|e| bad(format!("variant {vid}: {e}")))?;
        templates::validate_css(&css).map_err(|e| bad(format!("variant {vid}: {e}")))?;
        looks.push(VariantDoc {
            id: vid,
            name,
            vars,
            html,
            css,
        });
    }
    let entry = library().entry(id).cloned().unwrap_or_default();
    let mut listed: Vec<PresetVariantInfo> = looks
        .iter()
        .map(|v| PresetVariantInfo {
            id: v.id.clone(),
            name: v.name.clone(),
        })
        .collect();
    if let Some(pos) = listed.iter().position(|v| v.id == meta.default_variant) {
        let d = listed.remove(pos);
        listed.insert(0, d);
    }
    Ok(Preset {
        info: PresetInfo {
            id: id.to_string(),
            name: meta.name.trim().to_string(),
            description: meta.description.clone(),
            version,
            kind: entry.kind,
            legacy: entry.legacy,
            variants: listed,
        },
        meta,
        variants: looks,
    })
}

/// A variant's look: `variants/<vid>/` (schema 2); for `default`, the preset root (schema 1).
fn read_look(id: &str, vid: &str) -> Option<(String, String)> {
    let dir = format!("{id}/{VARIANTS_DIR}/{vid}");
    if let (Some(h), Some(c)) = (
        text(&format!("{dir}/{HTML_FILE}")),
        text(&format!("{dir}/{CSS_FILE}")),
    ) {
        return Some((h, c));
    }
    if vid == DEFAULT_VARIANT {
        return Some((
            text(&format!("{id}/{HTML_FILE}"))?,
            text(&format!("{id}/{CSS_FILE}"))?,
        ));
    }
    None
}

/// Presets with valid files, in gallery order. Invalid ones are logged and left out.
pub fn list() -> Vec<PresetInfo> {
    ids()
        .into_iter()
        .filter_map(|id| match load(&id) {
            Ok(p) => Some(p.info),
            Err(e) => {
                tracing::warn!("{e}");
                None
            }
        })
        .collect()
}

/// Whether `style` is a style id of the library, or a variant some library preset ships (e.g.
/// `classic`).
pub fn is_style(style: &str) -> bool {
    let lib = library();
    lib.styles.iter().any(|s| s.id == style)
        || lib.presets.iter().any(|p| {
            load(&p.id)
                .map(|preset| preset.variant(style).is_some())
                .unwrap_or(false)
        })
}

/// The variant `style` of library preset `id` (with the preset, for its field and variable
/// definitions), or `None` when it isn't a library preset or doesn't ship that style.
pub fn style_variant(id: &str, style: &str) -> Option<Preset> {
    library().entry(id)?;
    let p = load(id).ok()?;
    p.variant(style)?;
    Some(p)
}

/// Fixture presets for tests: installed for the current thread, so tests never depend on (or
/// disturb) the embedded presets other tests check.
#[cfg(test)]
pub mod testing {
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::rc::Rc;

    use super::Library;

    pub struct Fixture {
        pub library: Library,
        /// Path (`<id>/...`) → bytes.
        pub files: BTreeMap<String, Vec<u8>>,
    }

    thread_local! {
        static CURRENT: RefCell<Option<Rc<Fixture>>> = const { RefCell::new(None) };
    }

    pub(super) fn current() -> Option<Rc<Fixture>> {
        CURRENT.with(|c| c.borrow().clone())
    }

    /// Replaces the presets for this thread until the guard drops.
    pub fn install(f: Fixture) -> Guard {
        CURRENT.with(|c| *c.borrow_mut() = Some(Rc::new(f)));
        Guard
    }

    pub struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            CURRENT.with(|c| *c.borrow_mut() = None);
        }
    }

    /// A field definition as JSON.
    pub fn field(key: &str, kind: &str) -> serde_json::Value {
        serde_json::json!({ "key": key, "label": key, "type": kind, "default": "" })
    }

    /// A small style library: `frame` and `minimal` styles; presets `lt` (lower third: classic +
    /// frame), `poll` (classic + frame) and `legacy-bar` (legacy, schema 1).
    pub fn library() -> Fixture {
        let shared_vars = serde_json::json!([
            { "key": "accent", "label": "Accent", "type": "color", "default": "#ff0000" },
            { "key": "secondary", "label": "Secondary", "type": "color", "default": "#00ff00" },
            { "key": "text", "label": "Text", "type": "color", "default": "#ffffff" },
            { "key": "plate", "label": "Plate", "type": "color", "default": "#000000" },
            { "key": "font-heading", "label": "Heading font", "type": "font", "default": "IBM Plex Sans" },
            { "key": "font-body", "label": "Body font", "type": "font", "default": "IBM Plex Sans" },
            { "key": "radius", "label": "Radius", "type": "range", "min": 0, "max": 40, "step": 1, "unit": "px", "default": "4" },
            { "key": "speed", "label": "Speed", "type": "range", "min": 0.5, "max": 2, "step": 0.1, "unit": "", "default": "1" }
        ]);
        let mut files = BTreeMap::new();
        let mut put = |path: &str, text: String| {
            files.insert(path.to_string(), text.into_bytes());
        };
        let v2 = |name: &str, fields: serde_json::Value, variants: serde_json::Value| {
            serde_json::json!({
                "schemaVersion": 2,
                "name": name,
                "description": format!("{name} preset"),
                "fields": fields,
                "variables": shared_vars,
                "autoOutMs": null,
                "updatedAt": "",
                "defaultVariant": "classic",
                "variants": variants,
            })
            .to_string()
        };
        put(
            "lt/template.json",
            v2(
                "Lower third",
                serde_json::json!([
                    field("name", "text"),
                    field("title", "text"),
                    field("label", "text"),
                    field("logo", "image")
                ]),
                serde_json::json!([
                    { "id": "classic", "name": "Classic", "vars": {} },
                    { "id": "frame", "name": "Frame", "vars": { "accent": "#22b8cf", "font-heading": "Space Grotesk" } }
                ]),
            ),
        );
        for (id, vid, tag) in [
            ("lt", "classic", "lt-classic"),
            ("lt", "frame", "lt-frame"),
            ("poll", "classic", "poll-classic"),
            ("poll", "frame", "poll-frame"),
        ] {
            put(
                &format!("{id}/variants/{vid}/template.html"),
                format!("<div class=\"{tag}\">{{{{name}}}}</div>"),
            );
            put(
                &format!("{id}/variants/{vid}/template.css"),
                format!(".{tag} {{ color: var(--accent); }}"),
            );
        }
        put(
            "poll/template.json",
            v2(
                "Poll",
                serde_json::json!([
                    field("question", "text"),
                    field("status", "text"),
                    {
                        "key": "options", "label": "Options", "type": "list", "default": "",
                        "columns": [
                            { "key": "label", "label": "Label" },
                            { "key": "pct", "label": "Percent" },
                            { "key": "votes", "label": "Votes" }
                        ]
                    }
                ]),
                serde_json::json!([
                    { "id": "classic", "name": "Classic", "vars": {} },
                    { "id": "frame", "name": "Frame", "vars": {} }
                ]),
            ),
        );
        put(
            "legacy-bar/template.json",
            serde_json::json!({
                "schemaVersion": 1,
                "name": "Legacy bar",
                "fields": [field("name", "text")],
                "variables": [],
            })
            .to_string(),
        );
        put("legacy-bar/template.html", "<div>{{name}}</div>".into());
        put("legacy-bar/template.css", "div { color: red; }".into());
        let library = Library::parse(
            r#"{
              "styles": [
                { "id": "frame", "name": "Frame", "description": "Viewfinder brackets" },
                { "id": "minimal", "name": "Minimal", "description": "Quiet" }
              ],
              "presets": [
                { "id": "lt", "kind": "lowerThird" },
                { "id": "poll", "kind": "poll" },
                { "id": "legacy-bar", "legacy": true }
              ]
            }"#,
        );
        Fixture { library, files }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_preset_is_valid() {
        // Fails naming the preset if any embedded preset is broken; none is skipped.
        let ids = ids();
        for id in ORDER {
            assert!(
                ids.iter().any(|i| i == id),
                "built-in preset {id} is missing"
            );
        }
        for id in &ids {
            if let Err(e) = load(id) {
                panic!("preset {id} is invalid: {e}");
            }
        }
        let listed: Vec<String> = list().into_iter().map(|p| p.id).collect();
        assert_eq!(listed, ids, "every embedded preset is listed");
    }

    #[test]
    fn embedded_versions_and_histories_are_consistent() {
        for id in ids() {
            let p = load(&id).unwrap();
            assert_eq!(version(&id), Some(p.info.version), "{id}");
            let raw = file(&format!("{id}/history.json"));
            let h = history(&id);
            if let Some(raw) = raw {
                serde_json::from_slice::<serde_json::Value>(&raw)
                    .unwrap_or_else(|e| panic!("{id}/history.json: {e}"));
                assert_ne!(
                    h,
                    History::default(),
                    "{id}/history.json is empty or invalid"
                );
            }
            let has = |k: &str| {
                p.meta.fields.iter().any(|f| f.key == k)
                    || p.meta.variables.iter().any(|v| v.key == k)
            };
            for k in h.previous_defaults.keys() {
                assert!(has(k), "{id}/history.json: no field or variable \"{k}\"");
            }
            for k in &h.removed {
                assert!(!has(k), "{id}/history.json: \"{k}\" is not removed");
            }
        }
        assert_eq!(version("test-lower-third"), None);
        assert_eq!(version("nope"), None);
    }

    #[test]
    fn embedded_library_is_consistent() {
        let lib = library();
        let ids = ids();
        for p in &lib.presets {
            assert!(
                ids.contains(&p.id),
                "library.json lists {} but no such preset",
                p.id
            );
            assert!(p.kind.is_some() || p.legacy, "{} has no kind", p.id);
        }
        // Every style id is a valid variant id.
        for s in &lib.styles {
            assert!(templates::is_variant_id(&s.id), "style id {}", s.id);
        }
        // The library comes first, in its own order.
        let lib_ids: Vec<&String> = lib.presets.iter().map(|p| &p.id).collect();
        let head: Vec<&String> = ids.iter().take(lib_ids.len()).collect();
        assert_eq!(head, lib_ids);
    }

    #[test]
    fn dev_template_is_never_listed_and_order_is_fixed() {
        let ids = ids();
        assert!(!ids.iter().any(|i| i == "test-lower-third"));
        assert!(matches!(
            load("test-lower-third"),
            Err(TemplateError::UnknownPreset(_))
        ));
        for id in &ids {
            assert!(!id.contains('.'), "{id}");
        }
    }

    #[test]
    fn unknown_and_unsafe_ids() {
        for bad in ["nope", "../x", "", "a/b", "classic-bar/../test-lower-third"] {
            assert!(
                matches!(load(bad), Err(TemplateError::UnknownPreset(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn fixture_listing_has_kind_legacy_and_variants() {
        let _g = testing::install(testing::library());
        let list = list();
        let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["lt", "poll", "legacy-bar"], "library order");
        let lt = &list[0];
        assert_eq!(lt.kind.as_deref(), Some("lowerThird"));
        assert!(!lt.legacy);
        let v: Vec<&str> = lt.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(v, ["classic", "frame"], "default first");
        assert_eq!(lt.variants[1].name, "Frame");
        let legacy = &list[2];
        assert!(legacy.legacy);
        assert_eq!(legacy.kind, None);
        // Schema 1: one `default` look from the root files.
        assert_eq!(legacy.variants.len(), 1);
        assert_eq!(legacy.variants[0].id, DEFAULT_VARIANT);
        let p = load("legacy-bar").unwrap();
        assert_eq!(p.default_look().html, "<div>{{name}}</div>");
        let styles: Vec<String> = styles().into_iter().map(|s| s.id).collect();
        assert_eq!(styles, ["frame", "minimal"]);
        assert!(is_style("frame"));
        assert!(is_style("classic"), "a variant the library presets ship");
        assert!(!is_style("nope"));
        assert!(style_variant("lt", "frame").is_some());
        assert!(style_variant("lt", "minimal").is_none());
        assert!(style_variant("legacy-bar", "frame").is_none());
    }

    #[test]
    fn default_variant_is_put_first_and_order_kept_in_looks() {
        let mut f = testing::library();
        let path = "lt/template.json".to_string();
        let mut v: serde_json::Value = serde_json::from_slice(&f.files[&path]).unwrap();
        v["defaultVariant"] = "frame".into();
        f.files.insert(path, v.to_string().into_bytes());
        let _g = testing::install(f);
        let p = load("lt").unwrap();
        let listed: Vec<&str> = p.info.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(listed, ["frame", "classic"]);
        let looks: Vec<&str> = p.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(looks, ["classic", "frame"]);
        assert_eq!(p.default_look().id, "frame");
        assert_eq!(
            p.variant("frame")
                .unwrap()
                .vars
                .get("accent")
                .map(String::as_str),
            Some("#22b8cf")
        );
    }

    #[test]
    fn extra_files_and_unregistered_looks_in_variant_folders_are_ignored() {
        // Design work in progress: vars.json (read by scripts/register-variant.mjs), sample.json,
        // and a look on disk that template.json does not list yet.
        let mut f = testing::library();
        let before = {
            let _g = testing::install(testing::library());
            (ids(), load("lt").unwrap())
        };
        for (path, text) in [
            ("lt/variants/frame/vars.json", "{ \"accent\": \"#22b8cf\" }"),
            ("lt/variants/frame/sample.json", "{ \"name\": \"Ada\" }"),
            ("lt/variants/frame/notes.md", "draft"),
            ("lt/variants/minimal/template.html", "<div>{{name}}</div>"),
            ("lt/variants/minimal/template.css", "div { color: red; }"),
            ("lt/variants/minimal/vars.json", "not json"),
        ] {
            f.files.insert(path.into(), text.as_bytes().to_vec());
        }
        let _g = testing::install(f);
        assert_eq!(ids(), before.0, "no new preset ids");
        let p = load("lt").unwrap();
        let looks: Vec<&str> = p.variants.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(looks, ["classic", "frame"], "only registered looks");
        assert_eq!(p.variants, before.1.variants);
        assert_eq!(p.info, before.1.info);
        assert!(style_variant("lt", "minimal").is_none());
    }

    #[test]
    fn a_variant_without_files_is_a_bad_preset() {
        let mut f = testing::library();
        f.files.remove("lt/variants/frame/template.css");
        let _g = testing::install(f);
        assert!(matches!(load("lt"), Err(TemplateError::BadPreset { .. })));
    }
}
