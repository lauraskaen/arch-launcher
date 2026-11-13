//! Application lifecycle: single instance, toggling, resident (daemon) mode.
//!
//! `arch-launcher` is a `GtkApplication` with a fixed D-Bus id, so a second
//! invocation does not start a second process: it forwards an action to the
//! running instance and exits immediately. Binding `arch-launcher` to Super+D
//! therefore *toggles* the launcher.
//!
//! * Default mode: the process exits as soon as the launcher is dismissed or
//!   an app has been started — nothing stays in the background.
//! * `--daemon`: the process stays resident with the window hidden, making the
//!   next opening instantaneous (useful on slow disks).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use crate::config::Config;
use crate::discovery::AppEntry;
use crate::launcher::{self, LaunchError};
use crate::platform::Backend;
use crate::store::AppStore;
use crate::ui::Ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    Launcher,
    HiddenManager,
}

/// Shared state handed to the UI layer.
pub struct AppContext {
    app: glib::WeakRef<gtk::Application>,
    pub store: AppStore,
    pub config: RefCell<Config>,
    pub backend: Backend,
    /// `true` with `--daemon`: hide instead of quitting.
    pub resident: bool,
    launches_in_flight: Cell<u32>,
}

impl AppContext {
    pub fn application(&self) -> Option<gtk::Application> {
        self.app.upgrade()
    }

    /// Called by the UI after it hid its windows because the user dismissed
    /// the launcher. Quits unless resident or an app launch is still in flight.
    pub fn dismissed(&self) {
        if !self.resident && self.launches_in_flight.get() == 0 {
            if let Some(app) = self.application() {
                app.quit();
            }
        }
    }

    /// Launches an application through GIO (never through a shell).
    ///
    /// The XDG activation token is captured before this returns, so the UI
    /// should hide its window *after* calling this. On success a
    /// non-resident launcher quits; on failure `on_error` runs so the UI can
    /// re-present itself with a message.
    pub fn launch(self: &Rc<Self>, entry: &AppEntry, on_error: impl FnOnce(LaunchError) + 'static) {
        let launch_ctx = gtk::gdk::Display::default().map(|d| d.app_launch_context().upcast::<gio::AppLaunchContext>());
        let guard = self.application().map(|a| a.hold());
        self.launches_in_flight.set(self.launches_in_flight.get() + 1);
        let this = Rc::clone(self);
        let id = entry.id.clone();
        launcher::launch_with_callback(entry, launch_ctx.as_ref(), move |res| {
            this.launches_in_flight.set(this.launches_in_flight.get().saturating_sub(1));
            match res {
                Ok(()) => {
                    log::info!("launched {id}");
                    drop(guard);
                    this.dismissed();
                }
                Err(e) => {
                    log::error!("failed to launch {id}: {e}");
                    drop(guard);
                    on_error(e);
                }
            }
        });
    }
}

/// Development aid: with `ARCH_LAUNCHER_DEBUG_SNAPSHOT=/path/file.png`, the
/// visible window is rendered by GTK itself into a PNG ~1 s after it opens and
/// the launcher quits. Used to check the look without a screenshot tool.
/// `ARCH_LAUNCHER_DEBUG_QUERY` optionally types a search first.
fn schedule_debug_snapshot() {
    let Some(path) = std::env::var_os("ARCH_LAUNCHER_DEBUG_SNAPSHOT") else { return };
    glib::timeout_add_local_once(std::time::Duration::from_millis(400), || {
        if let (Ok(q), Some(win)) = (std::env::var("ARCH_LAUNCHER_DEBUG_QUERY"), active_window()) {
            if let Some(entry) = find_focus_entry(&win) {
                entry.set_text(&q);
            }
        }
    });
    glib::timeout_add_local_once(std::time::Duration::from_millis(1000), move || {
        let Some(win) = active_window() else { return };
        let (w, h) = (win.width(), win.height());
        let paintable = gtk::WidgetPaintable::new(Some(&win));
        let snap = gtk::Snapshot::new();
        paintable.snapshot(&snap, f64::from(w), f64::from(h));
        let saved = snap
            .to_node()
            .zip(win.renderer())
            .map(|(node, r)| r.render_texture(&node, None).save_to_png(&path));
        eprintln!("debug snapshot ({w}x{h}): {saved:?}");
        if let Some(app) = win.application() {
            app.quit();
        }
    });
}

