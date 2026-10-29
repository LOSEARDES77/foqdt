//! Android frontend. Rendering runs through eframe's NativeActivity backend.
//! Camera, permission and document pickers live in `java/dev/foqdt/FoqdtActivity.java`.
#![cfg(target_os = "android")]

use foqdt_ui::eframe::{self, egui};
use foqdt_ui::{Camera, File, Grab, Platform, threaded_camera};
use jni::objects::{Global, JByteArray, JObject, JValue};
use jni::{Env, JavaVM, jni_sig, jni_str};
use std::sync::{Arc, mpsc};
use winit::platform::android::activity::AndroidApp;

#[derive(Clone)]
struct Android {
    app: AndroidApp,
    vm: Arc<JavaVM>,
    activity: Arc<Global<JObject<'static>>>,
}

/// Call a `byte[]`-returning method on the activity.
macro_rules! call {
    ($self:expr, $name:literal, $sig:literal $(, $arg:ident)?) => {
        $self.call(|env, activity| {
            $(let $arg = env.byte_array_from_slice(&$arg)?;)?
            env.call_method(activity, jni_str!($name), jni_sig!($sig), &[$(JValue::Object(&$arg)),*])?.l()
        })
    };
}

impl Android {
    fn call<F>(&self, f: F) -> Result<Option<Vec<u8>>, String>
    where
        F: for<'l> FnOnce(&mut Env<'l>, &JObject<'static>) -> jni::errors::Result<JObject<'l>>,
    {
        self.vm
            .attach_current_thread(|env| -> jni::errors::Result<_> {
                let obj = f(env, self.activity.as_obj())?;
                if obj.is_null() {
                    return Ok(None);
                }
                let arr = env.cast_local::<JByteArray>(obj)?;
                Ok(Some(env.convert_byte_array(&arr)?))
            })
            .map_err(|e| e.to_string())
    }
}

fn decode_file(b: &[u8]) -> Option<File> {
    let n = u16::from_be_bytes(b.get(..2)?.try_into().ok()?) as usize;
    let name = String::from_utf8_lossy(b.get(2..2 + n)?).into_owned();
    Some(File {
        name,
        data: b[2 + n..].to_vec(),
    })
}

/// Calls `stopCamera` when the capture thread drops its grabber.
struct StopCamera(Android);

impl Drop for StopCamera {
    fn drop(&mut self) {
        let _ = call!(self.0, "stopCamera", "()[B");
    }
}

impl Platform for Android {
    fn start_camera(&self, ctx: egui::Context, _index: u32) -> Box<dyn Camera> {
        let this = self.clone();
        threaded_camera(ctx, move || {
            if let Some(err) = call!(this, "startCamera", "()[B")? {
                return Err(String::from_utf8_lossy(&err).into_owned());
            }
            let mode = call!(this, "cameraMode", "()[B")?
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            let guard = StopCamera(this);
            Ok((mode, move || {
                loop {
                    let Some(b) = call!(guard.0, "takeFrame", "()[B")? else {
                        continue;
                    };
                    let w = u32::from_be_bytes(b[0..4].try_into().expect("fixed-length slice"))
                        as usize;
                    let h = u32::from_be_bytes(b[4..8].try_into().expect("fixed-length slice"))
                        as usize;
                    let luma = b[8..].to_vec();
                    // Preview at most ~960 px wide: a 4K texture upload per frame is wasted work on a phone screen.
                    let step = w.div_ceil(960).max(1);
                    let (pw, ph) = (w / step, h / step);
                    let small: Vec<u8> = (0..pw * ph)
                        .map(|i| luma[(i / pw) * step * w + (i % pw) * step])
                        .collect();
                    let preview = egui::ColorImage::from_gray([pw, ph], &small);
                    return Ok(Grab {
                        w,
                        h,
                        luma,
                        preview,
                    });
                }
            }))
        })
    }

    fn pick_file(&self, done: mpsc::Sender<File>) {
        let this = self.clone();
        std::thread::spawn(move || {
            if call!(this, "pickFile", "()[B").is_err() {
                return;
            }
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                match call!(this, "pickResult", "()[B") {
                    Ok(None) => continue,
                    Ok(Some(b)) => {
                        if let Some(f) = decode_file(&b) {
                            let _ = done.send(f);
                        }
                        return;
                    }
                    Err(_) => return,
                }
            }
        });
    }

    fn save_file(&self, file: &File) -> Result<(), String> {
        let name = &file.name.as_bytes()[..file.name.len().min(u16::MAX as usize)];
        let mut encoded = (name.len() as u16).to_be_bytes().to_vec();
        encoded.extend_from_slice(name);
        encoded.extend_from_slice(&file.data);
        call!(self, "saveFile", "([B)[B", encoded).map(drop)
    }

    fn open_file(&self, file: &File) -> Result<(), String> {
        let name = &file.name.as_bytes()[..file.name.len().min(u16::MAX as usize)];
        let mut encoded = (name.len() as u16).to_be_bytes().to_vec();
        encoded.extend_from_slice(name);
        encoded.extend_from_slice(&file.data);
        match call!(self, "openFile", "([B)[B", encoded)? {
            Some(err) => Err(String::from_utf8_lossy(&err).into_owned()),
            None => Ok(()),
        }
    }

    // ponytail: one JNI call per frame; cache it if it ever shows up in a profile.
    fn safe_area(&self) -> [f32; 4] {
        let Ok(Some(b)) = call!(self, "insets", "()[B") else {
            return [0.0; 4];
        };
        std::array::from_fn(|i| {
            i32::from_be_bytes(b[i * 4..i * 4 + 4].try_into().expect("fixed-length slice")) as f32
        })
    }

    fn theme_changed(&self, dark: bool) {
        let arg = vec![dark as u8];
        let _ = call!(self, "systemBars", "([B)[B", arg);
    }

    fn show_keyboard(&self, show: bool) {
        if show {
            self.app.show_soft_input(/* show_implicit= */ true);
        } else {
            self.app.hide_soft_input(/* hide_implicit_only= */ false);
        }
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    // SAFETY: android-activity hands us the process JavaVM pointer, valid for the process lifetime.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let activity = vm
        .attach_current_thread(|env| -> jni::errors::Result<_> {
            // SAFETY: android-activity hands us a global ref to the activity, valid while it runs.
            let obj = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
            env.new_global_ref(obj)
        })
        .expect("activity global ref");
    let platform = Android {
        app: app.clone(),
        vm: Arc::new(vm),
        activity: Arc::new(activity),
    };
    let persistence_path = app.internal_data_path().map(|d| d.join("foqdt.ron"));
    let options = eframe::NativeOptions {
        android_app: Some(app),
        renderer: eframe::Renderer::Glow,
        persistence_path,
        ..Default::default()
    };
    eframe::run_native(
        "foqdt",
        options,
        Box::new(|cc| Ok(Box::new(foqdt_ui::App::new(cc, Box::new(platform))))),
    )
    .expect("eframe failed to start");
}
