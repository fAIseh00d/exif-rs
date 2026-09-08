//
// Copyright (c) 2026 Ivan Rodnov.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions
// are met:
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// THIS SOFTWARE IS PROVIDED BY THE AUTHOR AND CONTRIBUTORS ``AS IS'' AND
// ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
// IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
// ARE DISCLAIMED.  IN NO EVENT SHALL THE AUTHOR OR CONTRIBUTORS BE LIABLE
// FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
// DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
// OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
// HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
// LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
// OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
// SUCH DAMAGE.
//

//! Fujifilm RAF.
//!
//! Unlike every other raw this crate reads, a RAF is **not a TIFF and not a
//! box tree**: it opens with a 16-byte magic followed by fixed-position
//! fields, and nothing in it is discoverable by walking. The Exif attributes
//! are not in the RAF's own structure at all — they live in a complete JPEG
//! embedded in the file, whose offset and length are two big-endian `u32`s at
//! a constant position. So the whole of RAF support is: read those two
//! numbers, seek, and hand the JPEG to the JPEG reader.
//!
//! Verified against seven bodies spanning twenty years — FinePix E550 (2005),
//! S6000fd, SL1000, GFX100 II, GFX100RF, X-E5, X-T30 III (2025). All carry
//! version `0201` and a real `FFD8` at the declared offset. **The offset is
//! read, never assumed**: it is 148 in all seven, which is exactly the sort of
//! constant that invites hard-coding and would break on the first body that
//! sizes its header differently. The directory version is NOT a gate either —
//! six files say `0100` and the X-E5 says `0110`.

use std::io;

use crate::error::Error;

/// `FUJIFILMCCD-RAW ` — the trailing space is part of it.
const RAF_MAGIC: &[u8] = b"FUJIFILMCCD-RAW ";

/// Where the embedded JPEG's offset and length sit, as two big-endian `u32`s.
const JPEG_PTR_AT: u64 = 84;

/// Where the CFA header's offset and length sit, immediately after the JPEG's.
const CFA_PTR_AT: usize = 92;

/// `RawImageCroppedSize` in the CFA header directory — the dimensions the body
/// actually outputs. **Stored HEIGHT FIRST.**
const RAF_CROPPED_SIZE: u16 = 0x0111;

/// A CFA header larger than this is not a header; refuse rather than allocate.
const MAX_CFA_HEADER: u32 = 1 << 20;

/// The header bytes needed to reach both pointer pairs.
const HEADER_LEN: usize = 100;

// The two pointer pairs are read as fixed slices of `header`, so their range
// must be inside it. `CFA_PTR_AT + 8` is exactly `HEADER_LEN` today: without
// this, shrinking the header would still compile and panic on the first file.
const _: () = assert!(JPEG_PTR_AT as usize + 8 <= HEADER_LEN);
const _: () = assert!(CFA_PTR_AT + 8 <= HEADER_LEN);

/// A JPEG larger than this is not a preview; refuse rather than allocate it.
const MAX_EMBEDDED_JPEG: u32 = 64 * 1024 * 1024;

pub fn is_raf(buf: &[u8]) -> bool {
    buf.starts_with(RAF_MAGIC)
}

/// A RAF, as far as the Exif reader is concerned: the Exif of its embedded
/// JPEG, where in the file that JPEG starts, and what the RAF itself says
/// about the raw image.
#[derive(Debug)]
pub(crate) struct RafContents {
    pub exif: Vec<u8>,
    /// File offset of the embedded JPEG — **every offset in `exif` is relative
    /// to a TIFF header inside that JPEG**, so without this they point into
    /// nothing.
    pub jpeg_offset: u64,
    /// Its length. The JPEG is the body's own full-size rendering — 5.4 MB on
    /// an X-T30 III against an 8.8 KB IFD1 thumbnail — so it is worth
    /// reporting as an embedded image and not merely reading Exif out of.
    pub jpeg_length: u32,
    pub raw_image: Option<RafRawImage>,
}

