//! Main page: search entry + settings button + virtualised result list.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gtk::prelude::*;

use crate::discovery::{AppEntry, AppSource};

pub const FALLBACK_ICON: &str = "application-x-executable";

/// Called when the user right-clicks a row: `(position, row widget, x, y)`.
pub type ContextCallback = Rc<dyn Fn(u32, &gtk::Widget, f64, f64)>;

pub struct LauncherView {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    pub menu_button: gtk::MenuButton,
    pub list: gtk::ListView,
    pub model: gio::ListStore,
    pub selection: gtk::SingleSelection,
    pub empty: gtk::Label,
    pub count: gtk::Label,
    pub hint: gtk::Label,
}

/// Builds the best `GIcon` for an entry: absolute file path, themed name, or fallback.
pub fn icon_for(entry: &AppEntry) -> gio::Icon {
    if let Some(name) = entry.icon.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        if name.starts_with('/') {
            if Path::new(name).is_file() {
                return gio::FileIcon::new(&gio::File::for_path(name)).upcast();
            }
        } else {
            // Do NOT put the fallback in the same GThemedIcon: GTK looks up
            // every name in the current theme before inheriting (e.g. Adwaita
            // before hicolor), so Adwaita's generic icon would beat an app's
            // own hicolor icon. Check the theme chain first instead.
            let theme = gtk::gdk::Display::default().map(|d| gtk::IconTheme::for_display(&d));
            let has = |n: &str| theme.as_ref().is_none_or(|t| t.has_icon(n));
            if has(name) {
                return gio::ThemedIcon::new(name).upcast();
            }
            // Some desktop files wrongly include an extension (`foo.png`).
            if let Some(stem) = name.rsplit_once('.').map(|(s, _)| s).filter(|s| !s.is_empty() && has(s)) {
                return gio::ThemedIcon::new(stem).upcast();
            }
        }
    }
    gio::ThemedIcon::new(FALLBACK_ICON).upcast()
}

pub fn entry_of(obj: &glib::Object) -> Option<Rc<AppEntry>> {
    obj.downcast_ref::<glib::BoxedAnyObject>().map(|b| b.borrow::<Rc<AppEntry>>().clone())
}

fn nth_child(w: &impl IsA<gtk::Widget>, n: usize) -> Option<gtk::Widget> {
    let mut c = w.first_child();
    for _ in 0..n {
        c = c?.next_sibling();
    }
    c
}

