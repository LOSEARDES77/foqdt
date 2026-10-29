# foqdt: Fast Offline QR Data Transfer

foqdt sends a file from one device to another as an endless loop of QR codes. The receiving device reads the loop with its camera. You need no network, no pairing and no back-channel. The project is inspired by [Decimen](https://github.com/aryanjsx/Decimen).

The app is written in Rust with egui/eframe. One UI crate runs on the desktop, on the web (WebAssembly) and on Android.

## Crates

| Crate | What it holds |
|---|---|
| `crates/core` (`foqdt-core`) | Wire protocol, Brotli, RaptorQ fountain code, QR encode, tile and scan. No UI. |
| `crates/ui` (`foqdt-ui`) | The egui app, the `Platform` and `Camera` traits, and a threaded capture and decode pipeline. |
| `crates/desktop` (`foqdt-desktop`) | Native binary `foqdt`: camera through `nokhwa`, dialogs through `rfd`. |
| `crates/web` (`foqdt-web`) | WebAssembly build through trunk: `getUserMedia` camera, browser download. |
| `crates/android` (`foqdt-android`) | NativeActivity `cdylib`, plus a small Java activity for the camera, permission and document pickers. |

## Interface

The interface follows the system light or dark theme. The sun/moon button in the header switches it by hand.

- **Send:** drop a file anywhere on the window, click the drop zone, or paste text.
- **Receive:** shows live progress, speed and time remaining. When the file is complete, it shows a save button and a preview of text files.
- **Narrow screens:** below 760 px wide (phones), the two panes stack vertically.

Settings (codes per frame, bytes per code, frame rate, camera number and theme) are kept between runs.

On the Send screen, **Full screen** (or F11; Esc leaves) shows only the codes. Each module is drawn as a whole number of screen pixels: with fractional scaling, modules come out uneven, and the camera reads far fewer codes.

On the Receive screen, the camera card shows the exact capture resolution, the mode the driver agreed to, and the measured capture, analysis and code rates. When a file arrives, tap its name or **Open** to open it in the default app. A received APK opens the system installer. **Reset transfer** drops a partial transfer.

## How it works

- The file is packed with its name and a SHA-256 hash. The packed data is Brotli-compressed (quality 11, 16 MB window) only when that makes it smaller.
- The packed data is encoded with a RaptorQ fountain code (RFC 6330). The sender streams source symbols first, then new repair symbols forever.
- The receiver needs about K + 2 distinct packets, in any order. A missed frame costs time, not correctness.
- Each QR code describes itself: `"FQ" | version | session id | OTI | RaptorQ packet`. The QR codes use byte mode and ECC level L.
- The receiver checks the SHA-256 hash before it offers the file.
- The receiver rejects malformed frames instead of passing them to RaptorQ, which panics on some inconsistent values. Files are limited to 1 GiB, packed or unpacked, so a hostile stream cannot exhaust memory.
- Compression runs on a worker thread, so large files don't freeze the window.

There is no encryption. Any camera that can see the screen can read the data.

## Speed

Throughput is roughly: codes per frame × fps × bytes per code.

- **codes/frame (1–4):** shows several QR codes at once. Up to 3 codes are placed in a row, and 4 codes in a 2×2 grid. Use 4 codes only when the receiving camera has enough resolution (1080p).
- **bytes/code:** bigger codes carry more data, but they are harder to scan. Try 400–1200 bytes for webcams.
- **fps:** set this to match what the receiver can decode. On the desktop and on Android, the receiver decodes on up to 4 worker threads. On the web, decoding runs on the main thread.

The desktop and Android apps decode with zxing-cpp. On blurred, noisy camera frames it reads codes in 1–5 ms, where rqrr read none of them. zxing-cpp is C++ and does not compile to `wasm32-unknown-unknown`, so the web app decodes with rxing, a pure-Rust port of zxing. It reads the same frames, at about 25 ms for four codes in a 1080p frame. In the browser, decoding runs on the main thread.

On Android the camera uses Camera2. It picks the largest YUV size up to 4K that still streams at 30 fps, the highest frame rate the camera offers, and -1 EV of exposure. If Camera2 is not available (LEGACY hardware), it falls back to the old camera API, which is limited to about 1080p.

Data that is already compressed (JPEG, MP4, ZIP) does not get smaller. Compress a folder into an archive before you send it.

## Run

```sh
cargo run --release -p foqdt-desktop          # desktop
cargo run --release -p foqdt-desktop -- FILE  # desktop, start streaming FILE right away
cd crates/web && trunk serve --release        # web: camera needs HTTPS or localhost
crates/android/build-apk.sh                   # → target/apk/foqdt.apk
adb install target/apk/foqdt.apk
```

Building the desktop or Android app needs `cmake` and a C++ compiler, for zxing-cpp.

The APK build does not use Gradle. It uses the SDK tools directly: NDK clang, javac, d8, aapt2, zipalign and apksigner. It finds the SDK through `ANDROID_HOME`, or through `~/Android/Sdk` when that is not set. It signs the APK with `~/.android/debug.keystore`. If that file does not exist, the script creates it. To sign with your own keystore, set `KEYSTORE` and `KEYSTORE_PASS`. The APK contains only the arm64-v8a library (phones), and runs on Android 8.0 (API 26) or later. To add the x86_64 library for the emulator, run `ABIS="arm64-v8a x86_64" crates/android/build-apk.sh`.

## Tests

```sh
cargo test --release --workspace
```

The tests cover protocol round trips with dropped frames, the Brotli compression ratio, and a full encode, tile, scan and decode loop for 1–4 codes per frame.

## License

MIT. See [LICENSE](LICENSE).
