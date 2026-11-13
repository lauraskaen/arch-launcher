//! arch-launcher — a fast GTK4 application launcher for Arch Linux.
//!
//! Module map (see ARCHITECTURE.md):
//!
//! | Layer                 | Module         |
//! |-----------------------|----------------|
//! | Application Discovery | [`discovery`]  |
//! | Visibility Manager    | [`visibility`] |
//! | App model + watching  | [`store`]      |
//! | Search Engine         | [`search`]     |
//! | Usage history         | [`usage`]      |
//! | Configuration Manager | [`config`]     |
//! | Theme Manager         | [`theme`]      |
//! | Launcher Service      | [`launcher`]   |
//! | Wayland / X11         | [`platform`]   |
//! | Lifecycle             | [`app`]        |
//! | UI Components         | [`ui`]         |

pub mod app;
pub mod atomic;
pub mod config;
pub mod discovery;
pub mod launcher;
pub mod paths;
pub mod platform;
pub mod search;
pub mod store;
pub mod theme;
pub mod ui;
pub mod usage;
pub mod visibility;
