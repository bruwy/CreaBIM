//! Map the app's existing menus to AppKit and deliver selections on the next frame.

use super::CreaBimApp;
use crate::ui_model::Item;
use eframe::egui;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSControlStateValueOff, NSControlStateValueOn, NSEventModifierFlags, NSMenu,
    NSMenuItem,
};
use objc2_foundation::NSString;
use std::cell::RefCell;

const MENUS: [(&str, &str); 7] = [
    ("CreaBIM", "menu:CreaBIM"),
    ("File", "menu:File"),
    ("Edit", "menu:Edit"),
    ("View", "menu:View"),
    ("Create", "menu:Create"),
    ("Materials", "menu:Materials"),
    ("Help", "menu:Help"),
];

struct MenuTargetState {
    actions: RefCell<Vec<String>>,
    ctx: egui::Context,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetState]
    struct MenuTarget;

    impl MenuTarget {
        #[unsafe(method(runCreaBIMMenuAction:))]
        fn run_action(&self, sender: &NSMenuItem) {
            if native_modal(MainThreadMarker::from(self)) {
                return;
            }
            if let Some(action) = sender.representedObject().and_then(|id| id.downcast::<NSString>().ok()) {
                self.ivars().actions.borrow_mut().push(action.to_string());
                self.ivars().ctx.request_repaint();
            }
        }
    }
);

pub(super) struct MacMenu {
    target: Retained<MenuTarget>,
    menus: [Retained<NSMenu>; MENUS.len()],
    items: [Vec<Item>; MENUS.len()],
    blocked: bool,
}

impl MacMenu {
    fn new(mtm: MainThreadMarker, ctx: &egui::Context) -> Self {
        let target = mtm.alloc::<MenuTarget>().set_ivars(MenuTargetState {
            actions: RefCell::default(),
            ctx: ctx.clone(),
        });
        let target = unsafe { msg_send![super(target), init] };
        let app = NSApplication::sharedApplication(mtm);
        let bar = NSMenu::new(mtm);
        let menus = MENUS.map(|(title, _)| {
            if title == "Help" {
                let window = add_submenu(mtm, &bar, "Window", true);
                window.setAutoenablesItems(true);
                window.addItem(&menu_item(
                    mtm,
                    "Minimize",
                    Some(sel!(performMiniaturize:)),
                    "m",
                ));
                window.addItem(&menu_item(mtm, "Zoom", Some(sel!(performZoom:)), ""));
                window.addItem(&NSMenuItem::separatorItem(mtm));
                window.addItem(&menu_item(
                    mtm,
                    "Bring All to Front",
                    Some(sel!(arrangeInFront:)),
                    "",
                ));
                app.setWindowsMenu(Some(&window));
            }
            add_submenu(mtm, &bar, title, true)
        });
        app.setMainMenu(Some(&bar));
        app.setHelpMenu(menus.last().map(|menu| &**menu));
        Self {
            target,
            menus,
            items: std::array::from_fn(|_| Vec::new()),
            blocked: false,
        }
    }

    fn update(&mut self, mtm: MainThreadMarker, items: [Vec<Item>; MENUS.len()], blocked: bool) {
        for ((menu, previous), items) in self.menus.iter().zip(&mut self.items).zip(items) {
            if previous != &items || self.blocked != blocked {
                menu.removeAllItems();
                append_items(mtm, menu, &self.target, &items, blocked);
                *previous = items;
            }
        }
        self.blocked = blocked;
    }
}

fn native_modal(mtm: MainThreadMarker) -> bool {
    let app = NSApplication::sharedApplication(mtm);
    app.modalWindow().is_some()
        || app
            .windows()
            .iter()
            .any(|window| window.attachedSheet().is_some())
}

fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key: &str,
) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}

fn add_submenu(
    mtm: MainThreadMarker,
    parent: &NSMenu,
    title: &str,
    enabled: bool,
) -> Retained<NSMenu> {
    let item = menu_item(mtm, title, None, "");
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str(title));
    menu.setAutoenablesItems(false);
    item.setSubmenu(Some(&menu));
    item.setEnabled(enabled);
    parent.addItem(&item);
    menu
}