/// What a RAF states about its raw image, as opposed to its embedded JPEG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RafRawImage {
    /// `RawImageCroppedSize` — the size the body outputs, and the figure on
    /// the spec sheet: 6240×4160 on an X-T30 III, against 6336×4182 for the
    /// full readout and 4416×2944 for the embedded JPEG.
    pub width: u32,
    pub height: u32,
}

/// Reads the CFA header directory for the raw image's cropped size.
///
/// The directory is a 4-byte big-endian count followed by `{u16 tag, u16
/// size, data}` records, at the offset named by bytes 92..96 of the fixed
/// header. `None` when the file does not carry the record — this is a fact to
/// report or not, never a guess to synthesise.
fn raw_image_from(len: u32, whole: &mut impl io::Read) -> Option<RafRawImage> {
    if len == 0 || len > MAX_CFA_HEADER {
        return None;
    }
    let mut block = vec![0u8; len as usize];
    whole.read_exact(&mut block).ok()?;

    let count = u32::from_be_bytes(block.get(0..4)?.try_into().ok()?);
    let mut p = 4usize;
    for _ in 0..count.min(256) {
        let tag = u16::from_be_bytes(block.get(p..p + 2)?.try_into().ok()?);
        let size = u16::from_be_bytes(block.get(p + 2..p + 4)?.try_into().ok()?) as usize;
        let data = block.get(p + 4..p + 4 + size)?;
        if tag == RAF_CROPPED_SIZE && size >= 4 {
            // HEIGHT first: `10 40 18 60` is 4160 then 6240. Read the other
            // way round it yields a portrait 26 MP frame -- plausible, wrong.
            let height = u32::from(u16::from_be_bytes(data[0..2].try_into().ok()?));
            let width = u32::from(u16::from_be_bytes(data[2..4].try_into().ok()?));
            return (width > 0 && height > 0).then_some(RafRawImage { width, height });
        }
        p += 4 + size;
    }
    None
}

