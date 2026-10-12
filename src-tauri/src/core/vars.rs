//! Template variables: turning a value as the user sees it (colour hex, font name, number, option)
//! into the CSS value that becomes `--<key>` on the layer host.
//!
//! Every value is validated against a strict grammar and then re-emitted from parsed parts, so raw
//! user text never reaches CSS. That matters because API sources and flows set values too.
//!
//! - colour: `#rgb`, `#rrggbb`, `#rrggbbaa` or `transparent` (any case, emitted lowercase; no other
//!   names or functions);
//! - font: exactly one of `TEMPLATE_FONTS` by name (or a retired name in [`FONT_ALIASES`]),
//!   emitted as its full stack;
//! - range: a finite number within `min..=max`, re-formatted (no exponent) plus the unit, which is
//!   itself restricted to `[a-z%]{0,8}`;
//! - select: one of the variable's option values, each of which matches `[a-zA-Z0-9_-]{1,64}`.

use crate::core::model::{VariableDef, VariableKind, TEMPLATE_FONTS};

/// Longest select option value.
pub const OPTION_VALUE_MAX: usize = 64;
/// Longest range unit (`px`, `%`, `em`, `deg`, `ms`, ...).
pub const UNIT_MAX: usize = 8;

/// `#rgb`, `#rrggbb` or `#rrggbbaa`.
pub fn is_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The CSS keyword for no fill, accepted (ignoring case) wherever a colour is.
pub const TRANSPARENT: &str = "transparent";

/// A colour value: [`is_hex_color`] or `transparent` (ignoring case).
pub fn is_color(s: &str) -> bool {
    is_hex_color(s) || s.eq_ignore_ascii_case(TRANSPARENT)
}

/// Retired font names that projects may still hold, read as their replacement: (old, curated).
/// Inter is not allowed in Kinetra (no longer bundled); old values render in IBM Plex Sans and
/// are rewritten on open and on save ([`normalize_font`]).
pub const FONT_ALIASES: &[(&str, &str)] = &[("Inter", "IBM Plex Sans")];

/// The curated font name for `name`: itself when curated, its replacement when retired
/// ([`FONT_ALIASES`]), `None` otherwise.
pub fn canonical_font(name: &str) -> Option<&'static str> {
    let name = FONT_ALIASES
        .iter()
        .find(|(old, _)| *old == name)
        .map_or(name, |(_, new)| *new);
    TEMPLATE_FONTS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(n, _, _)| *n)
}

/// Replaces a retired font name in place with its curated replacement. True when it changed.
pub fn normalize_font(value: &mut String) -> bool {
    match FONT_ALIASES.iter().find(|(old, _)| *old == value.as_str()) {
        Some((_, new)) => {
            *value = (*new).to_string();
            true
        }
        None => false,
    }
}

/// The CSS font stack of a curated font, by its display name (retired names included).
pub fn font_stack(name: &str) -> Option<&'static str> {
    let name = canonical_font(name)?;
    TEMPLATE_FONTS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, _, stack)| *stack)
}

/// `[a-zA-Z0-9_-]{1,64}`: safe as a bare CSS identifier-ish token.
pub fn is_option_value(s: &str) -> bool {
    (1..=OPTION_VALUE_MAX).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `[a-z%]{0,8}`.
pub fn is_unit(s: &str) -> bool {
    s.len() <= UNIT_MAX && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'%')
}

/// A finite number as plain decimal text: at most 6 decimals, trailing zeros trimmed, never an
/// exponent, never `-0`.
pub fn format_number(v: f64) -> String {
    let mut s = format!("{v:.6}");
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.').len();
        s.truncate(trimmed);
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}

