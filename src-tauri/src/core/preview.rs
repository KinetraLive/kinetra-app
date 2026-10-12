//! Template operations on a live layer: loading a template (the editor's preview, the fixture's
//! `template` command) and applying unsaved editor drafts. Pure: they mutate a `LayerLive` and
//! return the protocol messages to broadcast; `LiveHub::update_layer` runs them under its lock.
//!
//! Draft diffing:
//! - HTML, CSS or field types (key → type) changed: `templateRev` + 1 and one `layer` message
//!   (which carries every field value and variable too, so nothing else is sent);
//! - otherwise a changed auto-out: one `layer` message with the same `templateRev` (the renderer
//!   updates `--auto-out-ms` and the rest in place);
//! - otherwise variables whose CSS value changed go out as one `vars` message (a removed variable
//!   is sent as `""`, which clears the custom property) and changed field values as one `fields`
//!   message.
//!
//! Everything is validated before anything changes: an invalid draft leaves the layer untouched
//! and sends nothing.

use std::sync::Arc;

use crate::core::fieldvals;
use crate::core::live::{next_out_gen, FieldDef, LayerLive, TemplateDef};
use crate::core::model::{TemplateDraft, TemplateMeta, VariableDef, VariantDoc};
use crate::core::protocol::{FieldType, Fields, ServerMsg};
use crate::core::templates::{self, TemplateError, TemplateResult};
use crate::core::vars;

/// Preview value for each field (never invented content): its label; for a list three rows of
/// its column labels, for a timer its default state, for a clock its format
/// (`fieldvals::sample_value`).
pub fn label_values(fields: &[FieldDef]) -> Fields {
    fields
        .iter()
        .map(|f| (f.key.clone(), fieldvals::sample_value(f)))
        .collect()
}

/// The value a field keeps in a draft: the draft's (validated per type: an invalid list or timer
/// is an error), else the current one if still valid for the (possibly new) type, else the sample.
fn pick_field(
    def: &FieldDef,
    draft: Option<&String>,
    current: Option<&String>,
) -> TemplateResult<String> {
    if let Some(v) = draft {
        return fieldvals::own_value(def, v)
            .map(|(v, _)| v)
            .map_err(|msg| TemplateError::Field {
                key: def.key.clone(),
                msg,
            });
    }
    if let Some(v) = current {
        if let Ok((v, _)) = fieldvals::own_value(def, v) {
            return Ok(v);
        }
    }
    Ok(fieldvals::sample_value(def))
}

/// Debug builds only (the e2e sandbox test): puts raw HTML/CSS into a layer WITHOUT validation
/// or hardening, so the test can prove the renderer's CSP blocks what the hardening would have
/// refused. Doesn't exist in release builds.
#[cfg(debug_assertions)]
#[doc(hidden)]
pub fn load_unchecked_for_tests(layer: &mut LayerLive, html: &str, css: &str) -> Vec<ServerMsg> {
    // The layer keeps its field definitions and values (later tests set them).
    let fields = layer.template.fields.clone();
    layer.template = Arc::new(TemplateDef {
        html: html.to_string(),
        css: css.to_string(),
        fields,
        design: None,
    });
    layer.template_rev = layer.template_rev.wrapping_add(1);
    layer.on_air = false;
    layer.out_gen = next_out_gen();
    vec![ServerMsg::Layer {
        layer: layer.to_wire(),
    }]
}

/// Replaces the layer's template with one variant's look (validated first): off air,
/// `templateRev` + 1, field values = labels, variables at the variant's defaults (else the
/// definition's), the template's auto-out. Emits one `layer` message.
pub fn load_template(
    layer: &mut LayerLive,
    meta: &TemplateMeta,
    variant: &VariantDoc,
) -> TemplateResult<Vec<ServerMsg>> {
    templates::validate_content(meta, &variant.html, &variant.css)?;
    let values: Fields = meta
        .variables
        .iter()
        .map(|d| {
            let v = variant
                .vars
                .get(&d.key)
                .filter(|v| vars::css_value(d, v).is_ok())
                .unwrap_or(&d.default);
            (d.key.clone(), v.clone())
        })
        .collect();
    let css_vars = templates::resolve_vars(&meta.variables, &values)?;
    layer.template = Arc::new(TemplateDef {
        html: variant.html.clone(),
        css: variant.css.clone(),
        fields: meta.fields.clone(),
        design: meta
            .design
            .map(|d| crate::core::protocol::DesignSize { w: d.w, h: d.h }),
    });
    layer.template_rev = layer.template_rev.wrapping_add(1);
    layer.variant = variant.id.clone();
    layer.fields = label_values(&meta.fields);
    layer.variables = meta.variables.clone();
    layer.var_values = values;
    layer.vars = css_vars;
    layer.auto_out_ms = meta.auto_out_ms;
    layer.on_air = false;
    layer.out_gen = next_out_gen();
    Ok(vec![ServerMsg::Layer {
        layer: layer.to_wire(),
    }])
}

