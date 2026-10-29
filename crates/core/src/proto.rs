//! Wire protocol.
//!
//! A file is packed into an *object*:
//!   flags u8 (bit0 = brotli) | sha256(original) [32] | name_len u16 BE | name | body
//! The object is RaptorQ fountain-coded. Every QR code is self-describing:
//!   "FQ" | version u8 | session u32 BE | OTI [12] | RaptorQ packet (payload id [4] + symbol)
//! The receiver needs roughly K + 2 distinct packets per block, in any order.

use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAGIC: &[u8; 2] = b"FQ";
const VERSION: u8 = 2;
/// Bytes of framing before the RaptorQ packet in every QR payload.
pub const HEADER: usize = 2 + 1 + 4 + 12;
/// Largest file a receiver will accept, packed or unpacked. Bounds memory for hostile input.
pub const MAX_FILE: u64 = 1 << 30;

/// A named blob: what is sent and what is received.
#[derive(Clone, Debug, PartialEq)]
pub struct File {
    /// File name only, no directories. Comes off the wire: never trust it as a path.
    pub name: String,
    pub data: Vec<u8>,
}

fn compress(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    // ponytail: quality 11 runs at ~1 MB/s; drop to 9 above 4 MB so big files don't stall the sender.
    let quality = if data.len() <= 4 << 20 { 11 } else { 9 };
    let mut w = brotli::CompressorWriter::new(Vec::new(), 1 << 16, quality, 24);
    w.write_all(data).expect("writing to a Vec cannot fail");
    w.into_inner()
}

