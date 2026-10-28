//! Live state: what is on air on each screen. Pure — no I/O, no async — so every operation is
//! unit-testable. Operations return the protocol messages to broadcast; `LiveHub` serializes each
//! one once and sends it to the screen's renderers.
//!
//! Types and signatures are mirrored by the UI and the server; keep them stable.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::core::fsx;
use crate::core::media::{media_rev, MediaContent};
use crate::core::model::{
    LiveEdit, MediaAction, MediaLayerView, MediaPlayback, MediaReadiness, PlaybackState,
    PreviewAfterTake, StudioCues, VariableDef,
};
use crate::core::protocol::{
    ContentSizing, DesignSize, FieldType, Fields, LayerState, Rect, ScreenInfo, ServerMsg,
    TakeTransition, VariantTransition, WireErrorCode, WireLayer,
};

/// Screen id of the hardcoded test screen (dev only; never listed).
pub const TEST_SCREEN_ID: &str = "test";
/// Layer id of the hardcoded test lower third.
pub const TEST_LAYER_ID: &str = "lower-third";
/// Unlisted screen the template editor previews into (one layer, `PREVIEW_LAYER_ID`). The editor's
/// preview is a normal `/screen/__preview` iframe, so there is one rendering path.
pub const PREVIEW_SCREEN_ID: &str = "__preview";
pub const PREVIEW_LAYER_ID: &str = "template";

/// Screens that are internal (the dev test screen, `__*` system screens) and never listed as user
/// screens.
pub fn is_reserved_screen(id: &str) -> bool {
    id == TEST_SCREEN_ID || id.starts_with("__")
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDef {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: FieldType,
    pub default: String,
    /// `list` fields: the row columns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<crate::core::model::SourceColumn>,
    /// `clock` fields: the display format (`HH`, `mm`, `ss` tokens; default `HH:mm:ss`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDef {
    pub html: String,
    pub css: String,
    pub fields: Vec<FieldDef>,
    /// The template's design size (`template.json` `design`), for `fit` sizing. Not part of the
    /// look: a change is a same-rev `layer` message.
    pub design: Option<DesignSize>,
}

#[derive(Debug, Clone)]
pub struct LayerLive {
    pub id: String,
    pub name: String,
    pub z: i32,
    pub template: Arc<TemplateDef>,
    pub template_rev: u32,
    /// The template variant shown (`""` for the built-in test template).
    pub variant: String,
    pub fields: Fields,
    /// Template variables as validated CSS values (`--<key>` on the layer host).
    pub vars: Fields,
    pub on_air: bool,
    /// Variable definitions of the loaded template (empty for the built-in test template).
    pub variables: Vec<VariableDef>,
    /// Variable values as the user sees them (colour hex, font name, ...); `vars` is their CSS.
    pub var_values: Fields,
    /// Take the layer off air this long after it goes in (`LiveHub` schedules it).
    pub auto_out_ms: Option<u32>,
    /// Changes on every on/off-air transition and template load; a scheduled auto-out only fires
    /// if the generation it was scheduled for is still current. Unique process-wide.
    pub out_gen: u64,
    /// The layer's box on its screen. `None` = full screen (test and preview screens).
    pub rect: Option<Rect>,
    /// How the content fills the box (content, like `rect`).
    pub sizing: ContentSizing,
    /// Layer opacity 0–100 (content, like `rect`).
    pub opacity: u8,
    /// Media layers: the file and its config (content); `template` is then the empty
    /// [`media_template`].
    pub media: Option<MediaContent>,
    /// Media layers: Program playback (not content; never saved).
    pub playback: MediaPlayback,
    /// Preload status over the screen's Program renderers (set by the hub): media layers, and
    /// template layers with a video or image field (see [`LiveLayer::ready_key`]).
    pub readiness: MediaReadiness,
}

/// The (empty) template every media layer points to.
pub fn media_template() -> Arc<TemplateDef> {
    static T: std::sync::OnceLock<Arc<TemplateDef>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        Arc::new(TemplateDef {
            html: String::new(),
            css: String::new(),
            fields: Vec::new(),
            design: None,
        })
    })
    .clone()
}

static OUT_GEN: AtomicU64 = AtomicU64::new(1);

/// A process-wide unique generation for [`LayerLive::out_gen`].
pub fn next_out_gen() -> u64 {
    OUT_GEN.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub struct ScreenLive {
    pub info: ScreenInfo,
    /// Ordered by `z`, bottom first.
    pub layers: Vec<LayerLive>,
    /// What differs in Preview (Studio mode), or what is staged for the next take (`onTake`).
    pub preview: ScreenPreview,
}

#[derive(Debug, Clone, Default)]
pub struct LiveState {
    pub screens: Vec<ScreenLive>,
    /// Studio mode: content edits go to each screen's Preview diff (on or off air) instead of
    /// Program; `show` (automation) leaves staged content in the diff; the Program views report
    /// Program only. See [`ScreenPreview`].
    pub studio: bool,
}

// ---- Preview diff (Studio mode) and On-take staging ------------------------------------------

/// One layer's entry in a screen's Preview diff. Outside Studio mode the same entry, with
/// `content` only, is On-take staging ("Changes on next take").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cue {
    /// Cued in (`true`) / out (`false`); `None` = as Program.
    pub visible: Option<bool>,
    /// Staged content (fields, vars, variant, template, rect, auto-out).
    pub content: Option<Box<LayerContent>>,
    /// Automation changed this layer on Program since it was cued (the diff's `rev` then).
    /// Cleared by the next take, or when the cue is cleared.
    pub program_changed_rev: Option<u64>,
}

impl Cue {
    /// No visibility cue and no staged content: the entry is dropped.
    pub fn is_empty(&self) -> bool {
        self.visible.is_none() && self.content.is_none()
    }
}

/// A T-bar transition in progress on a screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Tbar {
    /// The latest position (0..=1).
    pub p: f64,
    /// The position last sent to renderers (what `PreviewView.tbar` reports).
    pub sent: f64,
    /// What `prepare` mounted: `(layers, hide)`. A commit that differs (Program changed
    /// underneath meanwhile) is followed by a snapshot.
    pub prepared: (Vec<WireLayer>, Vec<String>),
}

/// A screen's Preview = Program + this diff. `rev` bumps on every change of the diff.
#[derive(Debug, Clone, Default)]
pub struct ScreenPreview {
    pub layers: BTreeMap<String, Cue>,
    pub rev: u64,
    pub tbar: Option<Tbar>,
}

impl ScreenPreview {
    /// The content staged for a layer.
    pub fn staged(&self, layer_id: &str) -> Option<&LayerContent> {
        self.layers.get(layer_id).and_then(|c| c.content.as_deref())
    }

    /// No cue at all: Preview equals Program.
    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// Sets (or drops) a layer's staged content; the rev bumps when it changed.
    fn set_content(&mut self, layer_id: &str, c: Option<LayerContent>) {
        let next = c.map(Box::new);
        let cue = self.layers.entry(layer_id.to_string()).or_default();
        if cue.content != next {
            cue.content = next;
            self.bump();
        }
        self.prune(layer_id);
    }

    /// Takes a layer's staged content out of the diff.
    fn take_content(&mut self, layer_id: &str) -> Option<LayerContent> {
        let c = self.layers.get_mut(layer_id)?.content.take()?;
        self.bump();
        self.prune(layer_id);
        Some(*c)
    }

    fn set_visible(&mut self, layer_id: &str, visible: Option<bool>) {
        let cue = self.layers.entry(layer_id.to_string()).or_default();
        if visible.is_none() {
            cue.program_changed_rev = None;
        }
        if cue.visible != visible {
            cue.visible = visible;
            self.bump();
        }
        self.prune(layer_id);
    }

    fn prune(&mut self, layer_id: &str) {
        if self.layers.get(layer_id).is_some_and(Cue::is_empty) {
            self.layers.remove(layer_id);
        }
    }

    /// Empties the diff and ends a T-bar. `true` if there was a cue.
    fn clear(&mut self) -> bool {
        self.tbar = None;
        if self.layers.is_empty() {
            return false;
        }
        self.layers.clear();
        self.bump();
        true
    }
}

/// One layer's entry in a [`PreviewView`] (src/lib/types.ts `LayerCue`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerCue {
    pub layer_id: String,
    pub visible: Option<bool>,
    pub staged: bool,
    pub program_changed: bool,
}

/// What Control shows for a screen's Preview (`preview-changed`, `studio_op`, `preview_view`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewView {
    pub screen_id: String,
    /// Bumps on every change of the diff.
    pub rev: u64,
    /// Empty = Preview equals Program. In z-order, bottom first.
    pub cues: Vec<LayerCue>,
    /// The layers as they look in Preview.
    pub layers: Vec<LiveLayerView>,
    /// A T-bar transition in progress (0..=1), or null.
    pub tbar: Option<f64>,
}

/// What a take (or a T-bar commit) did to a screen.
#[derive(Debug, Default)]
pub struct TakeOutcome {
    /// To Program, in order: content of layers staying off air, then `take` (or `commit`), then
    /// content of layers going off air (and a `snapshot` when a commit differs from what was
    /// prepared).
    pub msgs: Vec<ServerMsg>,
    /// Layers whose on-air state or look changed (timer bookkeeping).
    pub touched: Vec<String>,
}

/// A take computed on a copy of the screen's layers.
struct SimTake {
    layers: Vec<LayerLive>,
    pre: Vec<ServerMsg>,
    wire: Vec<WireLayer>,
    hide: Vec<String>,
    post: Vec<ServerMsg>,
    touched: Vec<String>,
}

