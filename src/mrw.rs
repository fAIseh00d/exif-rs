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

//! Minolta MRW — a block list with an ordinary TIFF inside it.
//!
//! `\0MRM`, a big-endian length, then blocks each of which is a four-byte
//! name and a big-endian length: `\0PRD` (sensor geometry), `\0WBG` (white
//! balance), `\0RIF` (picture settings), `\0PAD`, and **`\0TTW`, which holds
//! a complete TIFF** — Exif, MakerNote and all. So there is nothing to parse
//! here beyond finding that block and handing it over.
//!
//! **The block order is not fixed**, which is the only trap. Across the 14
//! bodies on raw.pixls.us the DSLRs (Dynax/Maxxum 5D and 7D, Alpha-7 Digital,
//! Alpha Sweet Digital) put `TTW` last, at offset 140, while the DiMAGE
//! compacts (5, 7, 7i, 7Hi, A1, A2, A200) put it second, at 48. A fixed
//! offset would read half the corpus correctly and the other half not at all.
//!
//! Offsets inside that TIFF count from its own start, so the caller sets
//! `tiff_base` to where the block begins — the same arrangement a RAF needs
//! for the Exif in its embedded JPEG.

use std::io;

use crate::error::Error;

/// `\0MRM` — the file magic.
const MRW_MAGIC: &[u8] = b"\x00MRM";

/// The block holding the TIFF.
const TTW: &[u8; 4] = b"\x00TTW";

/// The block holding the sensor and output geometry.
const PRD: &[u8; 4] = b"\x00PRD";

/// Where `PRD` states its four sizes, as big-endian `u16`s:
/// `[sensor height, sensor width, image height, image width]`.
const PRD_SIZES_AT: usize = 8;

/// A header longer than this is not a header. The largest in the corpus is
/// 106 KB (a Dynax 5D), and the block carries a full TIFF with a thumbnail.
const MAX_HEADER: u32 = 8 * 1024 * 1024;

/// Bytes needed before the block list starts: the magic and its length.
const PREAMBLE: usize = 8;

pub fn is_mrw(buf: &[u8]) -> bool {
    buf.starts_with(MRW_MAGIC)
}

/// The size the body actually outputs, from the `PRD` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MrwRawImage {
    pub width: u32,
    pub height: u32,
}

/// Where the TIFF is, and its bytes.
pub(crate) struct MrwContents {
    pub exif: Vec<u8>,
    /// Offset of the TIFF within the file — every offset inside it counts
    /// from here.
    pub tiff_offset: u64,
    /// What `PRD` says the picture is, when the block is present.
    ///
    /// **The TIFF inside `TTW` is not reliable for this.** A DiMAGE 5's IFD0
    /// states 1600x1200 while the body outputs 2048x1544 -- that IFD is
    /// describing a preview. `PRD` states the real geometry on every body in
    /// the corpus and agrees with the TIFF wherever the TIFF is right, which
    /// is what makes it the source rather than a second opinion.
    pub raw_image: Option<MrwRawImage>,
}

