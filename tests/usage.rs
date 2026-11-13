use arch_launcher::usage::{UsageTracker, MAX_ENTRIES};

const DAY: u64 = 86_400;

#[test]
fn persists_and_reloads() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("state/usage.toml");
    let mut t = UsageTracker::load_from(&path);
    assert!(t.is_empty());
    t.record("org.mozilla.firefox.desktop");
    t.record("org.mozilla.firefox.desktop");
    t.record("a.b.desktop");
    let again = UsageTracker::load_from(&path);
    assert_eq!(again.len(), 2);
    assert_eq!(again.record_of("org.mozilla.firefox.desktop").unwrap().count, 2);
    assert!(again.score("org.mozilla.firefox.desktop") > again.score("a.b.desktop"));
    assert_eq!(again.score("unknown.desktop"), 0.0);
}

#[test]
fn frecency_prefers_recent_and_frequent() {
    let now = 100 * DAY;
    let mut t = UsageTracker::in_memory();
    for _ in 0..5 {
        t.record_at("old-favourite.desktop", now - 90 * DAY);
    }
    t.record_at("recent-once.desktop", now - DAY);
    for _ in 0..5 {
        t.record_at("recent-favourite.desktop", now - DAY);
    }
    let s = |id: &str| t.score_at(id, now);
    assert!(s("recent-favourite.desktop") > s("recent-once.desktop"));
    assert!(s("recent-once.desktop") > s("old-favourite.desktop"));
    assert!(s("old-favourite.desktop") > 0.0);
}

#[test]
fn corrupt_file_starts_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("usage.toml");
    std::fs::write(&path, "this is [not valid toml").unwrap();
    let mut t = UsageTracker::load_from(&path);
    assert!(t.is_empty());
    t.record("x.desktop"); // overwrites the corrupt file atomically
    assert_eq!(UsageTracker::load_from(&path).len(), 1);
}

#[test]
fn history_is_capped_keeping_most_recent() {
    let mut t = UsageTracker::in_memory();
    for i in 0..(MAX_ENTRIES + 50) {
        t.record_at(&format!("app{i}.desktop"), 1_000 + i as u64);
    }
    assert_eq!(t.len(), MAX_ENTRIES);
    assert!(t.record_of("app0.desktop").is_none());
    assert!(t.record_of(&format!("app{}.desktop", MAX_ENTRIES + 49)).is_some());
}
