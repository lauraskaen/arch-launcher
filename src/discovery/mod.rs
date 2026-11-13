//! Application Discovery — finds installed applications by scanning the XDG
//! `applications/` directories and applying the Desktop Entry visibility rules.
//!
//! Precedence: directories are scanned from most to least important
//! (`$XDG_DATA_HOME` first, then `$XDG_DATA_DIRS` in order). The first file
//! found for a given desktop-file ID wins, *even if that file is `Hidden` or
//! `NoDisplay`* — this is how users and packages mask system entries.
//!
//! Discovery is pure Rust and `Send`, so it can run on a worker thread.

pub mod desktop_file;

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use desktop_file::{locale_variants, DesktopFile};

/// Maximum directory depth followed below an `applications/` dir. Guards
/// against symlink loops.
const MAX_DEPTH: usize = 6;

/// Where an entry came from. Purely informational (shown in the UI and in
/// `--list` output); visibility never depends on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppSource {
    /// `$XDG_DATA_HOME/applications` (user overrides, `.local` installs).
    User,
    /// Flatpak exports (system or per-user installation).
    Flatpak,
    /// Anything else in `$XDG_DATA_DIRS` (pacman/AUR packages, `/usr/local`).
    System,
}

impl AppSource {
    pub fn label(self) -> &'static str {
        match self {
            AppSource::User => "user",
            AppSource::Flatpak => "flatpak",
            AppSource::System => "system",
        }
    }
}

/// A launchable application, already filtered for visibility rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppEntry {
    /// Desktop-file ID, e.g. `org.mozilla.firefox.desktop` or
    /// `kde-org.kde.dolphin.desktop` for `applications/kde/org.kde.dolphin.desktop`.
    /// This is the stable identifier stored in `hidden-apps.toml`.
    pub id: String,
    /// Absolute path of the desktop file that won precedence.
    pub path: PathBuf,
    pub name: String,
    pub generic_name: Option<String>,
    pub comment: Option<String>,
    pub keywords: Vec<String>,
    pub categories: Vec<String>,
    /// Icon theme name or absolute path.
    pub icon: Option<String>,
    pub exec: Option<String>,
    pub terminal: bool,
    pub dbus_activatable: bool,
    pub source: AppSource,
}

/// Inputs to a discovery scan. `from_env()` reproduces what GIO would do;
/// tests construct it by hand to point at temporary directories.
#[derive(Debug, Clone)]
pub struct DiscoveryOptions {
    /// Data directories in precedence order (each gets `/applications` appended).
    pub data_dirs: Vec<PathBuf>,
    /// The user's data home (used only to classify `AppSource::User`).
    pub data_home: PathBuf,
    /// Desktops from `$XDG_CURRENT_DESKTOP`, for `OnlyShowIn`/`NotShowIn`.
    pub current_desktops: Vec<String>,
    /// Locale keys to try for localized values, most specific first.
    pub locales: Vec<String>,
    /// `$PATH` used to resolve `TryExec`.
    pub path_env: Option<OsString>,
}

impl DiscoveryOptions {
    pub fn from_env() -> Self {
        let data_home = crate::paths::xdg_data_home();
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();

        let mut system_dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned())
            .split(':')
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .collect();

        // Compositors often start programs without sourcing
        // /etc/profile.d/flatpak.sh, so the Flatpak export dirs may be missing
        // from XDG_DATA_DIRS. Add them (with the same precedence flatpak.sh
        // would give them: before the system dirs) when they exist.
        let flatpak_dirs = [
            home.join(".local/share/flatpak/exports/share"),
            PathBuf::from("/var/lib/flatpak/exports/share"),
        ];
        let mut prefix = Vec::new();
        for d in flatpak_dirs {
            if d.is_dir() && !system_dirs.contains(&d) {
                prefix.push(d);
            }
        }
        prefix.append(&mut system_dirs);

        let mut data_dirs = vec![data_home.clone()];
        for d in prefix {
            if !data_dirs.contains(&d) {
                data_dirs.push(d);
            }
        }

        Self {
            data_dirs,
            data_home,
            current_desktops: current_desktops_from_env(),
            locales: locales_from_env(),
            path_env: std::env::var_os("PATH"),
        }
    }

    /// The `applications/` directories that will be scanned, in precedence order.
    pub fn application_dirs(&self) -> Vec<PathBuf> {
        self.data_dirs.iter().map(|d| d.join("applications")).collect()
    }
}

