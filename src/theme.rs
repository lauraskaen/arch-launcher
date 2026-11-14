//! Theme Manager: turns the configuration into GTK CSS.
//!
//! * [`generate_css`] is pure (testable): palette + config overrides → CSS.
//! * [`ThemeManager::apply`] installs it on the default display at
//!   `STYLE_PROVIDER_PRIORITY_APPLICATION`, then loads the optional
//!   `~/.config/arch-launcher/style.css` at `STYLE_PROVIDER_PRIORITY_USER`.
//!   Calling it again replaces both providers (live reload).
//!
//! The default look is a Catppuccin-like palette: Mocha (dark) and Latte (light).
//!
//! With `theme = "gtk"` the stylesheet references the GTK named colours
//! (`@window_bg_color`, `@accent_bg_color`, …) instead, so it follows whatever
//! defines them: the GTK theme, or `~/.config/gtk-4.0/gtk.css` as written by
//! matugen / DankMaterialShell. [`fallback_css`] defines the same names at
//! `STYLE_PROVIDER_PRIORITY_FALLBACK`, so any of them the system leaves
//! undefined still resolves to the built-in palette.

use std::cell::RefCell;

use gtk::prelude::*;

use crate::config::{Appearance, Config, ThemeMode};

/// CSS class added to the window when `animations = false`.
pub const NO_ANIMATIONS_CLASS: &str = "no-animations";

