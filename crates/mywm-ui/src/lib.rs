//! The built-in shell's pictures, without a compositor: what the bar, the power menu and the notices on
//! the right show (`bar`, `power`, `notices` build a `Scene` from plain data), how a scene is painted into
//! pixels (`paint`, with the fonts of `text`), the theme, the texts in the system language (`i18n`) and the
//! clock. The compositor fills the models, uploads the pixels and routes input to the scenes' targets.
//!
//! Sizes, colors and fonts are those of mywm-shell's Quickshell bar (`bar.qml`, `BarButton.qml`,
//! `BarIsland.qml`, `Theme.qml`), so the built-in shell looks the same.
pub mod bar;
pub mod clock;
pub mod i18n;
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

pub use scene::{Align, Color, Measure, Node, Rect, Scene, TextStyle};
pub use theme::Theme;
