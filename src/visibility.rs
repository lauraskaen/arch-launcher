//! Visibility Manager — the launcher's own list of hidden applications.
//!
//! Design decisions:
//! * Hiding never touches the application's `.desktop` file (no `NoDisplay=true`
//!   overrides). The app therefore disappears **only from this launcher** and
//!   stays in every other menu, and nothing needs root.
//! * Apps are identified by their XDG desktop-file ID (`org.mozilla.firefox.desktop`),
//!   which is stable across package updates, cache rebuilds and locale changes,
//!   unlike the display name.
//! * IDs of apps that are currently not installed are **kept**: if the app is
//!   reinstalled it stays hidden.
//! * Every mutation is persisted atomically before it is reported as done; on
//!   a write failure the in-memory state is rolled back so the UI can never show
//!   a state that would be lost on restart.
//!
//! File format (`~/.config/arch-launcher/hidden-apps.toml`):
//! ```toml
//! version = 1
//! hidden = ["org.mozilla.firefox.desktop", "vim.desktop"]
//! ```

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;
use crate::discovery::AppEntry;

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct HiddenAppsFile {
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    hidden: Vec<String>,
}

fn default_version() -> u32 {
    FORMAT_VERSION
}

#[derive(Debug)]
pub enum VisibilityError {
    Io(PathBuf, io::Error),
    InvalidId(String),
}

impl fmt::Display for VisibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VisibilityError::Io(p, e) => write!(f, "could not write {}: {e}", p.display()),
            VisibilityError::InvalidId(id) => write!(f, "invalid desktop-file ID: {id:?}"),
        }
    }
}

impl std::error::Error for VisibilityError {}

/// Non-fatal problem found while loading, reported to the user once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadWarning {
    /// The file could not be parsed; it was copied to `backup` and the
    /// launcher started with an empty hidden list.
    Corrupt { backup: Option<PathBuf>, message: String },
    /// The file exists but could not be read (permissions, I/O).
    Unreadable { message: String },
}

#[derive(Debug)]
pub struct VisibilityManager {
    path: PathBuf,
    hidden: BTreeSet<String>,
}

/// Validates a desktop-file ID before it is stored.
pub fn validate_id(id: &str) -> Result<(), VisibilityError> {
    let ok = !id.is_empty()
        && id.len() <= 512
        && id.ends_with(".desktop")
        && !id.contains('/')
        && !id.chars().any(char::is_control);
    if ok {
        Ok(())
    } else {
        Err(VisibilityError::InvalidId(id.to_owned()))
    }
}

