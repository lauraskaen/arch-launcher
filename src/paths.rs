//! Locations of every file the launcher reads or writes.
//!
//! All paths follow the XDG Base Directory specification. For tests and for
//! running several isolated profiles, `ARCH_LAUNCHER_CONFIG_DIR` and
//! `ARCH_LAUNCHER_STATE_DIR` override the computed directories.

use std::path::{Path, PathBuf};

/// Name of the per-user sub-directory under `$XDG_CONFIG_HOME` / `$XDG_STATE_HOME`.
pub const APP_DIR_NAME: &str = "arch-launcher";

/// GApplication identifier (also used as the Wayland `app_id` / X11 `WM_CLASS`).
pub const APP_ID: &str = "io.github.archlauncher.ArchLauncher";

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Reads an XDG base-directory variable. Per the spec, relative values are
/// invalid and must be ignored.
fn xdg_var(name: &str, fallback: &[&str]) -> PathBuf {
    match std::env::var_os(name).map(PathBuf::from) {
        Some(p) if p.is_absolute() => p,
        _ => fallback.iter().fold(home(), |acc, part| acc.join(part)),
    }
}

pub fn xdg_config_home() -> PathBuf {
    xdg_var("XDG_CONFIG_HOME", &[".config"])
}

pub fn xdg_state_home() -> PathBuf {
    xdg_var("XDG_STATE_HOME", &[".local", "state"])
}

pub fn xdg_data_home() -> PathBuf {
    xdg_var("XDG_DATA_HOME", &[".local", "share"])
}

/// `~/.config/arch-launcher`
pub fn config_dir() -> PathBuf {
    match std::env::var_os("ARCH_LAUNCHER_CONFIG_DIR") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => xdg_config_home().join(APP_DIR_NAME),
    }
}

/// `~/.local/state/arch-launcher` (usage history lives here: it is state, not config).
pub fn state_dir() -> PathBuf {
    match std::env::var_os("ARCH_LAUNCHER_STATE_DIR") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => xdg_state_home().join(APP_DIR_NAME),
    }
}

/// `~/.config/arch-launcher/config.toml`
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// `~/.config/arch-launcher/hidden-apps.toml`
pub fn hidden_apps_file() -> PathBuf {
    config_dir().join("hidden-apps.toml")
}

/// `~/.config/arch-launcher/style.css` — optional user CSS loaded last.
pub fn user_css_file() -> PathBuf {
    config_dir().join("style.css")
}

/// `~/.local/state/arch-launcher/usage.toml`
pub fn usage_file() -> PathBuf {
    state_dir().join("usage.toml")
}

/// Returns `true` when `child` is located inside `parent` (purely lexical).
pub fn is_within(child: &Path, parent: &Path) -> bool {
    child.starts_with(parent)
}
