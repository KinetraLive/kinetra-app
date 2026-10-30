// Template CRUD, presets and the template preview screen.
// Split out of the core `$lib/api` so only the routes that use these commands load them.

import type {
  LiveLayerView,
  PresetInfo,
  TemplateDoc,
  TemplateDraft,
  TemplateSource,
  TemplateSummary,
} from "../types";
import { call } from "./index";

export const listTemplates = () => call<TemplateSummary[]>("list_templates");
export const getTemplate = (id: string) => call<TemplateDoc>("get_template", { id });
export const createTemplate = (name: string, source: TemplateSource) =>
  call<TemplateDoc>("create_template", { name, source });
export const saveTemplate = (doc: TemplateDoc) => call<TemplateDoc>("save_template", { doc });
export const renameTemplate = (id: string, name: string) =>
  call<TemplateSummary>("rename_template", { id, name });
export const duplicateTemplate = (id: string, name: string) =>
  call<TemplateDoc>("duplicate_template", { id, name });
export const deleteTemplate = (id: string) => call<void>("delete_template", { id });
export const listPresets = () => call<PresetInfo[]>("list_presets");
/** HTML and CSS pretty-printed by the core (the template code editor's Format). */
export const formatTemplateCode = (html: string, css: string) =>
  call<{ html: string; css: string }>("format_template_code", { html, css });
/** Loads the saved template into the preview screen (off air). */
export const previewTemplate = (id: string) => call<LiveLayerView>("preview_template", { id });
export const previewDraft = (draft: TemplateDraft) => call<LiveLayerView>("preview_draft", { draft });
export const previewShow = () => call<LiveLayerView>("preview_show");
export const previewHide = () => call<LiveLayerView>("preview_hide");

// ---- Variants (one field schema, several looks) ----
/** Saves one variant's look and variable defaults (coalesced autosave like save_template). */
export const saveTemplateVariant = (
  templateId: string,
  variantId: string,
  html: string,
  css: string,
  vars: Record<string, string>,
) => call<TemplateDoc>("save_template_variant", { templateId, variantId, html, css, vars });
/** Duplicates `fromVariant` as a new variant named `name`. */
export const createTemplateVariant = (templateId: string, fromVariant: string, name: string) =>
  call<TemplateDoc>("create_template_variant", { templateId, fromVariant, name });
export const renameTemplateVariant = (templateId: string, variantId: string, name: string) =>
  call<TemplateDoc>("rename_template_variant", { templateId, variantId, name });
/** Not the last variant; layers on it fall back to the default. */
export const deleteTemplateVariant = (templateId: string, variantId: string) =>
  call<TemplateDoc>("delete_template_variant", { templateId, variantId });
export const setDefaultTemplateVariant = (templateId: string, variantId: string) =>
  call<TemplateDoc>("set_default_template_variant", { templateId, variantId });

// ---- Style library ----
export const listStyles = () =>
  call<import("../types").StyleInfo[]>("list_styles");
/**
 * Applies a style: copies the style's variant from the library preset into project templates
 * that lack it (merging missing optional field definitions), then switches layers by the live
 * rules (on air: follows Live edit; `transition` for immediate). Project scope also sets
 * `settings.style`. One undo step: `undoApplyStyle(result.undo)`.
 */
export const applyStyle = (
  style: string,
  scope: import("../types").ApplyStyleScope,
  transition: import("../types").VariantTransition,
) => call<import("../types").ApplyStyleResult>("apply_style", { style, scope, transition });
export const undoApplyStyle = (token: string) => call<void>("undo_apply_style", { token });
/** Saves the project's brand kit (validated); every library layer re-applies by the live rules. */
export const saveBrandKit = (brand: import("../types").BrandKit) =>
  call<import("../types").BrandKit>("save_brand_kit", { brand });
