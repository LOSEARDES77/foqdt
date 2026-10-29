//! QR encode, tile and scan, on bitmaps and greyscale buffers.

/// A 1-bit bitmap, one pixel per QR module. `true` = dark.
pub struct Bitmap {
    /// Width in modules.
    pub w: usize,
    /// Height in modules.
    pub h: usize,
    /// Row-major, `w * h` long.
    pub dark: Vec<bool>,
}

/// Encode bytes in byte mode (no re-segmentation) at ECC level L, the densest QR layout.
/// The bitmap includes a 4-module quiet zone.
///
/// # Panics
///
/// If `data` does not fit in a version 40 code (2953 bytes).
#[must_use]
pub fn qr_encode(data: &[u8]) -> Bitmap {
    use qrcode::bits::Bits;
    use qrcode::{EcLevel, QrCode, Version};
    let ec = EcLevel::L;
    let code = (1..=40)
        .find_map(|v| {
            let mut bits = Bits::new(Version::Normal(v));
            bits.push_byte_data(data).ok()?;
            bits.push_terminator(ec).ok()?;
            QrCode::with_bits(bits, ec).ok()
        })
        .expect("payload exceeds QR capacity");
    let (w, quiet) = (code.width(), 4);
    let side = w + 2 * quiet;
    let mut dark = vec![false; side * side];
    for (i, c) in code.to_colors().into_iter().enumerate() {
        dark[(i / w + quiet) * side + i % w + quiet] = c == qrcode::Color::Dark;
    }
    Bitmap {
        w: side,
        h: side,
        dark,
    }
}

/// Lay out same-sized QR codes in one image: up to 3 in a row, 4 as a 2x2 grid.
///
/// # Panics
///
/// If `codes` is empty or the codes differ in size.
#[must_use]
pub fn qr_tile(codes: &[Bitmap]) -> Bitmap {
    let s = codes[0].w;
    let n = codes.len();
    let cols = if n == 4 { 2 } else { n };
    let (w, h) = (s * cols, s * n.div_ceil(cols));
    let mut dark = vec![false; w * h];
    for (i, code) in codes.iter().enumerate() {
        let (ox, oy) = (i % cols * s, i / cols * s);
        for y in 0..s {
            dark[(oy + y) * w + ox..][..s].copy_from_slice(&code.dark[y * s..][..s]);
        }
    }
    Bitmap { w, h, dark }
}

/// Decode every QR code in a greyscale image.
#[must_use]
#[cfg(not(target_arch = "wasm32"))]
pub fn qr_scan(w: usize, h: usize, luma: impl Fn(usize, usize) -> u8) -> Vec<Vec<u8>> {
    let mut img = Vec::with_capacity(w * h);
    for y in 0..h {
        img.extend((0..w).map(|x| luma(x, y)));
    }
    qr_scan_luma(w, h, &img)
}

/// [`qr_scan`] for a row-major luma buffer, without the copy.
#[must_use]
#[cfg(not(target_arch = "wasm32"))]
pub fn qr_scan_luma(w: usize, h: usize, img: &[u8]) -> Vec<Vec<u8>> {
    let Ok(view) = zxingcpp::ImageView::from_slice(img, w, h, zxingcpp::ImageFormat::Lum) else {
        return vec![];
    };
    let reader = zxingcpp::read()
        .formats(zxingcpp::BarcodeFormat::QRCode)
        .try_rotate(false)
        .try_invert(false)
        .max_number_of_symbols(8);
    reader
        .from(&view)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.is_valid())
        .map(|b| b.bytes())
        .collect()
}

/// Decode every QR code in a greyscale image.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn qr_scan(w: usize, h: usize, luma: impl Fn(usize, usize) -> u8) -> Vec<Vec<u8>> {
    let mut img = Vec::with_capacity(w * h);
    for y in 0..h {
        img.extend((0..w).map(|x| luma(x, y)));
    }
    rxing_scan(w, h, img)
}

/// [`qr_scan`] for a row-major luma buffer.
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn qr_scan_luma(w: usize, h: usize, img: &[u8]) -> Vec<Vec<u8>> {
    rxing_scan(w, h, img.to_vec())
}

