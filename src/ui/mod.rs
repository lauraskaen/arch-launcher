//! UI Components: the single launcher window.
//!
//! One `GtkApplicationWindow` (created once, configured with
//! `platform::prepare_window` before its first show) holding a `GtkStack`
//! with two pages — the launcher and the hidden-apps manager — so layer-shell
//! keyboard focus never has to move between surfaces.

mod banner;
mod hidden_view;
mod keybindings;
mod launcher_view;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gtk::prelude::*;

use crate::app::AppContext;
use crate::config::{default_config_toml, Config};
use crate::discovery::AppEntry;
use crate::search::{SearchEngine, SearchOptions};
use crate::theme::{ThemeManager, NO_ANIMATIONS_CLASS};
use crate::usage::UsageTracker;
use crate::visibility::LoadWarning;

use banner::{Banner, BannerKind};
use hidden_view::HiddenView;
use keybindings::{Action, KeyMap};
use launcher_view::LauncherView;

const PAGE_LAUNCHER: &str = "launcher";
const PAGE_HIDDEN: &str = "hidden";
/// Focus-loss events this soon after a popover opens/closes are ignored.
const FOCUS_GRACE: Duration = Duration::from_millis(350);

pub struct Ui {
    this: Weak<Ui>,
    ctx: Rc<AppContext>,
    window: gtk::ApplicationWindow,
    theme: ThemeManager,
    usage: RefCell<UsageTracker>,
    engine: RefCell<SearchEngine>,
    keymap: RefCell<KeyMap>,
    launcher: LauncherView,
    hidden: Rc<HiddenView>,
    stack: gtk::Stack,
    banner: Rc<Banner>,
    settings_menu: gio::Menu,
    icon_size: Rc<Cell<i32>>,
    show_desc: Rc<Cell<bool>>,
    settings_open: Cell<bool>,
    context_open: Cell<bool>,
    grace_until: Cell<Option<Instant>>,
    context_target: RefCell<Option<Rc<AppEntry>>>,
    context_popover: RefCell<Option<gtk::PopoverMenu>>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn search_options(cfg: &Config) -> SearchOptions {
    SearchOptions {
        max_results: cfg.behavior.max_results,
        sort_by_usage: cfg.behavior.sort_by_usage,
        search_keywords: cfg.behavior.search_keywords,
        search_descriptions: cfg.behavior.search_descriptions,
    }
}

impl Ui {
    pub fn new(ctx: Rc<AppContext>) -> Rc<Ui> {
        let config = ctx.config.borrow().clone();
        let theme = ThemeManager::new();
        theme.apply(&config);

        let window = match ctx.application() {
            Some(app) => gtk::ApplicationWindow::new(&app),
            None => gtk::ApplicationWindow::builder().build(),
        };
        // Must happen before the window is first shown.
        let kind = crate::platform::prepare_window(&window, &config.placement());
        log::info!("window surface: {kind:?}");
        window.set_hide_on_close(true);

        let icon_size = Rc::new(Cell::new(config.appearance.icon_size as i32));
        let show_desc = Rc::new(Cell::new(config.appearance.show_descriptions));

        let ui = Rc::new_cyclic(|weak: &Weak<Ui>| {
            let w = weak.clone();
            let launcher = LauncherView::new(
                icon_size.clone(),
                show_desc.clone(),
                Rc::new(move |pos, widget, x, y| {
                    if let Some(ui) = w.upgrade() {
                        ui.on_row_context(pos, widget, x, y);
                    }
                }),
            );
            let w = weak.clone();
            let hidden = HiddenView::new(
                icon_size.clone(),
                Rc::new(move |id, name, installed| {
                    let (w, id, name) = (w.clone(), id.to_owned(), name.to_owned());
                    // Deferred: the clicked button's row is rebuilt by the refresh.
                    glib::idle_add_local_once(move || {
                        if let Some(ui) = w.upgrade() {
                            ui.restore_hidden(&id, &name, installed);
                        }
                    });
                }),
            );

            let stack = gtk::Stack::builder().hhomogeneous(true).vhomogeneous(true).vexpand(true).build();
            stack.add_named(&launcher.root, Some(PAGE_LAUNCHER));
            stack.add_named(&hidden.root, Some(PAGE_HIDDEN));
            stack.set_visible_child_name(PAGE_LAUNCHER);

            let banner = Banner::new();
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.add_css_class("launcher-card");
            card.set_size_request(config.window.width, config.window.height);
            card.append(&stack);
            card.append(&banner.widget);
            window.set_child(Some(&card));

            let settings_menu = gio::Menu::new();
            launcher.menu_button.set_menu_model(Some(&settings_menu));

            Ui {
                this: weak.clone(),
                ctx: ctx.clone(),
                window: window.clone(),
                theme,
                usage: RefCell::new(UsageTracker::load()),
                engine: RefCell::new(SearchEngine::default()),
                keymap: RefCell::new(KeyMap::from_config(&config.keybindings)),
                launcher,
                hidden,
                stack,
                banner,
                settings_menu,
                icon_size: icon_size.clone(),
                show_desc: show_desc.clone(),
                settings_open: Cell::new(false),
                context_open: Cell::new(false),
                grace_until: Cell::new(None),
                context_target: RefCell::new(None),
                context_popover: RefCell::new(None),
            }
        });

        ui.wire();
        ui.apply_config();
        ui.rebuild_index();
        ui.run_query(false);
        ui.hidden.refresh(&ui.ctx.store);
        ui.show_startup_messages();
        ui
    }

