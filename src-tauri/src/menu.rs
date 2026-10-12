//! The native app menu, built from the `MenuSpec` the frontend sends (`set_app_menu`). Tauri types
//! live here only; the spec types are in `core::model`.
//!
//! - **Windows:** a menu bar on the main window (`window.set_menu`), never on other windows. Keys
//!   are display text after a tab ("Undo\tCtrl+Z"), never registered as accelerators: the
//!   registry's one keydown listener runs shortcuts, so a key never fires twice.
//! - **macOS:** the app menu (About, Preferences…, Services, Hide, Quit) is added here from the
//!   items with the [`APP_MENU_IDS`] ids, which are then left out of File / Help; keys become
//!   accelerators (Ctrl → CmdOrCtrl).
//!
//! The last spec and a map id → handle are kept: a spec with the same structure (kinds, ids and
//! order) updates text / enabled / checked / keys in place; anything else rebuilds the menu.
//! Clicks emit `menu` `{ id }` to the main window.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use serde_json::json;
#[cfg(target_os = "macos")]
use tauri::menu::IsMenuItem;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, State, Wry};

use crate::commands::app::MAIN_WINDOW;
use crate::core::events::names;
use crate::core::model::{MenuItemSpec, MenuNode, MenuPredefined, MenuSpec, MenuSubmenu};
use crate::error::AppResult;

/// Item ids macOS puts in the app menu instead of File / Help.
pub const APP_MENU_IDS: [&str; 3] = ["help.about", "app.preferences", "app.exit"];

const MAC: bool = cfg!(target_os = "macos");

// ---- Pure part (tested) -------------------------------------------------------------------------

/// The spec as this platform shows it: on macOS without the app-menu items (and the separators
/// they leave dangling).
pub fn platform_spec(spec: &MenuSpec, mac: bool) -> MenuSpec {
    if !mac {
        return spec.clone();
    }
    MenuSpec {
        menus: spec.menus.iter().map(strip_app_items).collect(),
    }
}

fn strip_app_items(sub: &MenuSubmenu) -> MenuSubmenu {
    let mut items: Vec<MenuNode> = Vec::new();
    for node in &sub.items {
        match node {
            MenuNode::Item(i) if APP_MENU_IDS.contains(&i.id.as_str()) => {}
            MenuNode::Submenu(s) => items.push(MenuNode::Submenu(strip_app_items(s))),
            MenuNode::Separator => {
                if !matches!(items.last(), None | Some(MenuNode::Separator)) {
                    items.push(MenuNode::Separator);
                }
            }
            other => items.push(other.clone()),
        }
    }
    if matches!(items.last(), Some(MenuNode::Separator)) {
        items.pop();
    }
    MenuSubmenu {
        items,
        ..sub.clone()
    }
}

/// One token of a spec's structure: what must match for an in-place update.
#[derive(Debug, PartialEq, Eq)]
enum Shape<'a> {
    Open(&'a str),
    Close,
    Item(&'a str, bool),
    Separator,
    Predefined(MenuPredefined),
}

fn shape(spec: &MenuSpec) -> Vec<Shape<'_>> {
    fn walk<'a>(sub: &'a MenuSubmenu, out: &mut Vec<Shape<'a>>) {
        out.push(Shape::Open(&sub.id));
        for node in &sub.items {
            match node {
                MenuNode::Item(i) => out.push(Shape::Item(&i.id, i.checked.is_some())),
                MenuNode::Submenu(s) => walk(s, out),
                MenuNode::Separator => out.push(Shape::Separator),
                MenuNode::Predefined { item } => out.push(Shape::Predefined(*item)),
            }
        }
        out.push(Shape::Close);
    }
    let mut out = Vec::new();
    for m in &spec.menus {
        walk(m, &mut out);
    }
    out
}

/// Same kinds and ids in the same order (a check item stays a check item).
pub fn same_structure(a: &MenuSpec, b: &MenuSpec) -> bool {
    shape(a) == shape(b)
}

/// What changed on one item or submenu between two specs of the same structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuChange {
    pub id: String,
    /// The new display text (title + keys on Windows) when it changed.
    pub text: Option<String>,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
    /// macOS: the new accelerator (`Some(None)` = removed) when the keys changed.
    pub accelerator: Option<Option<String>>,
}

