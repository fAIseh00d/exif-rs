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

/// The header bytes needed to reach the pointer pair.
const HEADER_LEN: usize = 92;

/// A JPEG larger than this is not a preview; refuse rather than allocate it.
const MAX_EMBEDDED_JPEG: u32 = 64 * 1024 * 1024;

pub fn is_raf(buf: &[u8]) -> bool {
    buf.starts_with(RAF_MAGIC)
}

/// Reads the Exif attributes out of a RAF's embedded JPEG.
///
/// The reader is expected to be positioned at the start of the file.
pub fn get_exif_attr<R>(reader: &mut R) -> Result<Vec<u8>, Error>
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

    reader.seek(io::SeekFrom::Start(u64::from(offset)))?;
    let mut jpeg = vec![0u8; length as usize];
    reader.read_exact(&mut jpeg)
        .map_err(|_| Error::InvalidFormat("Truncated RAF embedded JPEG"))?;
    crate::jpeg::get_exif_attr(&mut io::Cursor::new(jpeg))
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
        assert!(get_exif_attr(&mut io::Cursor::new(short)).is_err());
    }

    /// A length field is attacker-controlled; a bogus one must not become an
    /// allocation.
    #[test]
    fn refuses_an_absurd_embedded_length() {
        let mut f = RAF_MAGIC.to_vec();
        f.resize(HEADER_LEN, 0);
        f[84..88].copy_from_slice(&148u32.to_be_bytes());
        f[88..92].copy_from_slice(&u32::MAX.to_be_bytes());
        match get_exif_attr(&mut io::Cursor::new(f)) {
            Err(Error::InvalidFormat(m)) => assert_eq!(m, "Embedded JPEG too large"),
            other => panic!("expected a refusal, got {:?}", other),
        }
    }

    #[test]
    fn a_zero_length_jpeg_is_absence_not_corruption() {
        let mut f = RAF_MAGIC.to_vec();
        f.resize(HEADER_LEN, 0);
        assert!(matches!(get_exif_attr(&mut io::Cursor::new(f)),
                         Err(Error::NotFound(_))));
    }
}
