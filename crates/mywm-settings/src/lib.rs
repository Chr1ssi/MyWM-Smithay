//! Logic of the settings editor (everything but the window): which settings exist, reading and
//! editing the TOML file without losing comments, where the file is, and keybinding capture.
pub mod files;
pub mod keys;
pub mod model;