/// The in-place updates from `old` to `new`; only call with [`same_structure`] specs.
pub fn changes(old: &MenuSpec, new: &MenuSpec, mac: bool) -> Vec<MenuChange> {
    let mut out = Vec::new();
    for (a, b) in old.menus.iter().zip(&new.menus) {
        diff_sub(a, b, mac, &mut out);
    }
    out
}

fn diff_sub(a: &MenuSubmenu, b: &MenuSubmenu, mac: bool, out: &mut Vec<MenuChange>) {
    if a.text != b.text || a.enabled != b.enabled {
        out.push(MenuChange {
            id: b.id.clone(),
            text: (a.text != b.text).then(|| b.text.clone()),
            enabled: (a.enabled != b.enabled).then_some(b.enabled),
            checked: None,
            accelerator: None,
        });
    }
    for (x, y) in a.items.iter().zip(&b.items) {
        match (x, y) {
            (MenuNode::Item(x), MenuNode::Item(y)) => {
                let tx = display_text(x, mac);
                let ty = display_text(y, mac);
                let keys = mac && x.keys != y.keys;
                if tx != ty || x.enabled != y.enabled || x.checked != y.checked || keys {
                    out.push(MenuChange {
                        id: y.id.clone(),
                        text: (tx != ty).then_some(ty),
                        enabled: (x.enabled != y.enabled).then_some(y.enabled),
                        checked: if x.checked != y.checked {
                            y.checked
                        } else {
                            None
                        },
                        accelerator: keys.then(|| y.keys.as_deref().and_then(accelerator)),
                    });
                }
            }
            (MenuNode::Submenu(x), MenuNode::Submenu(y)) => diff_sub(x, y, mac, out),
            _ => {}
        }
    }
}

/// The item's label: on Windows the keys follow a tab (shown right-aligned, not registered).
pub fn display_text(item: &MenuItemSpec, mac: bool) -> String {
    match (&item.keys, mac) {
        (Some(keys), false) if !keys.is_empty() => format!("{}\t{}", item.text, windows_keys(keys)),
        _ => item.text.clone(),
    }
}

