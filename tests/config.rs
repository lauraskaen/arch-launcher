use arch_launcher::config::{default_config_toml, AnchorSetting, Config, ThemeMode};

#[test]
fn default_toml_round_trips_to_default_config() {
    let (cfg, warnings) = Config::from_toml_str(&default_config_toml());
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert_eq!(cfg, Config::default());
}

#[test]
fn empty_file_is_default() {
    let (cfg, w) = Config::from_toml_str("");
    assert_eq!(cfg, Config::default());
    assert!(w.is_empty());
}

#[test]
fn partial_file_keeps_other_defaults() {
    let (cfg, w) = Config::from_toml_str("[appearance]\ntheme = \"light\"\nicon_size = 48\n\n[window]\nanchor = \"top\"\n");
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg.appearance.theme, ThemeMode::Light);
    assert_eq!(cfg.appearance.icon_size, 48);
    assert_eq!(cfg.window.anchor, AnchorSetting::Top);
    assert_eq!(cfg.window.width, Config::default().window.width);
    assert_eq!(cfg.keybindings, Config::default().keybindings);
}

#[test]
fn invalid_numbers_are_clamped_with_warnings() {
    let (cfg, w) = Config::from_toml_str(
        "[appearance]\nopacity = 7.5\nicon_size = 4\nborder_radius = 9999\n[behavior]\nmax_results = 0\n[window]\nwidth = 10\n",
    );
    assert_eq!(cfg.appearance.opacity, 1.0);
    assert_eq!(cfg.appearance.icon_size, 16);
    assert_eq!(cfg.appearance.border_radius, 64);
    assert_eq!(cfg.behavior.max_results, 1);
    assert_eq!(cfg.window.width, 320);
    assert!(w.len() >= 5, "{w:?}");
}

#[test]
fn wrong_types_and_unknown_keys_warn_but_do_not_fail() {
    let (cfg, w) = Config::from_toml_str(
        "[appearance]\nfont_size = \"big\"\ntheme = \"light\"\nnonsense = 1\n[mystery]\nx = 1\n[window]\nanchor = \"middle\"\n",
    );
    assert_eq!(cfg.appearance.theme, ThemeMode::Light, "valid keys survive");
    assert_eq!(cfg.appearance.font_size, Config::default().appearance.font_size);
    assert_eq!(cfg.window.anchor, AnchorSetting::Center);
    let all = w.join("\n");
    assert!(all.contains("font_size"), "{all}");
    assert!(all.contains("appearance.nonsense"), "{all}");
    assert!(all.contains("mystery"), "{all}");
    assert!(all.contains("anchor"), "{all}");
}

#[test]
fn syntax_error_falls_back_to_defaults() {
    let (cfg, w) = Config::from_toml_str("[appearance\nthis is not toml");
    assert_eq!(cfg, Config::default());
    assert_eq!(w.len(), 1);
}

#[test]
fn invalid_color_is_ignored_valid_color_kept() {
    let (cfg, w) = Config::from_toml_str("[appearance]\naccent = \"not-a-colour\"\nbackground = \"#112233\"\n");
    assert_eq!(cfg.appearance.accent, None);
    assert_eq!(cfg.appearance.background.as_deref(), Some("#112233"));
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn bad_accelerator_warns_and_is_dropped() {
    let (cfg, w) = Config::from_toml_str("[keybindings]\nnext = [\"Down\", \"<Bogus>>>zz\"]\nclose = [\"%%%\"]\nclear_search = []\n");
    assert_eq!(cfg.keybindings.next, vec!["Down".to_owned()]);
    assert_eq!(cfg.keybindings.close, Config::default().keybindings.close, "all-invalid falls back to default");
    assert!(cfg.keybindings.clear_search.is_empty(), "explicit empty list disables the action");
    assert!(w.iter().any(|m| m.contains("next")), "{w:?}");
    assert!(w.iter().any(|m| m.contains("close")), "{w:?}");
}

#[test]
fn placement_mapping() {
    let (cfg, _) = Config::from_toml_str("[window]\nwidth = 700\nheight = 500\nanchor = \"top\"\nmargin_top = 80\nlayer_shell = false\n");
    let p = cfg.placement();
    assert_eq!((p.width, p.height, p.margin_top, p.layer_shell), (700, 500, 80, false));
    assert_eq!(p.anchor, arch_launcher::platform::Anchor::Top);
}

#[test]
fn load_from_missing_file_is_default() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, w) = Config::load_from(&tmp.path().join("nope.toml"));
    assert_eq!(cfg, Config::default());
    assert!(w.is_empty());
}

#[test]
fn gtk_theme_mode_parses() {
    let (cfg, w) = Config::from_toml_str("[appearance]\ntheme = \"gtk\"\n");
    assert!(w.is_empty(), "{w:?}");
    assert_eq!(cfg.appearance.theme, ThemeMode::Gtk);
}
