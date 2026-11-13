//! Usage history ("frecency") used to rank search results.
//!
//! Stored as TOML in `paths::usage_file()` (`~/.local/state/arch-launcher/usage.toml`):
//!
//! ```toml
//! [apps."org.mozilla.firefox.desktop"]
//! count = 12
//! last_used = 1730000000
//! ```
//!
//! The file is small (at most [`MAX_ENTRIES`] apps), written atomically and
//! never required: a missing or corrupt file simply means "no history".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Maximum number of apps kept in the history (most recently used win).
pub const MAX_ENTRIES: usize = 500;
/// Counts are capped so one app launched thousands of times cannot dominate forever.
const MAX_COUNT: u32 = 1000;
/// After this many days without use, the recency weight is halved.
const HALF_LIFE_DAYS: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub count: u32,
    /// Unix timestamp (seconds) of the last launch.
    pub last_used: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct UsageFile {
    #[serde(default)]
    apps: BTreeMap<String, UsageRecord>,
}

#[derive(Debug, Default)]
pub struct UsageTracker {
    path: Option<PathBuf>,
    apps: BTreeMap<String, UsageRecord>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl UsageTracker {
    /// Loads the history from the default location.
    pub fn load() -> Self {
        Self::load_from(crate::paths::usage_file())
    }

    /// Loads the history from `path`. Missing file → empty; corrupt file →
    /// empty plus a log warning. The tracker remembers `path` for saving.
    pub fn load_from(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let apps = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<UsageFile>(&text) {
                Ok(file) => file.apps,
                Err(e) => {
                    log::warn!("usage history {} is corrupt, starting empty: {e}", path.display());
                    BTreeMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                log::warn!("cannot read usage history {}: {e}", path.display());
                BTreeMap::new()
            }
        };
        Self { path: Some(path), apps }
    }

    /// A tracker that never touches the disk.
    pub fn in_memory() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    pub fn record_of(&self, id: &str) -> Option<UsageRecord> {
        self.apps.get(id).copied()
    }

    /// Registers a launch of `id` now and persists the history.
    pub fn record(&mut self, id: &str) {
        self.record_at(id, now_secs());
        if let Err(e) = self.save() {
            log::warn!("cannot save usage history: {e}");
        }
    }

    /// Registers a launch at an explicit time (no I/O). Used by tests.
    pub fn record_at(&mut self, id: &str, when: u64) {
        let rec = self.apps.entry(id.to_owned()).or_insert(UsageRecord { count: 0, last_used: when });
        rec.count = rec.count.saturating_add(1).min(MAX_COUNT);
        rec.last_used = rec.last_used.max(when);
        self.prune();
    }

    fn prune(&mut self) {
        if self.apps.len() <= MAX_ENTRIES {
            return;
        }
        let mut by_recency: Vec<(&String, u64)> = self.apps.iter().map(|(k, v)| (k, v.last_used)).collect();
        by_recency.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        let drop: Vec<String> = by_recency.into_iter().skip(MAX_ENTRIES).map(|(k, _)| k.clone()).collect();
        for k in drop {
            self.apps.remove(&k);
        }
    }

    /// Frecency score: launch count weighted by how recently the app was used.
    /// Higher is better; unknown apps score 0.
    pub fn score(&self, id: &str) -> f64 {
        self.score_at(id, now_secs())
    }

    pub fn score_at(&self, id: &str, now: u64) -> f64 {
        let Some(rec) = self.apps.get(id) else { return 0.0 };
        let age_days = now.saturating_sub(rec.last_used) as f64 / 86_400.0;
        let recency = 0.5f64.powf(age_days / HALF_LIFE_DAYS);
        // log-damped count, so 50 launches is not 50x better than one; the
        // recency factor keeps a floor so old favourites still beat nothing.
        let freq = (1.0 + f64::from(rec.count)).ln();
        freq * (0.15 + 0.85 * recency)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        self.save_to(path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let file = UsageFile { apps: self.apps.clone() };
        let text = toml::to_string(&file).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        crate::atomic::write_atomic(path, text.as_bytes())
    }
}
