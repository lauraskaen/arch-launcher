use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use arch_launcher::discovery::{AppEntry, AppSource, DiscoveryOptions};
use arch_launcher::search::{SearchEngine, SearchOptions};
use arch_launcher::store::AppStore;
use arch_launcher::visibility::VisibilityManager;

fn app(id: &str, name: &str) -> AppEntry {
    AppEntry {
        id: id.into(),
        path: PathBuf::from(format!("/usr/share/applications/{id}")),
        name: name.into(),
        generic_name: None,
        comment: None,
        keywords: vec![],
        categories: vec![],
        icon: None,
        exec: Some("true".into()),
        terminal: false,
        dbus_activatable: false,
        source: AppSource::System,
    }
}

fn rc(v: Vec<AppEntry>) -> Vec<Rc<AppEntry>> {
    v.into_iter().map(Rc::new).collect()
}

fn names(r: &[Rc<AppEntry>]) -> Vec<&str> {
    r.iter().map(|e| e.name.as_str()).collect()
}

fn no_usage(_: &str) -> f64 {
    0.0
}

fn sample() -> Vec<Rc<AppEntry>> {
    let mut firefox = app("firefox.desktop", "Firefox");
    firefox.generic_name = Some("Web Browser".into());
    firefox.keywords = vec!["internet".into(), "www".into()];
    firefox.comment = Some("Browse the World Wide Web".into());
    let mut files = app("files.desktop", "Files");
    files.comment = Some("Access and organize files".into());
    let mut code = app("code.desktop", "Visual Studio Code");
    code.keywords = vec!["editor".into(), "ide".into()];
    let mut term = app("term.desktop", "Terminal");
    term.generic_name = Some("Terminal Emulator".into());
    term.comment = Some("Run commands, open Firefox".into());
    rc(vec![firefox, files, code, term, app("fire.desktop", "Wildfire")])
}

#[test]
fn case_insensitive_and_fuzzy() {
    let e = SearchEngine::new(&sample());
    let o = SearchOptions::default();
    assert_eq!(names(&e.search("FIREFOX", &o, &no_usage))[0], "Firefox");
    assert_eq!(names(&e.search("firefox", &o, &no_usage))[0], "Firefox");
    assert_eq!(names(&e.search("vsc", &o, &no_usage))[0], "Visual Studio Code");
    assert_eq!(names(&e.search("vis code", &o, &no_usage))[0], "Visual Studio Code");
    assert!(e.search("zzzzqqq", &o, &no_usage).is_empty());
}

#[test]
fn ranking_order_exact_prefix_wordstart_fuzzy() {
    let apps = rc(vec![
        app("a.desktop", "My Fire Starter"), // word start
        app("b.desktop", "Firefox"),         // prefix
        app("c.desktop", "Fire"),            // exact
        app("d.desktop", "Wildfire"),        // fuzzy/substring
    ]);
    let e = SearchEngine::new(&apps);
    let r = e.search("fire", &SearchOptions::default(), &no_usage);
    assert_eq!(names(&r), ["Fire", "Firefox", "My Fire Starter", "Wildfire"]);
}

#[test]
fn name_beats_keywords_beats_generic_beats_description() {
    let mut kw = app("kw.desktop", "Alpha");
    kw.keywords = vec!["gizmo".into()];
    let mut gen = app("gen.desktop", "Beta");
    gen.generic_name = Some("Gizmo Tool".into());
    let mut desc = app("desc.desktop", "Gamma");
    desc.comment = Some("A tool for gizmo work".into());
    let named = app("n.desktop", "Gizmo Pro");
    // keywords are fuzzy-matched too, so use distinct strings.
    let e = SearchEngine::new(&rc(vec![desc, gen, kw, named]));
    let r = e.search("gizmo", &SearchOptions::default(), &no_usage);
    assert_eq!(names(&r), ["Gizmo Pro", "Alpha", "Beta", "Gamma"]);
}

#[test]
fn keyword_and_description_toggles() {
    let e = SearchEngine::new(&sample());
    let mut o = SearchOptions::default();
    assert_eq!(names(&e.search("internet", &o, &no_usage)), ["Firefox"]);
    o.search_keywords = false;
    assert!(e.search("internet", &o, &no_usage).is_empty());

    let mut o = SearchOptions::default();
    assert!(names(&e.search("organize", &o, &no_usage)).contains(&"Files"));
    o.search_descriptions = false;
    assert!(e.search("organize", &o, &no_usage).is_empty());
}

#[test]
fn generic_name_is_searched() {
    let e = SearchEngine::new(&sample());
    let r = e.search("emulator", &SearchOptions::default(), &no_usage);
    assert_eq!(names(&r)[0], "Terminal");
}

