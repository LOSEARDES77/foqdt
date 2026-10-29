//! The [`App`]: send and receive tabs, layout, and the persisted settings.

use eframe::egui;
use foqdt_core::{Receiver, Sender};
use std::sync::mpsc;

use crate::theme::{self, Icon, Palette};
use crate::{Camera, File, Platform};

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Send,
    Receive,
}

/// Side column width in the two-pane layout; below `WIDE` the panes stack.
const SIDE: f32 = 340.0;
const WIDE: f32 = 760.0;

/// The whole app; frontends hand it to eframe.
pub struct App {
    platform: Box<dyn Platform>,
    tab: Tab,
    // send
    picked_tx: mpsc::Sender<File>,
    picked_rx: mpsc::Receiver<File>,
    file: Option<File>,
    text: String,
    frame_bytes: usize,
    codes: usize,
    fps: f32,
    sender: Option<Sender>,
    /// A sender being built on a worker thread (Brotli at quality 11 takes seconds on big files).
    preparing: Option<mpsc::Receiver<Sender>>,
    send_error: String,
    fullscreen: bool,
    frame_n: u64,
    next_frame_at: f64,
    qr: Option<egui::TextureHandle>,
    // receive
    camera: Option<Box<dyn Camera>>,
    camera_index: u32,
    receiver: Receiver,
    preview: Option<egui::TextureHandle>,
    started_at: Option<f64>,
    codes_read: usize,
    cam: CamStats,
    received: Option<File>,
    error: String,
    keyboard: bool,
    dark: Option<bool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, platform: Box<dyn Platform>) -> App {
        theme::apply(&cc.egui_ctx);
        let saved = cc
            .storage
            .and_then(|s| s.get_string(SETTINGS))
            .unwrap_or_default();
        let mut saved = saved.split(',').map(|v| v.parse::<f32>().ok());
        let mut next = |default: f32, range: std::ops::RangeInclusive<f32>| {
            saved
                .next()
                .flatten()
                .filter(|v| range.contains(v))
                .unwrap_or(default)
        };
        let (frame_bytes, codes, fps, camera_index) = (
            next(1200.0, 100.0..=2900.0) as usize,
            next(2.0, 1.0..=4.0) as usize,
            next(15.0, 1.0..=60.0),
            next(0.0, 0.0..=63.0) as u32,
        );
        let (picked_tx, picked_rx) = mpsc::channel();
        App {
            platform,
            tab: Tab::Send,
            picked_tx,
            picked_rx,
            file: None,
            text: String::new(),
            // Defaults keep the frame rate at half a 30 fps camera's: a code that changes mid-exposure tears.
            frame_bytes,
            codes,
            fps,
            sender: None,
            preparing: None,
            send_error: String::new(),
            fullscreen: false,
            frame_n: 0,
            next_frame_at: 0.0,
            qr: None,
            camera: None,
            camera_index,
            receiver: Receiver::default(),
            preview: None,
            started_at: None,
            codes_read: 0,
            cam: CamStats::default(),
            received: None,
            error: String::new(),
            keyboard: false,
            dark: None,
        }
    }

    /// Start streaming `file` right away, e.g. one passed on the command line.
    #[must_use]
    pub fn send(mut self, file: File) -> App {
        self.start_sending(file);
        self
    }

    fn start_sending(&mut self, file: File) {
        self.sender = None;
        self.send_error.clear();
        if file.data.len() as u64 > foqdt_core::MAX_FILE {
            self.send_error = format!(
                "{} is {}; the limit is {}.",
                file.name,
                human(file.data.len()),
                human(foqdt_core::MAX_FILE as usize)
            );
            self.file = None;
            return;
        }
        let (tx, rx) = mpsc::channel();
        let frame_bytes = self.frame_bytes;
        let build = {
            let file = file.clone();
            move || {
                let _ = tx.send(Sender::new(&file, frame_bytes));
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(build);
        #[cfg(target_arch = "wasm32")]
        build(); // no threads on the web
        self.preparing = Some(rx);
        self.file = Some(file);
        self.frame_n = 0;
        self.next_frame_at = 0.0;
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        ui.horizontal(|ui| {
            theme::logo(ui, 34.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    egui::RichText::new("foqdt")
                        .size(19.0)
                        .strong()
                        .color(p.text),
                );
                if ui.available_width() > 420.0 {
                    ui.label(
                        egui::RichText::new("Fast Offline QR Data Transfer")
                            .small()
                            .color(p.weak),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::theme_toggle(ui);
                // right-to-left, so list the options in reverse
                theme::segmented(
                    ui,
                    &mut self.tab,
                    &[(Tab::Receive, "Receive"), (Tab::Send, "Send")],
                    84.0,
                );
            });
        });
    }

    // ---------------------------------------------------------------- send

    fn send_ui(&mut self, ui: &mut egui::Ui) {
        if let Ok(f) = self.picked_rx.try_recv() {
            self.start_sending(f);
        }
        if let Some(rx) = &self.preparing {
            match rx.try_recv() {
                Ok(s) => {
                    self.sender = Some(s);
                    self.preparing = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.send_error = "Could not prepare the file.".into();
                    self.preparing = None;
                    self.file = None;
                }
                Err(mpsc::TryRecvError::Empty) => ui
                    .ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100)),
            }
        }
        if self.sender.is_some() {
            self.advance_frame(ui);
        }
        if self.fullscreen && self.sender.is_some() {
            let max = ui.available_size();
            ui.vertical_centered(|ui| self.qr_view(ui, max));
            return;
        }
        if ui.available_width() >= WIDE {
            ui.horizontal_top(|ui| {
                let h = ui.available_height();
                ui.allocate_ui_with_layout(
                    egui::vec2(SIDE, h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("send-side")
                            .show(ui, |ui| self.send_side(ui));
                    },
                );
                ui.add_space(12.0);
                let max = ui.available_size();
                ui.vertical_centered(|ui| self.qr_view(ui, max));
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("send-stack")
                .show(ui, |ui| {
                    // Stacked (phone) layout: show the QR only while streaming; the cards explain the idle state.
                    if self.sender.is_some() || self.preparing.is_some() {
                        let side = ui.available_width();
                        // Only as tall as the code needs: a 2- or 3-code row is much wider than tall.
                        let aspect = self
                            .qr
                            .as_ref()
                            .map_or(1.0, |t| t.size_vec2().y / t.size_vec2().x);
                        ui.vertical_centered(|ui| {
                            self.qr_view(ui, egui::vec2(side, side * aspect + 80.0))
                        });
                        ui.add_space(12.0);
                    }
                    self.send_side(ui);
                });
        }
    }

    fn advance_frame(&mut self, ui: &egui::Ui) {
        let Some(sender) = &self.sender else { return };
        let now = ui.input(|i| i.time);
        if now >= self.next_frame_at {
            let codes = self.codes as u64;
            let qrs: Vec<_> = (0..codes)
                .map(|i| foqdt_core::qr_encode(&sender.frame(self.frame_n * codes + i)))
                .collect();
            let img = to_image(&foqdt_core::qr_tile(&qrs));
            match &mut self.qr {
                Some(t) => t.set(img, egui::TextureOptions::NEAREST),
                None => {
                    self.qr = Some(
                        ui.ctx()
                            .load_texture("qr", img, egui::TextureOptions::NEAREST),
                    )
                }
            }
            self.frame_n += 1;
            self.next_frame_at = now + 1.0 / self.fps as f64;
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(
                (self.next_frame_at - now).max(0.0),
            ));
    }

    fn send_side(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        theme::card(ui, |ui| {
            theme::section(ui, "Source");
            match &self.file {
                Some(f) => {
                    let mut stop = false;
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                        theme::paint_icon(ui.painter(), Icon::File, rect, p.weak, p.card);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(
                                egui::Label::new(egui::RichText::new(&f.name).strong()).truncate(),
                            );
                            ui.label(
                                egui::RichText::new(human(f.data.len()))
                                    .small()
                                    .color(p.weak),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            stop = ui.button("Stop").clicked();
                        });
                    });
                    if stop {
                        self.sender = None;
                        self.preparing = None;
                        self.file = None;
                    }
                }
                None => {
                    if theme::drop_zone(
                        ui,
                        "Choose a file",
                        "or drop it here · any type, Brotli-compressed",
                    )
                    .clicked()
                    {
                        self.platform.pick_file(self.picked_tx.clone());
                    }
                }
            }
            if !self.send_error.is_empty() {
                ui.label(
                    egui::RichText::new(&self.send_error)
                        .small()
                        .color(p.danger),
                );
            }
            ui.add(
                egui::TextEdit::multiline(&mut self.text)
                    .hint_text("…or paste text")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if ui
                .add_enabled(!self.text.is_empty(), egui::Button::new("Send text"))
                .clicked()
            {
                let data = self.text.clone().into_bytes();
                self.start_sending(File {
                    name: "text.txt".into(),
                    data,
                });
            }
        });

        theme::card(ui, |ui| {
            theme::section(ui, "Stream");
            ui.horizontal(|ui| {
                ui.label("Codes per frame");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // right-to-left, so list the options in reverse
                    theme::segmented(
                        ui,
                        &mut self.codes,
                        &[(4, "4"), (3, "3"), (2, "2"), (1, "1")],
                        30.0,
                    );
                });
            });
            let bytes = theme::slider_row(
                ui,
                "Bytes per code",
                &mut self.frame_bytes,
                100..=2900,
                |v| format!("{v} B"),
            );
            theme::slider_row(ui, "Frame rate", &mut self.fps, 1.0..=60.0, |v| {
                format!("{v:.0} fps")
            });
            if (bytes.drag_stopped() || (bytes.changed() && !bytes.dragged()))
                && let Some(f) = self.file.take()
            {
                self.start_sending(f);
            }
            ui.label(
                egui::RichText::new("More and bigger codes are faster, but need a sharper camera. Keep the frame rate at or below half the camera's (15 fps for a 30 fps camera).")
                    .small()
                    .color(p.weak),
            );
        });

        if let (Some(sender), Some(file)) = (&self.sender, &self.file) {
            let per_sec = self.codes as f32 * self.fps;
            let saved = 100.0 * (1.0 - sender.packed_len as f32 / file.data.len().max(1) as f32);
            theme::card(ui, |ui| {
                theme::section(ui, "Transfer");
                theme::stat_grid("send-stats", ui).show(ui, |ui| {
                    theme::stat(ui, "Original", human(file.data.len()));
                    theme::stat(
                        ui,
                        "Packed",
                        if saved > 0.5 {
                            format!("{} (−{saved:.0}%)", human(sender.packed_len))
                        } else {
                            human(sender.packed_len)
                        },
                    );
                    ui.end_row();
                    theme::stat(
                        ui,
                        "Throughput",
                        format!(
                            "{}/s",
                            human(
                                (per_sec * (self.frame_bytes - foqdt_core::HEADER - 4) as f32)
                                    as usize
                            )
                        ),
                    );
                    theme::stat(
                        ui,
                        "One pass",
                        format!("{:.1} s", sender.total_symbols as f32 / per_sec),
                    );
                    ui.end_row();
                    theme::stat(ui, "Symbols", sender.total_symbols.to_string());
                    theme::stat(ui, "Frame", self.frame_n.to_string());
                    ui.end_row();
                });
            });
        }
    }

    fn qr_view(&mut self, ui: &mut egui::Ui, max: egui::Vec2) {
        if self.preparing.is_some() {
            theme::empty_state(
                ui,
                max,
                None,
                "Compressing…",
                "Brotli is packing the file; big files take a few seconds.",
            );
            return;
        }
        let (Some(t), Some(_)) = (&self.qr, &self.sender) else {
            theme::empty_state(
                ui,
                max,
                Some(Icon::Qr),
                "Nothing to send yet",
                "Choose a file or paste text to start streaming.",
            );
            return;
        };
        let p = Palette::of(ui);
        let size = t.size_vec2();
        let pad = 18.0;
        let ppp = ui.ctx().pixels_per_point();
        let fit = ((max.x - 2.0 * pad) / size.x)
            .min((max.y - 2.0 * pad - 44.0) / size.y)
            .max(0.1)
            * ppp;
        // A whole number of screen pixels per module: fractional scaling makes modules uneven
        // (some one pixel wider than others), which costs the camera a lot of reads.
        let module_px = if fit >= 1.0 { fit.floor() } else { fit };
        let shown = size * module_px / ppp;
        ui.add_space(((max.y - 44.0 - shown.y - 2.0 * pad) / 2.0).max(0.0));
        let mut toggle = false;
        ui.horizontal(|ui| {
            ui.add_space(((ui.available_width() - shown.x - 2.0 * pad) / 2.0).max(0.0));
            theme::badge(
                ui,
                p.success,
                "LIVE",
                &format!(
                    "{} code{} · {:.0} fps · {module_px:.0} px/module",
                    self.codes,
                    if self.codes > 1 { "s" } else { "" },
                    self.fps
                ),
            );
            let label = if self.fullscreen {
                "Exit full screen"
            } else {
                "Full screen"
            };
            toggle = ui
                .small_button(label)
                .on_hover_text("F11 · Esc to leave")
                .clicked();
        });
        toggle |= ui.input(|i| {
            i.key_pressed(egui::Key::F11) || (self.fullscreen && i.key_pressed(egui::Key::Escape))
        });
        if toggle {
            self.fullscreen = !self.fullscreen;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
        }
        let frame = egui::Frame::new()
            .fill(egui::Color32::WHITE)
            .stroke(egui::Stroke::new(1.0, p.border))
            .corner_radius(18)
            .inner_margin(pad);
        frame.show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(shown, egui::Sense::hover());
            // Snap to the pixel grid too, or every module boundary lands mid-pixel.
            let min = (rect.min.to_vec2() * ppp).round() / ppp;
            let rect = egui::Rect::from_min_size(min.to_pos2(), shown);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(t.id(), rect, uv, egui::Color32::WHITE);
        });
    }

    // ------------------------------------------------------------- receive

    fn receive_ui(&mut self, ui: &mut egui::Ui) {
        self.poll_camera(ui);
        if ui.available_width() >= WIDE {
            ui.horizontal_top(|ui| {
                let h = ui.available_height();
                ui.allocate_ui_with_layout(
                    egui::vec2(SIDE, h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("recv-side")
                            .show(ui, |ui| self.receive_side(ui));
                    },
                );
                ui.add_space(12.0);
                let max = ui.available_size();
                ui.vertical_centered(|ui| self.camera_view(ui, max));
            });
        } else {
            egui::ScrollArea::vertical()
                .id_salt("recv-stack")
                .show(ui, |ui| {
                    if self.camera.is_some() {
                        let w = ui.available_width();
                        ui.vertical_centered(|ui| {
                            self.camera_view(ui, egui::vec2(w, w * 0.75 + 40.0))
                        });
                        ui.add_space(12.0);
                    }
                    self.receive_side(ui);
                });
        }
    }

    fn poll_camera(&mut self, ui: &egui::Ui) {
        let Some(cam) = &mut self.camera else { return };
        let mut preview = None;
        'shots: for shot in cam.poll() {
            let shot = match shot {
                Ok(s) => s,
                Err(e) => {
                    self.error = format!("Camera: {e}");
                    self.camera = None;
                    break;
                }
            };
            preview = shot.preview.or(preview);
            if let Some(m) = shot.mode {
                self.cam.mode = m;
            }
            if let Some(s) = shot.size {
                self.cam.size = s;
                self.cam.captured += 1;
            }
            self.cam.scanned += shot.scanned as u32;
            for p in shot.payloads {
                self.codes_read += 1;
                match self.receiver.push(&p) {
                    Ok(Some(f)) => {
                        self.received = Some(f);
                        self.camera = None;
                        break 'shots;
                    }
                    Ok(None) => {
                        self.started_at.get_or_insert(ui.input(|i| i.time));
                    }
                    Err(e) => self.error = e,
                }
            }
        }
        self.cam.tick(ui.input(|i| i.time), self.codes_read);
        if let Some(img) = preview {
            match &mut self.preview {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => {
                    self.preview = Some(ui.ctx().load_texture(
                        "cam",
                        img,
                        egui::TextureOptions::LINEAR,
                    ))
                }
            }
        }
        if self.camera.as_ref().is_some_and(|c| c.needs_polling()) {
            ui.ctx().request_repaint();
        }
    }

    /// Drop the partial (or finished) transfer; the camera, if on, keeps scanning.
    fn reset_receive(&mut self) {
        self.receiver = Receiver::default();
        self.received = None;
        self.started_at = None;
        self.codes_read = 0;
        self.error.clear();
    }

    fn receive_side(&mut self, ui: &mut egui::Ui) {
        let p = Palette::of(ui);
        theme::card(ui, |ui| {
            theme::section(ui, "Camera");
            if self.camera.is_none() {
                if self.platform.has_camera_index() {
                    ui.horizontal(|ui| {
                        ui.label("Device");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(egui::DragValue::new(&mut self.camera_index).prefix("#"));
                        });
                    });
                }
                if theme::primary(ui, "Start camera").clicked() {
                    self.camera = Some(
                        self.platform
                            .start_camera(ui.ctx().clone(), self.camera_index),
                    );
                    self.cam = CamStats::default();
                    if self.received.is_some() {
                        self.reset_receive();
                    }
                }
            } else if ui
                .add(
                    egui::Button::new("Stop camera")
                        .min_size(egui::vec2(ui.available_width(), 40.0)),
                )
                .clicked()
            {
                self.camera = None;
            }
            if self.camera.is_some() {
                let c = &self.cam;
                let dash = |v: f32, unit: &str| {
                    if c.window_start.is_none() || c.size[0] == 0 {
                        "—".to_string()
                    } else {
                        format!("{v:.1} {unit}")
                    }
                };
                theme::stat_grid("cam-stats", ui).show(ui, |ui| {
                    theme::stat(
                        ui,
                        "Resolution",
                        if c.size[0] == 0 {
                            "—".into()
                        } else {
                            format!("{} × {}", c.size[0], c.size[1])
                        },
                    );
                    theme::stat(
                        ui,
                        "Mode",
                        if c.mode.is_empty() {
                            "—".into()
                        } else {
                            c.mode.clone()
                        },
                    );
                    ui.end_row();
                    theme::stat(ui, "Captured", dash(c.capture_fps, "fps"));
                    theme::stat(ui, "Analysed", dash(c.scan_fps, "fps"));
                    ui.end_row();
                    theme::stat(ui, "Codes read", dash(c.code_rate, "/s"));
                    theme::stat(
                        ui,
                        "Megapixels",
                        if c.size[0] == 0 {
                            "—".into()
                        } else {
                            format!("{:.2} MP", (c.size[0] * c.size[1]) as f32 / 1e6)
                        },
                    );
                    ui.end_row();
                });
            }
            let started = self.camera.is_some()
                || self.receiver.needed > 0
                || self.received.is_some()
                || !self.error.is_empty();
            if started
                && ui
                    .add(
                        egui::Button::new("Reset transfer")
                            .min_size(egui::vec2(ui.available_width(), 40.0)),
                    )
                    .on_hover_text("Forget everything received so far and wait for a new transfer")
                    .clicked()
            {
                self.reset_receive();
            }
            if self.camera.is_none() && self.received.is_none() && self.receiver.needed == 0 {
                ui.label(
                    egui::RichText::new(
                        "Point the camera at a foqdt sender. Frames can arrive in any order.",
                    )
                    .small()
                    .color(p.weak),
                );
            }
            if !self.error.is_empty() {
                ui.label(egui::RichText::new(&self.error).small().color(p.danger));
            }
        });

        if self.receiver.needed > 0 && self.received.is_none() {
            let (got, need) = (self.receiver.received(), self.receiver.needed);
            let frac = (got as f32 / need as f32).min(1.0);
            let secs = self
                .started_at
                .map_or(0.0, |t| ui.input(|i| i.time) - t)
                .max(1.0);
            let rate = got as f64 / secs;
            theme::card(ui, |ui| {
                theme::section(ui, "Progress");
                ui.label(
                    egui::RichText::new(format!("{:.0}%", frac * 100.0))
                        .size(34.0)
                        .strong()
                        .color(p.text),
                );
                ui.add(
                    egui::ProgressBar::new(frac)
                        .fill(p.accent)
                        .corner_radius(5)
                        .desired_height(10.0),
                );
                theme::stat_grid("recv-stats", ui).show(ui, |ui| {
                    theme::stat(ui, "Incoming", human(self.receiver.packed_len as usize));
                    theme::stat(ui, "Symbols", format!("{got} / {need}"));
                    ui.end_row();
                    theme::stat(
                        ui,
                        "Speed",
                        format!("{}/s", human((rate * self.receiver.symbol as f64) as usize)),
                    );
                    ui.end_row();
                    let eta = if rate > 0.0 {
                        format!("{:.0} s", (need.saturating_sub(got) as f64 / rate).ceil())
                    } else {
                        "—".into()
                    };
                    theme::stat(ui, "Remaining", eta);
                    theme::stat(ui, "Elapsed", format!("{secs:.0} s"));
                    ui.end_row();
                });
            });
        }

        let (mut save, mut open) = (false, false);
        if let Some(f) = &self.received {
            theme::card(ui, |ui| {
                theme::badge(ui, p.success, "COMPLETE", "SHA-256 verified");
                let name = ui.add(
                    egui::Label::new(egui::RichText::new(&f.name).size(17.0).strong().underline())
                        .truncate()
                        .sense(egui::Sense::click()),
                );
                open = name
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Open")
                    .clicked();
                ui.label(
                    egui::RichText::new(human(f.data.len()))
                        .small()
                        .color(p.weak),
                );
                open |= theme::primary(ui, "Open").clicked();
                save = ui
                    .add(
                        egui::Button::new("Save as…")
                            .min_size(egui::vec2(ui.available_width(), 40.0)),
                    )
                    .clicked();
                if f.data.len() < 64 * 1024
                    && let Ok(text) = std::str::from_utf8(&f.data)
                {
                    ui.add(
                        egui::TextEdit::multiline(&mut &*text)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                }
            });
        }
        if let Some(f) = &self.received {
            let r = if open {
                self.platform.open_file(f)
            } else if save {
                self.platform.save_file(f)
            } else {
                Ok(())
            };
            if let Err(e) = r {
                self.error = e;
            }
        }
    }

    fn camera_view(&mut self, ui: &mut egui::Ui, max: egui::Vec2) {
        let p = Palette::of(ui);
        match (&self.preview, &self.camera, &self.received) {
            (Some(t), Some(_), _) => {
                let secs = self
                    .started_at
                    .map_or(0.0, |t| ui.input(|i| i.time) - t)
                    .max(1.0);
                theme::badge(
                    ui,
                    p.accent,
                    "SCANNING",
                    &format!(
                        "{} codes read · {:.0}/s",
                        self.codes_read,
                        self.codes_read as f64 / secs
                    ),
                );
                let size = t.size_vec2();
                let scale = (max.x / size.x).min((max.y - 40.0) / size.y).max(0.1);
                ui.add(
                    egui::Image::new(t)
                        .fit_to_exact_size(size * scale)
                        .corner_radius(18),
                );
            }
            (_, Some(_), _) => theme::empty_state(
                ui,
                max,
                None,
                "Starting camera…",
                "Allow camera access if your device asks.",
            ),
            (_, None, Some(f)) => theme::empty_state(
                ui,
                max,
                Some(Icon::Check),
                "File received",
                &format!("Tap {} to open it.", f.name),
            ),
            _ => theme::empty_state(
                ui,
                max,
                Some(Icon::Camera),
                "Camera is off",
                "Start the camera and point it at a foqdt sender.",
            ),
        }
    }
}