/// Parses a range value: a finite decimal number (surrounding whitespace allowed).
pub fn parse_number(s: &str) -> Option<f64> {
    let s = s.trim();
    // Only plain decimals: digits, one dot, an optional leading minus. Rejects `inf`, `NaN`,
    // exponents and anything else `f64::from_str` would accept.
    let body = s.strip_prefix('-').unwrap_or(s);
    let ok = !body.is_empty()
        && body.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        && body.bytes().filter(|&b| b == b'.').count() <= 1
        && body.bytes().any(|b| b.is_ascii_digit());
    if !ok {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The CSS value for `value` under `def`, or a user-facing reason it is rejected.
pub fn css_value(def: &VariableDef, value: &str) -> Result<String, String> {
    match &def.kind {
        VariableKind::Color => {
            if is_color(value) {
                Ok(value.to_ascii_lowercase())
            } else {
                Err(format!(
                    "\"{}\" is not a color; use #rgb, #rrggbb, #rrggbbaa or transparent",
                    clip(value)
                ))
            }
        }
        VariableKind::Font => font_stack(value)
            .map(str::to_string)
            .ok_or_else(|| format!("\"{}\" is not one of the available fonts", clip(value))),
        VariableKind::Range { min, max, unit, .. } => {
            if !is_unit(unit) {
                return Err(format!("unit \"{}\" is not allowed", clip(unit)));
            }
            if !(min.is_finite() && max.is_finite() && min < max) {
                return Err("the range bounds are not valid".into());
            }
            // Only the bounds are enforced (not step multiples), with a tolerance for float error
            // from fractional steps; the emitted value is clamped into the bounds.
            let eps = 1e-9 * (max - min).abs().max(1.0);
            match parse_number(value) {
                Some(v) if v >= min - eps && v <= max + eps => {
                    Ok(format!("{}{unit}", format_number(v.clamp(*min, *max))))
                }
                Some(_) => Err(format!(
                    "{} is outside {} to {}",
                    clip(value.trim()),
                    format_number(*min),
                    format_number(*max)
                )),
                None => Err(format!("\"{}\" is not a number", clip(value))),
            }
        }
        VariableKind::Select { options } => {
            if is_option_value(value) && options.iter().any(|o| o.value == value) {
                Ok(value.to_string())
            } else {
                Err(format!("\"{}\" is not one of the options", clip(value)))
            }
        }
    }
}

/// Shortens a rejected value for an error message.
fn clip(s: &str) -> String {
    const MAX: usize = 40;
    if s.chars().count() <= MAX {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(MAX).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::SelectOption;

    fn def(kind: VariableKind) -> VariableDef {
        VariableDef {
            key: "v".into(),
            label: "V".into(),
            kind,
            default: String::new(),
        }
    }

    fn range() -> VariableDef {
        def(VariableKind::Range {
            min: -10.0,
            max: 24.0,
            step: 0.5,
            unit: "px".into(),
        })
    }

    fn select() -> VariableDef {
        def(VariableKind::Select {
            options: vec![
                SelectOption {
                    value: "left".into(),
                    label: "Left".into(),
                },
                SelectOption {
                    value: "right".into(),
                    label: "Right".into(),
                },
            ],
        })
    }

    const INJECTION: &str = "red; } body { display:none";

    #[test]
    fn curated_fonts_match_the_ui_and_the_renderer() {
        // The mirrors: TEMPLATE_FONTS in types.ts (name, slug, stack) and FONTS in the renderer
        // (name, slug), so a font the server accepts is one the UI offers and the renderer links.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let ts = std::fs::read_to_string(root.join("src/lib/types.ts")).unwrap();
        let renderer = std::fs::read_to_string(root.join("renderer/src/fonts.ts")).unwrap();
        // The renderer keeps the names as one comma-separated string and derives each slug (lower
        // case, spaces as hyphens), to stay small.
        let list = renderer
            .split_once("FONTS")
            .and_then(|(_, rest)| rest.split_once("\""))
            .and_then(|(_, rest)| rest.split_once("\""))
            .map(|(names, _)| names)
            .expect("renderer fonts.ts has the FONTS name string");
        let renderer_names: Vec<&str> = list.split(',').collect();
        for (name, slug, stack) in TEMPLATE_FONTS {
            let ts_entry = format!("{{ name: \"{name}\", slug: \"{slug}\", stack: '{stack}' }}");
            assert!(ts.contains(&ts_entry), "types.ts lacks {ts_entry}");
            assert!(
                renderer_names.contains(name),
                "renderer fonts.ts lacks {name}"
            );
            assert_eq!(
                *slug,
                name.to_lowercase().replace(' ', "-"),
                "the renderer derives a different slug for {name}"
            );
            assert_eq!(font_stack(name), Some(*stack));
        }
        assert_eq!(
            ts.matches("{ name: \"").count(),
            TEMPLATE_FONTS.len(),
            "types.ts has fonts the server does not know"
        );
        assert_eq!(
            renderer_names.len(),
            TEMPLATE_FONTS.len(),
            "the renderer has fonts the server does not know"
        );
        assert_eq!(TEMPLATE_FONTS.len(), 17);
        // Retired names are not curated, and their replacement is.
        for (old, new) in FONT_ALIASES {
            assert!(!TEMPLATE_FONTS.iter().any(|(n, _, _)| n == old), "{old}");
            assert!(TEMPLATE_FONTS.iter().any(|(n, _, _)| n == new), "{new}");
        }
    }

    #[test]
    fn colours() {
        let c = def(VariableKind::Color);
        assert_eq!(css_value(&c, "#22B8CF").unwrap(), "#22b8cf");
        assert_eq!(css_value(&c, "#fff").unwrap(), "#fff");
        assert_eq!(css_value(&c, "#11223344").unwrap(), "#11223344");
        assert_eq!(css_value(&c, "transparent").unwrap(), "transparent");
        assert_eq!(css_value(&c, "Transparent").unwrap(), "transparent");
        assert!(is_color("TRANSPARENT") && !is_hex_color("transparent"));
        for bad in [
            "red",
            "#ff",
            "#ffff",
            "#fffff",
            "#ggg",
            "22b8cf",
            " #fff",
            "#fff;",
            "rgb(0,0,0)",
            " transparent",
            "transparent;",
            "transparentx",
            "currentcolor",
            "",
            INJECTION,
        ] {
            assert!(css_value(&c, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn fonts_expand_to_stacks() {
        let f = def(VariableKind::Font);
        assert_eq!(
            css_value(&f, "IBM Plex Sans").unwrap(),
            "\"IBM Plex Sans\", system-ui, sans-serif"
        );
        assert!(css_value(&f, "Barlow Condensed")
            .unwrap()
            .starts_with("\"Barlow Condensed\""));
        for bad in ["barlow", "Comic Sans", "", "Barlow;", "Plex;", INJECTION] {
            assert!(css_value(&f, bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn retired_fonts_read_as_their_replacement() {
        let f = def(VariableKind::Font);
        let (old, new) = FONT_ALIASES[0];
        assert_eq!(css_value(&f, old).unwrap(), css_value(&f, new).unwrap());
        assert_eq!(canonical_font(old), Some(new));
        assert_eq!(canonical_font(new), Some(new));
        assert_eq!(canonical_font("Comic Sans"), None);
        // Exact names only (as for curated fonts).
        assert!(css_value(&f, &old.to_lowercase()).is_err());
        assert!(css_value(&f, &format!("{old};")).is_err());
        let mut v = old.to_string();
        assert!(normalize_font(&mut v));
        assert_eq!(v, new);
        assert!(!normalize_font(&mut v));
        let mut other = "Barlow".to_string();
        assert!(!normalize_font(&mut other));
        assert_eq!(other, "Barlow");
    }

    #[test]
    fn ranges_reformat_numbers() {
        let r = range();
        assert_eq!(css_value(&r, "4").unwrap(), "4px");
        assert_eq!(css_value(&r, " 4.50 ").unwrap(), "4.5px");
        assert_eq!(css_value(&r, "-10").unwrap(), "-10px");
        assert_eq!(css_value(&r, "24").unwrap(), "24px");
        assert_eq!(css_value(&r, "-0").unwrap(), "0px");
        assert_eq!(css_value(&r, "0.1234567").unwrap(), "0.123457px");
        for bad in [
            "24.01", "-11", "1e1", "inf", "NaN", "", "-", ".", "4px", "1.2.3", "+4", INJECTION,
        ] {
            assert!(css_value(&r, bad).is_err(), "{bad}");
        }
        // A unit that isn't plain letters/% is refused even with a valid number.
        let mut evil = range();
        if let VariableKind::Range { unit, .. } = &mut evil.kind {
            *unit = "px;}".into();
        }
        assert!(css_value(&evil, "4").is_err());
        let mut unitless = range();
        if let VariableKind::Range { unit, .. } = &mut unitless.kind {
            unit.clear();
        }
        assert_eq!(css_value(&unitless, "1.5").unwrap(), "1.5");
    }

    #[test]
    fn fractional_steps_and_unitless_numbers() {
        let r = def(VariableKind::Range {
            min: 0.0,
            max: 1.0,
            step: 0.05,
            unit: String::new(),
        });
        assert_eq!(css_value(&r, "0.4").unwrap(), "0.4");
        assert_eq!(css_value(&r, "1").unwrap(), "1");
        assert_eq!(css_value(&r, "1.0").unwrap(), "1");
        // Accumulated float error from stepping (0.05 * 3) and values that aren't step multiples.
        assert_eq!(css_value(&r, "0.15000000000000002").unwrap(), "0.15");
        assert_eq!(css_value(&r, "0.123").unwrap(), "0.123");
        // Within float tolerance of a bound: accepted and clamped.
        assert_eq!(css_value(&r, "1.0000000000001").unwrap(), "1");
        assert!(css_value(&r, "1.001").is_err());
    }

    #[test]
    fn number_formatting_has_no_exponent() {
        assert_eq!(format_number(1e21), "1000000000000000000000");
        assert_eq!(format_number(1e-7), "0");
        assert_eq!(format_number(0.000001), "0.000001");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(100.0), "100");
    }

    #[test]
    fn selects_accept_only_options() {
        let s = select();
        assert_eq!(css_value(&s, "left").unwrap(), "left");
        for bad in ["Left", "center", "", "left;", INJECTION] {
            assert!(css_value(&s, bad).is_err(), "{bad}");
        }
        // Even an option whose value is unsafe is never emitted.
        let evil = def(VariableKind::Select {
            options: vec![SelectOption {
                value: INJECTION.into(),
                label: "x".into(),
            }],
        });
        assert!(css_value(&evil, INJECTION).is_err());
    }

    #[test]
    fn injection_is_rejected_for_every_kind() {
        for d in [
            def(VariableKind::Color),
            def(VariableKind::Font),
            range(),
            select(),
        ] {
            let err = css_value(&d, INJECTION).unwrap_err();
            assert!(!err.is_empty());
        }
    }
}