fn simulate_take(screen: &ScreenLive) -> SimTake {
    let mut sim = SimTake {
        layers: screen.layers.clone(),
        pre: Vec::new(),
        wire: Vec::new(),
        hide: Vec::new(),
        post: Vec::new(),
        touched: Vec::new(),
    };
    for l in sim.layers.iter_mut() {
        let Some(cue) = screen.preview.layers.get(&l.id) else {
            continue;
        };
        let was_on = l.on_air;
        let content_msgs = match &cue.content {
            Some(c) => apply_content(l, (**c).clone(), None),
            None => Vec::new(),
        };
        let changed = !content_msgs.is_empty();
        match (was_on, cue.visible.unwrap_or(was_on)) {
            (false, true) => {
                l.on_air = true;
                l.out_gen = next_out_gen();
                l.start_playback(fsx::now_epoch_ms());
                sim.wire.push(l.to_wire());
            }
            (true, true) if changed => sim.wire.push(l.to_wire()),
            (true, false) => {
                l.on_air = false;
                l.out_gen = next_out_gen();
                l.stop_playback();
                sim.hide.push(l.id.clone());
                sim.post.extend(content_msgs);
            }
            (false, false) if changed => sim.pre.extend(content_msgs),
            _ => continue,
        }
        sim.touched.push(l.id.clone());
    }
    sim
}

/// Folds a Program change into staged content: each part of `cue` the operator left as it was
/// (equal to the `old` Program) follows the `new` Program; parts the operator changed win.
fn rebase(cue: &mut LayerContent, old: &LayerContent, new: &LayerContent) {
    let look_untouched = cue.variant == old.variant
        && (Arc::ptr_eq(&cue.template, &old.template) || cue.template == old.template)
        && cue.var_values == old.var_values
        && cue.vars == old.vars;
    if look_untouched {
        cue.template = new.template.clone();
        cue.variant = new.variant.clone();
        cue.variables = new.variables.clone();
        cue.var_values = new.var_values.clone();
        cue.vars = new.vars.clone();
    }
    for (k, v) in &new.fields {
        if cue.template.has_field(k) && cue.fields.get(k) == old.fields.get(k) {
            cue.fields.insert(k.clone(), v.clone());
        }
    }
    if cue.rect == old.rect {
        cue.rect = new.rect;
    }
    if cue.sizing == old.sizing {
        cue.sizing = new.sizing;
    }
    if cue.opacity == old.opacity {
        cue.opacity = new.opacity;
    }
    if cue.auto_out_ms == old.auto_out_ms {
        cue.auto_out_ms = new.auto_out_ms;
    }
    if cue.media == old.media {
        cue.media = new.media.clone();
    }
}

/// Preview of a screen: Program with the diff applied (cued-in layers on air, cued-out ones off,
/// staged content applied). `templateRev`s are bumped where the look changed; a preview bus fixes
/// them up with [`bus_diff`].
pub fn preview_of(screen: &ScreenLive) -> ScreenLive {
    let mut s = ScreenLive {
        info: screen.info.clone(),
        layers: screen.layers.clone(),
        preview: ScreenPreview::default(),
    };
    for l in s.layers.iter_mut() {
        let Some(cue) = screen.preview.layers.get(&l.id) else {
            continue;
        };
        if let Some(c) = &cue.content {
            apply_content(l, (**c).clone(), None);
        }
        if let Some(v) = cue.visible {
            l.on_air = v;
        }
    }
    // Preview shows media layers settled: paused at the in point.
    for l in s.layers.iter_mut() {
        if let Some(m) = &l.media {
            l.playback = MediaPlayback {
                state: if m.plays() {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Stopped
                },
                at: 0,
                pos_ms: m.config.in_ms,
            };
        }
    }
    s
}

fn same_look(a: &LayerLive, b: &LayerLive) -> bool {
    a.media.as_ref().map(|m| &m.config.asset) == b.media.as_ref().map(|m| &m.config.asset)
        && a.variant == b.variant
        && (Arc::ptr_eq(&a.template, &b.template)
            || (a.template.html == b.template.html
                && a.template.css == b.template.css
                && field_types(&a.template.fields) == field_types(&b.template.fields)))
}

/// The messages that turn what a preview-bus renderer shows (`old`) into `new`. Fixes `new`'s
/// `templateRev`s first so they only grow on that bus: a layer keeps its rev while its look is
/// unchanged and gets `old rev + 1` when it changed. Same message types as Program.
pub fn bus_diff(old: &ScreenLive, new: &mut ScreenLive) -> Vec<ServerMsg> {
    for l in new.layers.iter_mut() {
        if let Some(o) = old.layers.iter().find(|o| o.id == l.id) {
            l.template_rev = if same_look(o, l) {
                o.template_rev
            } else {
                o.template_rev.wrapping_add(1)
            };
        }
    }
    if old.info != new.info {
        return vec![new.snapshot()];
    }
    let mut msgs: Vec<ServerMsg> = old
        .layers
        .iter()
        .filter(|o| new.layers.iter().all(|l| l.id != o.id))
        .map(|o| ServerMsg::Remove {
            layer_id: o.id.clone(),
        })
        .collect();
    for l in &new.layers {
        let Some(o) = old.layers.iter().find(|o| o.id == l.id) else {
            msgs.push(ServerMsg::Layer { layer: l.to_wire() });
            continue;
        };
        if o.template_rev != l.template_rev
            || o.z != l.z
            || o.auto_out_ms != l.auto_out_ms
            || o.sizing != l.sizing
            || o.opacity != l.opacity
            || o.template.design != l.template.design
            || o.media != l.media
        {
            msgs.push(ServerMsg::Layer { layer: l.to_wire() });
            continue;
        }
        if o.playback != l.playback && l.media.is_some() {
            msgs.push(ServerMsg::Media {
                layer_id: l.id.clone(),
                playback: l.playback,
            });
        }
        let mut vars: Fields = l
            .vars
            .iter()
            .filter(|(k, v)| o.vars.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for k in o.vars.keys().filter(|k| !l.vars.contains_key(*k)) {
            vars.insert(k.clone(), String::new());
        }
        if !vars.is_empty() {
            msgs.push(ServerMsg::Vars {
                layer_id: l.id.clone(),
                vars,
            });
        }
        let fields: Fields = l
            .fields
            .iter()
            .filter(|(k, v)| o.fields.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if !fields.is_empty() {
            msgs.push(ServerMsg::Fields {
                layer_id: l.id.clone(),
                fields,
            });
        }
        if let (true, Some(rect)) = (o.rect != l.rect, l.rect) {
            msgs.push(ServerMsg::Rect {
                layer_id: l.id.clone(),
                rect,
            });
        }
        if o.on_air != l.on_air {
            msgs.push(l.state_msg());
        }
    }
    msgs
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LiveError {
    #[error("Unknown screen: {0}")]
    UnknownScreen(String),
    #[error("Unknown layer: {0}")]
    UnknownLayer(String),
    #[error("Unknown field: {0}")]
    UnknownField(String),
    #[error("Not a media layer: {0}")]
    NotMedia(String),
}

impl From<LiveError> for crate::error::AppError {
    fn from(e: LiveError) -> Self {
        match e {
            LiveError::UnknownScreen(_) | LiveError::UnknownLayer(_) => {
                Self::NotFound(e.to_string())
            }
            LiveError::UnknownField(_) | LiveError::NotMedia(_) => Self::Invalid(e.to_string()),
        }
    }
}

// ---- What the app UI sees (mirrored in src/lib/types.ts) ----------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveLayerView {
    pub id: String,
    pub name: String,
    pub on_air: bool,
    /// The field values, or with `pending` the staged ones (what the next take shows).
    pub fields: Fields,
    /// The template's fields (with `pending`, the staged template's).
    pub field_defs: Vec<FieldDef>,
    /// On air with edits staged for the next take (`liveEdit = onTake`): "Changes on next take".
    #[serde(default)]
    pub pending: bool,
    /// The variant shown (with `pending`, the staged one); `""` when there is none.
    #[serde(default)]
    pub variant: String,
    /// The variant the output shows now (with `pending`, the one before the staged change).
    /// Remote control's `variant` / `variant_next`; not sent to the UI.
    #[serde(skip)]
    pub on_air_variant: String,
    /// Media layers only: playback (Program), readiness and the file's duration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaLayerView>,
    /// Template layers with a video or image field (non-empty): preload status of those files
    /// over the screen's Program renderers. Absent otherwise (media layers carry theirs in
    /// `media`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<MediaReadiness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveScreenView {
    pub screen: ScreenInfo,
    pub layers: Vec<LiveLayerView>,
}

/// The built-in test lower third (`presets/templates/test-lower-third`).
const TEST_TEMPLATE_HTML: &str =
    include_str!("../../../presets/templates/test-lower-third/template.html");
const TEST_TEMPLATE_CSS: &str =
    include_str!("../../../presets/templates/test-lower-third/template.css");

fn test_template() -> TemplateDef {
    TemplateDef {
        html: TEST_TEMPLATE_HTML.to_string(),
        css: TEST_TEMPLATE_CSS.to_string(),
        fields: vec![
            FieldDef {
                key: "name".into(),
                label: "Name".into(),
                kind: FieldType::Text,
                default: String::new(),
                ..Default::default()
            },
            FieldDef {
                key: "title".into(),
                label: "Title".into(),
                kind: FieldType::Text,
                default: String::new(),
                ..Default::default()
            },
        ],
        design: None,
    }
}

impl TemplateDef {
    /// Every field at its default value (as renderers get it: canonical lists and timers, a
    /// clock's format).
    pub fn default_fields(&self) -> Fields {
        self.fields
            .iter()
            .map(|f| (f.key.clone(), crate::core::fieldvals::wire_default(f)))
            .collect()
    }

    pub fn has_field(&self, key: &str) -> bool {
        self.fields.iter().any(|f| f.key == key)
    }
}

impl ScreenLive {
    pub fn snapshot(&self) -> ServerMsg {
        ServerMsg::Snapshot {
            screen: self.info.clone(),
            layers: self.layers.iter().map(LayerLive::to_wire).collect(),
        }
    }

    /// The Program view. Outside Studio mode a layer with staged content reports it (`pending`);
    /// in Studio mode the staged content belongs to the Preview ([`ScreenLive::preview_view`]).
    pub fn view_in(&self, studio: bool) -> LiveScreenView {
        LiveScreenView {
            screen: self.info.clone(),
            layers: self
                .layers
                .iter()
                .map(|l| {
                    let staged = if studio {
                        None
                    } else {
                        self.preview.staged(&l.id)
                    };
                    l.view_with(staged)
                })
                .collect(),
        }
    }

    /// The Program view outside Studio mode (see [`ScreenLive::view_in`]).
    pub fn view(&self) -> LiveScreenView {
        self.view_in(false)
    }

    /// What the Preview looks like and which cues make it so.
    pub fn preview_view(&self) -> PreviewView {
        let shown = preview_of(self);
        PreviewView {
            screen_id: self.info.id.clone(),
            rev: self.preview.rev,
            cues: self
                .layers
                .iter()
                .filter_map(|l| {
                    let c = self.preview.layers.get(&l.id)?;
                    Some(LayerCue {
                        layer_id: l.id.clone(),
                        visible: c.visible,
                        staged: c.content.is_some(),
                        program_changed: c.program_changed_rev.is_some(),
                    })
                })
                .collect(),
            layers: shown.layers.iter().map(LayerLive::view).collect(),
            tbar: self.preview.tbar.as_ref().map(|t| t.sent),
        }
    }

    /// Visibility cues worth keeping over a restart (when everything starts off air): cue-ins.
    pub fn persisted_cues(&self) -> BTreeMap<String, bool> {
        self.preview
            .layers
            .iter()
            .filter(|(_, c)| c.visible == Some(true))
            .map(|(id, _)| (id.clone(), true))
            .collect()
    }

    /// Takes the diff to Program (see [`LiveState::take_preview`]).
    fn take(&mut self, sim: SimTake, after: PreviewAfterTake) -> (Vec<WireLayer>, Vec<String>) {
        self.layers = sim.layers;
        self.preview.layers.clear();
        self.preview.tbar = None;
        if after == PreviewAfterTake::Clear {
            for l in self.layers.iter().filter(|l| l.on_air) {
                self.preview.layers.insert(
                    l.id.clone(),
                    Cue {
                        visible: Some(false),
                        ..Default::default()
                    },
                );
            }
        }
        self.preview.bump();
        (sim.wire, sim.hide)
    }

    fn layer(&self, layer_id: &str) -> Result<&LayerLive, LiveError> {
        self.layers
            .iter()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))
    }

    fn layer_mut(&mut self, layer_id: &str) -> Result<&mut LayerLive, LiveError> {
        self.layers
            .iter_mut()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))
    }
}