/// Pure-Rust decoder used on the web (and tested natively).
#[cfg(any(target_arch = "wasm32", test))]
fn rxing_scan(w: usize, h: usize, img: Vec<u8>) -> Vec<Vec<u8>> {
    let mut hints = rxing::DecodeHints {
        PossibleFormats: Some([rxing::BarcodeFormat::QR_CODE].into()),
        TryHarder: Some(false),
        ..Default::default()
    };
    // getRawBytes is the decoded byte-mode payload; getText would re-encode it as text and lose bytes.
    rxing::helpers::detect_multiple_in_luma_with_hints(img, w as u32, h as u32, &mut hints)
        .unwrap_or_default()
        .iter()
        .map(|r| r.getRawBytes().to_vec())
        .collect()
}

/// Rec. 601 luma of an RGB pixel.
#[must_use]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{File, Receiver, Sender};

    /// Tile `codes` QR codes per image (like the sender), scan them back, rebuild the file.
    fn qr_roundtrip(codes: usize) {
        let file = File {
            name: "x.bin".into(),
            data: (0..5000u32)
                .map(|i| i.wrapping_mul(40503).to_be_bytes()[1])
                .collect(),
        };
        let tx = Sender::new(&file, 600);
        let mut rx = Receiver::default();
        for f in 0..1000u64 {
            let qrs: Vec<_> = (0..codes as u64)
                .map(|i| qr_encode(&tx.frame(f * codes as u64 + i)))
                .collect();
            let img = qr_tile(&qrs);
            let scale = 3;
            let payloads = qr_scan(img.w * scale, img.h * scale, |x, y| {
                if img.dark[(y / scale) * img.w + x / scale] {
                    0
                } else {
                    255
                }
            });
            assert_eq!(payloads.len(), codes, "image {f}");
            for p in payloads {
                if let Some(got) = rx.push(&p).unwrap() {
                    assert_eq!(got, file);
                    return;
                }
            }
        }
        panic!("never finished");
    }

    #[test]
    fn qr_single() {
        qr_roundtrip(1);
    }

    /// Four codes as a camera might see them: 3.3 px per module, off-grid, low contrast, blurred, noisy.
    #[test]
    fn qr_camera_like() {
        let file = File {
            name: "x".into(),
            data: (0..50_000u32)
                .map(|i| i.wrapping_mul(2654435761).to_be_bytes()[0])
                .collect(),
        };
        let tx = Sender::new(&file, 1500);
        let qrs: Vec<_> = (0..4).map(|i| qr_encode(&tx.frame(i))).collect();
        let b = qr_tile(&qrs);
        let (w, h, px, off) = (1920, 1080, 3.3, 37.5);
        let sharp: Vec<u8> = (0..w * h)
            .map(|i| {
                let (mx, my) = (((i % w) as f32 - off) / px, ((i / w) as f32 - off) / px);
                let inside = mx >= 0.0 && my >= 0.0 && (mx as usize) < b.w && (my as usize) < b.h;
                if !inside {
                    90
                } else if b.dark[my as usize * b.w + mx as usize] {
                    45
                } else {
                    205
                }
            })
            .collect();
        let mut seed = 7u32;
        let img: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w).clamp(1, w - 2), (i / w).clamp(1, h - 2));
                let sum: u32 = (0..9)
                    .map(|k| sharp[(y + k / 3 - 1) * w + x + k % 3 - 1] as u32)
                    .sum();
                seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                ((sum / 9) as i32 + ((seed >> 16) % 25) as i32 - 12).clamp(0, 255) as u8
            })
            .collect();
        let mut got = qr_scan_luma(w, h, &img);
        let mut want: Vec<_> = (0..4).map(|i| tx.frame(i)).collect();
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }

    /// The web decoder must read the same camera-like frame and return the exact bytes.
    #[test]
    fn rxing_reads_camera_like() {
        let tx = Sender::new(
            &File {
                name: "x".into(),
                data: (0..50_000u32)
                    .map(|i| i.wrapping_mul(2654435761).to_be_bytes()[0])
                    .collect(),
            },
            1200,
        );
        let img = qr_tile(&(0..4).map(|i| qr_encode(&tx.frame(i))).collect::<Vec<_>>());
        let (w, h, px) = (1600, 1000, 3);
        let luma: Vec<u8> = (0..w * h)
            .map(|i| {
                let (x, y) = ((i % w) / px, (i / w) / px);
                if x >= img.w || y >= img.h {
                    128
                } else if img.dark[y * img.w + x] {
                    40
                } else {
                    210
                }
            })
            .collect();
        let mut got = rxing_scan(w, h, luma);
        let mut want: Vec<_> = (0..4).map(|i| tx.frame(i)).collect();
        got.sort();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn qr_tiled() {
        qr_roundtrip(2);
        qr_roundtrip(3);
        qr_roundtrip(4);
    }
}
