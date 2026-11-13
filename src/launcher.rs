//! Launcher Service — starts applications through GIO.
//!
//! Security model: the launcher never builds a shell command line.
//! `GDesktopAppInfo` parses `Exec=` with `g_shell_parse_argv` (quoting rules
//! only, no shell is spawned), expands field codes (`%f %u %i %c %k`…),
//! honours `Path=`, `Terminal=`, `StartupNotify=`, and uses D-Bus activation
//! for `DBusActivatable=true` apps. Flatpak entries already contain
//! `flatpak run …` in `Exec=` and need no special handling.
//!
//! The launch context comes from the GDK display, so Wayland apps receive an
//! `XDG_ACTIVATION_TOKEN` (focus-stealing prevention) and X11 apps a
//! `DESKTOP_STARTUP_ID`.

use std::fmt;
use std::path::Path;

use gio::prelude::*;

use crate::discovery::AppEntry;

#[derive(Debug)]
pub enum LaunchError {
    /// GIO refused the desktop file (removed since the last scan, TryExec
    /// failed, or the file became invalid).
    NotLaunchable(String),
    Gio(glib::Error),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::NotLaunchable(p) => write!(f, "application is no longer available ({p})"),
            LaunchError::Gio(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for LaunchError {}

impl From<glib::Error> for LaunchError {
    fn from(e: glib::Error) -> Self {
        LaunchError::Gio(e)
    }
}

/// Loads the `GDesktopAppInfo` for an entry's file. Loading by *filename*
/// (not by ID) guarantees we run exactly the file discovery selected.
pub fn app_info(path: &Path) -> Result<gio_unix::DesktopAppInfo, LaunchError> {
    gio_unix::DesktopAppInfo::from_filename(path)
        .ok_or_else(|| LaunchError::NotLaunchable(path.display().to_string()))
}

/// Starts launching `entry` **synchronously** (the startup-notification /
/// XDG activation token is taken from `context` before this returns, so the
/// caller may hide its window right afterwards) and reports completion via
/// `done` on the main loop.
pub fn launch_with_callback(
    entry: &AppEntry,
    context: Option<&gio::AppLaunchContext>,
    done: impl FnOnce(Result<(), LaunchError>) + 'static,
) {
    let info = match app_info(&entry.path) {
        Ok(i) => i,
        Err(e) => {
            // Defer so callers always observe the callback asynchronously.
            glib::idle_add_local_once(move || done(Err(e)));
            return;
        }
    };
    info.launch_uris_async(&[], context, gio::Cancellable::NONE, move |res| {
        done(res.map_err(LaunchError::from));
    });
}

/// Launches `entry` and resolves once GIO has finished starting it. For
/// D-Bus–activated apps this waits for the activation call to be delivered,
/// so it is safe to quit the launcher process right after the future resolves.
pub async fn launch(
    entry: &AppEntry,
    context: Option<gio::AppLaunchContext>,
) -> Result<(), LaunchError> {
    let info = app_info(&entry.path)?;
    info.launch_uris_future(&[], context.as_ref()).await?;
    Ok(())
}

/// Blocking variant for the command-line interface (`--launch`). Spins a
/// private main loop until the asynchronous launch completes.
pub fn launch_blocking(entry: &AppEntry) -> Result<(), LaunchError> {
    let ctx = glib::MainContext::new();
    let entry = entry.clone();
    ctx.with_thread_default(|| ctx.block_on(launch(&entry, None)))
        .map_err(|e| LaunchError::NotLaunchable(e.to_string()))?
}
