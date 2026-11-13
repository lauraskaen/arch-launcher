//! Search Engine: fuzzy matching over the *visible* applications.
//!
//! Pure Rust (no GTK). The UI builds a [`SearchEngine`] from
//! `AppStore::visible_apps()` only, so hidden apps can never be found: they
//! are simply not in the index.
//!
//! Ranking, best first: exact name ≫ name prefix ≫ word start in the name ≫
//! fuzzy name ≫ keywords ≫ generic name ≫ description. Ties are broken by
//! usage score (when enabled), then by name.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as MatcherConfig, Matcher, Utf32String};

use crate::discovery::AppEntry;

const TIER_EXACT: u64 = 6_000_000;
const TIER_PREFIX: u64 = 5_000_000;
const TIER_WORD_START: u64 = 4_000_000;
const TIER_FUZZY_NAME: u64 = 3_000_000;
const TIER_KEYWORD: u64 = 2_000_000;
const TIER_GENERIC: u64 = 1_000_000;
const TIER_DESCRIPTION: u64 = 0;
/// Fuzzy scores are clamped below this so they never leak into the next tier.
const TIER_SPAN: u64 = 900_000;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub max_results: usize,
    pub sort_by_usage: bool,
    pub search_keywords: bool,
    pub search_descriptions: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self { max_results: 50, sort_by_usage: true, search_keywords: true, search_descriptions: true }
    }
}

struct Item {
    entry: Rc<AppEntry>,
    name_lc: String,
    name: Utf32String,
    generic: Option<Utf32String>,
    keywords: Vec<Utf32String>,
    description: Option<Utf32String>,
}

pub struct SearchEngine {
    items: Vec<Item>,
    matcher: RefCell<Matcher>,
}

impl Default for SearchEngine {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl SearchEngine {
    /// Builds the index (haystacks are precomputed here, once).
    pub fn new(apps: &[Rc<AppEntry>]) -> Self {
        let items = apps
            .iter()
            .map(|e| Item {
                entry: Rc::clone(e),
                name_lc: e.name.to_lowercase(),
                name: Utf32String::from(e.name.as_str()),
                generic: e.generic_name.as_deref().filter(|s| !s.is_empty()).map(Utf32String::from),
                keywords: e.keywords.iter().filter(|k| !k.is_empty()).map(|k| Utf32String::from(k.as_str())).collect(),
                description: e.comment.as_deref().filter(|s| !s.is_empty()).map(Utf32String::from),
            })
            .collect();
        Self { items, matcher: RefCell::new(Matcher::new(MatcherConfig::DEFAULT)) }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Searches the index. `usage` maps an app ID to its frecency score
    /// (ignored when `opts.sort_by_usage` is false).
    pub fn search(&self, query: &str, opts: &SearchOptions, usage: &dyn Fn(&str) -> f64) -> Vec<Rc<AppEntry>> {
        let limit = opts.max_results.max(1);
        let query = query.trim();
        let usage_of = |id: &str| if opts.sort_by_usage { usage(id) } else { 0.0 };

        if query.is_empty() {
            let mut all: Vec<(f64, &Item)> = self.items.iter().map(|i| (usage_of(&i.entry.id), i)).collect();
            all.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.name_lc.cmp(&b.1.name_lc)));
            return all.into_iter().take(limit).map(|(_, i)| Rc::clone(&i.entry)).collect();
        }

        let query_lc = query.to_lowercase();
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut matcher = self.matcher.borrow_mut();

        let mut hits: Vec<(u64, f64, &Item)> = Vec::new();
        for item in &self.items {
            if let Some(score) = score_item(item, &query_lc, &pattern, &mut matcher, opts) {
                hits.push((score, usage_of(&item.entry.id), item));
            }
        }
        hits.sort_by(cmp_hits);
        hits.into_iter().take(limit).map(|(_, _, i)| Rc::clone(&i.entry)).collect()
    }
}

fn cmp_hits(a: &(u64, f64, &Item), b: &(u64, f64, &Item)) -> Ordering {
    b.0.cmp(&a.0)
        .then_with(|| b.1.total_cmp(&a.1))
        .then_with(|| a.2.name_lc.cmp(&b.2.name_lc))
}

fn fuzzy(pattern: &Pattern, hay: &Utf32String, matcher: &mut Matcher) -> Option<u64> {
    pattern.score(hay.slice(..), matcher).map(|s| u64::from(s).min(TIER_SPAN))
}

/// True if `needle` occurs in `hay` at the start of a word (not at position 0).
fn word_start(hay: &str, needle: &str) -> bool {
    let mut prev: Option<char> = None;
    for (i, c) in hay.char_indices() {
        if let Some(p) = prev {
            if !p.is_alphanumeric() && hay[i..].starts_with(needle) && c.is_alphanumeric() {
                return true;
            }
        }
        prev = Some(c);
    }
    false
}

fn score_item(item: &Item, query_lc: &str, pattern: &Pattern, matcher: &mut Matcher, opts: &SearchOptions) -> Option<u64> {
    if item.name_lc == query_lc {
        return Some(TIER_EXACT);
    }
    if item.name_lc.starts_with(query_lc) {
        return Some(TIER_PREFIX);
    }
    if word_start(&item.name_lc, query_lc) {
        return Some(TIER_WORD_START);
    }
    if let Some(s) = fuzzy(pattern, &item.name, matcher) {
        return Some(TIER_FUZZY_NAME + s);
    }
    if opts.search_keywords {
        let best = item.keywords.iter().filter_map(|k| fuzzy(pattern, k, matcher)).max();
        if let Some(s) = best {
            return Some(TIER_KEYWORD + s);
        }
    }
    if let Some(g) = &item.generic {
        if let Some(s) = fuzzy(pattern, g, matcher) {
            return Some(TIER_GENERIC + s);
        }
    }
    if opts.search_descriptions {
        if let Some(d) = &item.description {
            if let Some(s) = fuzzy(pattern, d, matcher) {
                return Some(TIER_DESCRIPTION + s);
            }
        }
    }
    None
}
