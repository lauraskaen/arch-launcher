use std::process::ExitCode;

use arch_launcher::app::{self, StartMode};
use arch_launcher::discovery::{discover, AppEntry, DiscoveryOptions};
use arch_launcher::visibility::{LoadWarning, VisibilityManager};
use arch_launcher::{config, launcher, paths};

const HELP: &str = "\
arch-launcher — fast GTK4 application launcher

USAGE:
    arch-launcher                 Toggle the launcher window (bind this to Super+D)
    arch-launcher --hidden-apps   Open the \"Hidden applications\" manager
    arch-launcher --daemon        Stay resident in the background for instant opening

APP VISIBILITY (no GUI; changes are picked up live by a running launcher):
    --list [--all]                List visible apps (ID, source, name); --all adds hidden ones
    --list-hidden                 List hidden app IDs (marks the ones not installed)
    --hide <ID|NAME>              Hide an app from the launcher (its .desktop file is untouched)
    --unhide <ID|NAME>            Show a hidden app again
    --unhide-all                  Show every hidden app again

OTHER:
    --launch <ID|NAME>            Launch an app exactly as the GUI would
    --print-default-config        Print a fully commented default config.toml
    --paths                       Show the config/state files in use
    -h, --help                    Show this help
    -V, --version                 Show the version

Logging: RUST_LOG=info arch-launcher
";

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter().map(String::as_str);
    let Some(first) = it.next() else {
        return gui(StartMode::Launcher, false);
    };
    let rest: Vec<&str> = it.collect();

    match (first, rest.as_slice()) {
        ("-h" | "--help", []) => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        ("-V" | "--version", []) => {
            println!("arch-launcher {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ("--hidden-apps" | "-H", []) => gui(StartMode::HiddenManager, false),
        ("--daemon", []) => gui(StartMode::Launcher, true),
        ("--list", []) => list(false),
        ("--list", ["--all"]) => list(true),
        ("--list-hidden", []) => list_hidden(),
        ("--hide", [q]) => hide(q),
        ("--unhide", [q]) => unhide(q),
        ("--unhide-all", []) => unhide_all(),
        ("--launch", [q]) => launch(q),
        ("--print-default-config", []) => {
            print!("{}", config::default_config_toml());
            ExitCode::SUCCESS
        }
        ("--paths", []) => {
            println!("config:       {}", paths::config_file().display());
            println!("hidden apps:  {}", paths::hidden_apps_file().display());
            println!("user css:     {}", paths::user_css_file().display());
            println!("usage:        {}", paths::usage_file().display());
            for d in DiscoveryOptions::from_env().application_dirs() {
                println!("apps dir:     {}", d.display());
            }
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("arch-launcher: invalid arguments: {}\n\n{HELP}", args.join(" "));
            ExitCode::from(2)
        }
    }
}

fn gui(mode: StartMode, resident: bool) -> ExitCode {
    let code = app::run(mode, resident);
    ExitCode::from(code.get() as u8)
}

fn load_visibility() -> VisibilityManager {
    let (vis, warning) = VisibilityManager::load(paths::hidden_apps_file());
    match warning {
        Some(LoadWarning::Corrupt { backup, message }) => eprintln!(
            "warning: {} was invalid ({message}); backup: {}",
            vis.path().display(),
            backup.map(|b| b.display().to_string()).unwrap_or_else(|| "none".into())
        ),
        Some(LoadWarning::Unreadable { message }) => {
            eprintln!("warning: cannot read {}: {message}", vis.path().display())
        }
        None => {}
    }
    vis
}

/// Resolves a user-supplied app reference: exact ID, ID without `.desktop`,
/// or a case-insensitive exact name (must be unambiguous).
fn resolve<'a>(apps: &'a [AppEntry], query: &str) -> Result<&'a AppEntry, String> {
    if let Some(a) = apps.iter().find(|a| a.id == query) {
        return Ok(a);
    }
    let with_suffix = format!("{query}.desktop");
    if let Some(a) = apps.iter().find(|a| a.id == with_suffix) {
        return Ok(a);
    }
    let q = query.to_lowercase();
    let by_name: Vec<_> = apps.iter().filter(|a| a.name.to_lowercase() == q).collect();
    match by_name.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no installed application matches {query:?} (try --list)")),
        many => Err(format!(
            "{query:?} is ambiguous: {}",
            many.iter().map(|a| a.id.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn list(all: bool) -> ExitCode {
    let apps = discover(&DiscoveryOptions::from_env());
    let vis = load_visibility();
    for a in &apps {
        let hidden = vis.is_hidden(&a.id);
        if hidden && !all {
            continue;
        }
        let mark = if hidden { "  [hidden]" } else { "" };
        println!("{}\t{}\t{}{mark}", a.id, a.source.label(), a.name);
    }
    ExitCode::SUCCESS
}

fn list_hidden() -> ExitCode {
    let apps = discover(&DiscoveryOptions::from_env());
    let vis = load_visibility();
    for id in vis.hidden_ids() {
        match apps.iter().find(|a| a.id == id) {
            Some(a) => println!("{id}\t{}", a.name),
            None => println!("{id}\t(not installed)"),
        }
    }
    ExitCode::SUCCESS
}

fn hide(query: &str) -> ExitCode {
    let apps = discover(&DiscoveryOptions::from_env());
    let mut vis = load_visibility();
    // Allow hiding by raw ID even if the app is not installed right now.
    let id = match resolve(&apps, query) {
        Ok(a) => a.id.clone(),
        Err(_) if query.ends_with(".desktop") => query.to_owned(),
        Err(e) => {
            eprintln!("arch-launcher: {e}");
            return ExitCode::FAILURE;
        }
    };
    match vis.hide(&id) {
        Ok(true) => {
            println!("hidden: {id}");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("already hidden: {id}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("arch-launcher: {e}");
            ExitCode::FAILURE
        }
    }
}

fn unhide(query: &str) -> ExitCode {
    let apps = discover(&DiscoveryOptions::from_env());
    let mut vis = load_visibility();
    let id = if vis.is_hidden(query) {
        query.to_owned()
    } else if vis.is_hidden(&format!("{query}.desktop")) {
        format!("{query}.desktop")
    } else {
        match resolve(&apps, query) {
            Ok(a) => a.id.clone(),
            Err(e) => {
                eprintln!("arch-launcher: {e}");
                return ExitCode::FAILURE;
            }
        }
    };
    match vis.unhide(&id) {
        Ok(true) => {
            println!("visible again: {id}");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("not hidden: {id}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("arch-launcher: {e}");
            ExitCode::FAILURE
        }
    }
}

fn unhide_all() -> ExitCode {
    let mut vis = load_visibility();
    match vis.unhide_all() {
        Ok(n) => {
            println!("{n} application(s) visible again");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("arch-launcher: {e}");
            ExitCode::FAILURE
        }
    }
}

fn launch(query: &str) -> ExitCode {
    let apps = discover(&DiscoveryOptions::from_env());
    let entry = match resolve(&apps, query) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("arch-launcher: {e}");
            return ExitCode::FAILURE;
        }
    };
    match launcher::launch_blocking(entry) {
        Ok(()) => {
            println!("launched: {}", entry.id);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("arch-launcher: failed to launch {}: {e}", entry.id);
            ExitCode::FAILURE
        }
    }
}