impl LiveState {
    /// The test seed: screen `test` (1920×1080, transparent) with the built-in lower third, off air,
    /// fields at their defaults.
    pub fn with_test_screen() -> Self {
        Self {
            screens: vec![ScreenLive {
                info: ScreenInfo {
                    id: TEST_SCREEN_ID.into(),
                    name: "Output test".into(),
                    width: 1920,
                    height: 1080,
                    background: "transparent".into(),
                },
                layers: vec![LayerLive::new(
                    TEST_LAYER_ID,
                    "Lower third",
                    test_template(),
                )],
                preview: ScreenPreview::default(),
            }],
            studio: false,
        }
    }

    /// Live state of a freshly opened project: the test screen plus the template editor's preview
    /// screen (`__preview`, 1920×1080, transparent) with one empty layer, off air.
    pub fn for_project() -> Self {
        let mut s = Self::with_test_screen();
        s.screens.push(ScreenLive {
            info: ScreenInfo {
                id: PREVIEW_SCREEN_ID.into(),
                name: "Template preview".into(),
                width: 1920,
                height: 1080,
                background: "transparent".into(),
            },
            layers: vec![LayerLive::new(
                PREVIEW_LAYER_ID,
                "Template",
                TemplateDef {
                    html: "<div></div>".into(),
                    css: String::new(),
                    fields: Vec::new(),
                    design: None,
                },
            )],
            preview: ScreenPreview::default(),
        });
        s
    }

    pub fn screen(&self, screen_id: &str) -> Option<&ScreenLive> {
        self.screens.iter().find(|s| s.info.id == screen_id)
    }

    fn screen_ref(&self, screen_id: &str) -> Result<&ScreenLive, LiveError> {
        self.screen(screen_id)
            .ok_or_else(|| LiveError::UnknownScreen(screen_id.to_string()))
    }

    fn screen_mut(&mut self, screen_id: &str) -> Result<&mut ScreenLive, LiveError> {
        self.screens
            .iter_mut()
            .find(|s| s.info.id == screen_id)
            .ok_or_else(|| LiveError::UnknownScreen(screen_id.to_string()))
    }

    pub fn layer_mut(
        &mut self,
        screen_id: &str,
        layer_id: &str,
    ) -> Result<&mut LayerLive, LiveError> {
        self.screen_mut(screen_id)?.layer_mut(layer_id)
    }

    /// `snapshot` message for a screen, or `None` if it doesn't exist.
    pub fn snapshot(&self, screen_id: &str) -> Option<ServerMsg> {
        self.screen(screen_id).map(ScreenLive::snapshot)
    }

    pub fn screen_view(&self, screen_id: &str) -> Result<LiveScreenView, LiveError> {
        let studio = self.studio;
        self.screen_ref(screen_id).map(|s| s.view_in(studio))
    }

    /// A view of every screen, in order.
    pub fn screen_views(&self) -> Vec<LiveScreenView> {
        self.screens
            .iter()
            .map(|s| s.view_in(self.studio))
            .collect()
    }

    /// Puts a layer on air. Outside Studio mode staged changes are applied first (their content
    /// messages go out before `state in`), and an on-air layer with staged changes takes them
    /// without a state change. In Studio mode (automation; operators take through the Preview)
    /// the staged content stays in the diff. Otherwise idempotent: showing an on-air layer emits
    /// nothing.
    pub fn show(&mut self, screen_id: &str, layer_id: &str) -> Result<Vec<ServerMsg>, LiveError> {
        let studio = self.studio;
        let (layer, preview) = self.layer_and_preview(screen_id, layer_id)?;
        let staged = if studio {
            None
        } else {
            preview.take_content(layer_id)
        };
        let mut msgs = match staged {
            Some(c) => apply_content(layer, c, None),
            None => Vec::new(),
        };
        if !layer.on_air {
            layer.on_air = true;
            layer.out_gen = next_out_gen();
            msgs.push(layer.state_msg());
            if layer.start_playback(fsx::now_epoch_ms()) {
                msgs.push(layer.media_msg());
            }
        }
        Ok(msgs)
    }

    /// Transport on a media layer's Program playback (Studio mode too: never staged). One
    /// `media` message when the playback changed.
    pub fn media_control(
        &mut self,
        screen_id: &str,
        layer_id: &str,
        action: MediaAction,
        ms: Option<u64>,
        now: u64,
    ) -> Result<Vec<ServerMsg>, LiveError> {
        let layer = self.layer_mut(screen_id, layer_id)?;
        let Some(m) = &layer.media else {
            return Err(LiveError::NotMedia(layer_id.to_string()));
        };
        let next = m.control(&layer.playback, action, ms, now);
        if next == layer.playback {
            return Ok(Vec::new());
        }
        layer.playback = next;
        Ok(vec![layer.media_msg()])
    }

    /// A media layer playing with `hold` reached its out point: paused there, if its playback
    /// is still `expected` (one `media` message), else nothing.
    pub fn media_hold(
        &mut self,
        screen_id: &str,
        layer_id: &str,
        expected: &MediaPlayback,
        at: u64,
    ) -> Vec<ServerMsg> {
        let Ok(layer) = self.layer_mut(screen_id, layer_id) else {
            return Vec::new();
        };
        let Some(m) = &layer.media else {
            return Vec::new();
        };
        if layer.playback != *expected || !layer.on_air {
            return Vec::new();
        }
        layer.playback = m.held(at);
        vec![layer.media_msg()]
    }

    /// Sets a layer's readiness (from the hub's per-connection reports). `true` if it changed.
    /// (See [`LiveLayer::ready_key`] for which layers have one.)
    pub fn set_readiness(&mut self, screen_id: &str, layer_id: &str, r: MediaReadiness) -> bool {
        match self.layer_mut(screen_id, layer_id) {
            Ok(l) if l.readiness != r => {
                l.readiness = r;
                true
            }
            _ => false,
        }
    }

    /// Takes a layer off air. Hiding an off-air layer emits nothing.
    pub fn hide(&mut self, screen_id: &str, layer_id: &str) -> Result<Vec<ServerMsg>, LiveError> {
        self.set_on_air(screen_id, layer_id, false)
    }

