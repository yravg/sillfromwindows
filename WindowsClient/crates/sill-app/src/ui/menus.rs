//! OWNER: worker U2. The Mac's menus as a menu tree (matrix B03/M01; iOSClient/MacMenuButton.swift,
//! MacMenuState.swift; protocol-spec §7).
//!
//! The top level arrives with the window list (`stream.menus`, kind 24). A menu's items are
//! fetched when it opens (`Command::FetchMenu { id, title }`, kind 27) and come back in
//! `stream.menu_items[id]`; an id is good only within one `version`, so an answer of another
//! version is asked for again. A leaf is `Command::PressMenu` (kind 25); the Mac presses it only
//! while the item still has the title shown. Shortcuts are text, never key commands.
use std::collections::HashMap;
use std::sync::mpsc::Sender;

use egui::{Button, RichText, Ui};
use sill_proto::menu::{MacMenu, MacMenuItem};

use crate::model::{AppModel, Command};
use crate::theme;

/// A fetch with no answer after this long is asked again (the iOS client's wait is
/// max(4 s, 4 × round trip), MacMenuState).
const REFETCH_AFTER: f64 = 4.0;

#[derive(Default)]
pub struct MenusUi {
    /// id -> (version asked for, when).
    asked: HashMap<String, (i64, f64)>,
}

/// Whether the Mac sent menus (the Menus button shows only then).
pub fn available(model: &AppModel) -> bool {
    model
        .stream
        .menus
        .as_ref()
        .and_then(|m| m.menus.as_ref())
        .is_some_and(|m| !m.is_empty())
}

/// The items of a fetched menu, when the answer is for the current version.
pub fn items_for<'a>(model: &'a AppModel, id: &str) -> Option<&'a MacMenu> {
    let version = model.stream.menus.as_ref().and_then(|m| m.version);
    model
        .stream
        .menu_items
        .get(id)
        .filter(|m| m.version == version && m.items.is_some())
}

/// The mark column's text: the Mac's own mark string, else nothing.
fn mark_prefix(item: &MacMenuItem) -> String {
    match item.mark.as_deref() {
        Some(m) if !m.is_empty() => format!("{m} "),
        _ => String::new(),
    }
}

impl MenusUi {
    /// The menu tree, inside a popup the caller opened.
    pub fn tree(&mut self, ui: &mut Ui, model: &AppModel, tx: &Sender<Command>) {
        let Some(top) = model.stream.menus.as_ref() else {
            return;
        };
        ui.set_min_width(180.0);
        if let Some(app) = &top.app {
            ui.label(RichText::new(app).size(12.0).color(theme::MUTED).strong());
        }
        note(ui, top.stale, top.note.as_deref());
        for item in top.menus.as_deref().unwrap_or(&[]) {
            self.item(ui, item, model, tx);
        }
    }

    fn item(&mut self, ui: &mut Ui, item: &MacMenuItem, model: &AppModel, tx: &Sender<Command>) {
        if item.separator == Some(true) {
            ui.separator();
            return;
        }
        let (Some(id), Some(title)) = (item.id.as_deref(), item.title.as_deref()) else {
            return;
        };
        let enabled = item.enabled != Some(false);
        let label = format!("{}{}", mark_prefix(item), title);
        if item.submenu == Some(true) {
            ui.add_enabled_ui(enabled, |ui| {
                ui.menu_button(label, |ui| self.submenu(ui, id, title, model, tx));
            });
            return;
        }
        let mut b = Button::new(label);
        if let Some(k) = item.key.as_deref().filter(|k| !k.is_empty()) {
            b = b.shortcut_text(k);
        }
        if ui.add_enabled(enabled, b).clicked() {
            let _ = tx.send(Command::PressMenu {
                id: id.to_string(),
                title: title.to_string(),
            });
            ui.close();
        }
    }

    pub fn submenu(
        &mut self,
        ui: &mut Ui,
        id: &str,
        title: &str,
        model: &AppModel,
        tx: &Sender<Command>,
    ) {
        ui.set_min_width(200.0);
        let now = ui.input(|i| i.time);
        let version = model
            .stream
            .menus
            .as_ref()
            .and_then(|m| m.version)
            .unwrap_or(0);
        let Some(entry) = items_for(model, id) else {
            // Not fetched, or fetched for another version of the tree: ask (again).
            let due = self
                .asked
                .get(id)
                .is_none_or(|(v, t)| *v != version || now - t >= REFETCH_AFTER);
            if due {
                self.asked.insert(id.to_string(), (version, now));
                let _ = tx.send(Command::FetchMenu {
                    id: id.to_string(),
                    title: title.to_string(),
                });
            }
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().color(theme::MUTED));
                ui.label(RichText::new("Loading…").size(13.0).color(theme::MUTED));
            });
            if let Some(stale) = model.stream.menu_items.get(id) {
                note(ui, stale.stale, stale.note.as_deref());
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(300));
            return;
        };
        note(ui, entry.stale, entry.note.as_deref());
        for item in entry.items.as_deref().unwrap_or(&[]) {
            self.item(ui, item, model, tx);
        }
        if let Some(more) = entry.more.filter(|m| *m > 0) {
            ui.label(
                RichText::new(format!("{more} more items not shown"))
                    .size(12.0)
                    .color(theme::MUTED),
            );
        }
    }
}

/// The Mac's "The menus changed…" or another note.
fn note(ui: &mut Ui, stale: Option<bool>, note: Option<&str>) {
    let text = match (stale, note) {
        (_, Some(n)) if !n.is_empty() => Some(n.to_string()),
        (Some(true), _) => Some("The menus changed. Open the menu again.".to_string()),
        _ => None,
    };
    if let Some(t) = text {
        ui.label(RichText::new(t).size(12.0).color(theme::PROBLEM));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(id: &str, t: &str) -> MacMenuItem {
        MacMenuItem {
            id: Some(id.into()),
            title: Some(t.into()),
            ..Default::default()
        }
    }

    #[test]
    fn available_only_with_menus() {
        let mut m = AppModel::default();
        assert!(!available(&m));
        m.stream.menus = Some(MacMenu {
            menus: Some(vec![]),
            ..Default::default()
        });
        assert!(!available(&m));
        m.stream.menus = Some(MacMenu {
            menus: Some(vec![leaf("1", "File")]),
            ..Default::default()
        });
        assert!(available(&m));
    }

    #[test]
    fn an_answer_of_another_version_is_not_used() {
        let mut m = AppModel::default();
        m.stream.menus = Some(MacMenu {
            version: Some(2),
            menus: Some(vec![leaf("1", "File")]),
            ..Default::default()
        });
        let answer = |v| MacMenu {
            version: Some(v),
            items: Some(vec![leaf("1.0", "New")]),
            ..Default::default()
        };
        m.stream.menu_items.insert("1".into(), answer(1));
        assert!(items_for(&m, "1").is_none());
        m.stream.menu_items.insert("1".into(), answer(2));
        assert!(items_for(&m, "1").is_some());
    }

    #[test]
    fn marks_prefix_the_title() {
        let mut i = leaf("1", "Bold");
        assert_eq!(mark_prefix(&i), "");
        i.mark = Some("✓".into());
        assert_eq!(mark_prefix(&i), "✓ ");
    }
}