/// Sets variable values (as the user sees them) on a layer. Unknown keys or invalid values are
/// rejected before anything changes; emits `vars` with the changed keys only.
pub fn set_variable_values(
    layer: &mut LayerLive,
    values: Fields,
) -> TemplateResult<Vec<ServerMsg>> {
    apply_draft(
        layer,
        TemplateDraft {
            variable_values: Some(values),
            ..Default::default()
        },
    )
}

/// The value a variable keeps: the draft's (validated, errors are reported), else the current one
/// if still valid under the (possibly new) definition, else the default.
fn pick_value(
    def: &VariableDef,
    draft: Option<&String>,
    current: Option<&String>,
) -> TemplateResult<(String, String)> {
    if let Some(v) = draft {
        return vars::css_value(def, v)
            .map(|css| (v.clone(), css))
            .map_err(|msg| TemplateError::Variable {
                key: def.key.clone(),
                msg,
            });
    }
    if let Some(v) = current {
        if let Ok(css) = vars::css_value(def, v) {
            return Ok((v.clone(), css));
        }
    }
    vars::css_value(def, &def.default)
        .map(|css| (def.default.clone(), css))
        .map_err(|msg| TemplateError::Variable {
            key: def.key.clone(),
            msg,
        })
}

/// Applies an unsaved editor draft. See the module docs for what is emitted.
///
/// `autoOutMs`: a number in 500..=600000 sets it; `0` (or an explicit `null`, if the client's
/// JSON layer preserves it) turns it off; absent keeps it.
pub fn apply_draft(layer: &mut LayerLive, draft: TemplateDraft) -> TemplateResult<Vec<ServerMsg>> {
    // ---- Validate everything first --------------------------------------------------------
    if let Some(v) = &draft.variant {
        if !templates::is_variant_id(v) {
            return Err(TemplateError::VariantId(v.clone()));
        }
    }
    if let Some(h) = &draft.html {
        templates::validate_html(h)?;
    }
    if let Some(c) = &draft.css {
        templates::validate_css(c)?;
    }
    let fields: &[FieldDef] = draft.fields.as_deref().unwrap_or(&layer.template.fields);
    let variables: &[VariableDef] = draft.variables.as_deref().unwrap_or(&layer.variables);
    if draft.fields.is_some() {
        templates::validate_fields(fields)?;
    }
    if draft.variables.is_some() {
        templates::validate_variables(variables)?;
    }
    templates::validate_unique_keys(fields, variables)?;

    let auto_out = match draft.auto_out_ms {
        None => layer.auto_out_ms,
        Some(None) | Some(Some(0)) => None,
        Some(Some(ms)) => {
            templates::validate_auto_out(Some(ms))?;
            Some(ms)
        }
    };

    let draft_values = draft.variable_values.as_ref();
    if let Some(k) =
        draft_values.and_then(|v| v.keys().find(|k| !variables.iter().any(|d| &d.key == *k)))
    {
        return Err(TemplateError::UnknownVariable(k.clone()));
    }
    let mut var_values = Fields::new();
    let mut css_vars = Fields::new();
    for def in variables {
        let (value, css) = pick_value(
            def,
            draft_values.and_then(|v| v.get(&def.key)),
            layer.var_values.get(&def.key),
        )?;
        var_values.insert(def.key.clone(), value);
        css_vars.insert(def.key.clone(), css);
    }

    let draft_fields = draft.field_values.as_ref();
    if let Some(k) =
        draft_fields.and_then(|v| v.keys().find(|k| !fields.iter().any(|f| &f.key == *k)))
    {
        return Err(TemplateError::UnknownField(k.clone()));
    }
    let mut field_values = Fields::new();
    for f in fields {
        // A field that changed type does not keep a value of the old type.
        let old_kind = layer
            .template
            .fields
            .iter()
            .find(|o| o.key == f.key)
            .map(|o| o.kind);
        let structured =
            |k: FieldType| matches!(k, FieldType::List | FieldType::Timer | FieldType::Clock);
        let keep = old_kind.is_some_and(|o| o == f.kind || !(structured(o) || structured(f.kind)));
        let current = layer.fields.get(&f.key).filter(|_| keep);
        let v = pick_field(f, draft_fields.and_then(|d| d.get(&f.key)), current)?;
        field_values.insert(f.key.clone(), v);
    }

    // ---- Diff ----------------------------------------------------------------------------
    let old = &layer.template;
    let html_changed = draft.html.as_ref().is_some_and(|h| *h != old.html);
    let css_changed = draft.css.as_ref().is_some_and(|c| *c != old.css);
    let fields_changed = draft.fields.as_ref().is_some_and(|f| *f != old.fields);
    let variant_changed = draft.variant.as_ref().is_some_and(|v| *v != layer.variant);
    let recompile = variant_changed
        || html_changed
        || css_changed
        || (fields_changed
            && templates::field_types(fields) != templates::field_types(&old.fields));

    let mut vars_diff: Fields = css_vars
        .iter()
        .filter(|(k, v)| layer.vars.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for k in layer.vars.keys().filter(|k| !css_vars.contains_key(*k)) {
        vars_diff.insert(k.clone(), String::new());
    }
    let fields_diff: Fields = field_values
        .iter()
        .filter(|(k, v)| layer.fields.get(*k) != Some(*v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    // ---- Commit --------------------------------------------------------------------------
    if html_changed || css_changed || fields_changed {
        let next = TemplateDef {
            html: draft.html.unwrap_or_else(|| old.html.clone()),
            css: draft.css.unwrap_or_else(|| old.css.clone()),
            fields: draft.fields.unwrap_or_else(|| old.fields.clone()),
            design: old.design,
        };
        layer.template = Arc::new(next);
    }
    if let Some(v) = draft.variables {
        layer.variables = v;
    }
    if let Some(v) = draft.variant {
        layer.variant = v;
    }
    layer.var_values = var_values;
    layer.vars = css_vars;
    layer.fields = field_values;
    let auto_out_changed = layer.auto_out_ms != auto_out;
    layer.auto_out_ms = auto_out;

    if recompile {
        layer.template_rev = layer.template_rev.wrapping_add(1);
    }
    if recompile || auto_out_changed {
        // A changed auto-out alone (--auto-out-ms on the host): a same-rev `layer` message.
        return Ok(vec![ServerMsg::Layer {
            layer: layer.to_wire(),
        }]);
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
    Ok(msgs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{SelectOption, VariableKind};
    use crate::core::protocol::FieldType;

    fn field(key: &str, label: &str, kind: FieldType) -> FieldDef {
        FieldDef {
            key: key.into(),
            label: label.into(),
            kind,
            default: String::new(),
            ..Default::default()
        }
    }

    fn meta() -> TemplateMeta {
        TemplateMeta {
            name: "T".into(),
            fields: vec![
                field("name", "Name", FieldType::Text),
                field("title", "Title", FieldType::Text),
            ],
            variables: vec![
                VariableDef {
                    key: "accent".into(),
                    label: "Accent".into(),
                    kind: VariableKind::Color,
                    default: "#22b8cf".into(),
                },
                VariableDef {
                    key: "font".into(),
                    label: "Font".into(),
                    kind: VariableKind::Font,
                    default: "IBM Plex Sans".into(),
                },
                VariableDef {
                    key: "radius".into(),
                    label: "Radius".into(),
                    kind: VariableKind::Range {
                        min: 0.0,
                        max: 24.0,
                        step: 1.0,
                        unit: "px".into(),
                    },
                    default: "4".into(),
                },
                VariableDef {
                    key: "align".into(),
                    label: "Align".into(),
                    kind: VariableKind::Select {
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
                    },
                    default: "left".into(),
                },
            ],
            auto_out_ms: Some(3000),
            ..Default::default()
        }
    }

    fn vdoc(html: &str, css: &str) -> VariantDoc {
        VariantDoc {
            id: "default".into(),
            name: "Default".into(),
            vars: Fields::new(),
            html: html.into(),
            css: css.into(),
        }
    }

    fn loaded() -> LayerLive {
        let mut l = LayerLive::new(
            "template",
            "Template",
            TemplateDef {
                html: "<div></div>".into(),
                css: String::new(),
                fields: Vec::new(),
                design: None,
            },
        );
        l.on_air = true;
        let msgs = load_template(&mut l, &meta(), &vdoc("<div>{{name}}</div>", ".a{}")).unwrap();
        assert_eq!(msgs.len(), 1);
        l
    }

    fn values(pairs: &[(&str, &str)]) -> Fields {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn variants_load_with_their_defaults_and_drafts_switch_them() {
        let mut l = loaded();
        assert_eq!(l.variant, "default");
        // A variant's own defaults; an invalid one falls back to the definition's.
        let mut v = vdoc("<b>{{name}}</b>", "");
        v.id = "compact".into();
        v.vars = values(&[("accent", "#00ff00"), ("radius", "99")]);
        let msgs = load_template(&mut l, &meta(), &v).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(l.variant, "compact");
        assert_eq!(l.vars["accent"], "#00ff00");
        assert_eq!(l.var_values["radius"], "4");
        let rev = l.template_rev;

        // Switching variants in a draft recompiles even when the look is the same.
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                variant: Some("default".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(msgs[..], [ServerMsg::Layer { .. }]));
        assert_eq!(l.template_rev, rev + 1);
        assert_eq!(l.variant, "default");
        assert_eq!(l.view().variant, "default");
        let same = TemplateDraft {
            variant: Some("default".into()),
            ..Default::default()
        };
        assert!(apply_draft(&mut l, same).unwrap().is_empty());
        let bad = TemplateDraft {
            variant: Some("Bad Id".into()),
            ..Default::default()
        };
        assert!(apply_draft(&mut l, bad).is_err());
        assert_eq!(l.variant, "default");
    }

    #[test]
    fn load_sets_labels_defaults_and_goes_off_air() {
        let l = loaded();
        assert!(!l.on_air);
        assert_eq!(l.template_rev, 2);
        assert_eq!(l.fields, values(&[("name", "Name"), ("title", "Title")]));
        assert_eq!(l.vars["accent"], "#22b8cf");
        assert_eq!(l.vars["font"], "\"IBM Plex Sans\", system-ui, sans-serif");
        assert_eq!(l.vars["radius"], "4px");
        assert_eq!(l.vars["align"], "left");
        assert_eq!(l.var_values["radius"], "4");
        assert_eq!(l.auto_out_ms, Some(3000));
        let wire = l.to_wire();
        assert_eq!(wire.vars, l.vars);
        assert_eq!(wire.html, "<div>{{name}}</div>");

        // An invalid template is refused and changes nothing.
        let mut l2 = loaded();
        let mut bad = meta();
        bad.variables[0].default = "red; } body { display:none".into();
        assert!(load_template(&mut l2, &bad, &vdoc("<p></p>", "")).is_err());
        assert_eq!(l2.template.html, "<div>{{name}}</div>");
    }

    #[test]
    fn html_change_bumps_rev_and_sends_layer() {
        let mut l = loaded();
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                html: Some("<div>{{title}}</div>".into()),
                variable_values: Some(values(&[("accent", "#000000")])),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(l.template_rev, 3);
        let [ServerMsg::Layer { layer }] = msgs.as_slice() else {
            panic!("{msgs:?}")
        };
        assert_eq!(layer.template_rev, 3);
        assert_eq!(layer.html, "<div>{{title}}</div>");
        assert_eq!(layer.vars["accent"], "#000000", "layer carries the vars");

        // Same HTML again: nothing.
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                html: Some("<div>{{title}}</div>".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(msgs.is_empty());
        assert_eq!(l.template_rev, 3);

        // CSS change and field type change both recompile; a label-only change does not.
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                css: Some(".b{}".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(msgs.as_slice(), [ServerMsg::Layer { .. }]));
        let mut fields = l.template.fields.clone();
        fields[0].label = "Full name".into();
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                fields: Some(fields.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(msgs.is_empty(), "label only: {msgs:?}");
        assert_eq!(l.template.fields[0].label, "Full name");
        assert_eq!(l.template_rev, 4);
        fields[0].kind = FieldType::Html;
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                fields: Some(fields),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(msgs.as_slice(), [ServerMsg::Layer { .. }]));
        assert_eq!(l.template_rev, 5);
    }

    #[test]
    fn variable_values_send_only_changed_css() {
        let mut l = loaded();
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                variable_values: Some(values(&[
                    ("accent", "#FF0000"),
                    ("font", "IBM Plex Sans"), // unchanged
                    ("radius", "12.50"),
                    ("align", "right"),
                ])),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            msgs,
            [ServerMsg::Vars {
                layer_id: "template".into(),
                vars: values(&[
                    ("accent", "#ff0000"),
                    ("align", "right"),
                    ("radius", "12.5px")
                ]),
            }]
        );
        assert_eq!(l.template_rev, 2, "no recompile");
        assert_eq!(l.var_values["radius"], "12.50");
        // Same values again: nothing.
        let msgs = set_variable_values(&mut l, values(&[("radius", "12.5")])).unwrap();
        assert!(msgs.is_empty(), "{msgs:?}");

        // Changing a definition: the default of a new variable goes out, a removed one is cleared,
        // and a kept value that is still valid stays.
        let mut defs = l.variables.clone();
        defs.retain(|d| d.key != "align");
        defs.push(VariableDef {
            key: "bg".into(),
            label: "Background".into(),
            kind: VariableKind::Color,
            default: "#111".into(),
        });
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                variables: Some(defs),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            msgs,
            [ServerMsg::Vars {
                layer_id: "template".into(),
                vars: values(&[("align", ""), ("bg", "#111")]),
            }]
        );
        assert_eq!(l.vars["accent"], "#ff0000");
    }

    #[test]
    fn field_values_send_fields_with_changed_keys() {
        let mut l = loaded();
        let msgs = apply_draft(
            &mut l,
            TemplateDraft {
                field_values: Some(values(&[("name", "Ada"), ("title", "Title")])),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            msgs,
            [ServerMsg::Fields {
                layer_id: "template".into(),
                fields: values(&[("name", "Ada")]),
            }]
        );
        assert_eq!(l.fields["name"], "Ada");
    }

    #[test]
    fn invalid_draft_changes_nothing() {
        let inj = "red; } body { display:none";
        let base = loaded();
        let bad_drafts = vec![
            TemplateDraft {
                html: Some("<p>ok</p>".into()),
                variable_values: Some(values(&[("accent", inj)])),
                ..Default::default()
            },
            TemplateDraft {
                variable_values: Some(values(&[("font", inj)])),
                ..Default::default()
            },
            TemplateDraft {
                variable_values: Some(values(&[("radius", inj)])),
                ..Default::default()
            },
            TemplateDraft {
                variable_values: Some(values(&[("radius", "25")])),
                ..Default::default()
            },
            TemplateDraft {
                variable_values: Some(values(&[("align", inj)])),
                ..Default::default()
            },
            TemplateDraft {
                variable_values: Some(values(&[("nope", "1")])),
                ..Default::default()
            },
            TemplateDraft {
                field_values: Some(values(&[("nope", "1")])),
                ..Default::default()
            },
            TemplateDraft {
                css: Some("x".repeat(crate::core::model::TEMPLATE_CSS_MAX + 1)),
                ..Default::default()
            },
            TemplateDraft {
                fields: Some(vec![field("accent", "Clash", FieldType::Text)]),
                ..Default::default()
            },
            TemplateDraft {
                fields: Some(vec![field("Bad Key", "x", FieldType::Text)]),
                ..Default::default()
            },
            TemplateDraft {
                auto_out_ms: Some(Some(100)),
                ..Default::default()
            },
        ];
        for d in bad_drafts {
            let mut l = base.clone();
            assert!(apply_draft(&mut l, d.clone()).is_err(), "{d:?}");
            assert_eq!(l.template, base.template);
            assert_eq!(l.template_rev, base.template_rev);
            assert_eq!(l.vars, base.vars);
            assert_eq!(l.var_values, base.var_values);
            assert_eq!(l.fields, base.fields);
            assert_eq!(l.auto_out_ms, base.auto_out_ms);
        }
    }

    #[test]
    fn auto_out_draft() {
        let mut l = loaded();
        let d = |v| TemplateDraft {
            auto_out_ms: v,
            ..Default::default()
        };
        let rev = l.template_rev;
        match apply_draft(&mut l, d(Some(Some(800)))).unwrap().as_slice() {
            [ServerMsg::Layer { layer: w }] => {
                assert_eq!(w.auto_out_ms, Some(800), "--auto-out-ms for the renderer");
                assert_eq!(w.template_rev, rev, "no recompile");
            }
            other => panic!("expected one layer message, got {other:?}"),
        }
        assert_eq!(l.auto_out_ms, Some(800));
        assert!(
            apply_draft(&mut l, d(Some(Some(800)))).unwrap().is_empty(),
            "unchanged"
        );
        assert!(apply_draft(&mut l, d(None)).unwrap().is_empty());
        assert_eq!(l.auto_out_ms, Some(800), "absent keeps it");
        apply_draft(&mut l, d(Some(Some(0)))).unwrap();
        assert_eq!(l.auto_out_ms, None, "0 turns it off");
        apply_draft(&mut l, d(Some(Some(800)))).unwrap();
        apply_draft(&mut l, d(Some(None))).unwrap();
        assert_eq!(l.auto_out_ms, None);
    }
}
