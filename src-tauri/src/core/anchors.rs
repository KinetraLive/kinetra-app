//! Layer position anchors (`ScreenLayer.anchor`): pure placement math and the save rules.
//!
//! - Placement ([`place`]): the margin is 5 % of the screen side (`round(W / 20)`) when `safe`
//!   (title safe), else 0. `left` → x = margin; `center` → x = round((W − w) / 2) (halves up);
//!   `right` → x = W − margin − w; `top` / `middle` / `bottom` likewise with H. A free axis keeps
//!   its coordinate. The result always overlaps the screen (w, h ≥ 1), so nothing is clamped.
//!   The layout editor mirrors this in `src/lib/components/screens/editor/anchor.ts`.
//! - Saves ([`release_moved`] then [`place_all`], from `screens::save_reporting`, `screens::get`
//!   and the watcher): a layer whose anchor is unchanged and whose rect moved with its size
//!   unchanged releases the moved axis (both released = no anchor); otherwise the anchored
//!   coordinates are recomputed (a new anchor wins over the rect's position; a resize keeps the
//!   anchor; a screen size change re-places every anchored layer).
//! - Drags (`set_layer_rect`, memory only): the same rule per layer ([`moved`]).

use crate::core::model::{AnchorH, AnchorV, LayerAnchor, ScreenMeta};
use crate::core::protocol::Rect;

/// The anchor margin on a side of `size` px: 5 % (rounded) inside title safe, else 0.
pub fn margin(size: u32, safe: bool) -> i64 {
    if safe {
        (i64::from(size) + 10) / 20
    } else {
        0
    }
}