/// Reads the block list and returns the `TTW` block's TIFF.
///
/// The reader is expected to be positioned at the start of the file.
pub(crate) fn get_exif_attr<R>(reader: &mut R) -> Result<MrwContents, Error>
where
    R: io::BufRead + io::Seek,
{
    let mut head = [0u8; PREAMBLE];
    reader
        .read_exact(&mut head)
        .map_err(|_| Error::InvalidFormat("Truncated MRW header"))?;
    if !is_mrw(&head) {
        return Err(Error::InvalidFormat("Not an MRW file"));
    }
    let header_len = u32::from_be_bytes([head[4], head[5], head[6], head[7]]);
    if header_len == 0 || header_len > MAX_HEADER {
        return Err(Error::InvalidFormat("Implausible MRW header length"));
    }

    // Walk the blocks. Each is a 4-byte name and a 4-byte big-endian length,
    // and the list runs to the end of the header.
    let end = u64::from(header_len) + PREAMBLE as u64;
    let mut at = PREAMBLE as u64;
    let mut raw_image = None;
    let mut ttw: Option<(Vec<u8>, u64)> = None;
    while at + 8 <= end {
        reader
            .seek(io::SeekFrom::Start(at))
            .map_err(|_| Error::InvalidFormat("Truncated MRW block list"))?;
        let mut blk = [0u8; 8];
        if reader.read_exact(&mut blk).is_err() {
            break;
        }
        let len = u32::from_be_bytes([blk[4], blk[5], blk[6], blk[7]]);
        let body = at + 8;
        match &blk[..4] {
            b if b == TTW => {
                if len == 0 || u64::from(len) > u64::from(MAX_HEADER) {
                    return Err(Error::InvalidFormat("Implausible MRW TTW length"));
                }
                let mut exif = vec![0u8; len as usize];
                reader
                    .read_exact(&mut exif)
                    .map_err(|_| Error::InvalidFormat("Truncated MRW TTW block"))?;
                ttw = Some((exif, body));
            }
            b if b == PRD => {
                // Four big-endian u16s; only the output pair is wanted.
                let want = PRD_SIZES_AT + 8;
                if len as usize >= want {
                    let mut prd = vec![0u8; want];
                    if reader.read_exact(&mut prd).is_ok() {
                        let at16 = |p: usize| u32::from(u16::from_be_bytes([prd[p], prd[p + 1]]));
                        let (h, w) = (at16(PRD_SIZES_AT + 4), at16(PRD_SIZES_AT + 6));
                        if w > 0 && h > 0 {
                            raw_image = Some(MrwRawImage { width: w, height: h });
                        }
                    }
                }
            }
            _ => {}
        }
        // Both blocks found; the rest of the list is not needed. `PRD` comes
        // first on every body in the corpus, but the order is the vendor's
        // and not something to rely on -- so the walk continues until TTW is
        // in hand rather than stopping at whichever came first.
        if let Some((exif, tiff_offset)) = ttw.take() {
            return Ok(MrwContents { exif, tiff_offset, raw_image });
        }
        // Not this one; step over it. A length that overflows the walk is a
        // malformed list rather than a block to trust.
        at = match body.checked_add(u64::from(len)) {
            Some(next) if next > at => next,
            _ => break,
        };
    }
    Err(Error::NotFound("MRW"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an MRW whose blocks appear in the given order.
    fn mrw(order: &[&[u8; 4]], ttw: &[u8]) -> Vec<u8> {
        let mut blocks = Vec::new();
        for name in order {
            let body: &[u8] = if *name == TTW { ttw } else { &[0u8; 16] };
            blocks.extend_from_slice(*name);
            blocks.extend_from_slice(&(body.len() as u32).to_be_bytes());
            blocks.extend_from_slice(body);
        }
        let mut f = MRW_MAGIC.to_vec();
        f.extend_from_slice(&(blocks.len() as u32).to_be_bytes());
        f.extend_from_slice(&blocks);
        f
    }

    const TIFF: &[u8] = b"MM\x00\x2a\x00\x00\x00\x08\x00\x00\x00\x00\x00\x00";

    /// **The block order is the vendor's business, not ours.** A Dynax 5D
    /// puts `TTW` last and a DiMAGE A2 puts it second; reading either from a
    /// fixed offset gets the other wrong.
    #[test]
    fn finds_the_tiff_wherever_the_block_sits() {
        for order in [
            // DSLR order: TTW last.
            &[b"\x00PRD", b"\x00WBG", b"\x00RIF", TTW][..],
            // DiMAGE order: TTW second.
            &[b"\x00PRD", TTW, b"\x00WBG", b"\x00RIF"][..],
        ] {
            let f = mrw(order, TIFF);
            let got = get_exif_attr(&mut io::Cursor::new(&f)).expect("TTW");
            assert_eq!(got.exif, TIFF);
            // The offset must point at the TIFF itself, since every offset
            // inside it counts from there.
            assert_eq!(&f[got.tiff_offset as usize..][..TIFF.len()], TIFF);
        }
    }

    #[test]
    fn refuses_a_file_that_is_not_mrw() {
        assert!(get_exif_attr(&mut io::Cursor::new(b"MM\x00\x2a".to_vec())).is_err());
        assert!(get_exif_attr(&mut io::Cursor::new(Vec::new())).is_err());
    }

    /// A block list with no TIFF in it is absence, not corruption.
    #[test]
    fn a_file_without_a_ttw_block_reports_nothing() {
        let f = mrw(&[b"\x00PRD", b"\x00WBG"], TIFF);
        assert!(matches!(get_exif_attr(&mut io::Cursor::new(&f)),
                         Err(Error::NotFound(_))));
    }

    /// The header length is the file's word; an absurd one must not become an
    /// allocation.
    #[test]
    fn refuses_an_absurd_header_length() {
        let mut f = MRW_MAGIC.to_vec();
        f.extend_from_slice(&u32::MAX.to_be_bytes());
        assert!(get_exif_attr(&mut io::Cursor::new(&f)).is_err());
    }
}