    fn set_on_air(
        &mut self,
        screen_id: &str,
        layer_id: &str,
        on_air: bool,
    ) -> Result<Vec<ServerMsg>, LiveError> {
        let layer = self.layer_mut(screen_id, layer_id)?;
        if layer.on_air == on_air {
            return Ok(Vec::new());
        }
        layer.on_air = on_air;
        layer.out_gen = next_out_gen();
        if !on_air {
            layer.stop_playback();
        }
        Ok(vec![layer.state_msg()])
    }

    /// Merges `fields`. Unknown keys are rejected before anything changes; the emitted `fields`
    /// message contains only keys whose value actually changed (nothing if none did).
    pub fn set_fields(
        &mut self,
        screen_id: &str,
        layer_id: &str,
        fields: Fields,
    ) -> Result<Vec<ServerMsg>, LiveError> {
        let layer = self.layer_mut(screen_id, layer_id)?;
        if let Some(bad) = fields.keys().find(|k| !layer.template.has_field(k)) {
            return Err(LiveError::UnknownField(bad.clone()));
        }
        let mut changed = Fields::new();
        for (key, value) in fields {
            if layer.fields.get(&key) != Some(&value) {
                layer.fields.insert(key.clone(), value.clone());
                changed.insert(key, value);
            }
        }
        if changed.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![ServerMsg::Fields {
            layer_id: layer.id.clone(),
            fields: changed,
        }])
    }

    /// Takes every layer of a screen off air.
    pub fn clear(&mut self, screen_id: &str) -> Result<Vec<ServerMsg>, LiveError> {
        let screen = self.screen_mut(screen_id)?;
        Ok(screen
            .layers
            .iter_mut()
            .filter(|l| l.on_air)
            .map(|l| {
                l.on_air = false;
                l.out_gen = next_out_gen();
                l.stop_playback();
                l.state_msg()
            })
            .collect())
    }

    /// PANIC: every layer of every screen except the `__*` system screens goes off air at once.
    /// Outside Studio mode staged changes are kept; in Studio mode every Preview diff is cleared
    /// (and every T-bar ended: the hub sends `abort` first). Returns the ids of the screens it
    /// applies to: each gets one `cut` (whether or not something was on air), never `state`
    /// messages.
    pub fn cut(&mut self) -> Vec<String> {
        let studio = self.studio;
        let mut ids = Vec::new();
        for screen in self
            .screens
            .iter_mut()
            .filter(|s| !s.info.id.starts_with("__"))
        {
            for l in screen.layers.iter_mut().filter(|l| l.on_air) {
                l.on_air = false;
                l.out_gen = next_out_gen();
                l.stop_playback();
            }
            screen.preview.tbar = None;
            if studio {
                screen.preview.clear();
            }
            ids.push(screen.info.id.clone());
        }
        ids
    }

    pub fn layer_view(&self, screen_id: &str, layer_id: &str) -> Result<LiveLayerView, LiveError> {
        let screen = self.screen_ref(screen_id)?;
        let staged = if self.studio {
            None
        } else {
            screen.preview.staged(layer_id)
        };
        Ok(screen.layer(layer_id)?.view_with(staged))
    }

    /// Whether a layer has content staged (for the next take, or in the Preview diff).
    pub fn has_staged(&self, screen_id: &str, layer_id: &str) -> bool {
        self.screen(screen_id)
            .is_some_and(|s| s.preview.staged(layer_id).is_some())
    }

    /// Outside Studio mode, automation's forced field values are patched into the layer's staged
    /// content so a later take keeps them; staged content then equal to what is on air is
    /// dropped.
    pub fn patch_staged_fields(&mut self, screen_id: &str, layer_id: &str, fields: &Fields) {
        let Ok((layer, preview)) = self.layer_and_preview(screen_id, layer_id) else {
            return;
        };
        let Some(staged) = preview.staged(layer_id) else {
            return;
        };
        let mut staged = staged.clone();
        for (k, v) in fields {
            if staged.template.has_field(k) {
                staged.fields.insert(k.clone(), v.clone());
            }
        }
        let next = (staged != layer.content()).then_some(staged);
        preview.set_content(layer_id, next);
    }

    /// The content of a layer while it has staged content (to rebase it after a Program change).
    pub fn content_if_staged(&self, screen_id: &str, layer_id: &str) -> Option<LayerContent> {
        let s = self.screen(screen_id)?;
        s.preview.staged(layer_id)?;
        s.layers
            .iter()
            .find(|l| l.id == layer_id)
            .map(LayerLive::content)
    }

    /// A layer and its screen's Preview diff, borrowed together.
    fn layer_and_preview(
        &mut self,
        screen_id: &str,
        layer_id: &str,
    ) -> Result<(&mut LayerLive, &mut ScreenPreview), LiveError> {
        let screen = self.screen_mut(screen_id)?;
        let layer = screen
            .layers
            .iter_mut()
            .find(|l| l.id == layer_id)
            .ok_or_else(|| LiveError::UnknownLayer(layer_id.to_string()))?;
        Ok((layer, &mut screen.preview))
    }

    // ---- Studio mode ------------------------------------------------------------------------

    /// A screen's Preview view.
    pub fn preview_view(&self, screen_id: &str) -> Result<PreviewView, LiveError> {
        self.screen_ref(screen_id).map(ScreenLive::preview_view)
    }

    /// Cues a layer in / out of Preview (`None`: as Program). A cue equal to Program's state is
    /// no cue. `true` if the diff changed.
    pub fn cue(
        &mut self,
        screen_id: &str,
        layer_id: &str,
        visible: Option<bool>,
    ) -> Result<bool, LiveError> {
        let (layer, preview) = self.layer_and_preview(screen_id, layer_id)?;
        let visible = visible.filter(|v| *v != layer.on_air);
        let rev = preview.rev;
        preview.set_visible(layer_id, visible);
        Ok(preview.rev != rev)
    }

    /// Empties a screen's diff: Preview = Program. `true` if there was a cue.
    pub fn clear_preview(&mut self, screen_id: &str) -> Result<bool, LiveError> {
        Ok(self.screen_mut(screen_id)?.preview.clear())
    }

    /// The screen's diff revision (`None` for an unknown screen).
    pub fn preview_rev(&self, screen_id: &str) -> Option<u64> {
        self.screen(screen_id).map(|s| s.preview.rev)
    }

    /// Whether a T-bar is in progress on the screen.
    pub fn tbar_active(&self, screen_id: &str) -> bool {
        self.screen(screen_id)
            .is_some_and(|s| s.preview.tbar.is_some())
    }

    /// Takes the screen's Preview to Program: staged content applied, cued layers in / out, all
    /// with ONE `take` message (see [`TakeOutcome`]). Then the diff follows `after`: `keep` →
    /// empty, `clear` → every on-air layer cued out. An empty diff does nothing.
    pub fn take_preview(
        &mut self,
        screen_id: &str,
        transition: TakeTransition,
        after: PreviewAfterTake,
    ) -> Result<TakeOutcome, LiveError> {
        let screen = self.screen_mut(screen_id)?;
        if screen.preview.is_empty() {
            return Ok(TakeOutcome::default());
        }
        let sim = simulate_take(screen);
        let mut msgs = sim.pre.clone();
        let post = sim.post.clone();
        let touched = sim.touched.clone();
        let (layers, hide) = screen.take(sim, after);
        if !layers.is_empty() || !hide.is_empty() {
            msgs.push(ServerMsg::Take {
                transition: transition.clamped(),
                layers,
                hide,
            });
        }
        msgs.extend(post);
        Ok(TakeOutcome { msgs, touched })
    }

    /// Starts a T-bar: one `prepare` with what a take would bring in / take out (Program is
    /// unchanged until the commit). A T-bar already in progress is kept (nothing sent).
    pub fn tbar_begin(&mut self, screen_id: &str) -> Result<Vec<ServerMsg>, LiveError> {
        let screen = self.screen_mut(screen_id)?;
        if screen.preview.tbar.is_some() {
            return Ok(Vec::new());
        }
        let sim = simulate_take(screen);
        let prepared = (sim.wire, sim.hide);
        screen.preview.tbar = Some(Tbar {
            p: 0.0,
            sent: 0.0,
            prepared: prepared.clone(),
        });
        Ok(vec![ServerMsg::Prepare {
            layers: prepared.0,
            hide: prepared.1,
        }])
    }

    /// Moves the T-bar (clamped into 0..=1; NaN is 0), starting one if none is in progress (its
    /// `prepare` is returned). The hub decides when `tbar` goes out.
    pub fn tbar_set(&mut self, screen_id: &str, p: f64) -> Result<Vec<ServerMsg>, LiveError> {
        let msgs = self.tbar_begin(screen_id)?;
        let p = if p.is_nan() { 0.0 } else { p.clamp(0.0, 1.0) };
        if let Some(t) = self.screen_mut(screen_id)?.preview.tbar.as_mut() {
            t.p = p;
        }
        Ok(msgs)
    }

    /// The T-bar position not sent yet, marked as sent now (`None` when none is in progress or
    /// it is up to date).
    pub fn tbar_unsent(&mut self, screen_id: &str) -> Option<f64> {
        let t = self.screens.iter_mut().find(|s| s.info.id == screen_id)?;
        let t = t.preview.tbar.as_mut()?;
        if t.p == t.sent {
            return None;
        }
        t.sent = t.p;
        Some(t.p)
    }

    /// Ends a T-bar. `commit`: the diff goes to Program like a take, renderers get `commit`
    /// instead of `take` (they already show the result), plus a `snapshot` if Program changed
    /// underneath since `prepare`. Otherwise `abort`: Program and the diff unchanged. Nothing
    /// when no T-bar is in progress.
    pub fn tbar_end(
        &mut self,
        screen_id: &str,
        commit: bool,
        after: PreviewAfterTake,
    ) -> Result<TakeOutcome, LiveError> {
        let screen = self.screen_mut(screen_id)?;
        let Some(tbar) = screen.preview.tbar.take() else {
            return Ok(TakeOutcome::default());
        };
        if !commit {
            return Ok(TakeOutcome {
                msgs: vec![ServerMsg::Abort {}],
                touched: Vec::new(),
            });
        }
        let mut sim = simulate_take(screen);
        keep_prepared_playback(&mut sim, &tbar.prepared.0);
        let mut msgs = sim.pre.clone();
        let post = sim.post.clone();
        let touched = sim.touched.clone();
        let done = screen.take(sim, after);
        msgs.push(ServerMsg::Commit {});
        msgs.extend(post);
        if done != tbar.prepared {
            msgs.push(screen.snapshot());
        }
        Ok(TakeOutcome { msgs, touched })
    }

    /// Ends a T-bar in progress without a take (`abort`), e.g. before another change of the diff.
    pub fn tbar_abort(&mut self, screen_id: &str) -> Vec<ServerMsg> {
        match self.screen_mut(screen_id).map(|s| s.preview.tbar.take()) {
            Ok(Some(_)) => vec![ServerMsg::Abort {}],
            _ => Vec::new(),
        }
    }

    /// Ids of the screens with a T-bar in progress.
    pub fn tbar_screens(&self) -> Vec<String> {
        self.screens
            .iter()
            .filter(|s| s.preview.tbar.is_some())
            .map(|s| s.info.id.clone())
            .collect()
    }

    /// Ids of the screens whose diff is not empty.
    pub fn screens_with_preview(&self) -> Vec<String> {
        self.screens
            .iter()
            .filter(|s| !s.preview.is_empty())
            .map(|s| s.info.id.clone())
            .collect()
    }

    /// Leaving Studio mode keeping the staged content: on an on-air layer it becomes On-take
    /// staging; on an off-air layer it applies now (the live rule off air). Visibility cues and
    /// T-bars are dropped. Returns the Program messages per screen. Sets `studio` off.
    pub fn studio_off_keep_staged(&mut self) -> Vec<(String, Vec<ServerMsg>)> {
        self.studio = false;
        let mut out = Vec::new();
        for screen in self.screens.iter_mut() {
            screen.preview.tbar = None;
            if screen.preview.is_empty() {
                continue;
            }
            let mut msgs = Vec::new();
            let cues = std::mem::take(&mut screen.preview.layers);
            for (id, cue) in cues {
                let Some(content) = cue.content else { continue };
                let Some(layer) = screen.layers.iter_mut().find(|l| l.id == id) else {
                    continue;
                };
                if layer.on_air {
                    screen.preview.layers.insert(
                        id,
                        Cue {
                            content: Some(content),
                            ..Default::default()
                        },
                    );
                } else {
                    msgs.extend(apply_content(layer, *content, None));
                }
            }
            screen.preview.bump();
            out.push((screen.info.id.clone(), msgs));
        }
        out
    }

    /// Automation changed `msgs` on Program (Studio mode): every layer they touch that has a cue
    /// is marked `programChanged`, and its staged content is rebased (`old` = the layer's
    /// content before, when the op could change content). A cue equal to Program afterwards is
    /// dropped. Nothing outside Studio mode.
    pub fn program_changed(
        &mut self,
        screen_id: &str,
        msgs: &[ServerMsg],
        old: Option<(&str, &LayerContent)>,
    ) {
        if !self.studio {
            return;
        }
        let Ok(screen) = self.screen_mut(screen_id) else {
            return;
        };
        let rev = screen.preview.rev;
        for msg in msgs {
            let id = match msg {
                ServerMsg::State { layer_id, .. }
                | ServerMsg::Fields { layer_id, .. }
                | ServerMsg::Vars { layer_id, .. }
                | ServerMsg::Rect { layer_id, .. } => layer_id,
                ServerMsg::Layer { layer } => &layer.id,
                _ => continue,
            };
            let Some(layer) = screen.layers.iter().find(|l| l.id == *id) else {
                continue;
            };
            let Some(cue) = screen.preview.layers.get_mut(id) else {
                continue;
            };
            cue.program_changed_rev.get_or_insert(rev);
            if let (Some((old_id, old)), Some(c)) = (old, cue.content.as_deref_mut()) {
                if old_id == id {
                    let now = layer.content();
                    rebase(c, old, &now);
                    if *c == now {
                        cue.content = None;
                    }
                }
            }
            // A visibility cue Program now matches stays (with its marker) until the operator
            // clears it or takes: it does nothing on the take.
            if cue.is_empty() {
                screen.preview.layers.remove(id);
                screen.preview.bump();
            }
        }
    }

    /// Program changed for every layer of these screens (PANIC aside: clear, clear all).
    pub fn program_changed_all(&mut self, screen_id: &str) {
        if !self.studio {
            return;
        }
        let Ok(screen) = self.screen_mut(screen_id) else {
            return;
        };
        let rev = screen.preview.rev;
        for cue in screen.preview.layers.values_mut() {
            cue.program_changed_rev.get_or_insert(rev);
        }
    }

    /// The visibility cues to persist (user screens): see [`StudioCues`].
    pub fn studio_cues(&self) -> StudioCues {
        self.screens
            .iter()
            .filter(|s| !is_reserved_screen(&s.info.id))
            .map(|s| (s.info.id.clone(), s.persisted_cues()))
            .filter(|(_, c)| !c.is_empty())
            .collect()
    }

    /// Drops every visibility cue (staged content stays).
    pub fn drop_visibility_cues(&mut self) {
        for screen in self.screens.iter_mut() {
            let ids: Vec<String> = screen.preview.layers.keys().cloned().collect();
            for id in ids {
                screen.preview.set_visible(&id, None);
            }
        }
    }

    /// Restores persisted cues after a project opens (everything is off air): cue-ins of layers
    /// that still exist.
    pub fn restore_cues(&mut self, cues: &StudioCues) {
        for screen in self.screens.iter_mut() {
            let Some(layers) = cues.get(&screen.info.id) else {
                continue;
            };
            for (id, visible) in layers {
                let Some(l) = screen.layers.iter().find(|l| l.id == *id) else {
                    continue;
                };
                if *visible != l.on_air {
                    screen.preview.set_visible(id, Some(*visible));
                }
            }
        }
    }

    /// Creates or updates a screen from its plan. The one place project screens change:
    /// saving, dragging (through [`LiveState::set_rect`]), template changes and the file watcher
    /// all end up here.
    ///
    /// - New screen: added off air (one `snapshot`).
    /// - Layer added: off air, one `layer`. Removed: off air → `remove`; on air → `state out` now
    ///   and listed in `removed_on_air` (the hub sends `remove` later). Reordered or renamed:
    ///   applies at once (`layer` with the same `templateRev` for a z change).
    /// - Content (template, fields, vars, rect): see [`LayerContent`].
    /// - Name, size or background changed: a fresh `snapshot` at the end.
    pub fn apply_screen(&mut self, plan: ScreenPlan, mode: LiveEdit) -> ApplyOutcome {
        self.apply_screen_with(plan, mode, &[])
    }

    /// [`LiveState::apply_screen`] with variant switches requested for some layers: an on-air
    /// layer whose variant changes now (`immediate`, or `switch.direct`) swaps with
    /// `switch.transition` (other variant changes on air use a crossfade). With `direct`
    /// (automation) the layer's new variant goes on air even under `onTake`; its other content
    /// changes stay staged.
    ///
    /// In Studio mode every content change goes to the Preview diff instead (on or off air;
    /// `mode` is ignored), structural changes apply as above, and a `direct` switch puts the new
    /// variant on Program (on or off air) with the rest staged in the diff.
    pub fn apply_screen_with(
        &mut self,
        plan: ScreenPlan,
        mode: LiveEdit,
        switches: &[VariantSwitch],
    ) -> ApplyOutcome {
        let rule = if self.studio {
            EditRule::Studio
        } else {
            EditRule::Live(mode)
        };
        let ScreenPlan {
            info,
            layers: plans,
        } = plan;
        let Some(idx) = self.screens.iter().position(|s| s.info.id == info.id) else {
            let screen = ScreenLive {
                info,
                layers: plans
                    .into_iter()
                    .enumerate()
                    .map(|(z, p)| LayerLive::from_plan(p, z))
                    .collect(),
                preview: ScreenPreview::default(),
            };
            let msgs = vec![screen.snapshot()];
            self.screens.push(screen);
            self.screens.sort_by(screen_order);
            return ApplyOutcome {
                msgs,
                removed_on_air: Vec::new(),
            };
        };
        let screen = &mut self.screens[idx];
        let mut out = ApplyOutcome::default();
        let mut old: Vec<LayerLive> = std::mem::take(&mut screen.layers);
        let preview = &mut screen.preview;
        old.retain(|l| {
            if plans.iter().any(|p| p.id == l.id) {
                return true;
            }
            if preview.layers.remove(&l.id).is_some() {
                preview.bump();
            }
            if l.on_air {
                out.msgs.push(ServerMsg::State {
                    layer_id: l.id.clone(),
                    state: LayerState::Out,
                });
                out.removed_on_air.push(l.id.clone());
            } else {
                out.msgs.push(ServerMsg::Remove {
                    layer_id: l.id.clone(),
                });
            }
            false
        });
        for (z, p) in plans.into_iter().enumerate() {
            let z = z as i32;
            let Some(pos) = old.iter().position(|l| l.id == p.id) else {
                let layer = LayerLive::from_plan(p, z as usize);
                out.msgs.push(ServerMsg::Layer {
                    layer: layer.to_wire(),
                });
                screen.layers.push(layer);
                continue;
            };
            let mut layer = old.swap_remove(pos);
            layer.name = p.name;
            let z_changed = layer.z != z;
            layer.z = z;
            let switch = switches.iter().find(|s| s.layer_id == layer.id);
            let transition = switch.map(|s| s.transition).unwrap_or_default();
            let direct = match (switch, rule) {
                (Some(s), EditRule::Studio) => s.direct,
                (Some(s), EditRule::Live(m)) => s.direct && layer.on_air && m == LiveEdit::OnTake,
                (None, _) => false,
            };
            let msgs = if direct {
                let before = screen.preview.rev;
                let msgs =
                    apply_variant_directly(&mut layer, &mut screen.preview, p.content, transition);
                if rule == EditRule::Studio && !msgs.is_empty() {
                    if let Some(cue) = screen.preview.layers.get_mut(&layer.id) {
                        cue.program_changed_rev.get_or_insert(before);
                    }
                }
                msgs
            } else {
                stage_or_apply(
                    &mut layer,
                    &mut screen.preview,
                    p.content,
                    rule,
                    Some(transition),
                )
            };
            let sent_layer = msgs.iter().any(|m| matches!(m, ServerMsg::Layer { .. }));
            out.msgs.extend(msgs);
            if z_changed && !sent_layer {
                out.msgs.push(ServerMsg::Layer {
                    layer: layer.to_wire(),
                });
            }
            screen.layers.push(layer);
        }
        if screen.info != info {
            let renamed = screen.info.name != info.name;
            screen.info = info;
            out.msgs.push(screen.snapshot());
            if renamed {
                self.screens.sort_by(screen_order);
            }
        }
        out
    }

    /// Removes a screen. Its renderers get `error unknownScreen` (they drop what they show and
    /// wait; if the screen comes back, its snapshot reaches them through the hub).
    pub fn remove_screen(&mut self, screen_id: &str) -> Option<(ScreenLive, Vec<ServerMsg>)> {
        let idx = self.screens.iter().position(|s| s.info.id == screen_id)?;
        let screen = self.screens.remove(idx);
        Some((
            screen,
            vec![ServerMsg::Error {
                code: WireErrorCode::UnknownScreen,
            }],
        ))
    }

    /// Moves or resizes a layer's box, following the live-edit rules (a staged change on an
    /// on-air layer under `onTake`, or in the Preview diff in Studio mode; otherwise one `rect`
    /// message).
    pub fn set_rect(
        &mut self,
        screen_id: &str,
        layer_id: &str,
        rect: Rect,
        mode: LiveEdit,
    ) -> Result<Vec<ServerMsg>, LiveError> {
        let rule = if self.studio {
            EditRule::Studio
        } else {
            EditRule::Live(mode)
        };
        let (layer, preview) = self.layer_and_preview(screen_id, layer_id)?;
        let mut c = match preview.staged(layer_id) {
            Some(c) => c.clone(),
            None => layer.content(),
        };
        c.rect = Some(rect);
        Ok(stage_or_apply(layer, preview, c, rule, None))
    }
}

