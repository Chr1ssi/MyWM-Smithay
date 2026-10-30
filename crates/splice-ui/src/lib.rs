//! The built-in shell's pictures, without a compositor: what the bar, the power menu and the notices on
//! the right show (`bar`, `power`, `notices` build a `Scene` from plain data), how a scene is painted into
//! pixels (`paint`, with the fonts of `text`), the theme, the texts in the system language (`i18n`) and the
//! clock. The compositor fills the models, uploads the pixels and routes input to the scenes' targets.
//!
//! Sizes and colors started as those of splice-shell's Quickshell bar (`bar.qml`, `BarButton.qml`, `BarIsland.qml`,
//! `Theme.qml`); the panels group their content in rounded cards (`theme` has the radii), and text is set in the
//! system's sans-serif font.
pub mod audio;
pub mod bar;
pub mod calendar;
pub mod clock;
pub mod control;
pub mod edit;
pub mod i18n;
pub mod launcher;
pub mod lock_screen;
pub mod markup;
pub mod media;
pub mod notices;
pub mod notifications;
pub mod paint;
pub mod panel;
pub mod power;
pub mod scene;
pub mod text;
pub mod theme;
pub mod tray_menu;
pub mod wallpaper_picker;

pub use scene::{Align, Color, Measure, Node, Rect, Scene, TextStyle};
pub use theme::{Shape, Theme};