const SETTINGS: &str = "foqdt-settings";

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(
            SETTINGS,
            format!(
                "{},{},{},{}",
                self.frame_bytes, self.codes, self.fps, self.camera_index
            ),
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for f in ui.input(|i| i.raw.dropped_files.clone()) {
            self.platform.read_dropped(f, self.picked_tx.clone());
            self.tab = Tab::Send;
        }
        let wants_keyboard = ui.ctx().egui_wants_keyboard_input();
        if wants_keyboard != self.keyboard {
            self.keyboard = wants_keyboard;
            self.platform.show_keyboard(wants_keyboard);
        }
        let dark = ui.visuals().dark_mode;
        if self.dark != Some(dark) {
            self.dark = Some(dark);
            self.platform.theme_changed(dark);
        }
        let p = Palette::of(ui);
        let mut margin = if ui.available_width() < WIDE {
            egui::Margin::symmetric(12, 10)
        } else {
            egui::Margin::symmetric(20, 16)
        };
        let ppp = ui.ctx().pixels_per_point();
        let [l, t, r, b] = self
            .platform
            .safe_area()
            .map(|px| (px / ppp).round().clamp(0.0, 100.0) as i8);
        margin.left = margin.left.saturating_add(l);
        margin.top = margin.top.saturating_add(t);
        margin.right = margin.right.saturating_add(r);
        margin.bottom = margin.bottom.saturating_add(b);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(margin))
            .show(ui, |ui| {
                if !(self.fullscreen && self.tab == Tab::Send && self.sender.is_some()) {
                    self.header(ui);
                    ui.add_space(14.0);
                }
                match self.tab {
                    Tab::Send => self.send_ui(ui),
                    Tab::Receive => self.receive_ui(ui),
                }
            });
        if ui.input(|i| !i.raw.hovered_files.is_empty()) {
            theme::drop_overlay(ui);
        }
    }
}

