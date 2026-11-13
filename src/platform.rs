//! Display-server integration (Wayland / X11).
//!
//! * **Wayland + wlr-layer-shell** (Hyprland, Sway, KDE Plasma, river, niri…):
//!   the window becomes an overlay layer surface — centred by the compositor,
//!   no decorations, exclusive keyboard focus, never tiled.
//! * **Wayland without layer-shell** (GNOME/Mutter): an ordinary undecorated
//!   toplevel. Placement is decided by the compositor.
//! * **X11**: an undecorated toplevel with `WM_CLASS` = app id. GTK4 cannot
//!   position windows on X11; most WMs centre new dialogs, and tiling WMs
//!   need a float rule (see README).
//!
//! Global shortcuts (Super+D) are *never* grabbed: on Wayland that is the
//! compositor's job, so the user binds `arch-launcher` there.

use gtk::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Wayland,
    X11,
    Other,
}

impl Backend {
    pub fn detect() -> Self {
        let Some(display) = gtk::gdk::Display::default() else {
            return Backend::Other;
        };
        let name = display.type_().name();
        if name.contains("Wayland") {
            Backend::Wayland
        } else if name.contains("X11") {
            Backend::X11
        } else {
            Backend::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Wayland => "wayland",
            Backend::X11 => "x11",
            Backend::Other => "other",
        }
    }
}

/// Where the launcher window appears on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Center,
    Top,
}

#[derive(Debug, Clone)]
pub struct Placement {
    pub width: i32,
    pub height: i32,
    pub anchor: Anchor,
    /// Distance from the top edge when `anchor == Top` (layer-shell only).
    pub margin_top: i32,
    /// Allow using wlr-layer-shell when available.
    pub layer_shell: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self { width: 640, height: 420, anchor: Anchor::Center, margin_top: 200, layer_shell: true }
    }
}

/// What kind of surface the window ended up as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceKind {
    LayerShell,
    Toplevel,
}

/// Configures a launcher window for the current display server. Must be
/// called **before** the window is first shown (layer-shell needs to be
/// initialised before the surface is realised).
pub fn prepare_window(window: &impl IsA<gtk::Window>, placement: &Placement) -> SurfaceKind {
    let window = window.upcast_ref::<gtk::Window>();
    window.set_title(Some("Arch Launcher"));
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_default_size(placement.width, placement.height);
    window.add_css_class("arch-launcher");

    #[cfg(feature = "layer-shell")]
    if placement.layer_shell && Backend::detect() == Backend::Wayland && try_layer_shell(window, placement) {
        window.add_css_class("layer-shell");
        return SurfaceKind::LayerShell;
    }

    window.add_css_class("toplevel");
    SurfaceKind::Toplevel
}

#[cfg(feature = "layer-shell")]
fn try_layer_shell(window: &gtk::Window, placement: &Placement) -> bool {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

    if !gtk4_layer_shell::is_supported() {
        log::info!("compositor has no wlr-layer-shell; using a regular window");
        return false;
    }
    window.init_layer_shell();
    window.set_namespace(Some(crate::paths::APP_DIR_NAME));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(-1); // do not get pushed by panels' exclusive zones
    match placement.anchor {
        Anchor::Center => {} // no anchors = centred on the output
        Anchor::Top => {
            window.set_anchor(Edge::Top, true);
            window.set_margin(Edge::Top, placement.margin_top);
        }
    }
    true
}

/// `true` if the window is a layer-shell surface.
pub fn is_layer_surface(window: &impl IsA<gtk::Window>) -> bool {
    #[cfg(feature = "layer-shell")]
    {
        use gtk4_layer_shell::LayerShell;
        return window.upcast_ref::<gtk::Window>().is_layer_window();
    }
    #[allow(unreachable_code)]
    {
        let _ = window;
        false
    }
}
