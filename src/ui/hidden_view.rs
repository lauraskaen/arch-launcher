//! "Aplicativos ocultos" page: restore hidden apps, forget stale IDs.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

use super::launcher_view::icon_for;
use crate::discovery::AppEntry;
use crate::store::AppStore;

struct Row {
    widget: gtk::ListBoxRow,
    id: String,
    name: String,
    installed: bool,
}

pub struct HiddenView {
    pub root: gtk::Box,
    pub back: gtk::Button,
    pub restore_all: gtk::Button,
    pub entry: gtk::Entry,
    title: gtk::Label,
    list: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    empty: gtk::Label,
    rows: RefCell<Vec<Row>>,
    confirming: Cell<bool>,
    confirm_timer: RefCell<Option<glib::SourceId>>,
    icon_size: Rc<Cell<i32>>,
    on_row: RowCallback,
}

/// `(id, display name, installed)` of the row the user acted on.
pub type RowCallback = Rc<dyn Fn(&str, &str, bool)>;

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl HiddenView {
    pub fn new(icon_size: Rc<Cell<i32>>, on_row: RowCallback) -> Rc<Self> {
        let back = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .css_classes(["flat", "icon-button"])
            .focus_on_click(false)
            .tooltip_text("Voltar")
            .build();
        let title = gtk::Label::builder()
            .label("Aplicativos ocultos")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["page-title"])
            .build();
        let restore_all = gtk::Button::builder().label("Reexibir todos").focus_on_click(false).build();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.append(&back);
        header.append(&title);
        header.append(&restore_all);

        let entry = gtk::Entry::builder()
            .placeholder_text("Filtrar aplicativos ocultos…")
            .primary_icon_name("system-search-symbolic")
            .css_classes(["search-entry"])
            .margin_top(8)
            .build();

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(false)
            .focusable(false)
            .css_classes(["app-rows"])
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .focusable(false)
            .child(&list)
            .build();
        scroller.set_margin_top(6);
        let empty = gtk::Label::builder()
            .label("Nenhum aplicativo oculto")
            .css_classes(["empty-state"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroller));
        overlay.add_overlay(&empty);
        overlay.set_vexpand(true);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&header);
        root.append(&entry);
        root.append(&overlay);

        Rc::new(Self {
            root,
            back,
            restore_all,
            entry,
            title,
            list,
            scroller,
            empty,
            rows: RefCell::new(Vec::new()),
            confirming: Cell::new(false),
            confirm_timer: RefCell::new(None),
            icon_size,
            on_row,
        })
    }

    /// Rebuilds the rows from the store, keeping the selection by ID.
    pub fn refresh(&self, store: &AppStore) {
        let on_row = self.on_row.clone();
        let filter = self.entry.text().trim().to_lowercase();
        let prev_id = self.selected_id();
        let prev_idx = self.selected_index();

        self.list.remove_all();
        self.rows.borrow_mut().clear();

        let hidden = store.hidden_apps();
        let orphans = store.orphan_hidden_ids();
        let total = hidden.len() + orphans.len();
        self.title.set_text(&format!("Aplicativos ocultos ({total})"));
        self.restore_all.set_sensitive(total > 0);
        if total == 0 {
            self.reset_confirm();
        }

        let matches_app = |a: &AppEntry| {
            filter.is_empty()
                || a.name.to_lowercase().contains(&filter)
                || a.id.to_lowercase().contains(&filter)
                || a.generic_name.as_deref().is_some_and(|g| g.to_lowercase().contains(&filter))
        };
        let shown_apps: Vec<_> = hidden.iter().filter(|a| matches_app(a)).collect();
        let shown_orphans: Vec<_> = orphans.iter().filter(|id| filter.is_empty() || id.to_lowercase().contains(&filter)).collect();

        for app in &shown_apps {
            let row = self.build_app_row(app, &on_row);
            self.list.append(&row.widget);
            self.rows.borrow_mut().push(row);
        }
        if !shown_orphans.is_empty() {
            let heading = gtk::ListBoxRow::new();
            heading.set_selectable(false);
            heading.set_activatable(false);
            heading.set_focusable(false);
            let label = gtk::Label::builder()
                .label(format!(
                    "NÃO INSTALADOS — {} (continuam ocultos se forem reinstalados)",
                    plural(shown_orphans.len(), "ID", "IDs")
                ))
                .xalign(0.0)
                .wrap(true)
                .css_classes(["section-title"])
                .build();
            heading.set_child(Some(&label));
            self.list.append(&heading);
            for id in &shown_orphans {
                let row = self.build_orphan_row(id, &on_row);
                self.list.append(&row.widget);
                self.rows.borrow_mut().push(row);
            }
        }

        self.empty.set_visible(self.rows.borrow().is_empty());
        self.empty.set_text(if total == 0 { "Nenhum aplicativo oculto" } else { "Nenhum resultado" });

        // Restore the selection.
        let rows = self.rows.borrow();
        let target = prev_id
            .and_then(|id| rows.iter().position(|r| r.id == id))
            .or_else(|| prev_idx.map(|i| i.min(rows.len().saturating_sub(1))))
            .or(if rows.is_empty() { None } else { Some(0) });
        if let Some(i) = target.filter(|i| *i < rows.len()) {
            self.list.select_row(Some(&rows[i].widget));
        }
    }

    fn build_app_row(&self, app: &Rc<AppEntry>, on_row: &RowCallback) -> Row {
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let image = gtk::Image::builder()
            .pixel_size(self.icon_size.get())
            .css_classes(["app-icon"])
            .valign(gtk::Align::Center)
            .build();
        image.set_from_gicon(&icon_for(app));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(
            &gtk::Label::builder()
                .label(&app.name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["app-name"])
                .build(),
        );
        text.append(
            &gtk::Label::builder()
                .label(&app.id)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .css_classes(["app-id"])
                .build(),
        );
        let button = gtk::Button::builder().label("Reexibir").valign(gtk::Align::Center).focus_on_click(false).build();
        {
            let (id, name, cb) = (app.id.clone(), app.name.clone(), on_row.clone());
            button.connect_clicked(move |_| cb(&id, &name, true));
        }
        hbox.append(&image);
        hbox.append(&text);
        hbox.append(&button);
        let widget = gtk::ListBoxRow::new();
        widget.set_focusable(false);
        widget.set_child(Some(&hbox));
        Row { widget, id: app.id.clone(), name: app.name.clone(), installed: true }
    }

    fn build_orphan_row(&self, id: &str, on_row: &RowCallback) -> Row {
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.add_css_class("orphan-section");
        let image = gtk::Image::builder()
            .pixel_size(self.icon_size.get().min(32))
            .css_classes(["app-icon"])
            .valign(gtk::Align::Center)
            .icon_name("application-x-executable")
            .build();
        let label = gtk::Label::builder()
            .label(id)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .css_classes(["app-id"])
            .build();
        let button = gtk::Button::builder().label("Esquecer").valign(gtk::Align::Center).focus_on_click(false).build();
        {
            let (id, cb) = (id.to_owned(), on_row.clone());
            button.connect_clicked(move |_| cb(&id, &id, false));
        }
        hbox.append(&image);
        hbox.append(&label);
        hbox.append(&button);
        let widget = gtk::ListBoxRow::new();
        widget.set_focusable(false);
        widget.set_child(Some(&hbox));
        Row { widget, id: id.to_owned(), name: id.to_owned(), installed: false }
    }

    fn selected_index(&self) -> Option<usize> {
        let sel = self.list.selected_row()?;
        self.rows.borrow().iter().position(|r| r.widget == sel)
    }

    pub fn selected_id(&self) -> Option<String> {
        self.selected_index().map(|i| self.rows.borrow()[i].id.clone())
    }

    /// `(id, name, installed)` of the selected row.
    pub fn selected(&self) -> Option<(String, String, bool)> {
        self.selected_index().map(|i| {
            let rows = self.rows.borrow();
            (rows[i].id.clone(), rows[i].name.clone(), rows[i].installed)
        })
    }

    pub fn move_selection(&self, delta: i32) {
        let rows = self.rows.borrow();
        if rows.is_empty() {
            return;
        }
        let cur = self.selected_index().map_or(0, |i| i as i64);
        let next = (cur + i64::from(delta)).clamp(0, rows.len() as i64 - 1) as usize;
        let row = &rows[next].widget;
        self.list.select_row(Some(row));
        let adj = self.scroller.vadjustment();
        if let Some(b) = row.compute_bounds(&self.list) {
            let (top, bottom) = (f64::from(b.y()), f64::from(b.y() + b.height()));
            if top < adj.value() {
                adj.set_value(top);
            } else if bottom > adj.value() + adj.page_size() {
                adj.set_value(bottom - adj.page_size());
            }
        }
    }

    pub fn reset_confirm(&self) {
        if let Some(id) = self.confirm_timer.borrow_mut().take() {
            id.remove();
        }
        self.confirming.set(false);
        self.restore_all.set_label("Reexibir todos");
        self.restore_all.remove_css_class("danger");
    }

    /// First click arms the button ("Confirmar?" for 3 s); the second click
    /// within that window returns `true` (the caller then restores everything).
    pub fn click_restore_all(self: &Rc<Self>) -> bool {
        if self.confirming.get() {
            self.reset_confirm();
            return true;
        }
        self.confirming.set(true);
        self.restore_all.set_label("Confirmar?");
        self.restore_all.add_css_class("danger");
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_secs(3), move || {
            if let Some(v) = weak.upgrade() {
                v.confirm_timer.borrow_mut().take();
                v.reset_confirm();
            }
        });
        *self.confirm_timer.borrow_mut() = Some(id);
        false
    }
}