impl LauncherView {
    pub fn new(icon_size: Rc<Cell<i32>>, show_desc: Rc<Cell<bool>>, on_context: ContextCallback) -> Self {
        let entry = gtk::Entry::builder()
            .placeholder_text("Buscar aplicativos…")
            .primary_icon_name("system-search-symbolic")
            .hexpand(true)
            .activates_default(false)
            .css_classes(["search-entry"])
            .build();
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .css_classes(["icon-button"])
            .focus_on_click(false)
            .tooltip_text("Menu")
            .valign(gtk::Align::Center)
            .build();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.append(&entry);
        header.append(&menu_button);

        let model = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(true);
        selection.set_can_unselect(false);

        let factory = gtk::SignalListItemFactory::new();
        {
            let on_context = on_context.clone();
            factory.connect_setup(move |_, obj| {
                let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
                item.set_focusable(false);
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let image = gtk::Image::builder().css_classes(["app-icon"]).valign(gtk::Align::Center).build();
                let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
                text.set_hexpand(true);
                text.set_valign(gtk::Align::Center);
                let name = gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["app-name"])
                    .build();
                let desc = gtk::Label::builder()
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["app-desc"])
                    .build();
                text.append(&name);
                text.append(&desc);
                let badge = gtk::Label::builder()
                    .label("Flatpak")
                    .css_classes(["badge"])
                    .valign(gtk::Align::Center)
                    .visible(false)
                    .build();
                row.append(&image);
                row.append(&text);
                row.append(&badge);

                let click = gtk::GestureClick::new();
                click.set_button(3);
                let weak_item = item.downgrade();
                let weak_row = row.downgrade();
                let on_context = on_context.clone();
                click.connect_pressed(move |gesture, _n, x, y| {
                    let (Some(item), Some(row)) = (weak_item.upgrade(), weak_row.upgrade()) else { return };
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    on_context(item.position(), row.upcast_ref(), x, y);
                });
                row.add_controller(click);
                item.set_child(Some(&row));
            });
        }
        {
            let icon_size = icon_size.clone();
            let show_desc = show_desc.clone();
            factory.connect_bind(move |_, obj| {
                let Some(item) = obj.downcast_ref::<gtk::ListItem>() else { return };
                let (Some(app), Some(row)) = (item.item().and_then(|o| entry_of(&o)), item.child()) else { return };
                let (Some(image), Some(text), Some(badge)) = (nth_child(&row, 0), nth_child(&row, 1), nth_child(&row, 2))
                else {
                    return;
                };
                if let Some(image) = image.downcast_ref::<gtk::Image>() {
                    image.set_pixel_size(icon_size.get());
                    image.set_from_gicon(&icon_for(&app));
                }
                if let (Some(name), Some(desc)) = (nth_child(&text, 0), nth_child(&text, 1)) {
                    if let Some(name) = name.downcast_ref::<gtk::Label>() {
                        name.set_text(&app.name);
                    }
                    if let Some(desc) = desc.downcast_ref::<gtk::Label>() {
                        let d = app.comment.as_deref().filter(|s| !s.is_empty()).or(app.generic_name.as_deref()).unwrap_or("");
                        desc.set_text(d);
                        desc.set_visible(show_desc.get() && !d.is_empty());
                    }
                }
                badge.set_visible(app.source == AppSource::Flatpak);
            });
        }

        let list = gtk::ListView::new(Some(selection.clone()), Some(factory));
        list.set_single_click_activate(true);
        list.set_focusable(false);
        list.set_vexpand(true);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .focusable(false)
            .child(&list)
            .build();

        let empty = gtk::Label::builder()
            .label("Nenhum aplicativo encontrado")
            .css_classes(["empty-state"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .visible(false)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroller));
        overlay.add_overlay(&empty);
        overlay.set_vexpand(true);

        let count = gtk::Label::builder().xalign(0.0).css_classes(["footer"]).build();
        let hint = gtk::Label::builder()
            .xalign(1.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["footer"])
            .build();
        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.append(&count);
        footer.append(&hint);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&header);
        root.append(&overlay);
        root.append(&footer);

        Self { root, entry, menu_button, list, model, selection, empty, count, hint }
    }

    pub fn n_items(&self) -> u32 {
        self.model.n_items()
    }

    pub fn entry_at(&self, pos: u32) -> Option<Rc<AppEntry>> {
        self.model.item(pos).and_then(|o| entry_of(&o))
    }

    pub fn selected_index(&self) -> Option<u32> {
        let s = self.selection.selected();
        (s != gtk::INVALID_LIST_POSITION && s < self.n_items()).then_some(s)
    }

    pub fn selected_entry(&self) -> Option<Rc<AppEntry>> {
        self.selected_index().and_then(|i| self.entry_at(i))
    }

    pub fn select(&self, pos: u32) {
        if pos < self.n_items() {
            self.selection.set_selected(pos);
            self.list.scroll_to(pos, gtk::ListScrollFlags::NONE, None);
        }
    }

    pub fn move_selection(&self, delta: i32) {
        let n = self.n_items();
        if n == 0 {
            return;
        }
        let cur = self.selected_index().map_or(0, i64::from);
        let next = (cur + i64::from(delta)).clamp(0, i64::from(n) - 1) as u32;
        self.select(next);
    }

    /// Replaces the list contents and selects `select` (clamped).
    pub fn set_results(&self, results: &[Rc<AppEntry>], select: u32) {
        let objs: Vec<glib::BoxedAnyObject> = results.iter().map(|e| glib::BoxedAnyObject::new(Rc::clone(e))).collect();
        self.model.splice(0, self.model.n_items(), &objs);
        self.empty.set_visible(results.is_empty());
        if !results.is_empty() {
            self.select(select.min(results.len() as u32 - 1));
        }
    }

    /// The widget of the currently selected row (found through its `:selected` state).
    pub fn selected_row_widget(&self) -> Option<gtk::Widget> {
        let mut child = self.list.first_child();
        while let Some(c) = child {
            if c.state_flags().contains(gtk::StateFlags::SELECTED) {
                return Some(c);
            }
            child = c.next_sibling();
        }
        None
    }
}
