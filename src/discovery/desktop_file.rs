//! Minimal, allocation-light parser for the `[Desktop Entry]` group of a
//! freedesktop.org Desktop Entry file.
//!
//! Implemented in pure Rust (instead of `GKeyFile`) so discovery can run on a
//! worker thread and be unit-tested without a GLib main loop.
//! Spec: <https://specifications.freedesktop.org/desktop-entry-spec/latest/>

use std::collections::HashMap;

const MAIN_GROUP: &str = "Desktop Entry";

#[derive(Debug, Default, Clone)]
pub struct DesktopFile {
    /// Raw (still escaped) values keyed by `Key` or `Key[locale]`.
    entries: HashMap<String, String>,
}

impl DesktopFile {
    /// Parses the text of a desktop file. Returns `None` if there is no
    /// `[Desktop Entry]` group. Malformed lines are skipped, matching the
    /// lenient behaviour of GLib.
    pub fn parse(text: &str) -> Option<Self> {
        let mut entries = HashMap::new();
        let mut in_main = false;
        let mut seen_main = false;

        for raw in text.lines() {
            let line = raw.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                let header = line.trim_end();
                in_main = header == "[Desktop Entry]";
                if in_main {
                    if seen_main {
                        // Duplicate group: spec says invalid; keep the first one.
                        in_main = false;
                    }
                    seen_main = true;
                }
                continue;
            }
            if !in_main {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim_end();
            if key.is_empty() {
                continue;
            }
            // First occurrence wins for duplicated keys.
            entries
                .entry(key.to_owned())
                .or_insert_with(|| value.trim_start().to_owned());
        }

        seen_main.then_some(Self { entries })
    }

    pub fn group_name() -> &'static str {
        MAIN_GROUP
    }

    fn raw(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Unescaped string value.
    pub fn string(&self, key: &str) -> Option<String> {
        self.raw(key).map(unescape)
    }

    /// Localized string value, trying each locale variant in order and
    /// falling back to the unlocalized key.
    pub fn locale_string(&self, key: &str, locales: &[String]) -> Option<String> {
        for loc in locales {
            if let Some(v) = self.raw(&format!("{key}[{loc}]")) {
                return Some(unescape(v));
            }
        }
        self.string(key)
    }

    pub fn boolean(&self, key: &str) -> bool {
        matches!(self.raw(key).map(str::trim), Some("true") | Some("1"))
    }

    /// `;`-separated list (unlocalized).
    pub fn list(&self, key: &str) -> Vec<String> {
        self.raw(key).map(split_list).unwrap_or_default()
    }

    /// `;`-separated localized list.
    pub fn locale_list(&self, key: &str, locales: &[String]) -> Vec<String> {
        for loc in locales {
            if let Some(v) = self.raw(&format!("{key}[{loc}]")) {
                return split_list(v);
            }
        }
        self.list(key)
    }
}

/// Resolves the escape sequences `\s \n \t \r \\` defined by the spec.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a list value on unescaped `;`, unescaping each element.
fn split_list(s: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => cur.push(';'),
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                }
                None => cur.push('\\'),
            },
            ';' => {
                let item = unescape(cur.trim());
                if !item.is_empty() {
                    items.push(item);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let item = unescape(cur.trim());
    if !item.is_empty() {
        items.push(item);
    }
    items
}

/// Produces the ordered list of locale keys to try for a POSIX locale such as
/// `pt_BR.UTF-8@euro`: `pt_BR@euro`, `pt_BR`, `pt@euro`, `pt`.
pub fn locale_variants(locale: &str) -> Vec<String> {
    let locale = locale.trim();
    if locale.is_empty() || locale == "C" || locale == "POSIX" || locale.starts_with("C.") {
        return Vec::new();
    }
    let (rest, modifier) = match locale.split_once('@') {
        Some((r, m)) => (r, Some(m)),
        None => (locale, None),
    };
    let rest = rest.split('.').next().unwrap_or(rest); // drop .ENCODING
    let (lang, country) = match rest.split_once('_') {
        Some((l, c)) => (l, Some(c)),
        None => (rest, None),
    };

    let mut out = Vec::new();
    if let (Some(c), Some(m)) = (country, modifier) {
        out.push(format!("{lang}_{c}@{m}"));
    }
    if let Some(c) = country {
        out.push(format!("{lang}_{c}"));
    }
    if let Some(m) = modifier {
        out.push(format!("{lang}@{m}"));
    }
    out.push(lang.to_owned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r"# comment
[Desktop Entry]
Type=Application
Name=Files
Name[pt_BR]=Arquivos
Name[pt]=Ficheiros
Comment = Access\sand organize files
Keywords=folder;manager;explore;disk\;drive;
NoDisplay=false
Terminal=true

[Desktop Action new-window]
Name=New Window
Exec=nautilus --new-window
";

    #[test]
    fn parses_main_group_only() {
        let f = DesktopFile::parse(SAMPLE).unwrap();
        assert_eq!(f.string("Type").as_deref(), Some("Application"));
        assert_eq!(f.string("Exec"), None, "action group must be ignored");
        assert!(f.boolean("Terminal"));
        assert!(!f.boolean("NoDisplay"));
    }

    #[test]
    fn unescapes_and_trims() {
        let f = DesktopFile::parse(SAMPLE).unwrap();
        assert_eq!(f.string("Comment").as_deref(), Some("Access and organize files"));
    }

    #[test]
    fn localizes() {
        let f = DesktopFile::parse(SAMPLE).unwrap();
        let br = locale_variants("pt_BR.UTF-8");
        assert_eq!(f.locale_string("Name", &br).as_deref(), Some("Arquivos"));
        let pt = locale_variants("pt_PT.UTF-8");
        assert_eq!(f.locale_string("Name", &pt).as_deref(), Some("Ficheiros"));
        assert_eq!(f.locale_string("Name", &[]).as_deref(), Some("Files"));
    }

    #[test]
    fn splits_lists_with_escapes() {
        let f = DesktopFile::parse(SAMPLE).unwrap();
        assert_eq!(f.list("Keywords"), vec!["folder", "manager", "explore", "disk;drive"]);
    }

    #[test]
    fn locale_variant_order() {
        assert_eq!(
            locale_variants("sr_RS.UTF-8@latin"),
            vec!["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
        assert!(locale_variants("C.UTF-8").is_empty());
    }

    #[test]
    fn rejects_files_without_main_group() {
        assert!(DesktopFile::parse("[Other]\nName=x\n").is_none());
    }
}
