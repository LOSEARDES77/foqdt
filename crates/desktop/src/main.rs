use foqdt_ui::eframe::{self, egui};
use foqdt_ui::{Camera, File, Grab, Platform, threaded_camera};
use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
};
use std::sync::mpsc;

struct Desktop;

impl Platform for Desktop {
    fn start_camera(&self, ctx: egui::Context, index: u32) -> Box<dyn Camera> {
        threaded_camera(ctx, move || {
            let want = CameraFormat::new(Resolution::new(1920, 1080), FrameFormat::MJPEG, 30);
            let fmt = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(want));
            let mut cam =
                nokhwa::Camera::new(CameraIndex::Index(index), fmt).map_err(|e| e.to_string())?;
            cam.open_stream().map_err(|e| e.to_string())?;
            let f = cam.camera_format();
            let mode = format!("{} · {} fps", f.format(), f.frame_rate());
            Ok((mode, move || {
                let img = cam
                    .frame()
                    .and_then(|f| f.decode_image::<RgbFormat>())
                    .map_err(|e| e.to_string())?;
                let (w, h) = (img.width() as usize, img.height() as usize);
                let luma = img
                    .as_raw()
                    .chunks_exact(3)
                    .map(|p| foqdt_core::luma(p[0], p[1], p[2]))
                    .collect();
                let preview = egui::ColorImage::from_rgb([w, h], img.as_raw());
                Ok(Grab {
                    w,
                    h,
                    luma,
                    preview,
                })
            }))
        })
    }

    fn pick_file(&self, done: mpsc::Sender<File>) {
        std::thread::spawn(move || {
            let Some(path) = rfd::FileDialog::new().pick_file() else {
                return;
            };
            match std::fs::read(&path) {
                Ok(data) => {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let _ = done.send(File { name, data });
                }
                Err(e) => eprintln!("read {}: {e}", path.display()),
            }
        });
    }

    fn save_file(&self, file: &File) -> Result<(), String> {
        if let Some(path) = rfd::FileDialog::new().set_file_name(&file.name).save_file() {
            std::fs::write(path, &file.data).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Write to a temp folder and hand it to the desktop's default app.
    fn open_file(&self, file: &File) -> Result<(), String> {
        let dir = std::env::temp_dir().join("foqdt");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // File names come off the wire: keep only the last path component.
        let name = std::path::Path::new(&file.name)
            .file_name()
            .unwrap_or("received".as_ref());
        let path = dir.join(name);
        std::fs::write(&path, &file.data).map_err(|e| e.to_string())?;
        #[cfg(target_os = "windows")]
        let mut cmd = std::process::Command::new("explorer");
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("open");
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(&path)
            .spawn()
            .map(drop)
            .map_err(|e| format!("open {}: {e}", path.display()))
    }

    fn has_camera_index(&self) -> bool {
        true
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        "foqdt — Fast Offline QR Data Transfer",
        eframe::NativeOptions {
            // Matches the .desktop file, so docks and taskbars show the right icon.
            viewport: egui::ViewportBuilder::default()
                .with_app_id("foqdt")
                .with_icon(
                    eframe::icon_data::from_png_bytes(include_bytes!(
                        "../../../packaging/icon-512.png"
                    ))
                    .expect("icon"),
                ),
            ..Default::default()
        },
        Box::new(|cc| {
            let app = foqdt_ui::App::new(cc, Box::new(Desktop));
            // `foqdt FILE` starts streaming FILE immediately.
            Ok(Box::new(
                match std::env::args_os().nth(1).map(std::path::PathBuf::from) {
                    Some(path) => {
                        let data =
                            std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        app.send(File { name, data })
                    }
                    None => app,
                },
            ))
        }),
    )
}