fn active_window() -> Option<gtk::Window> {
    gio::Application::default()
        .and_downcast::<gtk::Application>()
        .and_then(|a| a.windows().into_iter().find(|w| w.is_visible()))
}

fn find_focus_entry(win: &gtk::Window) -> Option<gtk::Editable> {
    gtk::prelude::GtkWindowExt::focus(win).and_then(|f| {
        f.downcast_ref::<gtk::Text>()
            .and_then(|t| t.parent())
            .or(Some(f))
            .and_then(|w| w.dynamic_cast::<gtk::Editable>().ok())
    })
}

/// Runs the GUI. Returns the process exit code.
pub fn run(mode: StartMode, resident: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(crate::paths::APP_ID)
        .flags(gio::ApplicationFlags::empty())
        .build();

    let ui: Rc<RefCell<Option<Rc<Ui>>>> = Rc::default();
    // Mode requested by *this* invocation for the first activation.
    let first_mode: Rc<Cell<Option<StartMode>>> = Rc::new(Cell::new(Some(mode)));

    {
        let ui = ui.clone();
        app.connect_startup(move |app| {
            let (config, warnings) = Config::load();
            for w in &warnings {
                log::warn!("config: {w}");
            }
            let store = AppStore::from_env();
            store.watch();
            let ctx = Rc::new(AppContext {
                app: app.downgrade(),
                store,
                config: RefCell::new(config),
                backend: Backend::detect(),
                resident,
                launches_in_flight: Cell::new(0),
            });
            log::info!(
                "startup: backend={}, apps={}, hidden={}",
                ctx.backend.label(),
                ctx.store.visible_apps().len(),
                ctx.store.hidden_count()
            );
            *ui.borrow_mut() = Some(Ui::new(ctx));
            if resident {
                // Keep the process alive with no visible window.
                std::mem::forget(app.hold());
            }
        });
    }

    {
        let ui = ui.clone();
        let first_mode = first_mode.clone();
        app.connect_activate(move |_| {
            let Some(ui) = ui.borrow().clone() else { return };
            match first_mode.take() {
                // A daemon started without an explicit request stays hidden.
                Some(StartMode::Launcher) if resident => {}
                Some(StartMode::Launcher) => ui.present_launcher(),
                Some(StartMode::HiddenManager) => ui.present_hidden_manager(),
                None => ui.toggle(),
            }
            schedule_debug_snapshot();
        });
    }

    // Remote-callable actions (`gapplication action io.github.archlauncher.ArchLauncher show-hidden`).
    let add_action = |name: &str, f: fn(&Ui)| {
        let action = gio::SimpleAction::new(name, None);
        let ui = ui.clone();
        let first_mode = first_mode.clone();
        action.connect_activate(move |_, _| {
            first_mode.set(None);
            if let Some(ui) = ui.borrow().clone() {
                f(&ui);
            }
        });
        app.add_action(&action);
    };
    add_action("show", Ui::present_launcher);
    add_action("toggle", Ui::toggle);
    add_action("show-hidden", Ui::present_hidden_manager);
    {
        let quit = gio::SimpleAction::new("quit", None);
        let weak = app.downgrade();
        quit.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.quit();
            }
        });
        app.add_action(&quit);
    }

    if let Err(e) = app.register(gio::Cancellable::NONE) {
        eprintln!("arch-launcher: cannot register application: {e}");
        return glib::ExitCode::FAILURE;
    }

    if app.is_remote() {
        // Another instance is running: forward the request and exit.
        match mode {
            StartMode::HiddenManager => app.activate_action("show-hidden", None),
            StartMode::Launcher if resident => {} // `--daemon` while one runs: nothing to do
            StartMode::Launcher => app.activate(),
        }
        // Remote calls are sent asynchronously; make sure they leave before exit.
        if let Some(conn) = app.dbus_connection() {
            let _ = conn.flush_sync(gio::Cancellable::NONE);
        }
        // The process exits now; skipping finalisation avoids GIO's
        // "did not unregister from D-Bus" warning for a remote instance.
        std::mem::forget(app);
        return glib::ExitCode::SUCCESS;
    }

    // Our own flags were already parsed; do not let GTK see them.
    app.run_with_args(&["arch-launcher"])
}