    // ---- public contract ---------------------------------------------------

    pub fn present_launcher(&self) {
        self.show_launcher_page();
        self.launcher.entry.set_text("");
        self.run_query(false);
        self.present_window();
    }

    pub fn present_hidden_manager(&self) {
        self.show_hidden_page();
        self.present_window();
        self.hidden.entry.grab_focus();
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.dismiss();
        } else {
            self.present_launcher();
        }
    }

    // ---- wiring ------------------------------------------------------------

    fn wire(&self) {
        let weak = self.this.clone();

        // Shortcuts: capture phase so they work whichever widget has focus.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = weak.clone();
            keys.connect_key_pressed(move |_, key, _code, state| match weak.upgrade() {
                Some(ui) => ui.on_key(key, state),
                None => glib::Propagation::Proceed,
            });
        }
        self.window.add_controller(keys);

        self.launcher.entry.connect_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.run_query(false);
                }
            }
        });
        self.launcher.list.connect_activate({
            let weak = weak.clone();
            move |_, pos| {
                if let Some(ui) = weak.upgrade() {
                    if let Some(app) = ui.launcher.entry_at(pos) {
                        ui.launch(app);
                    }
                }
            }
        });

        self.hidden.entry.connect_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.hidden.refresh(&ui.ctx.store);
                }
            }
        });
        self.hidden.back.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.show_launcher_page();
                }
            }
        });
        self.hidden.restore_all.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    if ui.hidden.click_restore_all() {
                        ui.restore_all();
                    }
                }
            }
        });

        // Live updates from the store (hide/unhide, installs, CLI changes).
        self.ctx.store.connect_changed({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.on_store_changed();
                }
            }
        });

        // Close on focus loss.
        self.window.connect_is_active_notify({
            let weak = weak.clone();
            move |w| {
                log::debug!("window active = {}", w.is_active());
                if w.is_active() {
                    return;
                }
                if let Some(ui) = weak.upgrade() {
                    ui.on_focus_lost();
                }
            }
        });
        self.window.connect_map(|_| log::debug!("window mapped"));
        self.window.connect_unmap(|_| log::debug!("window unmapped"));
        self.window.connect_close_request({
            let weak = weak.clone();
            move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.dismiss();
                }
                glib::Propagation::Stop
            }
        });

        // Settings popover bookkeeping.
        self.launcher.menu_button.connect_active_notify({
            let weak = weak.clone();
            move |b| {
                if let Some(ui) = weak.upgrade() {
                    ui.settings_open.set(b.is_active());
                    ui.grace_until.set(Some(Instant::now() + FOCUS_GRACE));
                    if !b.is_active() {
                        ui.focus_current_entry();
                    }
                }
            }
        });
        if let Some(pop) = self.launcher.menu_button.popover() {
            pop.set_has_arrow(false);
            pop.add_css_class("arch-launcher");
        }

        // Actions: "ui.*" (settings menu) and "ctx.*" (row context menu).
        let ui_group = gio::SimpleActionGroup::new();
        let ctx_group = gio::SimpleActionGroup::new();
        let add = |group: &gio::SimpleActionGroup, name: &str, f: fn(&Ui)| {
            let action = gio::SimpleAction::new(name, None);
            let weak = weak.clone();
            action.connect_activate(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    f(&ui);
                }
            });
            group.add_action(&action);
        };
        add(&ui_group, "hidden", Ui::show_hidden_page);
        add(&ui_group, "edit", Ui::edit_config);
        add(&ui_group, "reload", Ui::reload_config);
        add(&ui_group, "about", Ui::show_about);
        add(&ctx_group, "open", Ui::context_open_app);
        add(&ctx_group, "hide", Ui::context_hide_app);
        self.window.insert_action_group("ui", Some(&ui_group));
        self.window.insert_action_group("ctx", Some(&ctx_group));
    }

    // ---- configuration -----------------------------------------------------

    /// Applies the current `ctx.config` to theme, keymap, geometry and menus.
    fn apply_config(&self) {
        let cfg = self.ctx.config.borrow().clone();
        self.theme.apply(&cfg);

        let animated = cfg.appearance.animations;
        if animated {
            self.window.remove_css_class(NO_ANIMATIONS_CLASS);
        } else {
            self.window.add_css_class(NO_ANIMATIONS_CLASS);
        }
        self.stack.set_transition_duration(180);
        self.stack.set_transition_type(if animated {
            gtk::StackTransitionType::SlideLeftRight
        } else {
            gtk::StackTransitionType::None
        });
        self.banner.set_animated(animated);

        self.icon_size.set(cfg.appearance.icon_size as i32);
        self.show_desc.set(cfg.appearance.show_descriptions);

        if let Some(card) = self.window.child() {
            card.set_size_request(cfg.window.width, cfg.window.height);
        }
        self.window.set_default_size(cfg.window.width, cfg.window.height);
        update_layer_placement(&self.window, &cfg);

        *self.keymap.borrow_mut() = KeyMap::from_config(&cfg.keybindings);
        self.rebuild_settings_menu();

        let keymap = self.keymap.borrow();
        let hints: Vec<String> = [(Action::HideSelected, "ocultar"), (Action::HiddenManager, "ocultos")]
            .into_iter()
            .filter_map(|(a, what)| keymap.label(a).map(|l| format!("{l} {what}")))
            .collect();
        self.launcher.hint.set_text(&hints.join("  ·  "));
    }

    fn rebuild_settings_menu(&self) {
        let m = &self.settings_menu;
        m.remove_all();
        let hidden_label = match self.keymap.borrow().label(Action::HiddenManager) {
            Some(l) => format!("Aplicativos ocultos ({l})"),
            None => "Aplicativos ocultos".to_owned(),
        };
        m.append(Some(&hidden_label), Some("ui.hidden"));
        m.append(Some("Editar configuração"), Some("ui.edit"));
        m.append(Some("Recarregar configuração"), Some("ui.reload"));
        m.append(Some("Sobre"), Some("ui.about"));
    }

    fn reload_config(&self) {
        let (cfg, warnings) = Config::load();
        for w in &warnings {
            log::warn!("config: {w}");
        }
        *self.ctx.config.borrow_mut() = cfg;
        self.apply_config();
        self.rebuild_index();
        self.run_query(true);
        self.hidden.refresh(&self.ctx.store);
        if warnings.is_empty() {
            self.banner.info("Configuração recarregada");
        } else {
            let text = format!(
                "Configuração recarregada com {}: {}",
                plural(warnings.len(), "aviso", "avisos"),
                warnings[0]
            );
            self.banner.show(&text, BannerKind::Error, None, Some(Duration::from_secs(12)));
        }
    }

    fn edit_config(&self) {
        let path = crate::paths::config_file();
        if !path.exists() {
            if let Err(e) = crate::atomic::write_atomic(&path, default_config_toml().as_bytes()) {
                self.banner.error(&format!("Não foi possível criar {}: {e}", path.display()));
                return;
            }
        }
        let uri = gio::File::for_path(&path).uri();
        let launch_ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context());
        match gio::AppInfo::launch_default_for_uri(&uri, launch_ctx.as_ref()) {
            Ok(()) => self.dismiss(),
            Err(e) => self.banner.error(&format!("Não foi possível abrir {}: {e}", path.display())),
        }
    }

    fn show_about(&self) {
        let text = format!(
            "Arch Launcher {} — launcher de aplicativos em GTK4. Aplicativos ocultos somem só daqui, sem alterar os arquivos .desktop.",
            env!("CARGO_PKG_VERSION")
        );
        self.banner.show(&text, BannerKind::Info, None, Some(Duration::from_secs(9)));
    }

    fn show_startup_messages(&self) {
        if let Some(w) = self.ctx.store.take_load_warning() {
            let text = match w {
                LoadWarning::Corrupt { backup, message } => match backup {
                    Some(b) => format!(
                        "hidden-apps.toml estava corrompido ({message}); começando sem ocultos. Cópia em {}",
                        b.display()
                    ),
                    None => format!("hidden-apps.toml estava corrompido ({message}); começando sem ocultos."),
                },
                LoadWarning::Unreadable { message } => format!("Não foi possível ler hidden-apps.toml: {message}"),
            };
            self.banner.show(&text, BannerKind::Error, None, None);
            return;
        }
        let (_, warnings) = Config::load();
        if let Some(first) = warnings.first() {
            let text = format!("Configuração com {}: {first}", plural(warnings.len(), "aviso", "avisos"));
            self.banner.show(&text, BannerKind::Error, None, None);
        }
    }

    // ---- search ------------------------------------------------------------

    fn rebuild_index(&self) {
        // Invariant: the index is fed ONLY with visible (non-hidden) apps.
        *self.engine.borrow_mut() = SearchEngine::new(&self.ctx.store.visible_apps());
    }

    /// Runs the current query. With `preserve`, the selection stays on the
    /// same app (or the same index if that app is gone, e.g. just hidden).
    fn run_query(&self, preserve: bool) {
        let opts = search_options(&self.ctx.config.borrow());
        let text = self.launcher.entry.text();
        let prev_id = preserve.then(|| self.launcher.selected_entry().map(|e| e.id.clone())).flatten();
        let prev_idx = preserve.then(|| self.launcher.selected_index()).flatten();

        let results = {
            let usage = self.usage.borrow();
            self.engine.borrow().search(&text, &opts, &|id| usage.score(id))
        };
        let idx = prev_id
            .and_then(|id| results.iter().position(|e| e.id == id))
            .or(prev_idx.map(|i| i as usize))
            .unwrap_or(0);
        self.launcher.set_results(&results, idx as u32);
        self.launcher.count.set_text(&plural(results.len(), "aplicativo", "aplicativos"));
    }

    fn on_store_changed(&self) {
        self.rebuild_index();
        self.run_query(true);
        self.hidden.refresh(&self.ctx.store);
    }

    // ---- window state ------------------------------------------------------

    fn popover_open(&self) -> bool {
        self.settings_open.get() || self.context_open.get()
    }

    fn present_window(&self) {
        self.window.present();
        self.focus_current_entry();
    }

    fn focus_current_entry(&self) {
        if self.stack.visible_child_name().as_deref() == Some(PAGE_HIDDEN) {
            self.hidden.entry.grab_focus();
        } else {
            self.launcher.entry.grab_focus();
        }
    }

    fn on_hidden_page(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some(PAGE_HIDDEN)
    }

    fn show_launcher_page(&self) {
        self.hidden.reset_confirm();
        self.stack.set_visible_child_name(PAGE_LAUNCHER);
        self.run_query(true);
        self.launcher.entry.grab_focus();
    }

    fn show_hidden_page(&self) {
        self.hidden.reset_confirm();
        self.hidden.entry.set_text("");
        self.hidden.refresh(&self.ctx.store);
        self.stack.set_visible_child_name(PAGE_HIDDEN);
        self.hidden.entry.grab_focus();
    }

    fn hide_window(&self) {
        self.close_context_menu();
        self.window.set_visible(false);
    }

    /// The user closed the launcher (Esc, focus loss, toggle, window close).
    fn dismiss(&self) {
        self.hide_window();
        self.banner.hide();
        self.ctx.dismissed();
    }

    fn on_focus_lost(&self) {
        if !self.window.is_visible() || !self.ctx.config.borrow().window.close_on_focus_loss || self.popover_open() {
            return;
        }
        if self.grace_until.get().is_some_and(|t| Instant::now() < t) {
            return;
        }
        log::debug!("focus lost: closing");
        self.dismiss();
    }

    // ---- keyboard ----------------------------------------------------------

    fn on_key(&self, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> glib::Propagation {
        // While a popover (context / settings menu) is open it owns the keyboard.
        if self.popover_open() {
            return glib::Propagation::Proceed;
        }
        let Some(action) = self.keymap.borrow().lookup(key, state) else {
            return glib::Propagation::Proceed;
        };
        let hidden_page = self.on_hidden_page();
        match (hidden_page, action) {
            (true, Action::Close) => self.show_launcher_page(),
            (false, Action::Close) => self.dismiss(),

            (false, Action::Launch) => {
                if let Some(app) = self.launcher.selected_entry() {
                    self.launch(app);
                }
            }
            (false, Action::Next) => self.launcher.move_selection(1),
            (false, Action::Previous) => self.launcher.move_selection(-1),
            (false, Action::HideSelected) => {
                if let Some(app) = self.launcher.selected_entry() {
                    self.hide_app(&app);
                }
            }
            (false, Action::ContextMenu) => self.context_menu_for_selected(),
            (false, Action::HiddenManager) => self.show_hidden_page(),
            (false, Action::ClearSearch) => self.launcher.entry.set_text(""),

            (true, Action::Launch) => {
                if let Some((id, name, installed)) = self.hidden.selected() {
                    self.restore_hidden(&id, &name, installed);
                }
            }
            (true, Action::Next) => self.hidden.move_selection(1),
            (true, Action::Previous) => self.hidden.move_selection(-1),
            (true, Action::HiddenManager) => self.show_launcher_page(),
            (true, Action::ClearSearch) => self.hidden.entry.set_text(""),
            (true, Action::HideSelected | Action::ContextMenu) => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    // ---- actions on apps ---------------------------------------------------

    fn launch(&self, app: Rc<AppEntry>) {
        self.usage.borrow_mut().record(&app.id);
        let weak = self.this.clone();
        let name = app.name.clone();
        // Launch FIRST (the activation token is captured synchronously), then hide.
        self.ctx.launch(&app, move |err| {
            if let Some(ui) = weak.upgrade() {
                ui.show_launcher_page();
                ui.present_window();
                ui.banner.error(&format!("Não foi possível abrir “{name}”: {err}"));
            }
        });
        self.hide_window();
    }

    /// Hides an app from this launcher only, with an "undo" banner.
    fn hide_app(&self, app: &AppEntry) {
        match self.ctx.store.hide(&app.id) {
            Ok(_) => {
                // The store event already refreshed both lists.
                let weak = self.this.clone();
                let id = app.id.clone();
                let undo: Rc<dyn Fn()> = Rc::new(move || {
                    if let Some(ui) = weak.upgrade() {
                        if let Err(e) = ui.ctx.store.unhide(&id) {
                            ui.banner.error(&format!("Não foi possível desfazer: {e}"));
                        }
                        ui.focus_current_entry();
                    }
                });
                self.banner.show(
                    &format!("“{}” foi ocultado", app.name),
                    BannerKind::Info,
                    Some(("Desfazer", undo)),
                    Some(Duration::from_secs(6)),
                );
            }
            Err(e) => self.banner.error(&format!("Não foi possível ocultar “{}”: {e}", app.name)),
        }
    }

    fn restore_hidden(&self, id: &str, name: &str, installed: bool) {
        match self.ctx.store.unhide(id) {
            Ok(_) if installed => self.banner.info(&format!("“{name}” foi reexibido")),
            Ok(_) => self.banner.info(&format!("ID esquecido: {id}")),
            Err(e) => self.banner.error(&format!("Não foi possível reexibir “{name}”: {e}")),
        }
    }

    fn restore_all(&self) {
        match self.ctx.store.unhide_all() {
            Ok(n) => self.banner.info(&format!("{} reexibido(s)", plural(n, "item", "itens"))),
            Err(e) => self.banner.error(&format!("Não foi possível reexibir todos: {e}")),
        }
    }

    // ---- context menu ------------------------------------------------------

    fn on_row_context(&self, pos: u32, widget: &gtk::Widget, x: f64, y: f64) {
        let Some(app) = self.launcher.entry_at(pos) else { return };
        self.launcher.selection.set_selected(pos);
        let point = widget
            .compute_point(&self.launcher.list, &gtk::graphene::Point::new(x as f32, y as f32))
            .unwrap_or_else(|| gtk::graphene::Point::new(x as f32, y as f32));
        let rect = gtk::gdk::Rectangle::new(point.x() as i32, point.y() as i32, 1, 1);
        self.show_context_menu(app, &rect);
    }

    fn context_menu_for_selected(&self) {
        let Some(app) = self.launcher.selected_entry() else { return };
        let rect = self
            .launcher
            .selected_row_widget()
            .and_then(|w| w.compute_bounds(&self.launcher.list))
            .map(|b| gtk::gdk::Rectangle::new(b.x() as i32, b.y() as i32, b.width() as i32, b.height() as i32))
            .unwrap_or_else(|| gtk::gdk::Rectangle::new(0, 0, 1, 1));
        self.show_context_menu(app, &rect);
    }

    fn show_context_menu(&self, app: Rc<AppEntry>, rect: &gtk::gdk::Rectangle) {
        self.close_context_menu();
        *self.context_target.borrow_mut() = Some(app);

        let menu = gio::Menu::new();
        menu.append(Some("Abrir"), Some("ctx.open"));
        menu.append(Some("Ocultar aplicativo"), Some("ctx.hide"));
        let pop = gtk::PopoverMenu::from_model(Some(&menu));
        pop.set_has_arrow(false);
        pop.set_position(gtk::PositionType::Bottom);
        pop.add_css_class("arch-launcher");
        pop.set_parent(&self.launcher.list);
        pop.set_pointing_to(Some(rect));
        let weak = self.this.clone();
        pop.connect_closed(move |p| {
            let p = p.clone();
            glib::idle_add_local_once(move || {
                if p.parent().is_some() {
                    p.unparent();
                }
            });
            if let Some(ui) = weak.upgrade() {
                ui.context_open.set(false);
                ui.grace_until.set(Some(Instant::now() + FOCUS_GRACE));
                ui.focus_current_entry();
            }
        });
        self.context_open.set(true);
        *self.context_popover.borrow_mut() = Some(pop.clone());
        pop.popup();
    }

    fn close_context_menu(&self) {
        let pop = self.context_popover.borrow_mut().take();
        if let Some(pop) = pop {
            pop.popdown();
            if pop.parent().is_some() {
                pop.unparent();
            }
        }
        self.context_open.set(false);
    }

    fn take_context_target(&self) -> Option<Rc<AppEntry>> {
        let target = self.context_target.borrow_mut().take();
        self.close_context_menu();
        target
    }

    fn context_open_app(&self) {
        if let Some(app) = self.take_context_target() {
            self.launch(app);
        }
    }

    fn context_hide_app(&self) {
        if let Some(app) = self.take_context_target() {
            self.hide_app(&app);
            self.focus_current_entry();
        }
    }
}

/// Re-applies anchor/margin to an existing layer-shell surface (live reload).
#[cfg(feature = "layer-shell")]
fn update_layer_placement(window: &gtk::ApplicationWindow, cfg: &Config) {
    use gtk4_layer_shell::{Edge, LayerShell};
    if !crate::platform::is_layer_surface(window) {
        return;
    }
    match cfg.window.anchor {
        crate::config::AnchorSetting::Center => window.set_anchor(Edge::Top, false),
        crate::config::AnchorSetting::Top => {
            window.set_anchor(Edge::Top, true);
            window.set_margin(Edge::Top, cfg.window.margin_top);
        }
    }
}

#[cfg(not(feature = "layer-shell"))]
fn update_layer_placement(_window: &gtk::ApplicationWindow, _cfg: &Config) {}
