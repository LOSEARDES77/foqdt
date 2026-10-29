//! The egui app shared by the desktop, web and Android frontends.
//! Each frontend supplies a [`Platform`] for camera access and file dialogs.

mod app;
mod platform;
mod theme;
#[cfg(not(target_arch = "wasm32"))]
mod threaded;

pub use app::App;
pub use eframe;
pub use foqdt_core::File;
pub use platform::{Camera, Platform, Shot};
#[cfg(not(target_arch = "wasm32"))]
pub use threaded::{Grab, threaded_camera};