impl VisibilityManager {
    /// An empty manager that will persist to `path`. Nothing is written until
    /// the first mutation.
    pub fn empty(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), hidden: BTreeSet::new() }
    }

    /// Loads the hidden list. Never fails: a missing file means "nothing
    /// hidden"; a corrupt file is backed up and reported as a warning so a
    /// later save cannot silently destroy the user's data.
    pub fn load(path: impl Into<PathBuf>) -> (Self, Option<LoadWarning>) {
        let mut mgr = Self::empty(path);
        let warning = mgr.reload();
        (mgr, warning)
    }

    /// Re-reads the file from disk (used when another process — e.g. the CLI —
    /// changed it). On error the current in-memory state is kept.
    pub fn reload(&mut self) -> Option<LoadWarning> {
        let text = match fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.hidden.clear();
                return None;
            }
            Err(e) => return Some(LoadWarning::Unreadable { message: e.to_string() }),
        };
        match parse(&text) {
            Ok(set) => {
                self.hidden = set;
                None
            }
            Err(message) => {
                let backup = backup_corrupt(&self.path);
                self.hidden.clear();
                Some(LoadWarning::Corrupt { backup, message })
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.hidden.contains(id)
    }

    /// All hidden IDs, including apps that are not currently installed.
    pub fn hidden_ids(&self) -> impl Iterator<Item = &str> {
        self.hidden.iter().map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.hidden.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hidden.is_empty()
    }

    /// Hides `id` and persists. Returns `Ok(false)` if it was already hidden.
    pub fn hide(&mut self, id: &str) -> Result<bool, VisibilityError> {
        validate_id(id)?;
        if !self.hidden.insert(id.to_owned()) {
            return Ok(false);
        }
        if let Err(e) = self.save() {
            self.hidden.remove(id);
            return Err(e);
        }
        Ok(true)
    }

    /// Makes `id` visible again and persists. Returns `Ok(false)` if it was not hidden.
    pub fn unhide(&mut self, id: &str) -> Result<bool, VisibilityError> {
        if !self.hidden.remove(id) {
            return Ok(false);
        }
        if let Err(e) = self.save() {
            self.hidden.insert(id.to_owned());
            return Err(e);
        }
        Ok(true)
    }

    /// Restores every app. Returns how many IDs were removed.
    pub fn unhide_all(&mut self) -> Result<usize, VisibilityError> {
        if self.hidden.is_empty() {
            return Ok(0);
        }
        let previous = std::mem::take(&mut self.hidden);
        if let Err(e) = self.save() {
            self.hidden = previous;
            return Err(e);
        }
        Ok(previous.len())
    }

    /// Restores only the given IDs (e.g. "show all *installed* hidden apps").
    pub fn unhide_many<'a>(
        &mut self,
        ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<usize, VisibilityError> {
        let previous = self.hidden.clone();
        let mut n = 0;
        for id in ids {
            if self.hidden.remove(id) {
                n += 1;
            }
        }
        if n == 0 {
            return Ok(0);
        }
        if let Err(e) = self.save() {
            self.hidden = previous;
            return Err(e);
        }
        Ok(n)
    }

    /// Removes IDs that do not correspond to any installed app.
    pub fn forget_missing(&mut self, installed: &[AppEntry]) -> Result<usize, VisibilityError> {
        let installed: BTreeSet<&str> = installed.iter().map(|a| a.id.as_str()).collect();
        let orphans: Vec<String> = self
            .hidden
            .iter()
            .filter(|id| !installed.contains(id.as_str()))
            .cloned()
            .collect();
        self.unhide_many(orphans.iter().map(String::as_str))
    }

    /// Splits `apps` into `(visible, hidden)`, preserving order.
    pub fn partition<'a>(&self, apps: &'a [AppEntry]) -> (Vec<&'a AppEntry>, Vec<&'a AppEntry>) {
        apps.iter().partition(|a| !self.is_hidden(&a.id))
    }

    fn save(&self) -> Result<(), VisibilityError> {
        let file = HiddenAppsFile {
            version: FORMAT_VERSION,
            hidden: self.hidden.iter().cloned().collect(),
        };
        let body = toml::to_string_pretty(&file).expect("hidden-apps serialization cannot fail");
        let text = format!(
            "# Applications hidden from arch-launcher (desktop-file IDs).\n\
             # Managed by the launcher: use Ctrl+Shift+H or `arch-launcher --unhide <id>`.\n\
             # Hiding here never modifies the application or its .desktop file.\n\n{body}"
        );
        write_atomic(&self.path, text.as_bytes())
            .map_err(|e| VisibilityError::Io(self.path.clone(), e))
    }
}

fn parse(text: &str) -> Result<BTreeSet<String>, String> {
    let file: HiddenAppsFile = toml::from_str(text).map_err(|e| e.to_string())?;
    if file.version > FORMAT_VERSION {
        // A newer launcher wrote this. Read what we understand; do not treat
        // as corrupt (that would back it up and clear it).
        log::warn!(
            "hidden-apps.toml has version {} (this build understands {FORMAT_VERSION})",
            file.version
        );
    }
    Ok(file
        .hidden
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| validate_id(s).is_ok())
        .collect())
}

fn backup_corrupt(path: &Path) -> Option<PathBuf> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.file_name()?.to_os_string();
    name.push(format!(".corrupt-{ts}"));
    let backup = path.with_file_name(name);
    fs::copy(path, &backup).ok().map(|_| backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_ids() {
        assert!(validate_id("firefox.desktop").is_ok());
        assert!(validate_id("").is_err());
        assert!(validate_id("Firefox").is_err());
        assert!(validate_id("../x.desktop").is_err());
        assert!(validate_id("a\nb.desktop").is_err());
    }

    #[test]
    fn ignores_invalid_entries_in_file() {
        let set = parse("hidden = [\"ok.desktop\", \"bad\", \"  spaced.desktop \"]").unwrap();
        assert_eq!(set.into_iter().collect::<Vec<_>>(), vec!["ok.desktop", "spaced.desktop"]);
    }

    #[test]
    fn newer_version_is_still_read() {
        let set = parse("version = 99\nhidden = [\"a.desktop\"]\nfuture = true\n").unwrap();
        assert!(set.contains("a.desktop"));
    }
}
