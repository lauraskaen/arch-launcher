//! Configuration Manager: `~/.config/arch-launcher/config.toml`.
//!
//! Every section and key is optional (`#[serde(default)]`). Loading never
//! fails: unreadable files fall back to the defaults, wrongly typed keys are
//! ignored one by one, out-of-range numbers are clamped, invalid colours and
//! accelerators are dropped — each correction produces a warning string.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::platform::{Anchor, Placement};

// ---- appearance ------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Dark,
    Light,
    System,
    /// Like `System`, but the colours come from the GTK theme's named colours
    /// (`@window_bg_color`, `@accent_bg_color`, …), e.g. those that
    /// matugen / DankMaterialShell write to `~/.config/gtk-4.0/gtk.css`.
    Gtk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeMode,
    pub font_family: String,
    /// Points.
    pub font_size: f64,
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub accent: Option<String>,
    pub selection: Option<String>,
    pub border_color: Option<String>,
    pub opacity: f64,
    pub border_radius: u32,
    pub border_width: u32,
    pub padding: u32,
    pub icon_size: u32,
    pub show_descriptions: bool,
    pub animations: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            font_family: "sans-serif".into(),
            font_size: 11.0,
            background: None,
            foreground: None,
            accent: None,
            selection: None,
            border_color: None,
            opacity: 0.94,
            border_radius: 16,
            border_width: 1,
            padding: 12,
            icon_size: 32,
            show_descriptions: true,
            animations: true,
        }
    }
}

// ---- window ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorSetting {
    Center,
    Top,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: i32,
    pub height: i32,
    pub anchor: AnchorSetting,
    pub margin_top: i32,
    pub layer_shell: bool,
    pub close_on_focus_loss: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 640, height: 480, anchor: AnchorSetting::Center, margin_top: 200, layer_shell: true, close_on_focus_loss: true }
    }
}

// ---- behavior --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Behavior {
    pub max_results: usize,
    pub sort_by_usage: bool,
    pub search_keywords: bool,
    pub search_descriptions: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self { max_results: 50, sort_by_usage: true, search_keywords: true, search_descriptions: true }
    }
}

// ---- keybindings -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keybindings {
    pub launch: Vec<String>,
    pub close: Vec<String>,
    pub next: Vec<String>,
    pub previous: Vec<String>,
    pub hide_selected: Vec<String>,
    pub hidden_manager: Vec<String>,
    pub clear_search: Vec<String>,
    pub context_menu: Vec<String>,
}

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).to_owned()).collect()
}

impl Default for Keybindings {
    fn default() -> Self {
        Self {
            launch: strs(&["Return", "KP_Enter"]),
            close: strs(&["Escape"]),
            next: strs(&["Down", "<Control>n", "<Control>j"]),
            previous: strs(&["Up", "<Control>p", "<Control>k"]),
            hide_selected: strs(&["<Control><Shift>x"]),
            hidden_manager: strs(&["<Control><Shift>h"]),
            clear_search: strs(&["<Control>BackSpace"]),
            context_menu: strs(&["Menu", "<Shift>F10"]),
        }
    }
}

impl Keybindings {
    fn fields_mut(&mut self) -> [(&'static str, &mut Vec<String>); 8] {
        [
            ("launch", &mut self.launch),
            ("close", &mut self.close),
            ("next", &mut self.next),
            ("previous", &mut self.previous),
            ("hide_selected", &mut self.hide_selected),
            ("hidden_manager", &mut self.hidden_manager),
            ("clear_search", &mut self.clear_search),
            ("context_menu", &mut self.context_menu),
        ]
    }
}

// ---- root ------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub appearance: Appearance,
    pub window: WindowConfig,
    pub behavior: Behavior,
    pub keybindings: Keybindings,
}

/// Deserialises `T` from a TOML table. If the table has wrongly typed keys,
/// those are ignored one by one (with a warning) instead of losing the whole
/// section.
fn lenient_section<T>(section: &str, value: Option<&toml::Value>, warnings: &mut Vec<String>) -> T
where
    T: Default + Serialize + for<'de> Deserialize<'de>,
{
    let Some(value) = value else { return T::default() };
    let Some(user) = value.as_table() else {
        warnings.push(format!("[{section}] deve ser uma tabela; usando os valores padrão"));
        return T::default();
    };
    if let Ok(v) = value.clone().try_into::<T>() {
        return v;
    }
    let mut merged = match toml::Value::try_from(T::default()) {
        Ok(toml::Value::Table(t)) => t,
        _ => return T::default(),
    };
    for (k, v) in user {
        let previous = merged.insert(k.clone(), v.clone());
        if toml::Value::Table(merged.clone()).try_into::<T>().is_err() {
            warnings.push(format!("[{section}] valor inválido para `{k}`; usando o padrão"));
            match previous {
                Some(p) => merged.insert(k.clone(), p),
                None => merged.remove(k),
            };
        }
    }
    toml::Value::Table(merged).try_into::<T>().unwrap_or_default()
}

