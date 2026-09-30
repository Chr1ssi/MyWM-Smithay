//! Compositor-independent layout logic, ported from the River-based MyWM.
mod appearance;
mod arrange;
mod geometry;
pub mod scrolling;
mod workspaces;

pub use appearance::{Appearance, Color};
pub use arrange::{Placement, WindowInfo, arrange, place_floating};
pub use geometry::{DragKind, Edges, Rect};
pub use workspaces::{GAMING, Kind, MAX_NUMBER, Workspace, Workspaces};
