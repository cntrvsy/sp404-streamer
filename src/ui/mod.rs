//! Immediate-mode graphical user interface built with egui.
//! Provides the Roland SP-404MKII styled desktop HUD, dual stereo VU meters with peak hold,
//! live output sink dropdown selection, and local monitor mute controls.

pub mod app;
pub mod crate_view;
pub mod theme;
pub mod widgets;

pub use app::Sp404StreamerApp;
pub use theme::RolandTheme;