/// Where a box of `len` px starts on a side of `size` px for an anchor at the start / centre /
/// end (`0` / `1` / `2`).
fn coord(at: u8, len: u32, size: u32, safe: bool) -> i32 {
    let m = margin(size, safe);
    let (len, size) = (i64::from(len), i64::from(size));
    let v = match at {
        0 => m,
        1 => (size - len + 1).div_euclid(2),
        _ => size - m - len,
    };
    // Sides are at most 4 × 8192 and screens 8192: always in range.
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// `rect` placed by `anchor` on a `width` × `height` screen (free axes unchanged).
pub fn place(rect: Rect, anchor: &LayerAnchor, width: u32, height: u32) -> Rect {
    let mut r = rect;
    if let Some(h) = anchor.h {
        let at = match h {
            AnchorH::Left => 0,
            AnchorH::Center => 1,
            AnchorH::Right => 2,
        };
        r.x = coord(at, r.w, width, anchor.safe);
    }
    if let Some(v) = anchor.v {
        let at = match v {
            AnchorV::Top => 0,
            AnchorV::Middle => 1,
            AnchorV::Bottom => 2,
        };
        r.y = coord(at, r.h, height, anchor.safe);
    }
    r
}

/// Both axes free and `safe` off is no anchor.
pub fn normalize(anchor: Option<LayerAnchor>) -> Option<LayerAnchor> {
    anchor.filter(|a| a.h.is_some() || a.v.is_some() || a.safe)
}

/// A layer's box moved from `prev` to `rect` with its anchor unchanged: a move with the size
/// unchanged releases the moved axes (both released = no anchor); the rest stays anchored and is
/// placed. Returns the box and the anchor.
pub fn moved(
    prev: Rect,
    anchor: Option<LayerAnchor>,
    rect: Rect,
    width: u32,
    height: u32,
) -> (Rect, Option<LayerAnchor>) {
    let Some(a) = released(prev, normalize(anchor), rect) else {
        return (rect, None);
    };
    (place(rect, &a, width, height), Some(a))
}

/// The anchor after a move from `prev` to `rect` (unchanged when the size changed).
fn released(prev: Rect, anchor: Option<LayerAnchor>, rect: Rect) -> Option<LayerAnchor> {
    let mut a = anchor?;
    if prev.w != rect.w || prev.h != rect.h {
        return Some(a);
    }
    let had = a.h.is_some() || a.v.is_some();
    if rect.x != prev.x {
        a.h = None;
    }
    if rect.y != prev.y {
        a.v = None;
    }
    if had && a.h.is_none() && a.v.is_none() {
        return None;
    }
    Some(a)
}

/// Normalizes every layer's anchor and places the anchored ones.
pub fn place_all(meta: &mut ScreenMeta) {
    let (w, h) = (meta.width, meta.height);
    for l in &mut meta.layers {
        l.anchor = normalize(l.anchor);
        if let Some(a) = &l.anchor {
            l.rect = place(l.rect, a, w, h);
        }
    }
}

/// The save rule against the previous saved version `prev` (already placed): each layer whose
/// anchor is unchanged and whose box moved with its size unchanged releases the moved axes.
/// Nothing is released when the screen size changed (every anchored layer is re-placed).
/// Returns whether an anchor was released. [`place_all`] follows.
pub fn release_moved(prev: &ScreenMeta, meta: &mut ScreenMeta) -> bool {
    if prev.width != meta.width || prev.height != meta.height {
        return false;
    }
    let mut changed = false;
    for l in &mut meta.layers {
        let anchor = normalize(l.anchor);
        if anchor.is_none() {
            continue;
        }
        let Some(p) = prev.layers.iter().find(|p| p.id == l.id) else {
            continue;
        };
        if normalize(p.anchor) != anchor {
            continue; // the anchor itself changed: it wins
        }
        let next = released(p.rect, anchor, l.rect);
        if next != anchor {
            l.anchor = next;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{ScreenLayer, ScreenLayerKind};

    const W: u32 = 1920;
    const H: u32 = 1080;

    fn r(x: i32, y: i32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    fn a(h: Option<AnchorH>, v: Option<AnchorV>, safe: bool) -> LayerAnchor {
        LayerAnchor { h, v, safe }
    }

    #[test]
    fn nine_points_with_and_without_safe() {
        let b = r(5, 7, 400, 100);
        let hs = [
            (AnchorH::Left, 0, 96),
            (AnchorH::Center, 760, 760),
            (AnchorH::Right, 1520, 1424),
        ];
        let vs = [
            (AnchorV::Top, 0, 54),
            (AnchorV::Middle, 490, 490),
            (AnchorV::Bottom, 980, 926),
        ];
        for (h, x0, xs) in hs {
            for (v, y0, ys) in vs {
                assert_eq!(
                    place(b, &a(Some(h), Some(v), false), W, H),
                    r(x0, y0, 400, 100)
                );
                assert_eq!(
                    place(b, &a(Some(h), Some(v), true), W, H),
                    r(xs, ys, 400, 100)
                );
            }
        }
    }

    #[test]
    fn single_axes_keep_the_other_coordinate() {
        let b = r(5, 7, 400, 100);
        assert_eq!(
            place(b, &a(Some(AnchorH::Center), None, false), W, H),
            r(760, 7, 400, 100)
        );
        assert_eq!(
            place(b, &a(None, Some(AnchorV::Bottom), true), W, H),
            r(5, 926, 400, 100)
        );
        assert_eq!(place(b, &a(None, None, true), W, H), b);
    }

    #[test]
    fn centre_rounds_halves_up_and_large_boxes_follow_the_formula() {
        // (1920 - 401) / 2 = 759.5 -> 760; a box wider than the screen: (1920 - 2001) / 2 = -40.5 -> -40.
        let c = a(Some(AnchorH::Center), Some(AnchorV::Middle), false);
        assert_eq!(place(r(0, 0, 401, 1), &c, W, H).x, 760);
        assert_eq!(place(r(0, 0, 2001, 1), &c, W, H).x, -40);
        let br = a(Some(AnchorH::Right), Some(AnchorV::Bottom), true);
        let p = place(r(0, 0, 3000, 2000), &br, W, H);
        assert_eq!(p, r(1824 - 3000, 1026 - 2000, 3000, 2000));
        // Still overlaps the screen.
        assert!(i64::from(p.x) + 3000 > 0 && p.x < W as i32);
    }

    #[test]
    fn normalize_drops_empty_anchors() {
        assert_eq!(normalize(Some(a(None, None, false))), None);
        assert!(normalize(Some(a(None, None, true))).is_some());
        assert!(normalize(Some(a(Some(AnchorH::Left), None, false))).is_some());
    }

    #[test]
    fn a_move_releases_the_moved_axis_only() {
        let c = Some(a(Some(AnchorH::Center), Some(AnchorV::Middle), true));
        let prev = r(760, 490, 400, 100);
        // Horizontal drag: h released, v kept.
        let (rect, an) = moved(prev, c, r(800, 490, 400, 100), W, H);
        assert_eq!(rect, r(800, 490, 400, 100));
        assert_eq!(an, Some(a(None, Some(AnchorV::Middle), true)));
        // Vertical drag.
        let (_, an) = moved(prev, c, r(760, 10, 400, 100), W, H);
        assert_eq!(an, Some(a(Some(AnchorH::Center), None, true)));
        // Both: the anchor goes (safe too).
        assert_eq!(
            moved(prev, c, r(1, 2, 400, 100), W, H),
            (r(1, 2, 400, 100), None)
        );
        // No anchor: the rect as is.
        assert_eq!(
            moved(prev, None, r(1, 2, 3, 4), W, H),
            (r(1, 2, 3, 4), None)
        );
    }

    #[test]
    fn a_resize_keeps_the_anchor() {
        let c = Some(a(Some(AnchorH::Center), Some(AnchorV::Bottom), false));
        let (rect, an) = moved(r(760, 980, 400, 100), c, r(760, 980, 600, 100), W, H);
        assert_eq!(an, c);
        assert_eq!(rect, r(660, 980, 600, 100));
        // Resized from the top-left handle (position and size change): still anchored.
        let (rect, _) = moved(r(760, 980, 400, 100), c, r(700, 900, 460, 180), W, H);
        assert_eq!(rect, r(730, 900, 460, 180));
    }

    fn layer(id: &str, rect: Rect, anchor: Option<LayerAnchor>) -> ScreenLayer {
        ScreenLayer {
            id: id.into(),
            name: id.into(),
            rect,
            kind: ScreenLayerKind::Template {
                template_id: "t".into(),
                fields: Default::default(),
                vars: Default::default(),
                bindings: Default::default(),
                variant: None,
                sizing: None,
                opacity: None,
            },
            anchor,
        }
    }

    fn meta(w: u32, h: u32, layers: Vec<ScreenLayer>) -> ScreenMeta {
        ScreenMeta {
            name: "S".into(),
            width: w,
            height: h,
            background: "transparent".into(),
            layers,
            guides: Default::default(),
            safe_areas: Vec::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn save_rules() {
        let c = Some(a(Some(AnchorH::Center), Some(AnchorV::Middle), false));
        let prev = meta(W, H, vec![layer("a", r(760, 490, 400, 100), c)]);
        // Moved horizontally, anchor unchanged: h released.
        let mut m = meta(W, H, vec![layer("a", r(10, 490, 400, 100), c)]);
        assert!(release_moved(&prev, &mut m));
        place_all(&mut m);
        assert_eq!(m.layers[0].rect, r(10, 490, 400, 100));
        assert_eq!(
            m.layers[0].anchor,
            Some(a(None, Some(AnchorV::Middle), false))
        );
        // The anchor changed in this save: it wins over the rect.
        let br = Some(a(Some(AnchorH::Right), Some(AnchorV::Bottom), false));
        let mut m = meta(W, H, vec![layer("a", r(10, 20, 400, 100), br)]);
        assert!(!release_moved(&prev, &mut m));
        place_all(&mut m);
        assert_eq!(m.layers[0].rect, r(1520, 980, 400, 100));
        // A screen size change re-places (no release even though x/y differ).
        let mut m = meta(1280, 720, vec![layer("a", r(760, 490, 400, 100), c)]);
        assert!(!release_moved(&prev, &mut m));
        place_all(&mut m);
        assert_eq!(m.layers[0].rect, r(440, 310, 400, 100));
        // New layer and empty anchor.
        let mut m = meta(
            W,
            H,
            vec![
                layer("a", r(760, 490, 400, 100), c),
                layer("b", r(1, 1, 10, 10), Some(a(None, None, false))),
            ],
        );
        assert!(!release_moved(&prev, &mut m));
        place_all(&mut m);
        assert_eq!(m.layers[1].anchor, None);
        assert_eq!(m.layers[1].rect, r(1, 1, 10, 10));
    }

    #[test]
    fn serde_omits_absent_and_reads_nulls() {
        let l = layer("a", r(0, 0, 1, 1), None);
        let j = serde_json::to_string(&l).unwrap();
        assert!(!j.contains("anchor"), "{j}");
        let l = layer(
            "a",
            r(0, 0, 1, 1),
            Some(a(Some(AnchorH::Center), None, true)),
        );
        let j = serde_json::to_string(&l).unwrap();
        assert!(
            j.contains(r#""anchor":{"h":"center","v":null,"safe":true}"#),
            "{j}"
        );
        let back: ScreenLayer = serde_json::from_str(&j).unwrap();
        assert_eq!(back, l);
        let x: LayerAnchor = serde_json::from_str(r#"{"v":"bottom"}"#).unwrap();
        assert_eq!(x, a(None, Some(AnchorV::Bottom), false));
        assert!(serde_json::from_str::<LayerAnchor>(r#"{"h":"middle"}"#).is_err());
    }
}