/// A T-bar commit keeps the playback its `prepare` started (renderers are already playing it).
fn keep_prepared_playback(sim: &mut SimTake, prepared: &[WireLayer]) {
    for w in sim.wire.iter_mut() {
        let Some(pb) = prepared
            .iter()
            .find(|p| p.id == w.id)
            .and_then(|p| p.media.as_ref())
            .map(|m| m.playback)
        else {
            continue;
        };
        if let Some(m) = w.media.as_mut() {
            m.playback = pb;
        }
        if let Some(l) = sim.layers.iter_mut().find(|l| l.id == w.id) {
            l.playback = pb;
        }
    }
}

/// Where a content change goes: the live-edit rule, or the Preview diff (Studio mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditRule {
    Live(LiveEdit),
    Studio,
}

// ---- Screens built from project files -------------------------------------------------

/// Everything about a layer's look that follows the live-edit rules: off air it applies at once;
/// on air under `liveEdit = onTake` it is staged until the next show. (Name and z are structural
/// and always apply at once.)
#[derive(Debug, Clone, PartialEq)]
pub struct LayerContent {
    pub template: Arc<TemplateDef>,
    /// The template variant `template` is the look of.
    pub variant: String,
    pub fields: Fields,
    pub variables: Vec<VariableDef>,
    pub var_values: Fields,
    /// CSS values of `var_values` (validated).
    pub vars: Fields,
    pub auto_out_ms: Option<u32>,
    pub rect: Option<Rect>,
    /// Content sizing (absent in the file = `reflow`).
    pub sizing: ContentSizing,
    /// Layer opacity 0–100 (absent in the file = 100).
    pub opacity: u8,
    /// Media layers: the file and its config (`template` is [`media_template`]).
    pub media: Option<MediaContent>,
}

