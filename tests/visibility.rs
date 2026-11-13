//! Acceptance tests for the hidden-apps feature (the project's top priority).
//!
//! Each test builds an isolated XDG tree in a temporary directory, so they
//! never touch the real `~/.config` or the real installed applications.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use arch_launcher::discovery::{discover, DiscoveryOptions};
use arch_launcher::store::{AppStore, StoreEvent};
use arch_launcher::visibility::{LoadWarning, VisibilityManager};

struct Env {
    _tmp: tempfile::TempDir,
    data_home: PathBuf,
    system: PathBuf,
    hidden_file: PathBuf,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let data_home = tmp.path().join("home/.local/share");
        let system = tmp.path().join("usr/share");
        fs::create_dir_all(data_home.join("applications")).unwrap();
        fs::create_dir_all(system.join("applications")).unwrap();
        let hidden_file = tmp.path().join("home/.config/arch-launcher/hidden-apps.toml");
        Self { _tmp: tmp, data_home, system, hidden_file }
    }

    fn opts(&self) -> DiscoveryOptions {
        DiscoveryOptions {
            data_dirs: vec![self.data_home.clone(), self.system.clone()],
            data_home: self.data_home.clone(),
            current_desktops: vec![],
            locales: vec![],
            path_env: Some("/usr/bin:/bin".into()),
        }
    }

    /// "Installs" an application into the system data dir.
    fn install(&self, id: &str, name: &str) -> PathBuf {
        let p = self.system.join("applications").join(id);
        fs::write(
            &p,
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/usr/bin/true\nKeywords=test;\n"),
        )
        .unwrap();
        p
    }

    fn uninstall(&self, id: &str) {
        fs::remove_file(self.system.join("applications").join(id)).unwrap();
    }

    fn store(&self) -> AppStore {
        let (vis, warn) = VisibilityManager::load(&self.hidden_file);
        assert!(warn.is_none(), "unexpected load warning: {warn:?}");
        AppStore::new(self.opts(), vis)
    }
}

fn names(apps: &[std::rc::Rc<arch_launcher::discovery::AppEntry>]) -> Vec<String> {
    apps.iter().map(|a| a.name.clone()).collect()
}

fn read(p: &Path) -> Vec<u8> {
    fs::read(p).unwrap()
}

#[test]
fn installed_app_appears() {
    let env = Env::new();
    env.install("org.example.Editor.desktop", "Editor");
    let store = env.store();
    assert_eq!(names(&store.visible_apps()), vec!["Editor"]);
}

#[test]
fn hiding_removes_app_immediately_and_never_modifies_desktop_file() {
    let env = Env::new();
    let path = env.install("org.example.Editor.desktop", "Editor");
    env.install("org.example.Viewer.desktop", "Viewer");
    let before = read(&path);

    let store = env.store();
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    {
        let events = events.clone();
        store.connect_changed(move |e| events.borrow_mut().push(e));
    }

    assert!(store.hide("org.example.Editor.desktop").unwrap());
    assert_eq!(names(&store.visible_apps()), vec!["Viewer"]);
    assert_eq!(names(&store.hidden_apps()), vec!["Editor"]);
    assert_eq!(*events.borrow(), vec![StoreEvent::VisibilityChanged]);

    // Hiding twice is a no-op and does not notify again.
    assert!(!store.hide("org.example.Editor.desktop").unwrap());
    assert_eq!(events.borrow().len(), 1);

    assert_eq!(read(&path), before, "the .desktop file must never be modified");
    assert!(!String::from_utf8(read(&path)).unwrap().contains("NoDisplay"));
}

#[test]
fn hidden_state_survives_restart() {
    let env = Env::new();
    env.install("org.example.Editor.desktop", "Editor");
    env.store().hide("org.example.Editor.desktop").unwrap();

    // A brand-new process: new store, file re-read from disk.
    let store = env.store();
    assert!(store.visible_apps().is_empty());
    assert!(store.is_hidden("org.example.Editor.desktop"));

    let text = fs::read_to_string(&env.hidden_file).unwrap();
    assert!(text.contains("org.example.Editor.desktop"), "stores the XDG ID, not the name:\n{text}");
}

#[test]
fn rescan_and_metadata_updates_keep_visibility_preferences() {
    let env = Env::new();
    env.install("org.example.Editor.desktop", "Editor");
    let store = env.store();
    store.hide("org.example.Editor.desktop").unwrap();

    // Simulate a package update that renames the app and adds a new one.
    env.install("org.example.Editor.desktop", "Editor 2");
    env.install("org.example.New.desktop", "New");
    store.rescan_blocking();

    assert_eq!(names(&store.visible_apps()), vec!["New"]);
    assert_eq!(names(&store.hidden_apps()), vec!["Editor 2"]);
}