fn collect_unknown(prefix: &str, user: &toml::Table, known: &toml::Table, out: &mut Vec<String>) {
    for (k, v) in user {
        let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
        match known.get(k) {
            None => out.push(format!("chave desconhecida `{path}` (ignorada)")),
            Some(toml::Value::Table(kt)) => {
                if let toml::Value::Table(ut) = v {
                    collect_unknown(&path, ut, kt, out);
                }
            }
            Some(_) => {}
        }
    }
}

/// Keys that are valid but absent from the serialised default (`None` options are skipped).
const OPTIONAL_APPEARANCE_KEYS: [&str; 5] = ["background", "foreground", "accent", "selection", "border_color"];

/// Parses a GTK accelerator string such as `<Control><Shift>x`.
///
/// Calls `gtk_accelerator_parse` directly: the safe wrapper insists on an
/// initialised GTK, which unit tests (and config loading) do not have.
pub fn parse_accel(accel: &str) -> Option<(gtk::gdk::Key, gtk::gdk::ModifierType)> {
    let c = std::ffi::CString::new(accel).ok()?;
    let mut key: u32 = 0;
    let mut mods: gtk::gdk::ffi::GdkModifierType = 0;
    // SAFETY: `c` is a valid NUL-terminated string and both out-pointers
    // point to live locals; the function does not retain them.
    let ok = unsafe { gtk::ffi::gtk_accelerator_parse(c.as_ptr(), &mut key, &mut mods) };
    // GTK reports failure by returning TRUE with key == 0.
    if ok == 0 || key == 0 {
        return None;
    }
    use glib::translate::from_glib;
    // SAFETY: plain conversions of values GTK just produced.
    unsafe { Some((from_glib(key), from_glib(mods))) }
}

fn parse_color(s: &str) -> bool {
    gtk::gdk::RGBA::parse(s).is_ok()
}

fn clamp_u32(name: &str, v: &mut u32, lo: u32, hi: u32, w: &mut Vec<String>) {
    let c = (*v).clamp(lo, hi);
    if c != *v {
        w.push(format!("{name} = {v} fora do intervalo {lo}–{hi}; usando {c}"));
        *v = c;
    }
}

fn clamp_i32(name: &str, v: &mut i32, lo: i32, hi: i32, w: &mut Vec<String>) {
    let c = (*v).clamp(lo, hi);
    if c != *v {
        w.push(format!("{name} = {v} fora do intervalo {lo}–{hi}; usando {c}"));
        *v = c;
    }
}

impl Config {
    /// `~/.config/arch-launcher/config.toml`, never fails.
    pub fn load() -> (Config, Vec<String>) {
        Self::load_from(&crate::paths::config_file())
    }

    pub fn load_from(path: &Path) -> (Config, Vec<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::from_toml_str(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
            Err(e) => (Config::default(), vec![format!("não foi possível ler {}: {e}", path.display())]),
        }
    }

    /// Parses and validates a config document.
    pub fn from_toml_str(text: &str) -> (Config, Vec<String>) {
        let mut warnings = Vec::new();
        let root: toml::Table = match toml::from_str(text) {
            Ok(t) => t,
            Err(e) => {
                let msg = e.message().to_owned();
                return (Config::default(), vec![format!("config.toml inválido ({msg}); usando os valores padrão")]);
            }
        };

        // Unknown keys → warnings.
        if let Ok(toml::Value::Table(known)) = toml::Value::try_from(Config::default()) {
            let mut known = known;
            if let Some(toml::Value::Table(app)) = known.get_mut("appearance") {
                for k in OPTIONAL_APPEARANCE_KEYS {
                    app.insert(k.to_owned(), toml::Value::String(String::new()));
                }
            }
            collect_unknown("", &root, &known, &mut warnings);
        }

        let mut cfg = Config {
            appearance: lenient_section("appearance", root.get("appearance"), &mut warnings),
            window: lenient_section("window", root.get("window"), &mut warnings),
            behavior: lenient_section("behavior", root.get("behavior"), &mut warnings),
            keybindings: lenient_section("keybindings", root.get("keybindings"), &mut warnings),
        };
        cfg.validate(&mut warnings);
        (cfg, warnings)
    }