/// A variant switch requested for one layer (`set_layer_variant`, flows): how an on-air layer
/// swaps looks, and whether it bypasses `liveEdit = onTake` (automation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantSwitch {
    pub layer_id: String,
    pub transition: VariantTransition,
    pub direct: bool,
}

/// One layer of a [`ScreenPlan`].
#[derive(Debug, Clone, PartialEq)]
pub struct LayerPlan {
    pub id: String,
    pub name: String,
    pub content: LayerContent,
}

/// What a screen should look like, built from its file and its templates (`core::screens::plan`).
/// Layers whose template is missing are not in it. Order = z-order, bottom first.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenPlan {
    pub info: ScreenInfo,
    pub layers: Vec<LayerPlan>,
}

/// Result of [`LiveState::apply_screen`].
#[derive(Debug, Default)]
pub struct ApplyOutcome {
    pub msgs: Vec<ServerMsg>,
    /// Layers that were on air when they were removed: they got `state out` and are already gone
    /// from the live state; `LiveHub` sends their `remove` once the out-animation has had time.
    pub removed_on_air: Vec<String>,
}

/// Applies `c` to the layer now. Variant, HTML, CSS or field types changed: `templateRev` + 1
/// and one `layer` message (it carries everything; on an on-air layer whose variant changed it
/// carries `swap` when one is given). The auto-out changed (no recompile): one same-rev `layer`
/// message (it carries the rest too). Otherwise `vars` (changed keys; a removed variable is
/// `""`), `fields` (changed keys) and `rect`, each only if something changed.
pub fn apply_content(
    layer: &mut LayerLive,
    c: LayerContent,
    swap: Option<VariantTransition>,
) -> Vec<ServerMsg> {
    let old = &layer.template;
    let variant_changed = layer.variant != c.variant;
    let file_changed =
        layer.media.as_ref().map(|m| &m.config.asset) != c.media.as_ref().map(|m| &m.config.asset);
    let recompile = variant_changed
        || file_changed
        || (!Arc::ptr_eq(old, &c.template)
            && (old.html != c.template.html
                || old.css != c.template.css
                || field_types(&old.fields) != field_types(&c.template.fields)));
    let mut vars_diff: Fields = c
        .vars
        .iter()
        .filter(|(k, v)| layer.vars.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for k in layer.vars.keys().filter(|k| !c.vars.contains_key(*k)) {
        vars_diff.insert(k.clone(), String::new());
    }
    let fields_diff: Fields = c
        .fields
        .iter()
        .filter(|(k, v)| layer.fields.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let rect_changed = layer.rect != c.rect;
    // The renderer gets these on a same-rev `layer` message (no recompile).
    let head_changed = layer.auto_out_ms != c.auto_out_ms
        || layer.sizing != c.sizing
        || layer.opacity != c.opacity
        || layer.template.design != c.template.design
        || layer.media != c.media;

    layer.template = c.template;
    layer.variant = c.variant;
    layer.fields = c.fields;
    layer.variables = c.variables;
    layer.var_values = c.var_values;
    layer.vars = c.vars;
    layer.auto_out_ms = c.auto_out_ms;
    layer.rect = c.rect;
    layer.sizing = c.sizing;
    layer.opacity = c.opacity;
    layer.media = c.media;

    if file_changed {
        // A new file: on air it starts like a take; off air it waits stopped.
        if layer.on_air {
            layer.playback = MediaPlayback::default();
            layer.start_playback(fsx::now_epoch_ms());
        } else {
            layer.stop_playback();
        }
    }

    if recompile {
        layer.template_rev = match &layer.media {
            Some(m) if file_changed => {
                let rev = media_rev(&m.config.asset);
                if rev == layer.template_rev {
                    rev.wrapping_add(1)
                } else {
                    rev
                }
            }
            _ => layer.template_rev.wrapping_add(1),
        };
        let mut wire = layer.to_wire();
        if layer.on_air && variant_changed {
            wire.swap = swap;
        }
        return vec![ServerMsg::Layer { layer: wire }];
    }
    if head_changed {
        // The renderer's --auto-out-ms, sizing, opacity or design size: a same-rev `layer` message carries everything (vars,
        // fields, rect too) and updates the host in place, without a recompile.
        return vec![ServerMsg::Layer {
            layer: layer.to_wire(),
        }];
    }
    let mut msgs = Vec::new();
    if !vars_diff.is_empty() {
        msgs.push(ServerMsg::Vars {
            layer_id: layer.id.clone(),
            vars: vars_diff,
        });
    }
    if !fields_diff.is_empty() {
        msgs.push(ServerMsg::Fields {
            layer_id: layer.id.clone(),
            fields: fields_diff,
        });
    }
    if let (true, Some(rect)) = (rect_changed, layer.rect) {
        msgs.push(ServerMsg::Rect {
            layer_id: layer.id.clone(),
            rect,
        });
    }
    msgs
}

fn field_types(fields: &[FieldDef]) -> Vec<(&str, FieldType)> {
    fields.iter().map(|f| (f.key.as_str(), f.kind)).collect()
}

/// The live-edit rule for a content change: on air under `onTake`, or on or off air in Studio
/// mode, it is staged (or the staged change dropped, when the content is back to what is
/// showing); otherwise it applies now.
fn stage_or_apply(
    layer: &mut LayerLive,
    preview: &mut ScreenPreview,
    c: LayerContent,
    rule: EditRule,
    swap: Option<VariantTransition>,
) -> Vec<ServerMsg> {
    let stage = match rule {
        EditRule::Studio => true,
        EditRule::Live(mode) => layer.on_air && mode == LiveEdit::OnTake,
    };
    if stage {
        let staged = (c != layer.content()).then_some(c);
        preview.set_content(&layer.id, staged);
        Vec::new()
    } else {
        preview.set_content(&layer.id, None);
        apply_content(layer, c, swap)
    }
}

/// Automation under `onTake` (on air) or in Studio mode: the planned variant (its look and
/// variable values) goes on air now with `transition`; everything else of `c` stays staged for
/// the next take (dropped when nothing else differs).
fn apply_variant_directly(
    layer: &mut LayerLive,
    preview: &mut ScreenPreview,
    c: LayerContent,
    transition: VariantTransition,
) -> Vec<ServerMsg> {
    let mut now = layer.content();
    now.template = c.template.clone();
    now.variant = c.variant.clone();
    now.variables = c.variables.clone();
    now.var_values = c.var_values.clone();
    now.vars = c.vars.clone();
    let msgs = apply_content(layer, now, Some(transition));
    let staged = (c != layer.content()).then_some(c);
    preview.set_content(&layer.id, staged);
    msgs
}

/// Reserved screens first (in their seeded order), then user screens by name (case-insensitive),
/// then id.
fn screen_order(a: &ScreenLive, b: &ScreenLive) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (
        is_reserved_screen(&a.info.id),
        is_reserved_screen(&b.info.id),
    ) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a
            .info
            .name
            .to_lowercase()
            .cmp(&b.info.name.to_lowercase())
            .then_with(|| a.info.id.cmp(&b.info.id)),
    }
}