fn current_desktops_from_env() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Mirrors `g_get_language_names()` closely enough for desktop files:
/// `$LANGUAGE` (a colon list) first, then the first of
/// `$LC_ALL`, `$LC_MESSAGES`, `$LANG`.
fn locales_from_env() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let messages = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty());
    // $LANGUAGE is ignored when the locale is "C", as in GNU gettext.
    let is_c = messages
        .as_deref()
        .map(|m| locale_variants(m).is_empty())
        .unwrap_or(true);
    if !is_c {
        if let Ok(lang) = std::env::var("LANGUAGE") {
            for l in lang.split(':') {
                for v in locale_variants(l) {
                    if !out.contains(&v) {
                        out.push(v);
                    }
                }
            }
        }
    }
    if let Some(m) = messages {
        for v in locale_variants(&m) {
            if !out.contains(&v) {
                out.push(v);
            }
        }
    }
    out
}

/// Scans all application directories and returns the visible entries,
/// de-duplicated by desktop-file ID and sorted by name.
pub fn discover(opts: &DiscoveryOptions) -> Vec<AppEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut apps = Vec::new();

    for apps_dir in opts.application_dirs() {
        let mut files = Vec::new();
        collect_desktop_files(&apps_dir, &apps_dir, 0, &mut files);
        files.sort();
        for (id, path) in files {
            // Record the ID *before* filtering: a hidden/NoDisplay entry with
            // higher precedence must still mask lower-precedence copies.
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(entry) = load_entry(&id, &path, opts) {
                apps.push(entry);
            }
        }
    }

    apps.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    apps
}

/// Recursively collects `(desktop_id, path)` pairs. The ID is the path
/// relative to the `applications/` dir with `/` replaced by `-`.
fn collect_desktop_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let path = entry.path();
        // `metadata` follows symlinks: Flatpak exports are symlinks.
        let Ok(meta) = fs::metadata(&path) else {
            continue; // dangling symlink
        };
        if meta.is_dir() {
            collect_desktop_files(root, &path, depth + 1, out);
        } else if meta.is_file() && path.extension().is_some_and(|e| e == "desktop") {
            if let Ok(rel) = path.strip_prefix(root) {
                let id = rel.to_string_lossy().replace('/', "-");
                out.push((id, path));
            }
        }
    }
}

/// Parses one file and applies the visibility rules. Returns `None` when the
/// entry must not be shown.
pub fn load_entry(id: &str, path: &Path, opts: &DiscoveryOptions) -> Option<AppEntry> {
    let text = fs::read_to_string(path).ok()?;
    let df = DesktopFile::parse(&text)?;
    entry_from_desktop_file(id, path, &df, opts)
}

pub fn entry_from_desktop_file(
    id: &str,
    path: &Path,
    df: &DesktopFile,
    opts: &DiscoveryOptions,
) -> Option<AppEntry> {
    if df.string("Type").as_deref() != Some("Application") {
        return None;
    }
    if df.boolean("Hidden") || df.boolean("NoDisplay") {
        return None;
    }
    if !shown_in_desktop(df, &opts.current_desktops) {
        return None;
    }
    if let Some(try_exec) = df.string("TryExec") {
        if !try_exec.is_empty() && !is_executable(&try_exec, opts.path_env.as_deref()) {
            return None;
        }
    }
    let dbus_activatable = df.boolean("DBusActivatable");
    let exec = df.string("Exec").filter(|e| !e.trim().is_empty());
    if exec.is_none() && !dbus_activatable {
        return None; // nothing to launch
    }

    let name = df
        .locale_string("Name", &opts.locales)
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())?;

    let source = classify(path, opts);

    Some(AppEntry {
        id: id.to_owned(),
        path: path.to_path_buf(),
        name,
        generic_name: df
            .locale_string("GenericName", &opts.locales)
            .filter(|s| !s.is_empty()),
        comment: df.locale_string("Comment", &opts.locales).filter(|s| !s.is_empty()),
        keywords: df.locale_list("Keywords", &opts.locales),
        categories: df.list("Categories"),
        icon: df.string("Icon").filter(|s| !s.is_empty()),
        exec,
        terminal: df.boolean("Terminal"),
        dbus_activatable,
        source,
    })
}

fn shown_in_desktop(df: &DesktopFile, desktops: &[String]) -> bool {
    let only = df.list("OnlyShowIn");
    if !only.is_empty() && !only.iter().any(|d| desktops.contains(d)) {
        return false;
    }
    let not = df.list("NotShowIn");
    !not.iter().any(|d| desktops.contains(d))
}

fn classify(path: &Path, opts: &DiscoveryOptions) -> AppSource {
    let s = path.to_string_lossy();
    if s.contains("/flatpak/exports/") {
        AppSource::Flatpak
    } else if path.starts_with(opts.data_home.join("applications")) {
        AppSource::User
    } else {
        AppSource::System
    }
}