/// The Exif attributes out of a RAF's embedded JPEG, plus what the RAF states
/// about its RAW image — which the embedded JPEG's own Exif cannot.
///
/// The reader is expected to be positioned at the start of the file.
pub(crate) fn get_exif_and_raw_image<R>(reader: &mut R) -> Result<RafContents, Error>
where R: io::BufRead + io::Seek {
    let mut header = [0u8; HEADER_LEN];
    reader.read_exact(&mut header)
        .map_err(|_| Error::InvalidFormat("Truncated RAF header"))?;
    if !is_raf(&header) {
        return Err(Error::InvalidFormat("Not a RAF file"));
    }
    let at = JPEG_PTR_AT as usize;
    let offset = u32::from_be_bytes(
        header[at..at + 4].try_into().expect("4 bytes"));
    let length = u32::from_be_bytes(
        header[at + 4..at + 8].try_into().expect("4 bytes"));
    if length == 0 {
        return Err(Error::NotFound("RAF"));
    }
    if length > MAX_EMBEDDED_JPEG {
        return Err(Error::InvalidFormat("Embedded JPEG too large"));
    }

    // The CFA header first: it is a separate region and reading it here keeps
    // one forward pass over the file.
    let cfa_offset = u32::from_be_bytes(
        header[CFA_PTR_AT..CFA_PTR_AT + 4].try_into().unwrap_or([0; 4]));
    let cfa_len = u32::from_be_bytes(
        header[CFA_PTR_AT + 4..CFA_PTR_AT + 8].try_into().unwrap_or([0; 4]));
    let raw_image = if cfa_offset > 0
        && reader.seek(io::SeekFrom::Start(u64::from(cfa_offset))).is_ok() {
        raw_image_from(cfa_len, reader)
    } else {
        None
    };

    reader.seek(io::SeekFrom::Start(u64::from(offset)))?;
    let mut jpeg = vec![0u8; length as usize];
    reader.read_exact(&mut jpeg)
        .map_err(|_| Error::InvalidFormat("Truncated RAF embedded JPEG"))?;
    Ok(RafContents {
        exif: crate::jpeg::get_exif_attr(&mut io::Cursor::new(jpeg))?,
        jpeg_offset: u64::from(offset),
        jpeg_length: length,
        raw_image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_magic_including_its_trailing_space() {
        assert!(is_raf(b"FUJIFILMCCD-RAW 0201"));
        // Without the space it is not the signature.
        assert!(!is_raf(b"FUJIFILMCCD-RAW0201"));
        assert!(!is_raf(b"\xff\xd8\xff\xe1"));
        assert!(!is_raf(b""));
    }

    /// A header that ends before the pointer pair must be refused, not read
    /// out of bounds.
    #[test]
    fn refuses_a_truncated_header() {
        let mut short = RAF_MAGIC.to_vec();
        short.resize(64, 0);
        assert!(get_exif_and_raw_image(&mut io::Cursor::new(short)).is_err());
    }

    /// A length field is attacker-controlled; a bogus one must not become an
    /// allocation.
    #[test]
    fn refuses_an_absurd_embedded_length() {
        let mut f = RAF_MAGIC.to_vec();
        f.resize(HEADER_LEN, 0);
        f[84..88].copy_from_slice(&148u32.to_be_bytes());
        f[88..92].copy_from_slice(&u32::MAX.to_be_bytes());
        match get_exif_and_raw_image(&mut io::Cursor::new(f)) {
            Err(Error::InvalidFormat(m)) => assert_eq!(m, "Embedded JPEG too large"),
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    /// The CFA header directory: a big-endian count, then
    /// `{u16 tag, u16 size, data}`.
    fn cfa(records: &[(u16, &[u8])]) -> Vec<u8> {
        let mut b = (records.len() as u32).to_be_bytes().to_vec();
        for &(tag, data) in records {
            b.extend_from_slice(&tag.to_be_bytes());
            b.extend_from_slice(&(data.len() as u16).to_be_bytes());
            b.extend_from_slice(data);
        }
        b
    }

    /// `RawImageCroppedSize` is stored HEIGHT FIRST. Read the other way round
    /// an X-T30 III becomes a portrait 4160x6240 frame — a plausible-looking
    /// number that no Fujifilm body produces.
    #[test]
    fn the_cropped_size_is_height_then_width() {
        // The bytes from a real X-T30 III: 0x1040 = 4160, 0x1860 = 6240.
        let block = cfa(&[
            (0x0100, &[0x10, 0x56, 0x18, 0xc0]),          // full size, skipped
            (0x0111, &[0x10, 0x40, 0x18, 0x60]),          // cropped size
        ]);
        let got = raw_image_from(block.len() as u32, &mut io::Cursor::new(&block));
        assert_eq!(got, Some(RafRawImage { width: 6240, height: 4160 }));
    }

    /// A file that does not carry the record gets no answer, rather than one
    /// invented from the full readout.
    #[test]
    fn a_raf_without_the_record_reports_nothing() {
        let block = cfa(&[(0x0100, &[0x10, 0x56, 0x18, 0xc0])]);
        assert_eq!(raw_image_from(block.len() as u32, &mut io::Cursor::new(&block)), None);
    }

    /// The record count and each size come from the file, so a truncated or
    /// lying directory must run out rather than read past the block.
    #[test]
    fn a_truncated_directory_stops() {
        let mut block = cfa(&[(0x0111, &[0x10, 0x40, 0x18, 0x60])]);
        block.truncate(6);
        assert_eq!(raw_image_from(block.len() as u32, &mut io::Cursor::new(&block)), None);
        // A count far larger than the data.
        let mut lying = 9999u32.to_be_bytes().to_vec();
        lying.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(raw_image_from(lying.len() as u32, &mut io::Cursor::new(&lying)), None);
    }

    #[test]
    fn a_zero_length_jpeg_is_absence_not_corruption() {
        let mut f = RAF_MAGIC.to_vec();
        f.resize(HEADER_LEN, 0);
        assert!(matches!(get_exif_and_raw_image(&mut io::Cursor::new(f)),
                         Err(Error::NotFound(_))));
    }
}