#[derive(Debug, Clone, Copy, PartialEq)]
struct Color {
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

impl Color {
    const fn hex(rgb: u32) -> Self {
        Self {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    fn parse(s: Option<&str>) -> Option<Self> {
        let c = gtk::gdk::RGBA::parse(s?).ok()?;
        Some(Self { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() })
    }

    /// CSS value with the alpha multiplied by `alpha`.
    fn css(self, alpha: f32) -> String {
        format!(
            "rgba({}, {}, {}, {:.3})",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8,
            (self.a * alpha).clamp(0.0, 1.0)
        )
    }
}

/// A colour in the generated CSS: a literal value, or a GTK named colour
/// (`@name`) with an alpha factor.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Paint {
    Rgba(Color),
    Named(&'static str, f32),
}

impl Paint {
    /// CSS value with the alpha multiplied by `alpha`.
    fn css(self, alpha: f32) -> String {
        match self {
            Paint::Rgba(c) => c.css(alpha),
            Paint::Named(name, base) => {
                let a = (base * alpha).clamp(0.0, 1.0);
                if (a - 1.0).abs() < 0.0005 {
                    format!("@{name}")
                } else {
                    format!("alpha(@{name}, {a:.3})")
                }
            }
        }
    }

    /// The same colour with its own alpha replaced by `alpha`.
    fn with_alpha(self, alpha: f32) -> Self {
        match self {
            Paint::Rgba(c) => Paint::Rgba(Color { a: alpha, ..c }),
            Paint::Named(name, _) => Paint::Named(name, alpha),
        }
    }
}

/// GTK named colours used by `theme = "gtk"`.
const GTK_WINDOW_BG: &str = "window_bg_color";
const GTK_WINDOW_FG: &str = "window_fg_color";
const GTK_CARD_BG: &str = "card_bg_color";
const GTK_POPOVER_BG: &str = "popover_bg_color";
const GTK_ACCENT_BG: &str = "accent_bg_color";
const GTK_ACCENT_FG: &str = "accent_fg_color";
const GTK_DESTRUCTIVE_BG: &str = "destructive_bg_color";

/// `@define-color` for every named colour `theme = "gtk"` uses, with the
/// built-in palette's values. Installed at the lowest priority, so the GTK
/// theme and `~/.config/gtk-4.0/gtk.css` override each one they define.
pub fn fallback_css(dark: bool) -> String {
    let p = palette(dark);
    [
        (GTK_WINDOW_BG, p.base),
        (GTK_WINDOW_FG, p.text),
        (GTK_CARD_BG, p.surface),
        (GTK_POPOVER_BG, p.base),
        (GTK_ACCENT_BG, p.accent),
        (GTK_ACCENT_FG, p.accent_text),
        (GTK_DESTRUCTIVE_BG, p.danger),
    ]
    .iter()
    .map(|(name, c)| format!("@define-color {name} {};\n", c.css(1.0)))
    .collect()
}

struct Palette {
    base: Color,
    surface: Color,
    text: Color,
    subtext: Color,
    border: Color,
    accent: Color,
    accent_text: Color,
    danger: Color,
    shade: Color,
}

fn palette(dark: bool) -> Palette {
    if dark {
        // Catppuccin Mocha
        Palette {
            base: Color::hex(0x1e1e2e),
            surface: Color::hex(0x313244),
            text: Color::hex(0xcdd6f4),
            subtext: Color::hex(0xa6adc8),
            border: Color::hex(0x45475a),
            accent: Color::hex(0xcba6f7),
            accent_text: Color::hex(0x1e1e2e),
            danger: Color::hex(0xf38ba8),
            shade: Color::hex(0x000000),
        }
    } else {
        // Catppuccin Latte
        Palette {
            base: Color::hex(0xeff1f5),
            surface: Color::hex(0xe6e9ef),
            text: Color::hex(0x4c4f69),
            subtext: Color::hex(0x6c6f85),
            border: Color::hex(0xccd0da),
            accent: Color::hex(0x8839ef),
            accent_text: Color::hex(0xeff1f5),
            danger: Color::hex(0xd20f39),
            shade: Color::hex(0x4c4f69),
        }
    }
}

const COLOR_SCHEME_PROP: &str = "gtk-interface-color-scheme";

/// GTK ≥ 4.20 exposes `gtk-interface-color-scheme` and deprecates
/// `gtk-application-prefer-dark-theme` (setting it logs a warning).
fn has_color_scheme(settings: &gtk::Settings) -> bool {
    settings.find_property(COLOR_SCHEME_PROP).is_some()
}

fn set_color_scheme(settings: &gtk::Settings, nick: &str) {
    let Some(pspec) = settings.find_property(COLOR_SCHEME_PROP) else { return };
    let Some(class) = glib::EnumClass::with_type(pspec.value_type()) else { return };
    if let Some(value) = class.to_value_by_nick(nick) {
        settings.set_property_from_value(COLOR_SCHEME_PROP, &value);
    }
}

fn read_color_scheme(settings: &gtk::Settings) -> Option<bool> {
    let value = settings.property_value(COLOR_SCHEME_PROP);
    let (_, v) = glib::EnumValue::from_value(&value)?;
    match v.nick() {
        "dark" => Some(true),
        "light" => Some(false),
        _ => None,
    }
}

/// Whether the effective theme is dark. `system` follows the desktop colour
/// scheme / `gtk-application-prefer-dark-theme` / a theme name containing
/// "dark", defaulting to dark.
pub fn resolve_dark(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System | ThemeMode::Gtk => {
            let Some(s) = gtk::Settings::default() else { return true };
            if has_color_scheme(&s) {
                if let Some(dark) = read_color_scheme(&s) {
                    return dark;
                }
            } else if s.is_gtk_application_prefer_dark_theme() {
                return true;
            }
            match s.gtk_theme_name() {
                Some(name) => {
                    let n = name.to_lowercase();
                    // A theme explicitly named light means light; anything else → dark.
                    n.contains("dark") || !(n.contains("light") || n == "adwaita" || n == "default")
                }
                None => true,
            }
        }
    }
}

/// Turns a CSS-like family list (`Inter, Noto Sans, sans-serif`) into a safe
/// `font-family` value: each family is sanitised and quoted, except the CSS
/// generic families, which must stay unquoted to work as fallbacks.
fn font_family_css(family: &str) -> String {
    const GENERIC: [&str; 6] = ["serif", "sans-serif", "monospace", "cursive", "fantasy", "system-ui"];
    let parts: Vec<String> = family
        .split(',')
        .map(|part| {
            part.chars()
                .filter(|c| !matches!(c, '"' | '\'' | ';' | '{' | '}' | '\\' | '\n' | '\r'))
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|p| !p.is_empty())
        .map(|p| if GENERIC.contains(&p.to_lowercase().as_str()) { p.to_lowercase() } else { format!("\"{p}\"") })
        .collect();
    if parts.is_empty() {
        "sans-serif".to_owned()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod font_tests {
    use super::font_family_css;

    #[test]
    fn family_lists_keep_generic_fallbacks_unquoted() {
        assert_eq!(font_family_css("Inter, Noto Sans, sans-serif"), "\"Inter\", \"Noto Sans\", sans-serif");
        let injected = font_family_css("x}; * { color: red");
        assert!(!injected.contains(['{', '}', ';']), "{injected}");
        assert_eq!(font_family_css(" , "), "sans-serif");
    }
}

/// Generates the application stylesheet.
pub fn generate_css(config: &Config, dark: bool) -> String {
    let a: &Appearance = &config.appearance;
    let p = palette(dark);
    let gtk_colors = a.theme == ThemeMode::Gtk;
    // A colour from the config, else the GTK named colour (theme = "gtk"),
    // else the built-in palette.
    let pick = |user: Option<&str>, named: &'static str, builtin: Color| match Color::parse(user) {
        Some(c) => Paint::Rgba(c),
        None if gtk_colors => Paint::Named(named, 1.0),
        None => Paint::Rgba(builtin),
    };

    let bg = pick(a.background.as_deref(), GTK_WINDOW_BG, p.base);
    let fg = pick(a.foreground.as_deref(), GTK_WINDOW_FG, p.text);
    let accent = pick(a.accent.as_deref(), GTK_ACCENT_BG, p.accent);
    let border = match Color::parse(a.border_color.as_deref()) {
        Some(c) => Paint::Rgba(c),
        // GTK themes have no standard border colour: derive it from the text.
        None if gtk_colors => fg.with_alpha(0.15),
        None => Paint::Rgba(p.border),
    };
    let sub = if a.foreground.is_some() {
        fg.with_alpha(0.75)
    } else if gtk_colors {
        fg.with_alpha(0.7)
    } else {
        Paint::Rgba(p.subtext)
    };
    let selection = Color::parse(a.selection.as_deref());
    let sel_css = selection.map_or_else(|| accent.css(if dark { 0.22 } else { 0.16 }), |c| c.css(1.0));
    let hover_css = fg.css(0.07);
    let surface_css = if a.background.is_some() {
        // Derive the "raised" colour from the user's background.
        fg.css(if dark { 0.09 } else { 0.06 })
    } else if gtk_colors {
        Paint::Named(GTK_CARD_BG, 1.0).css(1.0)
    } else {
        p.surface.css(1.0)
    };
    // Popovers must be opaque to stay readable over any wallpaper.
    let popover_bg = if gtk_colors && a.background.is_none() {
        Paint::Named(GTK_POPOVER_BG, 1.0)
    } else {
        bg.with_alpha(1.0)
    }
    .css(1.0);
    let accent_text = if gtk_colors && a.accent.is_none() {
        Paint::Named(GTK_ACCENT_FG, 1.0)
    } else {
        Paint::Rgba(p.accent_text)
    };
    let danger_paint = if gtk_colors { Paint::Named(GTK_DESTRUCTIVE_BG, 1.0) } else { Paint::Rgba(p.danger) };
    let opacity = a.opacity as f32;

    let radius = a.border_radius;
    let inner_radius = radius.saturating_sub(4).max(4);
    let pad = a.padding;
    let animated = a.animations;
    let tr = |props: &str| if animated { format!("transition: {props};") } else { String::new() };
    let icon = a.icon_size;
    let danger = danger_paint.css(1.0);
    let danger_bg = danger_paint.css(0.16);
    let shadow = p.shade.css(0.35);

    format!(
        r#"
window.arch-launcher {{
  background: transparent;
  background-color: transparent;
  color: {fg};
  font-family: {family};
  font-size: {size}pt;
}}
window.arch-launcher.toplevel {{ box-shadow: none; }}

.arch-launcher .launcher-card {{
  background-color: {card_bg};
  border: {bw}px solid {border};
  border-radius: {radius}px;
  padding: {pad}px;
  color: {fg};
}}

.arch-launcher label {{ color: inherit; }}
.arch-launcher .dim, .arch-launcher .app-desc, .arch-launcher .app-id, .arch-launcher .footer {{ color: {sub}; }}
.arch-launcher .footer {{ font-size: 0.82em; padding: 2px 6px 0 6px; }}
.arch-launcher .empty-state {{ color: {sub}; font-size: 1.05em; padding: 24px; }}
.arch-launcher .page-title {{ font-weight: 700; font-size: 1.12em; }}
.arch-launcher .section-title {{ color: {sub}; font-size: 0.82em; font-weight: 700; padding: 10px 10px 4px 10px; }}

.arch-launcher entry.search-entry {{
  min-height: 40px;
  padding: 2px 12px;
  border-radius: {inner}px;
  background-color: {surface};
  background-image: none;
  color: {fg};
  border: 1px solid transparent;
  box-shadow: none;
  outline: none;
  font-size: 1.15em;
  caret-color: {accent};
  {tr_entry}
}}
.arch-launcher entry.search-entry:focus-within {{ border-color: {accent}; }}
.arch-launcher entry.search-entry image {{ color: {sub}; }}
.arch-launcher entry.search-entry text {{ color: {fg}; }}
.arch-launcher entry.search-entry placeholder {{ color: {sub}; opacity: 1; }}
.arch-launcher entry.search-entry selection {{ background-color: {sel}; color: {fg}; }}

.arch-launcher scrolledwindow, .arch-launcher listview, .arch-launcher list,
.arch-launcher listview > row.activatable {{ background: transparent; background-color: transparent; }}
.arch-launcher listview {{ margin-top: 6px; }}
.arch-launcher listview > row, .arch-launcher list.app-rows > row {{
  padding: 6px 10px;
  margin: 1px 0;
  border-radius: {inner}px;
  background-color: transparent;
  color: {fg};
  box-shadow: none;
  outline: none;
  {tr_row}
}}
.arch-launcher listview > row:hover, .arch-launcher list.app-rows > row:hover {{ background-color: {hover}; }}
.arch-launcher listview > row:selected, .arch-launcher list.app-rows > row:selected {{
  background-color: {sel};
  color: {fg};
  box-shadow: inset 3px 0 0 {accent};
}}
.arch-launcher .app-name {{ font-weight: 600; }}
.arch-launcher .app-desc {{ font-size: 0.86em; }}
.arch-launcher .app-icon {{ -gtk-icon-size: {icon}px; }}
.arch-launcher .badge {{
  font-size: 0.72em; font-weight: 700;
  padding: 1px 7px; border-radius: 999px;
  color: {accent}; background-color: {badge_bg};
}}
.arch-launcher .orphan-section {{ opacity: 0.6; }}

.arch-launcher button {{
  background-image: none;
  background-color: {surface};
  color: {fg};
  border: 1px solid transparent;
  border-radius: {inner}px;
  box-shadow: none;
  padding: 4px 12px;
  min-height: 28px;
  {tr_btn}
}}
.arch-launcher button:hover {{ background-color: {hover}; border-color: {border}; }}
.arch-launcher button:active, .arch-launcher button:checked {{ background-color: {sel}; }}
.arch-launcher button.flat {{ background-color: transparent; }}
.arch-launcher button.flat:hover {{ background-color: {hover}; }}
.arch-launcher button.icon-button {{ padding: 4px 8px; min-width: 32px; min-height: 36px; }}
.arch-launcher button.accent {{ background-color: {accent}; color: {accent_text}; font-weight: 600; }}
.arch-launcher button.accent:hover {{ background-color: {accent}; border-color: {fg_soft}; }}
.arch-launcher button.danger {{ background-color: {danger_bg}; color: {danger}; font-weight: 600; }}

.arch-launcher .banner {{
  background-color: {surface};
  border: 1px solid {border};
  border-radius: {inner}px;
  padding: 6px 8px 6px 12px;
  margin-top: 8px;
}}
.arch-launcher .banner.error {{ background-color: {danger_bg}; border-color: {danger}; }}
.arch-launcher .banner.error label {{ color: {danger}; }}
.arch-launcher revealer {{ background: transparent; }}

.arch-launcher scrollbar {{ background: transparent; border: none; }}
.arch-launcher scrollbar slider {{ background-color: {border}; border-radius: 999px; min-width: 5px; min-height: 30px; }}

popover.arch-launcher > contents, .arch-launcher popover > contents {{
  background-color: {popover_bg};
  color: {fg};
  border: 1px solid {border};
  border-radius: {radius_pop}px;
  padding: 4px;
  box-shadow: 0 6px 20px {shadow};
}}
popover.arch-launcher modelbutton, .arch-launcher popover modelbutton {{
  color: {fg};
  border-radius: {inner}px;
  padding: 4px 12px;
  min-height: 30px;
}}
popover.arch-launcher modelbutton:hover, .arch-launcher popover modelbutton:hover {{ background-color: {sel}; }}
popover.arch-launcher separator, .arch-launcher popover separator {{ background-color: {border}; min-height: 1px; margin: 4px 0; }}

.arch-launcher.no-animations *, .arch-launcher .no-animations * {{ transition: none; animation: none; }}
"#,
        fg = fg.css(1.0),
        fg_soft = fg.css(0.4),
        family = font_family_css(&a.font_family),
        size = a.font_size,
        card_bg = bg.css(opacity),
        bw = a.border_width,
        border = border.css(1.0),
        radius = radius,
        radius_pop = radius.min(14),
        pad = pad,
        sub = sub.css(1.0),
        inner = inner_radius,
        surface = surface_css,
        accent = accent.css(1.0),
        accent_text = accent_text.css(1.0),
        sel = sel_css,
        hover = hover_css,
        badge_bg = accent.css(0.16),
        icon = icon,
        popover_bg = popover_bg,
        danger = danger,
        danger_bg = danger_bg,
        shadow = shadow,
        tr_entry = tr("border-color 150ms ease, background-color 150ms ease"),
        tr_row = tr("background-color 120ms ease, box-shadow 120ms ease"),
        tr_btn = tr("background-color 120ms ease, border-color 120ms ease"),
    )
}

/// Owns the CSS providers installed on the default display.
#[derive(Default)]
pub struct ThemeManager {
    fallback: RefCell<Option<gtk::CssProvider>>,
    app: RefCell<Option<gtk::CssProvider>>,
    user: RefCell<Option<gtk::CssProvider>>,
}

fn log_parsing_errors(provider: &gtk::CssProvider, what: &'static str) {
    provider.connect_parsing_error(move |_, section, err| {
        log::warn!("{what}: CSS {}: {err}", section.to_str());
    });
}

impl ThemeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// (Re)installs the generated stylesheet and the user's `style.css`.
    /// Returns the effective darkness.
    pub fn apply(&self, config: &Config) -> bool {
        let Some(display) = gtk::gdk::Display::default() else {
            log::warn!("no display: theme not applied");
            return true;
        };
        // Make built-in widgets (menus, scrollbars) follow the chosen palette.
        let settings = gtk::Settings::for_display(&display);
        let modern = has_color_scheme(&settings);
        match (config.appearance.theme, modern) {
            (ThemeMode::Dark, true) => set_color_scheme(&settings, "dark"),
            (ThemeMode::Light, true) => set_color_scheme(&settings, "light"),
            (ThemeMode::System | ThemeMode::Gtk, true) => set_color_scheme(&settings, "default"),
            (ThemeMode::Dark, false) => settings.set_gtk_application_prefer_dark_theme(true),
            (ThemeMode::Light, false) => settings.set_gtk_application_prefer_dark_theme(false),
            (ThemeMode::System | ThemeMode::Gtk, false) => {}
        }
        let dark = resolve_dark(config.appearance.theme);

        if let Some(old) = self.fallback.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        if config.appearance.theme == ThemeMode::Gtk {
            let fallback = gtk::CssProvider::new();
            log_parsing_errors(&fallback, "theme fallback");
            fallback.load_from_string(&fallback_css(dark));
            gtk::style_context_add_provider_for_display(&display, &fallback, gtk::STYLE_PROVIDER_PRIORITY_FALLBACK);
            *self.fallback.borrow_mut() = Some(fallback);
        }

        if let Some(old) = self.app.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }
        if let Some(old) = self.user.borrow_mut().take() {
            gtk::style_context_remove_provider_for_display(&display, &old);
        }

        let provider = gtk::CssProvider::new();
        log_parsing_errors(&provider, "theme");
        provider.load_from_string(&generate_css(config, dark));
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        *self.app.borrow_mut() = Some(provider);

        let user_css = crate::paths::user_css_file();
        match std::fs::read_to_string(&user_css) {
            Ok(text) => {
                let user = gtk::CssProvider::new();
                log_parsing_errors(&user, "style.css");
                user.load_from_string(&text);
                gtk::style_context_add_provider_for_display(&display, &user, gtk::STYLE_PROVIDER_PRIORITY_USER);
                *self.user.borrow_mut() = Some(user);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("cannot read {}: {e}", user_css.display()),
        }
        dark
    }
}

#[cfg(test)]
mod gtk_color_tests {
    use super::{fallback_css, generate_css};
    use crate::config::{Config, ThemeMode};

    #[test]
    fn builtin_themes_use_literal_colours() {
        let css = generate_css(&Config::default(), true);
        assert!(!css.contains("@window_bg_color"), "{css}");
        assert!(css.contains("rgba(203, 166, 247"), "Mocha accent expected");
    }

    #[test]
    fn gtk_theme_references_named_colours() {
        let mut cfg = Config::default();
        cfg.appearance.theme = ThemeMode::Gtk;
        let css = generate_css(&cfg, true);
        for name in ["window_fg_color", "card_bg_color", "popover_bg_color", "accent_bg_color", "accent_fg_color"] {
            assert!(css.contains(&format!("@{name}")), "missing @{name}");
        }
        assert!(css.contains("alpha(@window_bg_color, 0.940)"), "card keeps the configured opacity");
        assert!(!css.contains("rgba(203, 166, 247"), "no built-in accent in gtk mode");
    }

    #[test]
    fn config_colours_still_win_in_gtk_mode() {
        let mut cfg = Config::default();
        cfg.appearance.theme = ThemeMode::Gtk;
        cfg.appearance.accent = Some("#ff0000".into());
        let css = generate_css(&cfg, true);
        assert!(!css.contains("@accent_bg_color"), "{css}");
        assert!(css.contains("rgba(255, 0, 0, 1.000)"));
        assert!(css.contains("@window_fg_color"), "other colours still follow GTK");
    }

    #[test]
    fn fallback_defines_every_name() {
        let css = fallback_css(true);
        for name in [
            "window_bg_color", "window_fg_color", "card_bg_color", "popover_bg_color",
            "accent_bg_color", "accent_fg_color", "destructive_bg_color",
        ] {
            assert!(css.contains(&format!("@define-color {name} rgba(")), "missing {name}");
        }
    }
}