/// Canonical keys ("Ctrl+Shift+P", "Escape", "ArrowUp") in Windows menu spelling ("Esc", "Up").
pub fn windows_keys(keys: &str) -> String {
    keys.split('+')
        .map(|k| match k {
            "Escape" => "Esc",
            "ArrowUp" => "Up",
            "ArrowDown" => "Down",
            "ArrowLeft" => "Left",
            "ArrowRight" => "Right",
            "PageUp" => "PgUp",
            "PageDown" => "PgDn",
            "Delete" => "Del",
            "Insert" => "Ins",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Canonical keys as a macOS accelerator (Ctrl → CmdOrCtrl); `None` for an empty spelling.
pub fn accelerator(keys: &str) -> Option<String> {
    if keys.is_empty() {
        return None;
    }
    Some(
        keys.split('+')
            .map(|k| if k == "Ctrl" { "CmdOrCtrl" } else { k })
            .collect::<Vec<_>>()
            .join("+"),
    )
}

/// The spec's check state of item `id` (`None`: not a check item or not in the spec).
fn checked_of(spec: &MenuSpec, id: &str) -> Option<bool> {
    fn find(items: &[MenuNode], id: &str) -> Option<Option<bool>> {
        items.iter().find_map(|n| match n {
            MenuNode::Item(i) if i.id == id => Some(i.checked),
            MenuNode::Submenu(s) => find(&s.items, id),
            _ => None,
        })
    }
    spec.menus.iter().find_map(|m| find(&m.items, id)).flatten()
}

// ---- Native menu -------------------------------------------------------------------------------

enum Handle {
    Item(MenuItem<Wry>),
    Check(CheckMenuItem<Wry>),
    Sub(Submenu<Wry>),
}

struct Built {
    /// The platform spec the menu shows.
    spec: MenuSpec,
    handles: HashMap<String, Handle>,
}

/// Managed state: the menu on screen (none until the frontend sends one).
#[derive(Default)]
pub struct MenuState(Mutex<Option<Built>>);

/// Replaces the app menu. Sync on purpose: menu calls run on the main thread.
#[tauri::command]
pub fn set_app_menu(app: AppHandle, menus: State<'_, MenuState>, spec: MenuSpec) -> AppResult<()> {
    let started = Instant::now();
    let spec = platform_spec(&spec, MAC);
    let mut g = menus.0.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(built) = g.as_mut() {
        if same_structure(&built.spec, &spec) {
            let list = changes(&built.spec, &spec, MAC);
            for c in &list {
                apply_change(&built.handles, c)?;
            }
            built.spec = spec;
            tracing::debug!(
                "menu: {} in-place updates in {:.2} ms",
                list.len(),
                started.elapsed().as_secs_f64() * 1000.0
            );
            return Ok(());
        }
    }
    let mut handles = HashMap::new();
    let menu = build(&app, &spec, &mut handles)?;
    install(&app, menu)?;
    *g = Some(Built { spec, handles });
    tracing::debug!(
        "menu: rebuilt in {:.2} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

fn apply_change(handles: &HashMap<String, Handle>, c: &MenuChange) -> tauri::Result<()> {
    match handles.get(&c.id) {
        Some(Handle::Item(i)) => {
            if let Some(t) = &c.text {
                i.set_text(t)?;
            }
            if let Some(e) = c.enabled {
                i.set_enabled(e)?;
            }
            if let Some(a) = &c.accelerator {
                i.set_accelerator(a.as_deref())?;
            }
        }
        Some(Handle::Check(i)) => {
            if let Some(t) = &c.text {
                i.set_text(t)?;
            }
            if let Some(e) = c.enabled {
                i.set_enabled(e)?;
            }
            if let Some(ch) = c.checked {
                i.set_checked(ch)?;
            }
            if let Some(a) = &c.accelerator {
                i.set_accelerator(a.as_deref())?;
            }
        }
        Some(Handle::Sub(s)) => {
            if let Some(t) = &c.text {
                s.set_text(t)?;
            }
            if let Some(e) = c.enabled {
                s.set_enabled(e)?;
            }
        }
        None => {}
    }
    Ok(())
}

fn build(
    app: &AppHandle,
    spec: &MenuSpec,
    handles: &mut HashMap<String, Handle>,
) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    #[cfg(target_os = "macos")]
    menu.append(&mac_app_menu(app)?)?;
    for sub in &spec.menus {
        menu.append(&build_sub(app, sub, handles)?)?;
    }
    Ok(menu)
}

fn build_sub(
    app: &AppHandle,
    sub: &MenuSubmenu,
    handles: &mut HashMap<String, Handle>,
) -> tauri::Result<Submenu<Wry>> {
    let s = Submenu::with_id(app, sub.id.as_str(), &sub.text, sub.enabled)?;
    for node in &sub.items {
        match node {
            MenuNode::Item(i) => {
                let text = display_text(i, MAC);
                let accel = if MAC {
                    i.keys.as_deref().and_then(accelerator)
                } else {
                    None
                };
                match i.checked {
                    Some(checked) => {
                        let item = CheckMenuItem::with_id(
                            app,
                            i.id.as_str(),
                            text,
                            i.enabled,
                            checked,
                            accel,
                        )?;
                        s.append(&item)?;
                        handles.insert(i.id.clone(), Handle::Check(item));
                    }
                    None => {
                        let item = MenuItem::with_id(app, i.id.as_str(), text, i.enabled, accel)?;
                        s.append(&item)?;
                        handles.insert(i.id.clone(), Handle::Item(item));
                    }
                }
            }
            MenuNode::Submenu(child) => {
                let c = build_sub(app, child, handles)?;
                s.append(&c)?;
            }
            MenuNode::Separator => s.append(&PredefinedMenuItem::separator(app)?)?,
            MenuNode::Predefined { item } => s.append(&predefined(app, *item)?)?,
        }
    }
    handles.insert(sub.id.clone(), Handle::Sub(s.clone()));
    Ok(s)
}

fn predefined(app: &AppHandle, item: MenuPredefined) -> tauri::Result<PredefinedMenuItem<Wry>> {
    match item {
        MenuPredefined::Cut => PredefinedMenuItem::cut(app, None),
        MenuPredefined::Copy => PredefinedMenuItem::copy(app, None),
        MenuPredefined::Paste => PredefinedMenuItem::paste(app, None),
        MenuPredefined::SelectAll => PredefinedMenuItem::select_all(app, None),
        MenuPredefined::Undo => PredefinedMenuItem::undo(app, None),
        MenuPredefined::Redo => PredefinedMenuItem::redo(app, None),
    }
}

/// macOS app menu: About Kinetra, Preferences… (Cmd+,), Services, Hide, Hide others, Show all,
/// Quit. About / Preferences / Quit are our items so they run through the registry like the rest.
#[cfg(target_os = "macos")]
fn mac_app_menu(app: &AppHandle) -> tauri::Result<Submenu<Wry>> {
    let about = MenuItem::with_id(app, "help.about", "About Kinetra", true, None::<&str>)?;
    let prefs = MenuItem::with_id(
        app,
        "app.preferences",
        "Preferences…",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let quit = MenuItem::with_id(app, "app.exit", "Quit Kinetra", true, Some("CmdOrCtrl+Q"))?;
    let items: [&dyn IsMenuItem<Wry>; 9] = [
        &about,
        &PredefinedMenuItem::separator(app)?,
        &prefs,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::services(app, None)?,
        &PredefinedMenuItem::separator(app)?,
        &PredefinedMenuItem::hide(app, None)?,
        &PredefinedMenuItem::hide_others(app, None)?,
        &PredefinedMenuItem::show_all(app, None)?,
    ];
    let sub = Submenu::with_items(app, "Kinetra", true, &items)?;
    sub.append(&PredefinedMenuItem::separator(app)?)?;
    sub.append(&quit)?;
    Ok(sub)
}

/// Windows: on the main window only. macOS: the app menu.
fn install(app: &AppHandle, menu: Menu<Wry>) -> tauri::Result<()> {
    if MAC {
        app.set_menu(menu)?;
    } else if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        w.set_menu(menu)?;
    }
    Ok(())
}

/// The app's menu event handler: forwards the click to the main window. A check item toggles
/// itself natively on click; its state is put back to the spec's (the frontend sends the real
/// state once the command ran).
pub fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().0.as_str();
    if let Some(menus) = app.try_state::<MenuState>() {
        let g = menus.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(built) = g.as_ref() {
            if let (Some(Handle::Check(item)), Some(checked)) =
                (built.handles.get(id), checked_of(&built.spec, id))
            {
                let _ = item.set_checked(checked);
            }
        }
    }
    if let Err(e) = app.emit_to(MAIN_WINDOW, names::MENU, json!({ "id": id })) {
        tracing::warn!("menu event {id} not sent: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, text: &str, keys: Option<&str>) -> MenuNode {
        MenuNode::Item(MenuItemSpec {
            id: id.into(),
            text: text.into(),
            keys: keys.map(str::to_string),
            enabled: true,
            checked: None,
        })
    }

    fn spec() -> MenuSpec {
        serde_json::from_value(json!({
            "menus": [
                { "kind": "submenu", "id": "menu.file", "text": "&File", "enabled": true, "items": [
                    { "kind": "item", "id": "app.newProject", "text": "New project…", "keys": "Ctrl+N", "enabled": true },
                    { "kind": "submenu", "id": "menu.file.recent", "text": "Open recent", "enabled": true, "items": [
                        { "kind": "item", "id": "recent.a", "text": "Show A", "keys": null, "enabled": true, "checked": null }
                    ]},
                    { "kind": "separator" },
                    { "kind": "item", "id": "app.preferences", "text": "Preferences…", "keys": "Ctrl+,", "enabled": true },
                    { "kind": "separator" },
                    { "kind": "item", "id": "app.exit", "text": "Exit", "keys": null, "enabled": true }
                ]},
                { "kind": "submenu", "id": "menu.edit", "text": "&Edit", "enabled": true, "items": [
                    { "kind": "predefined", "item": "cut" },
                    { "kind": "predefined", "item": "selectAll" }
                ]},
                { "kind": "submenu", "id": "menu.view", "text": "&View", "enabled": true, "items": [
                    { "kind": "item", "id": "view.simple", "text": "Simple mode", "keys": null, "enabled": true, "checked": true },
                    { "kind": "item", "id": "view.advanced", "text": "Advanced mode", "keys": null, "enabled": true, "checked": false }
                ]}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn parses_the_frontend_spec() {
        let s = spec();
        assert_eq!(s.menus.len(), 3);
        assert!(matches!(
            s.menus[1].items[1],
            MenuNode::Predefined {
                item: MenuPredefined::SelectAll
            }
        ));
        assert_eq!(checked_of(&s, "view.simple"), Some(true));
        assert_eq!(checked_of(&s, "app.exit"), None);
    }

    #[test]
    fn same_structure_ignores_labels_states_and_keys() {
        let a = spec();
        let mut b = spec();
        b.menus[0].text = "Fil".into();
        if let MenuNode::Item(i) = &mut b.menus[0].items[0] {
            i.text = "Nytt prosjekt…".into();
            i.keys = Some("Ctrl+Shift+N".into());
            i.enabled = false;
        }
        if let MenuNode::Item(i) = &mut b.menus[2].items[0] {
            i.checked = Some(false);
        }
        assert!(same_structure(&a, &b));

        // An item added, removed or turned into a check item is a new structure.
        let mut c = spec();
        if let MenuNode::Submenu(r) = &mut c.menus[0].items[1] {
            r.items.push(item("recent.b", "Show B", None));
        }
        assert!(!same_structure(&a, &c));
        let mut d = spec();
        if let MenuNode::Item(i) = &mut d.menus[0].items[0] {
            i.checked = Some(false);
        }
        assert!(!same_structure(&a, &d));
        let mut e = spec();
        e.menus.swap(1, 2);
        assert!(!same_structure(&a, &e));
    }

    #[test]
    fn changes_list_only_what_changed() {
        let a = spec();
        assert!(changes(&a, &a, false).is_empty());
        let mut b = spec();
        if let MenuNode::Item(i) = &mut b.menus[0].items[0] {
            i.keys = Some("Ctrl+Shift+N".into());
        }
        if let MenuNode::Item(i) = &mut b.menus[2].items[0] {
            i.checked = Some(false);
        }
        if let MenuNode::Item(i) = &mut b.menus[2].items[1] {
            i.checked = Some(true);
            i.enabled = false;
        }
        b.menus[1].enabled = false;
        let win = changes(&a, &b, false);
        assert_eq!(
            win,
            vec![
                MenuChange {
                    id: "app.newProject".into(),
                    text: Some("New project…\tCtrl+Shift+N".into()),
                    enabled: None,
                    checked: None,
                    accelerator: None,
                },
                MenuChange {
                    id: "menu.edit".into(),
                    text: None,
                    enabled: Some(false),
                    checked: None,
                    accelerator: None,
                },
                MenuChange {
                    id: "view.simple".into(),
                    text: None,
                    enabled: None,
                    checked: Some(false),
                    accelerator: None,
                },
                MenuChange {
                    id: "view.advanced".into(),
                    text: None,
                    enabled: Some(false),
                    checked: Some(true),
                    accelerator: None,
                },
            ]
        );
        // macOS: keys are an accelerator, the text stays the title.
        let mac = changes(&a, &b, true);
        assert_eq!(mac[0].text, None);
        assert_eq!(
            mac[0].accelerator,
            Some(Some("CmdOrCtrl+Shift+N".to_string()))
        );
    }

    #[test]
    fn keys_as_windows_text_and_mac_accelerators() {
        assert_eq!(windows_keys("Ctrl+Shift+P"), "Ctrl+Shift+P");
        assert_eq!(windows_keys("Shift+Escape"), "Shift+Esc");
        assert_eq!(windows_keys("Alt+ArrowUp"), "Alt+Up");
        assert_eq!(windows_keys("Ctrl+="), "Ctrl+=");
        assert_eq!(windows_keys("F11"), "F11");
        assert_eq!(accelerator("Ctrl+,").as_deref(), Some("CmdOrCtrl+,"));
        assert_eq!(accelerator("Shift+F3").as_deref(), Some("Shift+F3"));
        assert_eq!(accelerator(""), None);
        let MenuNode::Item(i) = item("x", "Undo", Some("Ctrl+Z")) else {
            unreachable!()
        };
        assert_eq!(display_text(&i, false), "Undo\tCtrl+Z");
        assert_eq!(display_text(&i, true), "Undo");
        let MenuNode::Item(n) = item("x", "Exit", None) else {
            unreachable!()
        };
        assert_eq!(display_text(&n, false), "Exit");
    }

    #[test]
    fn mac_moves_app_items_out_and_tidies_separators() {
        let a = spec();
        assert_eq!(platform_spec(&a, false), a);
        let m = platform_spec(&a, true);
        let file = &m.menus[0].items;
        // New project…, Open recent ▸ (Preferences and Exit gone, no dangling separators).
        assert_eq!(file.len(), 2, "{file:?}");
        assert!(matches!(&file[0], MenuNode::Item(i) if i.id == "app.newProject"));
        assert!(matches!(&file[1], MenuNode::Submenu(s) if s.id == "menu.file.recent"));
        assert_eq!(m.menus[1], a.menus[1]);
    }
}