fn pack(file: &File) -> Vec<u8> {
    let packed = compress(&file.data);
    let (flags, body) = if packed.len() < file.data.len() {
        (1u8, &packed)
    } else {
        (0, &file.data)
    };
    let name = &file.name.as_bytes()[..file.name.len().min(u16::MAX as usize)];
    let mut out = Vec::with_capacity(35 + name.len() + body.len());
    out.push(flags);
    out.extend_from_slice(&Sha256::digest(&file.data));
    out.extend_from_slice(&(name.len() as u16).to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(body);
    out
}

fn unpack(obj: &[u8]) -> Result<File, String> {
    use std::io::Read;
    let bad = || "corrupt object".to_string();
    let flags = *obj.first().ok_or_else(bad)?;
    let hash = obj.get(1..33).ok_or_else(bad)?;
    let name_len = u16::from_be_bytes(
        obj.get(33..35)
            .ok_or_else(bad)?
            .try_into()
            .expect("fixed-length slice"),
    ) as usize;
    let name = String::from_utf8_lossy(obj.get(35..35 + name_len).ok_or_else(bad)?).into_owned();
    let body = &obj[35 + name_len..];
    let data = if flags & 1 != 0 {
        let mut out = Vec::new();
        // Cap the output: a tiny hostile stream could otherwise expand without bound.
        brotli::Decompressor::new(body, 1 << 16)
            .take(MAX_FILE + 1)
            .read_to_end(&mut out)
            .map_err(|e| e.to_string())?;
        if out.len() as u64 > MAX_FILE {
            return Err("file too large".into());
        }
        out
    } else {
        body.to_vec()
    };
    if Sha256::digest(&data).as_slice() != hash {
        return Err("SHA-256 mismatch".into());
    }
    Ok(File { name, data })
}

/// Turns a [`File`] into an endless stream of QR payloads.
pub struct Sender {
    prefix: Vec<u8>,
    encoder: Encoder,
    source: Vec<Vec<EncodingPacket>>,
    /// Source symbols across all blocks: the minimum number of frames to send.
    pub total_symbols: usize,
    /// Size of the packed (maybe compressed) object actually sent.
    pub packed_len: usize,
}

impl Sender {
    /// `frame_bytes` is the target size of one QR payload.
    pub fn new(file: &File, frame_bytes: usize) -> Sender {
        let obj = pack(file);
        let session = u32::from_be_bytes(
            Sha256::digest(&obj)[..4]
                .try_into()
                .expect("fixed-length slice"),
        );
        let mtu = frame_bytes
            .saturating_sub(HEADER + 4)
            .max(64)
            .min(u16::MAX as usize) as u16;
        let encoder = Encoder::with_defaults(&obj, mtu);
        let mut prefix = Vec::with_capacity(HEADER);
        prefix.extend_from_slice(MAGIC);
        prefix.push(VERSION);
        prefix.extend_from_slice(&session.to_be_bytes());
        prefix.extend_from_slice(&encoder.get_config().serialize());
        let source: Vec<_> = encoder
            .get_block_encoders()
            .iter()
            .map(|b| b.source_packets())
            .collect();
        let total_symbols = source.iter().map(Vec::len).sum();
        Sender {
            prefix,
            encoder,
            source,
            total_symbols,
            packed_len: obj.len(),
        }
    }

    /// Packet `n` of an endless stream: source symbols first, then fresh repair symbols forever.
    #[must_use]
    pub fn frame(&self, n: u64) -> Vec<u8> {
        let blocks = self.source.len() as u64;
        let block = (n % blocks) as usize;
        let esi = (n / blocks) as usize;
        let src = &self.source[block];
        let packet = if esi < src.len() {
            src[esi].clone()
        } else {
            let repair = ((esi - src.len()) % (1 << 23)) as u32;
            self.encoder.get_block_encoders()[block]
                .repair_packets(repair, 1)
                .pop()
                .expect("asked for one repair packet")
        };
        let mut out = self.prefix.clone();
        out.extend_from_slice(&packet.serialize());
        out
    }
}

/// Rebuilds a [`File`] from QR payloads that arrive in any order, with gaps.
#[derive(Default)]
pub struct Receiver {
    /// Session id plus the raw OTI: a sender that changes its symbol size keeps the session id
    /// (same file) but starts a new, incompatible stream.
    session: Option<[u8; 16]>,
    decoder: Option<Decoder>,
    seen: HashSet<[u8; 4]>,
    /// Distinct packets needed to decode; 0 until the first valid frame.
    pub needed: usize,
    /// Symbol size in bytes.
    pub symbol: usize,
    /// Size of the packed object being received.
    pub packed_len: u64,
}

impl Receiver {
    /// Distinct packets received so far.
    #[must_use]
    pub fn received(&self) -> usize {
        self.seen.len()
    }

    /// Feed one QR payload. Returns the file once complete and verified.
    /// A frame from a different session resets the receiver.
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<File>, String> {
        if frame.len() <= HEADER + 4 || &frame[..2] != MAGIC || frame[2] != VERSION {
            return Err("not a foqdt frame".into());
        }
        let session: [u8; 16] = frame[3..HEADER].try_into().expect("fixed-length slice");
        if self.session != Some(session) {
            let oti = ObjectTransmissionInformation::deserialize(
                frame[7..HEADER].try_into().expect("fixed-length slice"),
            );
            // Senders always use raptorq's defaults, so anything else is corrupt or hostile, and
            // raptorq panics on some inconsistent values (zero sub-blocks, misaligned symbols).
            let len = oti.transfer_length();
            if oti.symbol_size() < 8
                || len == 0
                || len > MAX_FILE
                || oti != ObjectTransmissionInformation::with_defaults(len, oti.symbol_size())
            {
                return Err("bad OTI".into());
            }
            *self = Receiver {
                session: Some(session),
                decoder: Some(Decoder::new(oti)),
                seen: HashSet::new(),
                needed: oti.transfer_length().div_ceil(oti.symbol_size() as u64) as usize,
                symbol: oti.symbol_size() as usize,
                packed_len: len,
            };
        }
        let packet = &frame[HEADER..];
        let oti = ObjectTransmissionInformation::deserialize(
            session[4..].try_into().expect("fixed-length slice"),
        );
        // raptorq indexes blocks and asserts symbol sizes; check both before handing it the packet.
        if packet.len() != 4 + self.symbol || packet[0] >= oti.source_blocks() {
            return Err("malformed packet".into());
        }
        if !self
            .seen
            .insert(packet[..4].try_into().expect("fixed-length slice"))
        {
            return Ok(None);
        }
        match self
            .decoder
            .as_mut()
            .expect("decoder is set with the session")
            .decode(EncodingPacket::deserialize(packet))
        {
            Some(obj) => unpack(&obj).map(Some),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: Vec<u8>, frame_bytes: usize, skip_every: u64) {
        let file = File {
            name: "a.bin".into(),
            data,
        };
        let tx = Sender::new(&file, frame_bytes);
        let mut rx = Receiver::default();
        for n in 0.. {
            if n % skip_every == 0 {
                continue; // simulate dropped frames
            }
            let f = tx.frame(n);
            assert!(
                f.len() <= frame_bytes.max(HEADER + 4 + 64),
                "frame {n} is {} bytes",
                f.len()
            );
            if let Some(got) = rx.push(&f).unwrap() {
                assert_eq!(got, file);
                return;
            }
            assert!(n < 10 * tx.total_symbols as u64 + 100, "never finished");
        }
    }

    #[test]
    fn roundtrips() {
        roundtrip(b"hello".to_vec(), 300, 3);
        roundtrip(
            (0..200_000u32)
                .map(|i| i.wrapping_mul(2654435761).to_be_bytes()[0])
                .collect(),
            800,
            2,
        ); // incompressible-ish
        roundtrip(vec![7; 100_000], 500, 5); // compressible
    }

    #[test]
    fn compresses_text() {
        let text = include_str!("proto.rs").repeat(4).into_bytes();
        let tx = Sender::new(
            &File {
                name: "t".into(),
                data: text.clone(),
            },
            800,
        );
        assert!(
            tx.packed_len * 5 < text.len(),
            "{} -> {}",
            text.len(),
            tx.packed_len
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(
            Receiver::default().push(b"http://example.com").is_err(),
            "accepted a URL"
        );
    }

    /// Corrupt or hostile frames must be rejected, never panic.
    #[test]
    fn survives_hostile_frames() {
        let tx = Sender::new(
            &File {
                name: "a".into(),
                data: vec![1; 5000],
            },
            400,
        );
        let good = tx.frame(0);
        let mut rx = Receiver::default();
        rx.push(&good).unwrap();
        let mut short = good.clone();
        short.pop();
        assert!(rx.push(&short).is_err(), "accepted a truncated packet");
        let mut bad_block = good.clone();
        bad_block[HEADER] = 200;
        assert!(
            rx.push(&bad_block).is_err(),
            "accepted an out-of-range block"
        );
        for i in 7..HEADER {
            for v in [0u8, 1, 0xff] {
                let mut f = good.clone();
                f[i] = v;
                let _ = Receiver::default().push(&f);
            }
        }
    }

    /// Changing bytes/code mid-stream keeps the session id but must restart the decoder.
    #[test]
    fn restarts_when_symbol_size_changes() {
        let file = File {
            name: "a.bin".into(),
            data: (0..20_000u32)
                .map(|i| i.wrapping_mul(2654435761).to_be_bytes()[0])
                .collect(),
        };
        let (a, b) = (Sender::new(&file, 300), Sender::new(&file, 900));
        let mut rx = Receiver::default();
        for n in 0..5 {
            rx.push(&a.frame(n)).unwrap();
        }
        for n in 0.. {
            if let Some(got) = rx.push(&b.frame(n)).unwrap() {
                assert_eq!(got, file);
                break;
            }
            assert!(n < 1000, "never finished");
        }
    }
}