#[test]
fn uninstalled_hidden_app_stays_hidden_when_reinstalled() {
    let env = Env::new();
    env.install("org.example.Editor.desktop", "Editor");
    let store = env.store();
    store.hide("org.example.Editor.desktop").unwrap();

    env.uninstall("org.example.Editor.desktop");
    store.rescan_blocking();
    assert!(store.hidden_apps().is_empty());
    assert_eq!(store.orphan_hidden_ids(), vec!["org.example.Editor.desktop"]);

    env.install("org.example.Editor.desktop", "Editor");
    store.rescan_blocking();
    assert!(store.visible_apps().is_empty());
    assert_eq!(names(&store.hidden_apps()), vec!["Editor"]);
}

#[test]
fn unhide_restores_immediately_and_persists() {
    let env = Env::new();
    env.install("a.desktop", "Alpha");
    env.install("b.desktop", "Beta");
    let store = env.store();
    store.hide("a.desktop").unwrap();
    store.hide("b.desktop").unwrap();
    assert!(store.visible_apps().is_empty());

    assert!(store.unhide("a.desktop").unwrap());
    assert_eq!(names(&store.visible_apps()), vec!["Alpha"]);
    assert_eq!(names(&env.store().visible_apps()), vec!["Alpha"], "persisted");

    assert_eq!(store.unhide_all().unwrap(), 1);
    assert_eq!(names(&store.visible_apps()), vec!["Alpha", "Beta"]);
    assert_eq!(names(&env.store().visible_apps()), vec!["Alpha", "Beta"]);
}

#[test]
fn hidden_id_is_scoped_to_desktop_id_not_name() {
    let env = Env::new();
    env.install("firefox.desktop", "Firefox");
    env.install("org.mozilla.firefox.desktop", "Firefox");
    let store = env.store();
    store.hide("firefox.desktop").unwrap();
    let visible = store.visible_apps();
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "org.mozilla.firefox.desktop");
}

#[test]
fn user_override_with_same_id_is_still_hidden() {
    // Precedence: a user copy in ~/.local/share/applications replaces the
    // system file but has the same ID, so it must stay hidden.
    let env = Env::new();
    env.install("org.example.Editor.desktop", "Editor");
    let store = env.store();
    store.hide("org.example.Editor.desktop").unwrap();

    fs::write(
        env.data_home.join("applications/org.example.Editor.desktop"),
        "[Desktop Entry]\nType=Application\nName=My Editor\nExec=/usr/bin/true\n",
    )
    .unwrap();
    store.rescan_blocking();
    assert!(store.visible_apps().is_empty());
    assert_eq!(names(&store.hidden_apps()), vec!["My Editor"]);
}

#[test]
fn corrupt_file_is_backed_up_not_destroyed() {
    let env = Env::new();
    fs::create_dir_all(env.hidden_file.parent().unwrap()).unwrap();
    fs::write(&env.hidden_file, "hidden = [unterminated").unwrap();

    let (mut vis, warning) = VisibilityManager::load(&env.hidden_file);
    let Some(LoadWarning::Corrupt { backup: Some(backup), .. }) = warning else {
        panic!("expected a corrupt-file warning, got {warning:?}");
    };
    assert_eq!(fs::read_to_string(&backup).unwrap(), "hidden = [unterminated");
    assert!(vis.is_empty());

    // The launcher keeps working and can save again.
    vis.hide("a.desktop").unwrap();
    let (vis2, w2) = VisibilityManager::load(&env.hidden_file);
    assert!(w2.is_none());
    assert!(vis2.is_hidden("a.desktop"));
}

#[test]
fn failed_write_rolls_back_in_memory_state() {
    if unsafe_is_root() {
        eprintln!("skipping: running as root ignores directory permissions");
        return;
    }
    let env = Env::new();
    env.install("a.desktop", "Alpha");
    let dir = env.hidden_file.parent().unwrap().to_path_buf();
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();

    let store = env.store();
    let result = store.hide("a.desktop");
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(result.is_err(), "write into read-only dir must fail");
    assert!(!store.is_hidden("a.desktop"), "UI must not show an unsaved state");
    assert_eq!(names(&store.visible_apps()), vec!["Alpha"]);
}

#[test]
fn external_edit_is_picked_up_by_reload() {
    // The CLI (`arch-launcher --hide`) writes the file while a GUI is open.
    let env = Env::new();
    env.install("a.desktop", "Alpha");
    let (mut gui, _) = VisibilityManager::load(&env.hidden_file);
    let (mut cli, _) = VisibilityManager::load(&env.hidden_file);
    cli.hide("a.desktop").unwrap();
    assert!(!gui.is_hidden("a.desktop"));
    assert!(gui.reload().is_none());
    assert!(gui.is_hidden("a.desktop"));
}

#[test]
fn many_entries_scale() {
    let env = Env::new();
    for i in 0..2000 {
        env.install(&format!("app{i}.desktop"), &format!("App {i}"));
    }
    let store = env.store();
    for i in (0..2000).step_by(2) {
        store.hide(&format!("app{i}.desktop")).unwrap();
    }
    assert_eq!(store.visible_apps().len(), 1000);
    assert_eq!(store.hidden_apps().len(), 1000);
    assert_eq!(discover(&env.opts()).len(), 2000);
}

fn unsafe_is_root() -> bool {
    // Avoid a libc dependency: root can write to the 0o555 dir.
    fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m) == 0).unwrap_or(false)
}