impl LayerLive {
    /// An off-air layer at rev 1 with the template's default field values and no variables.
    pub fn new(id: &str, name: &str, template: TemplateDef) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            z: 0,
            fields: template.default_fields(),
            template: Arc::new(template),
            template_rev: 1,
            variant: String::new(),
            vars: Fields::new(),
            on_air: false,
            variables: Vec::new(),
            var_values: Fields::new(),
            auto_out_ms: None,
            out_gen: next_out_gen(),
            rect: None,
            sizing: ContentSizing::Reflow,
            opacity: 100,
            media: None,
            playback: MediaPlayback::default(),
            readiness: MediaReadiness::default(),
        }
    }

    /// An off-air project-screen layer at rev 1.
    pub fn from_plan(p: LayerPlan, z: usize) -> Self {
        let c = p.content;
        let playback = c
            .media
            .as_ref()
            .map(MediaContent::stopped)
            .unwrap_or_default();
        Self {
            id: p.id,
            name: p.name,
            z: i32::try_from(z).unwrap_or(i32::MAX),
            template_rev: c.media.as_ref().map_or(1, |m| media_rev(&m.config.asset)),
            template: c.template,
            variant: c.variant,
            fields: c.fields,
            vars: c.vars,
            on_air: false,
            variables: c.variables,
            var_values: c.var_values,
            auto_out_ms: c.auto_out_ms,
            out_gen: next_out_gen(),
            rect: c.rect,
            sizing: c.sizing,
            opacity: c.opacity,
            media: c.media,
            playback,
            readiness: MediaReadiness::default(),
        }
    }

    /// A media layer that just went on air starts playing (Take). `true` when its playback
    /// changed. Playback the operator set while off air (paused at a point, or playing) is kept.
    pub fn start_playback(&mut self, now: u64) -> bool {
        let Some(m) = &self.media else {
            return false;
        };
        if !m.plays() || self.playback.state != PlaybackState::Stopped {
            return false;
        }
        self.playback = m.take(now);
        true
    }

    /// Off air: stopped at the in point (renderers stop after their out-animation; no message).
    pub fn stop_playback(&mut self) {
        if let Some(m) = &self.media {
            self.playback = m.stopped();
        }
    }

    /// `media` message with the current playback.
    pub fn media_msg(&self) -> ServerMsg {
        ServerMsg::Media {
            layer_id: self.id.clone(),
            playback: self.playback,
        }
    }

    /// The content currently applied (what renderers show), staged changes aside.
    pub fn content(&self) -> LayerContent {
        LayerContent {
            template: self.template.clone(),
            variant: self.variant.clone(),
            fields: self.fields.clone(),
            variables: self.variables.clone(),
            var_values: self.var_values.clone(),
            vars: self.vars.clone(),
            auto_out_ms: self.auto_out_ms,
            rect: self.rect,
            sizing: self.sizing,
            opacity: self.opacity,
            media: self.media.clone(),
        }
    }

    pub fn to_wire(&self) -> WireLayer {
        WireLayer {
            id: self.id.clone(),
            z: self.z,
            template_rev: self.template_rev,
            html: self.template.html.clone(),
            css: self.template.css.clone(),
            fields: self.fields.clone(),
            field_types: self
                .template
                .fields
                .iter()
                .map(|f| (f.key.clone(), f.kind))
                .collect(),
            state: self.wire_state(),
            vars: self.vars.clone(),
            rect: self.rect,
            sizing: (self.sizing == ContentSizing::Fit).then_some(ContentSizing::Fit),
            opacity: (self.opacity < 100).then_some(self.opacity),
            design: self.template.design,
            swap: None,
            auto_out_ms: self.auto_out_ms,
            media: self.media.as_ref().map(|m| m.wire(self.playback)),
        }
    }

    /// What the app UI sees, without staged content.
    pub fn view(&self) -> LiveLayerView {
        self.view_with(None)
    }

    /// What a renderer's `ready` report is kept for, or `None` when the layer has no media to
    /// preload. Media layers: the `templateRev` (the file). Template layers: a hash of the rev and
    /// the values of their non-empty video / image fields, so a report for an old file never
    /// counts for a new one.
    pub fn ready_key(&self) -> Option<u32> {
        if self.media.is_some() {
            return Some(self.template_rev);
        }
        let mut h: u32 = 0x811c_9dc5;
        let mut mix = |bytes: &[u8]| {
            for b in bytes {
                h ^= u32::from(*b);
                h = h.wrapping_mul(0x0100_0193);
            }
        };
        mix(&self.template_rev.to_le_bytes());
        let mut any = false;
        for f in &self.template.fields {
            if !matches!(f.kind, FieldType::Image | FieldType::Video) {
                continue;
            }
            if let Some(v) = self.fields.get(&f.key).filter(|v| !v.is_empty()) {
                any = true;
                mix(f.key.as_bytes());
                mix(&[0]);
                mix(v.as_bytes());
                mix(&[0]);
            }
        }
        any.then_some(h.max(1))
    }

    /// What the app UI sees. With staged content (`pending`), `fields` and `field_defs` are the
    /// staged ones: what the next take will show (the output keeps showing the current content
    /// until then).
    pub fn view_with(&self, staged: Option<&LayerContent>) -> LiveLayerView {
        let (fields, template, variant) = match staged {
            Some(c) => (&c.fields, &c.template, &c.variant),
            None => (&self.fields, &self.template, &self.variant),
        };
        LiveLayerView {
            id: self.id.clone(),
            name: self.name.clone(),
            on_air: self.on_air,
            fields: fields.clone(),
            field_defs: template.fields.clone(),
            pending: staged.is_some(),
            variant: variant.clone(),
            on_air_variant: self.variant.clone(),
            media: self.media.as_ref().map(|m| MediaLayerView {
                playback: self.playback,
                readiness: self.readiness.clone(),
                duration_ms: staged
                    .and_then(|c| c.media.as_ref())
                    .unwrap_or(m)
                    .duration_ms,
            }),
            readiness: (self.media.is_none() && self.ready_key().is_some())
                .then(|| self.readiness.clone()),
        }
    }

    fn wire_state(&self) -> LayerState {
        if self.on_air {
            LayerState::In
        } else {
            LayerState::Out
        }
    }

    pub fn state_msg(&self) -> ServerMsg {
        ServerMsg::State {
            layer_id: self.id.clone(),
            state: self.wire_state(),
        }
    }
}

#[cfg(test)]
mod studio_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fields(pairs: &[(&str, &str)]) -> Fields {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn state_msg(state: LayerState) -> ServerMsg {
        ServerMsg::State {
            layer_id: TEST_LAYER_ID.into(),
            state,
        }
    }