    /// Clamps numbers and drops invalid colours / accelerators.
    pub fn validate(&mut self, w: &mut Vec<String>) {
        let a = &mut self.appearance;
        if !a.font_size.is_finite() || !(6.0..=48.0).contains(&a.font_size) {
            let c = if a.font_size.is_finite() { a.font_size.clamp(6.0, 48.0) } else { 11.0 };
            w.push(format!("appearance.font_size = {} fora do intervalo 6–48; usando {c}", a.font_size));
            a.font_size = c;
        }
        if a.font_family.trim().is_empty() {
            w.push("appearance.font_family vazio; usando sans-serif".into());
            a.font_family = "sans-serif".into();
        }
        if !a.opacity.is_finite() || !(0.0..=1.0).contains(&a.opacity) {
            let c = if a.opacity.is_finite() { a.opacity.clamp(0.0, 1.0) } else { 0.94 };
            w.push(format!("appearance.opacity = {} fora do intervalo 0–1; usando {c}", a.opacity));
            a.opacity = c;
        }
        clamp_u32("appearance.border_radius", &mut a.border_radius, 0, 64, w);
        clamp_u32("appearance.border_width", &mut a.border_width, 0, 8, w);
        clamp_u32("appearance.padding", &mut a.padding, 0, 64, w);
        clamp_u32("appearance.icon_size", &mut a.icon_size, 16, 128, w);
        for (name, slot) in [
            ("background", &mut a.background),
            ("foreground", &mut a.foreground),
            ("accent", &mut a.accent),
            ("selection", &mut a.selection),
            ("border_color", &mut a.border_color),
        ] {
            if let Some(c) = slot.as_deref() {
                if !parse_color(c) {
                    w.push(format!("appearance.{name} = {c:?} não é uma cor válida; ignorada"));
                    *slot = None;
                }
            }
        }

        let win = &mut self.window;
        clamp_i32("window.width", &mut win.width, 320, 3840, w);
        clamp_i32("window.height", &mut win.height, 240, 2160, w);
        clamp_i32("window.margin_top", &mut win.margin_top, 0, 4000, w);

        if self.behavior.max_results < 1 {
            w.push("behavior.max_results deve ser pelo menos 1; usando 1".into());
            self.behavior.max_results = 1;
        }
        self.behavior.max_results = self.behavior.max_results.min(10_000);

        let mut defaults = Keybindings::default();
        let default_fields: Vec<(&'static str, Vec<String>)> =
            defaults.fields_mut().into_iter().map(|(n, v)| (n, v.clone())).collect();
        for ((name, list), (_, default)) in self.keybindings.fields_mut().into_iter().zip(default_fields) {
            let had = list.len();
            list.retain(|accel| {
                let ok = parse_accel(accel).is_some();
                if !ok {
                    w.push(format!("keybindings.{name}: atalho inválido {accel:?}; ignorado"));
                }
                ok
            });
            if list.is_empty() && had > 0 {
                w.push(format!("keybindings.{name}: nenhum atalho válido; usando o padrão"));
                *list = default;
            }
        }
    }

    /// Window geometry/placement for `platform::prepare_window`.
    pub fn placement(&self) -> Placement {
        Placement {
            width: self.window.width,
            height: self.window.height,
            anchor: match self.window.anchor {
                AnchorSetting::Center => Anchor::Center,
                AnchorSetting::Top => Anchor::Top,
            },
            margin_top: self.window.margin_top,
            layer_shell: self.window.layer_shell,
        }
    }
}

/// The fully commented default configuration file. Parsing it yields exactly
/// `Config::default()`.
pub fn default_config_toml() -> String {
    r##"# Configuração do arch-launcher
# Todas as chaves são opcionais: remova qualquer uma para usar o padrão.
# Depois de editar, use o menu ⚙ → "Recarregar configuração".
# CSS extra (opcional): ~/.config/arch-launcher/style.css

[appearance]
# "dark", "light", "system" (segue o tema do GTK; se não der para saber, escuro)
# ou "gtk" (usa as cores do tema GTK / ~/.config/gtk-4.0/gtk.css, como as do matugen/DMS)
theme = "dark"
font_family = "sans-serif"
# Tamanho da fonte em pontos
font_size = 11.0
# Sobrescreva cores com qualquer valor aceito pelo GTK (#rrggbb, rgba(...), nomes):
# background = "#1e1e2e"
# foreground = "#cdd6f4"
# accent = "#cba6f7"
# selection = "rgba(203, 166, 247, 0.22)"
# border_color = "#45475a"
# Opacidade do fundo do cartão (0.0 a 1.0)
opacity = 0.94
# Valores em pixels
border_radius = 16
border_width = 1
padding = 12
# Tamanho dos ícones (16 a 128)
icon_size = 32
show_descriptions = true
# Transições suaves
animations = true

[window]
width = 640
height = 480
# "center" ou "top"
anchor = "center"
# Distância do topo quando anchor = "top"
margin_top = 200
# Usar wlr-layer-shell no Wayland quando disponível
layer_shell = true
# Fechar quando a janela perde o foco
close_on_focus_loss = true

[behavior]
max_results = 50
# Ordenar por frequência/recência de uso
sort_by_usage = true
search_keywords = true
search_descriptions = true

[keybindings]
# Cada ação aceita uma lista de atalhos no formato de aceleradores do GTK.
launch = ["Return", "KP_Enter"]
close = ["Escape"]
next = ["Down", "<Control>n", "<Control>j"]
previous = ["Up", "<Control>p", "<Control>k"]
hide_selected = ["<Control><Shift>x"]
hidden_manager = ["<Control><Shift>h"]
clear_search = ["<Control>BackSpace"]
context_menu = ["Menu", "<Shift>F10"]
"##
    .to_owned()
}