#[test]
fn max_results_is_respected() {
    let apps = rc((0..100).map(|i| app(&format!("a{i}.desktop"), &format!("App {i:03}"))).collect());
    let e = SearchEngine::new(&apps);
    let o = SearchOptions { max_results: 7, ..Default::default() };
    assert_eq!(e.search("app", &o, &no_usage).len(), 7);
    assert_eq!(e.search("", &o, &no_usage).len(), 7);
}

#[test]
fn empty_query_orders_by_usage_then_name() {
    let apps = rc(vec![app("a.desktop", "Alpha"), app("b.desktop", "Bravo"), app("c.desktop", "Charlie")]);
    let e = SearchEngine::new(&apps);
    let usage = |id: &str| match id {
        "c.desktop" => 5.0,
        "b.desktop" => 2.0,
        _ => 0.0,
    };
    let o = SearchOptions::default();
    assert_eq!(names(&e.search("", &o, &usage)), ["Charlie", "Bravo", "Alpha"]);
    assert_eq!(names(&e.search("   ", &o, &usage)), ["Charlie", "Bravo", "Alpha"]);
    let off = SearchOptions { sort_by_usage: false, ..Default::default() };
    assert_eq!(names(&e.search("", &off, &usage)), ["Alpha", "Bravo", "Charlie"]);
}

#[test]
fn usage_breaks_ties_between_equal_matches() {
    let apps = rc(vec![app("a.desktop", "Editor One"), app("b.desktop", "Editor Two")]);
    let e = SearchEngine::new(&apps);
    let usage = |id: &str| if id == "b.desktop" { 3.0 } else { 0.0 };
    assert_eq!(names(&e.search("editor", &SearchOptions::default(), &usage))[0], "Editor Two");
    let off = SearchOptions { sort_by_usage: false, ..Default::default() };
    assert_eq!(names(&e.search("editor", &off, &usage))[0], "Editor One");
}

#[test]
fn handles_thousands_of_entries() {
    let apps = rc((0..3000)
        .map(|i| {
            let mut a = app(&format!("app{i}.desktop"), &format!("Synthetic Application {i}"));
            a.comment = Some(format!("Description number {i} of a synthetic program"));
            a.keywords = vec![format!("kw{i}"), "synthetic".into()];
            a
        })
        .collect());
    let e = SearchEngine::new(&apps);
    assert_eq!(e.len(), 3000);
    let o = SearchOptions::default();
    let start = std::time::Instant::now();
    let r = e.search("synth app 2999", &o, &no_usage);
    let elapsed = start.elapsed();
    assert!(!r.is_empty());
    assert_eq!(r[0].name, "Synthetic Application 2999");
    assert!(elapsed.as_secs() < 5, "search took {elapsed:?}");
}

// ---- end to end with the real store -----------------------------------------

#[test]
fn hidden_app_is_not_found_by_exact_name_until_unhidden() {
    let tmp = tempfile::tempdir().unwrap();
    let data_home = tmp.path().join("home/.local/share");
    let system = tmp.path().join("usr/share");
    fs::create_dir_all(data_home.join("applications")).unwrap();
    fs::create_dir_all(system.join("applications")).unwrap();
    for (id, name) in [("secret.desktop", "Secret Tool"), ("other.desktop", "Other Thing")] {
        fs::write(
            system.join("applications").join(id),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/usr/bin/true\n"),
        )
        .unwrap();
    }
    let opts = DiscoveryOptions {
        data_dirs: vec![data_home.clone(), system],
        data_home,
        current_desktops: vec![],
        locales: vec![],
        path_env: Some("/usr/bin:/bin".into()),
    };
    let (vis, _) = VisibilityManager::load(tmp.path().join("cfg/hidden-apps.toml"));
    let store = AppStore::new(opts, vis);
    let o = SearchOptions::default();

    let found = SearchEngine::new(&store.visible_apps()).search("Secret Tool", &o, &no_usage);
    assert_eq!(names(&found), ["Secret Tool"]);

    store.hide("secret.desktop").unwrap();
    let engine = SearchEngine::new(&store.visible_apps());
    assert!(engine.search("Secret Tool", &o, &no_usage).is_empty());
    assert!(engine.search("secret", &o, &no_usage).is_empty());
    assert!(engine.search("", &o, &no_usage).iter().all(|e| e.id != "secret.desktop"));

    store.unhide("secret.desktop").unwrap();
    let found = SearchEngine::new(&store.visible_apps()).search("Secret Tool", &o, &no_usage);
    assert_eq!(names(&found), ["Secret Tool"]);
}