fn append_items(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    target: &MenuTarget,
    items: &[Item],
    blocked: bool,
) {
    for item in items {
        match item {
            Item::Action {
                id,
                label,
                shortcut,
                enabled,
                checked,
                hint,
                ..
            } => {
                let title = if id == "app.exit" {
                    append_app_items(mtm, menu);
                    "Quit CreaBIM"
                } else {
                    label
                };
                let (key, modifiers) = key_equivalent(id, shortcut);
                let native = menu_item(mtm, title, Some(sel!(runCreaBIMMenuAction:)), key);
                unsafe {
                    native.setTarget(Some(target));
                    native.setRepresentedObject(Some(&NSString::from_str(id)));
                }
                native.setKeyEquivalentModifierMask(modifiers);
                native.setEnabled(*enabled && !blocked);
                if let Some(checked) = checked {
                    native.setState(if *checked {
                        NSControlStateValueOn
                    } else {
                        NSControlStateValueOff
                    });
                }
                if !hint.is_empty() {
                    native.setToolTip(Some(&NSString::from_str(hint)));
                }
                menu.addItem(&native);
            }
            Item::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            Item::Header(title) => {
                let header = menu_item(mtm, title, None, "");
                header.setEnabled(false);
                menu.addItem(&header);
            }
            Item::Sub {
                label,
                enabled,
                items,
                ..
            } => {
                let submenu = add_submenu(mtm, menu, label, *enabled && !blocked);
                append_items(mtm, &submenu, target, items, blocked);
            }
        }
    }
}

fn append_app_items(mtm: MainThreadMarker, menu: &NSMenu) {
    let services = add_submenu(mtm, menu, "Services", true);
    services.setAutoenablesItems(true);
    NSApplication::sharedApplication(mtm).setServicesMenu(Some(&services));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    menu.addItem(&menu_item(mtm, "Hide CreaBIM", Some(sel!(hide:)), "h"));
    let hide_others = menu_item(mtm, "Hide Others", Some(sel!(hideOtherApplications:)), "h");
    hide_others
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    menu.addItem(&hide_others);
    menu.addItem(&menu_item(
        mtm,
        "Show All",
        Some(sel!(unhideAllApplications:)),
        "",
    ));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
}

fn key_equivalent(id: &str, shortcut: &str) -> (&'static str, NSEventModifierFlags) {
    use NSEventModifierFlags as Mod;
    // Leave edit, drawing and view-sensitive keys with the focused editor or view.
    let key = match (id, shortcut) {
        ("app.exit", _) => "q",
        (_, "Ctrl+N") => "n",
        (_, "Ctrl+O") => "o",
        (_, "Ctrl+S") => "s",
        (_, "Ctrl+,") => ",",
        (_, "Ctrl+/") => "/",
        (_, "Ctrl+K") => "k",
        (_, "Ctrl+Shift+T") => return ("t", Mod::Command | Mod::Shift),
        _ => return ("", Mod::empty()),
    };
    (key, Mod::Command)
}

impl CreaBimApp {
    pub(super) fn mac_menu_blocked(&self) -> bool {
        self.close_ask || MainThreadMarker::new().is_some_and(native_modal)
    }

    pub(super) fn refresh_mac_menu(&mut self, ctx: &egui::Context) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let items = MENUS.map(|(_, key)| self.menu_items(key));
        let blocked = self.mac_menu_blocked();
        self.mac_menu
            .get_or_insert_with(|| MacMenu::new(mtm, ctx))
            .update(mtm, items, blocked);
    }

    pub(super) fn dispatch_mac_menu(&mut self, ctx: &egui::Context) {
        if let Some(menu) = &self.mac_menu {
            let actions = std::mem::take(&mut *menu.target.ivars().actions.borrow_mut());
            self.run_mac_actions(ctx, actions);
        }
    }

    fn run_mac_actions(&mut self, ctx: &egui::Context, actions: Vec<String>) {
        for action in actions {
            if self.mac_menu_blocked() {
                break;
            }
            self.run_action(ctx, &action);
        }
    }
}

#[cfg(test)]
#[test]
fn mac_menu_cannot_clear_recent_projects_during_close_confirmation() {
    let mut app = super::test_app();
    let ctx = egui::Context::default();
    app.recent.files = vec!["recent.creabim".into()];
    app.close_ask = true;
    app.run_mac_actions(&ctx, vec!["recent.clear".into()]);
    assert_eq!(app.recent.files, ["recent.creabim"]);

    app.close_ask = false;
    app.run_mac_actions(&ctx, vec!["recent.clear".into()]);
    assert!(app.recent.files.is_empty());
}
