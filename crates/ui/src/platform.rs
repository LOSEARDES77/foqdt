//! Platform hooks each frontend implements: camera access, file dialogs and system UI.

use eframe::egui;
use std::sync::mpsc;

use crate::File;

/// Camera events: a captured frame (`size`, usually with a preview), a scanned frame
/// (`scanned`, with any QR payloads found), and once at start the negotiated `mode`.
#[derive(Default)]
pub struct Shot {
    pub preview: Option<egui::ColorImage>,
    /// Exact capture size of this frame, in pixels.
    pub size: Option<[usize; 2]>,
    pub scanned: bool,
    pub payloads: Vec<Vec<u8>>,
    /// What the driver agreed to, e.g. "MJPEG · 30 fps".
    pub mode: Option<String>,
}

/// A running capture. Dropping it stops the camera.
pub trait Camera {
    /// Frames captured since the last call. An `Err` ends the capture.
    fn poll(&mut self) -> Vec<Result<Shot, String>>;
    /// Whether the UI must keep repainting to drive capture (the web camera is polled).
    fn needs_polling(&self) -> bool {
        false
    }
}

/// What a frontend provides: camera, file dialogs and system UI hooks.
pub trait Platform {
    /// Open camera `index` (ignored where the OS picks) and start capturing.
    /// `ctx` lets capture threads request a repaint when a frame arrives.
    fn start_camera(&self, ctx: egui::Context, index: u32) -> Box<dyn Camera>;
    /// Pick a file and send it to `done`; nothing is sent when the user cancels.
    fn pick_file(&self, done: mpsc::Sender<File>);
    /// Ask where to save the file and write it there; cancelling is not an error.
    fn save_file(&self, file: &File) -> Result<(), String>;
    /// Open the file with the system's default app for its type.
    fn open_file(&self, file: &File) -> Result<(), String>;
    /// Whether the UI should offer a camera number picker.
    fn has_camera_index(&self) -> bool {
        false
    }
    /// Space covered by system bars, display cutouts or the on-screen keyboard,
    /// in physical pixels: `[left, top, right, bottom]`.
    fn safe_area(&self) -> [f32; 4] {
        [0.0; 4]
    }
    /// Show or hide the on-screen keyboard (only Android needs to be told).
    fn show_keyboard(&self, _show: bool) {}
    /// The app switched between dark and light; Android recolours its system bar icons.
    fn theme_changed(&self, _dark: bool) {}
    /// Read a file dropped onto the window. The web frontend overrides this: browsers read files asynchronously.
    fn read_dropped(&self, file: egui::DroppedFileHandle, done: mpsc::Sender<File>) {
        #[cfg(not(target_arch = "wasm32"))]
        match file.bytes() {
            Ok(data) => {
                let name = file
                    .path()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let _ = done.send(File { name, data });
            }
            Err(e) => eprintln!("read {}: {e}", file.path().display()),
        }
        #[cfg(target_arch = "wasm32")]
        let _ = (file, done);
    }
}
