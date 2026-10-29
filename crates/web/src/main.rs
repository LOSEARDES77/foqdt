#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("foqdt-web targets wasm32: run `trunk serve --release` in crates/web");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    let canvas = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("app"))
        .and_then(|c| c.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        .expect("no <canvas id=app>");
    wasm_bindgen_futures::spawn_local(async move {
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(foqdt_ui::App::new(cc, Box::new(web::Web))))),
            )
            .await
            .expect("failed to start eframe");
    });
}

#[cfg(target_arch = "wasm32")]
mod web {
    use eframe::egui;
    use foqdt_ui::{Camera, File, Platform, Shot};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlVideoElement, MediaStream};

    pub struct Web;

    fn js_err(e: JsValue) -> String {
        e.as_string().unwrap_or_else(|| format!("{e:?}"))
    }

    struct WebCamera {
        video: HtmlVideoElement,
        canvas: HtmlCanvasElement,
        ctx2d: CanvasRenderingContext2d,
        stream: Rc<RefCell<Option<MediaStream>>>,
        error: Rc<RefCell<Option<String>>>,
        last_time: f64,
        mode_sent: bool,
    }

    impl Camera for WebCamera {
        /// Grab the current video frame and decode it on the main thread.
        fn poll(&mut self) -> Vec<Result<Shot, String>> {
            if let Some(e) = self.error.borrow_mut().take() {
                return vec![Err(e)];
            }
            let (w, h) = (self.video.video_width(), self.video.video_height());
            if self.video.ready_state() < 2 || w == 0 || h == 0 {
                return vec![];
            }
            // The UI repaints faster than the camera delivers: only scan new video frames.
            let t = self.video.current_time();
            if t == self.last_time {
                return vec![];
            }
            self.last_time = t;
            let mut out = vec![];
            if !self.mode_sent {
                self.mode_sent = true;
                let fps = self.stream.borrow().as_ref().and_then(|ms| {
                    let track: web_sys::MediaStreamTrack =
                        ms.get_video_tracks().get(0).dyn_into().ok()?;
                    js_sys::Reflect::get(&track.get_settings(), &"frameRate".into())
                        .ok()?
                        .as_f64()
                });
                out.push(Ok(Shot {
                    mode: Some(fps.map_or("browser".into(), |f| format!("browser · {f:.0} fps"))),
                    ..Default::default()
                }));
            }
            if self.canvas.width() != w || self.canvas.height() != h {
                self.canvas.set_width(w);
                self.canvas.set_height(h);
            }
            let grab = || -> Result<Vec<u8>, JsValue> {
                self.ctx2d
                    .draw_image_with_html_video_element(&self.video, 0.0, 0.0)?;
                Ok(self
                    .ctx2d
                    .get_image_data(0.0, 0.0, w as f64, h as f64)?
                    .data()
                    .0)
            };
            let px = match grab() {
                Ok(px) => px,
                Err(e) => return vec![Err(js_err(e))],
            };
            let (w, h) = (w as usize, h as usize);
            let luma: Vec<u8> = px
                .chunks_exact(4)
                .map(|p| foqdt_core::luma(p[0], p[1], p[2]))
                .collect();
            let payloads = foqdt_core::qr_scan_luma(w, h, &luma);
            // Small grey preview: uploading a 1080p RGBA texture every frame costs as much as decoding.
            let step = w.div_ceil(640).max(1);
            let (pw, ph) = (w / step, h / step);
            let small: Vec<u8> = (0..pw * ph)
                .map(|i| luma[(i / pw) * step * w + (i % pw) * step])
                .collect();
            let preview = egui::ColorImage::from_gray([pw, ph], &small);
            out.push(Ok(Shot {
                preview: Some(preview),
                size: Some([w, h]),
                scanned: true,
                payloads,
                mode: None,
            }));
            out
        }

        fn needs_polling(&self) -> bool {
            true
        }
    }

    impl Drop for WebCamera {
        fn drop(&mut self) {
            if let Some(ms) = self.stream.borrow_mut().take() {
                for t in ms.get_tracks().iter() {
                    t.unchecked_into::<web_sys::MediaStreamTrack>().stop();
                }
            }
        }
    }

    impl Platform for Web {
        fn start_camera(&self, ctx: egui::Context, _index: u32) -> Box<dyn Camera> {
            let doc = web_sys::window()
                .expect("no window")
                .document()
                .expect("no document");
            let video: HtmlVideoElement = doc
                .create_element("video")
                .expect("create <video>")
                .dyn_into()
                .expect("<video> element");
            video.set_muted(true);
            let _ = video.set_attribute("playsinline", "");
            let canvas: HtmlCanvasElement = doc
                .create_element("canvas")
                .expect("create <canvas>")
                .dyn_into()
                .expect("<canvas> element");
            // willReadFrequently keeps the canvas on the CPU, so getImageData doesn't read back from the GPU.
            let opts = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&opts, &"willReadFrequently".into(), &true.into());
            let ctx2d: CanvasRenderingContext2d = canvas
                .get_context_with_context_options("2d", &opts)
                .expect("get 2d context")
                .expect("2d context is supported")
                .dyn_into()
                .expect("2d context type");
            let stream = Rc::new(RefCell::new(None));
            let error = Rc::new(RefCell::new(None));

            let (v, s, e) = (video.clone(), stream.clone(), error.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let run = async {
                    let devices = web_sys::window()
                        .expect("no window")
                        .navigator()
                        .media_devices()?;
                    let constraints = web_sys::MediaStreamConstraints::new();
                    let video_opts = js_sys::Object::new();
                    js_sys::Reflect::set(&video_opts, &"facingMode".into(), &"environment".into())?;
                    // 1080p: four dense codes per frame need the pixels.
                    js_sys::Reflect::set(&video_opts, &"width".into(), &1920.into())?;
                    js_sys::Reflect::set(&video_opts, &"height".into(), &1080.into())?;
                    constraints.set_video(&video_opts);
                    let promise = devices.get_user_media_with_constraints(&constraints)?;
                    let ms: MediaStream = wasm_bindgen_futures::JsFuture::from(promise)
                        .await?
                        .dyn_into()?;
                    v.set_src_object(Some(&ms));
                    *s.borrow_mut() = Some(ms);
                    wasm_bindgen_futures::JsFuture::from(v.play()?).await?;
                    Ok::<_, JsValue>(())
                };
                if let Err(err) = run.await {
                    *e.borrow_mut() = Some(js_err(err));
                }
                ctx.request_repaint();
            });
            Box::new(WebCamera {
                video,
                canvas,
                ctx2d,
                stream,
                error,
                last_time: -1.0,
                mode_sent: false,
            })
        }

        fn pick_file(&self, done: mpsc::Sender<File>) {
            wasm_bindgen_futures::spawn_local(async move {
                if let Some(h) = rfd::AsyncFileDialog::new().pick_file().await {
                    let _ = done.send(File {
                        name: h.file_name(),
                        data: h.read().await,
                    });
                }
            });
        }

        fn read_dropped(&self, file: egui::DroppedFileHandle, done: mpsc::Sender<File>) {
            wasm_bindgen_futures::spawn_local(async move {
                if let Ok(data) = file.bytes_async().await {
                    let name = file
                        .path()
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let _ = done.send(File { name, data });
                }
            });
        }

        /// Show the file in a new tab; the browser renders what it can and downloads the rest.
        fn open_file(&self, file: &File) -> Result<(), String> {
            let run = || -> Result<(), JsValue> {
                let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(file.data.as_slice()));
                let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)?;
                let url = web_sys::Url::create_object_url_with_blob(&blob)?;
                web_sys::window()
                    .expect("no window")
                    .open_with_url_and_target(&url, "_blank")?;
                Ok(())
            };
            run().map_err(js_err)
        }

        /// Trigger a browser download through a temporary object URL.
        fn save_file(&self, file: &File) -> Result<(), String> {
            let run = || -> Result<(), JsValue> {
                let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(file.data.as_slice()));
                let blob = web_sys::Blob::new_with_u8_array_sequence(&parts)?;
                let url = web_sys::Url::create_object_url_with_blob(&blob)?;
                let doc = web_sys::window()
                    .expect("no window")
                    .document()
                    .expect("no document");
                let a: web_sys::HtmlAnchorElement = doc.create_element("a")?.dyn_into()?;
                a.set_href(&url);
                a.set_download(&file.name);
                a.click();
                // Revoking right away can cancel the download in some browsers; give it a minute.
                let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
                    let _ = web_sys::Url::revoke_object_url(&url);
                });
                web_sys::window()
                    .expect("no window")
                    .set_timeout_with_callback_and_timeout_and_arguments_0(
                        revoke.unchecked_ref(),
                        60_000,
                    )?;
                Ok(())
            };
            run().map_err(js_err)
        }
    }
}