/// `TryExec` semantics: an absolute path must be executable; a bare name is
/// searched in `$PATH`.
pub fn is_executable(program: &str, path_env: Option<&std::ffi::OsStr>) -> bool {
    fn exec_file(p: &Path) -> bool {
        fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    let p = Path::new(program);
    if p.is_absolute() {
        return exec_file(p);
    }
    if program.contains('/') {
        return false; // relative paths are not allowed by the spec
    }
    let Some(path_env) = path_env else {
        return false;
    };
    std::env::split_paths(path_env).any(|dir| exec_file(&dir.join(program)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    pub(crate) fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, body).unwrap();
        p
    }

    fn app(name: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/usr/bin/true\n{extra}")
    }

    fn opts(dirs: &[&Path]) -> DiscoveryOptions {
        DiscoveryOptions {
            data_dirs: dirs.iter().map(|d| d.to_path_buf()).collect(),
            data_home: dirs[0].to_path_buf(),
            current_desktops: vec!["KDE".into()],
            locales: vec![],
            path_env: Some("/usr/bin:/bin".into()),
        }
    }

    #[test]
    fn finds_apps_and_builds_ids_from_subdirs() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "applications/foo.desktop", &app("Foo", ""));
        write(t.path(), "applications/kde/bar.desktop", &app("Bar", ""));
        let apps = discover(&opts(&[t.path()]));
        let ids: Vec<_> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["kde-bar.desktop", "foo.desktop"]); // sorted by name: Bar, Foo
    }

    #[test]
    fn respects_precedence_and_masking() {
        let user = tempfile::tempdir().unwrap();
        let sys = tempfile::tempdir().unwrap();
        write(sys.path(), "applications/a.desktop", &app("System A", ""));
        write(user.path(), "applications/a.desktop", &app("User A", ""));
        // A Hidden user copy masks the system entry entirely.
        write(sys.path(), "applications/b.desktop", &app("B", ""));
        write(user.path(), "applications/b.desktop", &app("B", "Hidden=true"));

        let apps = discover(&opts(&[user.path(), sys.path()]));
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name, "User A");
        assert_eq!(apps[0].source, AppSource::User);
    }

    #[test]
    fn applies_visibility_keys() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "applications/nodisplay.desktop", &app("ND", "NoDisplay=true"));
        write(t.path(), "applications/only-gnome.desktop", &app("OG", "OnlyShowIn=GNOME;"));
        write(t.path(), "applications/only-kde.desktop", &app("OK", "OnlyShowIn=GNOME;KDE;"));
        write(t.path(), "applications/not-kde.desktop", &app("NK", "NotShowIn=KDE;"));
        write(t.path(), "applications/tryexec-missing.desktop", &app("TM", "TryExec=definitely-not-installed-xyz"));
        write(t.path(), "applications/tryexec-ok.desktop", &app("TO", "TryExec=sh"));
        write(t.path(), "applications/link.desktop", "[Desktop Entry]\nType=Link\nName=L\nURL=https://x\n");
        write(t.path(), "applications/noname.desktop", "[Desktop Entry]\nType=Application\nExec=true\n");
        let names: Vec<_> = discover(&opts(&[t.path()])).into_iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["OK", "TO"]);
    }

    #[test]
    fn follows_symlinks_like_flatpak_exports() {
        let t = tempfile::tempdir().unwrap();
        let real = write(t.path(), "app/files/org.example.App.desktop", &app("Example", ""));
        let export = t.path().join("flatpak/exports/share/applications");
        fs::create_dir_all(&export).unwrap();
        std::os::unix::fs::symlink(&real, export.join("org.example.App.desktop")).unwrap();
        std::os::unix::fs::symlink("/nonexistent", export.join("dangling.desktop")).unwrap();

        let data = t.path().join("flatpak/exports/share");
        let home = tempfile::tempdir().unwrap();
        let o = opts(&[home.path(), &data]);
        let apps = discover(&o);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].id, "org.example.App.desktop");
        assert_eq!(apps[0].source, AppSource::Flatpak);
    }

    #[test]
    fn dbus_activatable_without_exec_is_launchable() {
        let t = tempfile::tempdir().unwrap();
        write(
            t.path(),
            "applications/org.example.D.desktop",
            "[Desktop Entry]\nType=Application\nName=D\nDBusActivatable=true\n",
        );
        write(
            t.path(),
            "applications/broken.desktop",
            "[Desktop Entry]\nType=Application\nName=Broken\n",
        );
        let apps = discover(&opts(&[t.path()]));
        assert_eq!(apps.len(), 1);
        assert!(apps[0].dbus_activatable);
    }
}
