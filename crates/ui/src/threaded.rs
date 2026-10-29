//! A [`Camera`] that captures on one thread and decodes QR codes on a small worker pool.

use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use crate::{Camera, Shot};

/// One captured frame, as the grabber passed to [`threaded_camera`] returns it.
pub struct Grab {
    pub w: usize,
    pub h: usize,
    /// Row-major greyscale, `w * h` bytes; this is what gets scanned.
    pub luma: Vec<u8>,
    /// What the UI shows; may be smaller than `w` x `h`.
    pub preview: egui::ColorImage,
}

struct ThreadCamera {
    rx: mpsc::Receiver<Result<Shot, String>>,
    stop: Arc<AtomicBool>,
}

impl Camera for ThreadCamera {
    fn poll(&mut self) -> Vec<Result<Shot, String>> {
        self.rx.try_iter().collect()
    }
}

impl Drop for ThreadCamera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Run `open` and the returned grabber on a capture thread, and decode QR codes on worker
/// threads. When every worker is busy the frame is only previewed, so latency stays low.
/// `open` also returns a description of the negotiated camera mode.
pub fn threaded_camera<F, G>(ctx: egui::Context, open: F) -> Box<dyn Camera>
where
    F: FnOnce() -> Result<(String, G), String> + Send + 'static,
    G: FnMut() -> Result<Grab, String>,
{
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 4));
    let (job_tx, job_rx): (mpsc::SyncSender<Grab>, mpsc::Receiver<Grab>) =
        mpsc::sync_channel(workers);
    let job_rx = Arc::new(Mutex::new(job_rx));
    for _ in 0..workers {
        let (job_rx, tx, ctx) = (job_rx.clone(), tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            loop {
                let Ok(g) = job_rx.lock().expect("job queue mutex poisoned").recv() else {
                    break;
                };
                let payloads = foqdt_core::qr_scan_luma(g.w, g.h, &g.luma);
                if tx
                    .send(Ok(Shot {
                        scanned: true,
                        payloads,
                        ..Default::default()
                    }))
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
    }
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let (mode, mut grab) = open()?;
            let _ = tx.send(Ok(Shot {
                mode: Some(mode),
                ..Default::default()
            }));
            while !stopped.load(Ordering::Relaxed) {
                let mut g = grab()?;
                let preview = std::mem::take(&mut g.preview);
                let size = Some([g.w, g.h]);
                let _ = job_tx.try_send(g);
                if tx
                    .send(Ok(Shot {
                        preview: Some(preview),
                        size,
                        ..Default::default()
                    }))
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
            Ok(())
        };
        if let Err(e) = run() {
            let _ = tx.send(Err(e));
            ctx.request_repaint();
        }
    });
    Box::new(ThreadCamera { rx, stop })
}
