//! foqdt wire protocol: RaptorQ fountain code and Brotli ([`Sender`], [`Receiver`]),
//! plus QR encoding and scanning.

mod proto;
mod qr;

pub use proto::{File, HEADER, MAX_FILE, Receiver, Sender};
pub use qr::{Bitmap, luma, qr_encode, qr_scan, qr_scan_luma, qr_tile};