/// Measured camera rates, recomputed once per second.
#[derive(Default)]
struct CamStats {
    mode: String,
    size: [usize; 2],
    captured: u32,
    scanned: u32,
    window_start: Option<Window>,
    capture_fps: f32,
    scan_fps: f32,
    code_rate: f32,
}

impl CamStats {
    fn tick(&mut self, now: f64, codes: usize) {
        let now_window = Window {
            time: now,
            captured: self.captured,
            scanned: self.scanned,
            codes,
        };
        let Some(start) = self.window_start else {
            self.window_start = Some(now_window);
            return;
        };
        let dt = now - start.time;
        if dt >= 1.0 {
            self.capture_fps = ((self.captured - start.captured) as f64 / dt) as f32;
            self.scan_fps = ((self.scanned - start.scanned) as f64 / dt) as f32;
            self.code_rate = (codes.saturating_sub(start.codes) as f64 / dt) as f32;
            self.window_start = Some(now_window);
        }
    }
}

/// Counters at the start of a [`CamStats`] measuring window.
#[derive(Clone, Copy)]
struct Window {
    time: f64,
    captured: u32,
    scanned: u32,
    codes: usize,
}

fn to_image(b: &foqdt_core::Bitmap) -> egui::ColorImage {
    let pixels = b
        .dark
        .iter()
        .map(|&d| {
            if d {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            }
        })
        .collect();
    egui::ColorImage::new([b.w, b.h], pixels)
}

fn human(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}
