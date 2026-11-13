//! `AppStore` — the single source of truth the UI reads from.
//!
//! It combines Application Discovery with the Visibility Manager and keeps
//! both up to date:
//! * `applications/` directories are watched with `GFileMonitor`; changes are
//!   debounced and the rescan runs on a worker thread, so installing or
//!   removing packages never blocks the UI.
//! * `hidden-apps.toml` is watched too, so `arch-launcher --hide …` from a
//!   terminal is reflected in an open window.
//!
//! **Invariant:** `visible_apps()` never returns an app whose ID is hidden.
//! Every UI list and every search must be fed from `visible_apps()`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gio::prelude::*;

use crate::discovery::{discover, AppEntry, DiscoveryOptions};
use crate::visibility::{LoadWarning, VisibilityError, VisibilityManager};

const RESCAN_DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreEvent {
    /// The set of installed applications (or their metadata) changed.
    AppsChanged,
    /// An app was hidden or restored.
    VisibilityChanged,
}

type Listener = Rc<dyn Fn(StoreEvent)>;

struct Inner {
    opts: DiscoveryOptions,
    apps: RefCell<Vec<Rc<AppEntry>>>,
    visibility: RefCell<VisibilityManager>,
    listeners: RefCell<Vec<Listener>>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    pending_rescan: RefCell<Option<glib::SourceId>>,
    scan_generation: Cell<u64>,
    load_warning: RefCell<Option<LoadWarning>>,
}

#[derive(Clone)]
pub struct AppStore(Rc<Inner>);

impl AppStore {
    /// Performs the initial (synchronous) scan and loads the hidden list.
    pub fn new(opts: DiscoveryOptions, visibility: VisibilityManager) -> Self {
        let apps = discover(&opts).into_iter().map(Rc::new).collect();
        Self(Rc::new(Inner {
            opts,
            apps: RefCell::new(apps),
            visibility: RefCell::new(visibility),
            listeners: RefCell::default(),
            monitors: RefCell::default(),
            pending_rescan: RefCell::default(),
            scan_generation: Cell::new(0),
            load_warning: RefCell::default(),
        }))
    }

    /// Convenience constructor using the real environment and config paths.
    pub fn from_env() -> Self {
        let (vis, warning) = VisibilityManager::load(crate::paths::hidden_apps_file());
        let store = Self::new(DiscoveryOptions::from_env(), vis);
        *store.0.load_warning.borrow_mut() = warning;
        store
    }

    /// A warning produced while loading `hidden-apps.toml` (shown once by the UI).
    pub fn take_load_warning(&self) -> Option<LoadWarning> {
        self.0.load_warning.borrow_mut().take()
    }

    // ---- queries ---------------------------------------------------------

    /// Installed apps that are **not** hidden, sorted by name.
    pub fn visible_apps(&self) -> Vec<Rc<AppEntry>> {
        let vis = self.0.visibility.borrow();
        self.0
            .apps
            .borrow()
            .iter()
            .filter(|a| !vis.is_hidden(&a.id))
            .cloned()
            .collect()
    }

    /// Installed apps that are hidden, sorted by name.
    pub fn hidden_apps(&self) -> Vec<Rc<AppEntry>> {
        let vis = self.0.visibility.borrow();
        self.0
            .apps
            .borrow()
            .iter()
            .filter(|a| vis.is_hidden(&a.id))
            .cloned()
            .collect()
    }

    /// Hidden IDs whose app is not installed right now (kept so the app stays
    /// hidden if it is reinstalled).
    pub fn orphan_hidden_ids(&self) -> Vec<String> {
        let apps = self.0.apps.borrow();
        self.0
            .visibility
            .borrow()
            .hidden_ids()
            .filter(|id| !apps.iter().any(|a| a.id == *id))
            .map(str::to_owned)
            .collect()
    }

    pub fn all_apps(&self) -> Vec<Rc<AppEntry>> {
        self.0.apps.borrow().clone()
    }

    pub fn get(&self, id: &str) -> Option<Rc<AppEntry>> {
        self.0.apps.borrow().iter().find(|a| a.id == id).cloned()
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.0.visibility.borrow().is_hidden(id)
    }

    pub fn hidden_count(&self) -> usize {
        self.hidden_apps().len()
    }

    // ---- mutations -------------------------------------------------------

    /// Hides an app. Persisted before returning; listeners are notified.
    pub fn hide(&self, id: &str) -> Result<bool, VisibilityError> {
        let changed = self.0.visibility.borrow_mut().hide(id)?;
        if changed {
            self.emit(StoreEvent::VisibilityChanged);
        }
        Ok(changed)
    }

    pub fn unhide(&self, id: &str) -> Result<bool, VisibilityError> {
        let changed = self.0.visibility.borrow_mut().unhide(id)?;
        if changed {
            self.emit(StoreEvent::VisibilityChanged);
        }
        Ok(changed)
    }

    /// Restores every hidden app (installed or not).
    pub fn unhide_all(&self) -> Result<usize, VisibilityError> {
        let n = self.0.visibility.borrow_mut().unhide_all()?;
        if n > 0 {
            self.emit(StoreEvent::VisibilityChanged);
        }
        Ok(n)
    }