    #[test]
    fn seed_has_test_screen_off_air_with_defaults() {
        let live = LiveState::with_test_screen();
        let view = live.screen_view(TEST_SCREEN_ID).unwrap();
        assert_eq!(view.screen.width, 1920);
        assert_eq!(view.screen.height, 1080);
        assert_eq!(view.screen.background, "transparent");
        assert_eq!(view.screen.name, "Output test");
        assert_eq!(view.layers.len(), 1);
        let l = &view.layers[0];
        assert_eq!(l.id, TEST_LAYER_ID);
        assert_eq!(l.name, "Lower third");
        assert!(!l.on_air);
        assert_eq!(l.fields, fields(&[("name", ""), ("title", "")]));
        assert_eq!(l.field_defs.len(), 2);
        assert_eq!(l.field_defs[0].label, "Name");
        assert!(LiveState::default().screens.is_empty());
    }

    #[test]
    fn project_seed_adds_the_reserved_preview_screen() {
        let live = LiveState::for_project();
        let ids: Vec<&str> = live.screens.iter().map(|s| s.info.id.as_str()).collect();
        assert_eq!(ids, [TEST_SCREEN_ID, PREVIEW_SCREEN_ID]);
        let p = live.screen_view(PREVIEW_SCREEN_ID).unwrap();
        assert_eq!((p.screen.width, p.screen.height), (1920, 1080));
        assert_eq!(p.screen.background, "transparent");
        assert_eq!(p.layers.len(), 1);
        assert_eq!(p.layers[0].id, PREVIEW_LAYER_ID);
        assert!(!p.layers[0].on_air);
        assert!(ids.iter().all(|id| is_reserved_screen(id)));
        assert!(!is_reserved_screen("main"));
    }

    #[test]
    fn on_air_changes_bump_the_generation() {
        let mut live = LiveState::with_test_screen();
        let gen = |l: &mut LiveState| l.layer_mut(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap().out_gen;
        let g0 = gen(&mut live);
        live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let g1 = gen(&mut live);
        live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(gen(&mut live), g1, "idempotent show keeps it");
        live.clear(TEST_SCREEN_ID).unwrap();
        let g2 = gen(&mut live);
        assert!(g0 != g1 && g1 != g2 && g0 != g2);
    }

    #[test]
    fn screen_views_lists_every_screen_in_order() {
        assert!(LiveState::default().screen_views().is_empty());
        let mut live = LiveState::with_test_screen();
        let mut second = live.screens[0].clone();
        second.info.id = "second".into();
        live.screens.push(second);
        live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let views = live.screen_views();
        let ids: Vec<&str> = views.iter().map(|v| v.screen.id.as_str()).collect();
        assert_eq!(ids, [TEST_SCREEN_ID, "second"]);
        assert!(views[0].layers[0].on_air);
        assert!(!views[1].layers[0].on_air);
        assert_eq!(views[0], live.screen_view(TEST_SCREEN_ID).unwrap());
    }

    #[test]
    fn preset_template_has_root_and_placeholders() {
        let t = test_template();
        assert!(t.html.trim_start().starts_with('<'));
        assert!(t.html.contains("{{name}}") && t.html.contains("{{title}}"));
        assert!(t.css.contains("[data-state=\"in\"]") && t.css.contains("[data-state=\"out\"]"));
        assert!(!t.css.contains("url("), "no external resources");
    }

    #[test]
    fn auto_out_goes_on_the_wire_and_a_change_resends_the_layer_at_the_same_rev() {
        let mut live = LiveState::with_test_screen();
        let layer = live.layer_mut(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(layer.to_wire().auto_out_ms, None);
        let rev = layer.template_rev;
        let mut c = layer.content();
        c.auto_out_ms = Some(5000);
        let msgs = apply_content(layer, c.clone(), None);
        match msgs.as_slice() {
            [ServerMsg::Layer { layer: w }] => {
                assert_eq!(w.auto_out_ms, Some(5000));
                assert_eq!(w.template_rev, rev, "no recompile");
                assert_eq!(w.swap, None);
            }
            other => panic!("expected one layer message, got {other:?}"),
        }
        assert!(
            apply_content(layer, c.clone(), None).is_empty(),
            "unchanged"
        );
        c.auto_out_ms = None;
        match apply_content(layer, c, None).as_slice() {
            [ServerMsg::Layer { layer: w }] => assert_eq!(w.auto_out_ms, None),
            other => panic!("expected one layer message, got {other:?}"),
        }
    }

    #[test]
    fn show_and_hide_are_idempotent() {
        let mut live = LiveState::with_test_screen();
        assert_eq!(
            live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap(),
            [state_msg(LayerState::In)]
        );
        assert!(live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap().is_empty());
        assert!(
            live.layer_view(TEST_SCREEN_ID, TEST_LAYER_ID)
                .unwrap()
                .on_air
        );
        assert_eq!(
            live.hide(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap(),
            [state_msg(LayerState::Out)]
        );
        assert!(live.hide(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap().is_empty());
        assert!(
            !live
                .layer_view(TEST_SCREEN_ID, TEST_LAYER_ID)
                .unwrap()
                .on_air
        );
    }

    #[test]
    fn unknown_screen_and_layer() {
        let mut live = LiveState::with_test_screen();
        assert_eq!(
            live.show("nope", TEST_LAYER_ID),
            Err(LiveError::UnknownScreen("nope".into()))
        );
        assert_eq!(
            live.hide(TEST_SCREEN_ID, "nope"),
            Err(LiveError::UnknownLayer("nope".into()))
        );
        assert_eq!(
            live.clear("nope"),
            Err(LiveError::UnknownScreen("nope".into()))
        );
        assert!(live.snapshot("nope").is_none());
        assert!(live.screen_view("nope").is_err());
        assert!(live.layer_view(TEST_SCREEN_ID, "nope").is_err());
    }

    #[test]
    fn set_fields_emits_only_changed_keys() {
        let mut live = LiveState::with_test_screen();
        let msgs = live
            .set_fields(
                TEST_SCREEN_ID,
                TEST_LAYER_ID,
                fields(&[
                    ("name", "Grace Hopper"),
                    // Same as the (empty) default: unchanged, so not sent.
                    ("title", ""),
                ]),
            )
            .unwrap();
        assert_eq!(
            msgs,
            [ServerMsg::Fields {
                layer_id: TEST_LAYER_ID.into(),
                fields: fields(&[("name", "Grace Hopper")]),
            }]
        );
        // Same values again: nothing changes, nothing is sent.
        let again = live
            .set_fields(
                TEST_SCREEN_ID,
                TEST_LAYER_ID,
                fields(&[("name", "Grace Hopper")]),
            )
            .unwrap();
        assert!(again.is_empty());
        assert!(live
            .set_fields(TEST_SCREEN_ID, TEST_LAYER_ID, Fields::new())
            .unwrap()
            .is_empty());
        let view = live.layer_view(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(view.fields["name"], "Grace Hopper");
        assert_eq!(view.fields["title"], "");
    }

    #[test]
    fn set_fields_rejects_unknown_key_atomically() {
        let mut live = LiveState::with_test_screen();
        let err = live
            .set_fields(
                TEST_SCREEN_ID,
                TEST_LAYER_ID,
                fields(&[("name", "Changed"), ("zzz", "x")]),
            )
            .unwrap_err();
        assert_eq!(err, LiveError::UnknownField("zzz".into()));
        let view = live.layer_view(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(view.fields["name"], "", "nothing changed");
        assert_eq!(view.fields.len(), 2);
    }

    #[test]
    fn clear_takes_on_air_layers_out() {
        let mut live = LiveState::with_test_screen();
        assert!(live.clear(TEST_SCREEN_ID).unwrap().is_empty());
        live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        assert_eq!(
            live.clear(TEST_SCREEN_ID).unwrap(),
            [state_msg(LayerState::Out)]
        );
        assert!(live.clear(TEST_SCREEN_ID).unwrap().is_empty());
        assert!(
            !live
                .layer_view(TEST_SCREEN_ID, TEST_LAYER_ID)
                .unwrap()
                .on_air
        );
    }

    #[test]
    fn snapshot_wire_shape() {
        let mut live = LiveState::with_test_screen();
        live.show(TEST_SCREEN_ID, TEST_LAYER_ID).unwrap();
        let msg = live.snapshot(TEST_SCREEN_ID).unwrap();
        let v: Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["t"], "snapshot");
        assert_eq!(
            v["screen"],
            json!({
                "id": "test",
                "name": "Output test",
                "width": 1920,
                "height": 1080,
                "background": "transparent"
            })
        );
        let layers = v["layers"].as_array().unwrap();
        assert_eq!(layers.len(), 1);
        let l = layers[0].as_object().unwrap();
        let mut keys: Vec<&str> = l.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "css",
                "fieldTypes",
                "fields",
                "html",
                "id",
                "state",
                "templateRev",
                "vars",
                "z"
            ]
        );
        assert_eq!(l["id"], "lower-third");
        assert_eq!(l["z"], 0);
        assert_eq!(l["templateRev"], 1);
        assert_eq!(l["state"], "in");
        assert_eq!(l["fields"], json!({"name": "", "title": ""}));
        assert_eq!(l["fieldTypes"], json!({"name": "text", "title": "text"}));
        assert!(l["html"].as_str().unwrap().contains("{{name}}"));
    }

    #[test]
    fn views_serialize_camel_case() {
        let live = LiveState::with_test_screen();
        let v = serde_json::to_value(live.screen_view(TEST_SCREEN_ID).unwrap()).unwrap();
        let layer = &v["layers"][0];
        assert_eq!(layer["onAir"], false);
        assert_eq!(
            layer["fieldDefs"][0],
            json!({"key": "name", "label": "Name", "type": "text", "default": ""})
        );
    }
}