    // ---- change notification --------------------------------------------

    pub fn connect_changed(&self, f: impl Fn(StoreEvent) + 'static) {
        self.0.listeners.borrow_mut().push(Rc::new(f));
    }

    fn emit(&self, ev: StoreEvent) {
        // Clone first: a listener may call back into the store.
        let listeners: Vec<Listener> = self.0.listeners.borrow().clone();
        for l in listeners {
            l(ev);
        }
    }

    // ---- live updates ----------------------------------------------------

    /// Starts watching the application directories and the hidden-apps file.
    /// Requires a running GLib main loop. Idempotent.
    pub fn watch(&self) {
        if !self.0.monitors.borrow().is_empty() {
            return;
        }
        let mut monitors = Vec::new();

        for dir in self.0.opts.application_dirs() {
            // If `applications/` does not exist yet (fresh ~/.local/share),
            // watch its parent so its creation is noticed.
            let target = if dir.is_dir() {
                dir.clone()
            } else if let Some(parent) = dir.parent().filter(|p| p.is_dir()) {
                parent.to_path_buf()
            } else {
                continue;
            };
            let file = gio::File::for_path(&target);
            match file.monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
                Ok(m) => {
                    let weak = Rc::downgrade(&self.0);
                    m.connect_changed(move |_, _, _, _| {
                        if let Some(store) = upgrade(&weak) {
                            store.schedule_rescan();
                        }
                    });
                    monitors.push(m);
                }
                Err(e) => log::warn!("cannot watch {}: {e}", target.display()),
            }
        }

        let hidden_path = self.0.visibility.borrow().path().to_path_buf();
        let file = gio::File::for_path(&hidden_path);
        match file.monitor_file(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE) {
            Ok(m) => {
                let weak = Rc::downgrade(&self.0);
                m.connect_changed(move |_, _, _, ev| {
                    use gio::FileMonitorEvent as E;
                    if !matches!(ev, E::ChangesDoneHint | E::Created | E::Deleted | E::Renamed | E::MovedIn) {
                        return;
                    }
                    if let Some(store) = upgrade(&weak) {
                        store.reload_visibility();
                    }
                });
                monitors.push(m);
            }
            Err(e) => log::warn!("cannot watch {}: {e}", hidden_path.display()),
        }

        *self.0.monitors.borrow_mut() = monitors;
    }

    /// Re-reads `hidden-apps.toml`; emits only if the set actually changed
    /// (our own atomic writes also trigger the monitor).
    fn reload_visibility(&self) {
        let before: Vec<String> = self.0.visibility.borrow().hidden_ids().map(str::to_owned).collect();
        if let Some(w) = self.0.visibility.borrow_mut().reload() {
            log::warn!("hidden-apps.toml: {w:?}");
        }
        let changed = {
            let vis = self.0.visibility.borrow();
            !before.iter().map(String::as_str).eq(vis.hidden_ids())
        };
        if changed {
            self.emit(StoreEvent::VisibilityChanged);
        }
    }

    /// Debounced rescan: bursts of file events (a pacman transaction) cause a
    /// single scan.
    pub fn schedule_rescan(&self) {
        if let Some(id) = self.0.pending_rescan.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(&self.0);
        let id = glib::timeout_add_local_once(RESCAN_DEBOUNCE, move || {
            if let Some(store) = upgrade(&weak) {
                store.0.pending_rescan.borrow_mut().take();
                store.rescan();
            }
        });
        *self.0.pending_rescan.borrow_mut() = Some(id);
    }

    /// Rescans on a worker thread and swaps the result in on the main thread.
    /// Visibility preferences are untouched by a rescan.
    pub fn rescan(&self) {
        let generation = self.0.scan_generation.get() + 1;
        self.0.scan_generation.set(generation);
        let opts = self.0.opts.clone();
        let weak = Rc::downgrade(&self.0);
        glib::MainContext::default().spawn_local(async move {
            let Ok(apps) = gio::spawn_blocking(move || discover(&opts)).await else {
                log::error!("application scan panicked");
                return;
            };
            let Some(store) = upgrade(&weak) else { return };
            if store.0.scan_generation.get() != generation {
                return; // a newer scan superseded this one
            }
            store.replace_apps(apps);
        });
    }

    /// Synchronous rescan (CLI, tests).
    pub fn rescan_blocking(&self) {
        let apps = discover(&self.0.opts);
        self.replace_apps(apps);
    }

    fn replace_apps(&self, apps: Vec<AppEntry>) {
        let changed = {
            let cur = self.0.apps.borrow();
            cur.len() != apps.len() || cur.iter().zip(&apps).any(|(a, b)| **a != *b)
        };
        if changed {
            *self.0.apps.borrow_mut() = apps.into_iter().map(Rc::new).collect();
            self.emit(StoreEvent::AppsChanged);
        }
    }
}

fn upgrade(weak: &Weak<Inner>) -> Option<AppStore> {
    weak.upgrade().map(AppStore)
}
