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

//! Sigma/Foveon X3F — a directory at the END, and no TIFF anywhere.
//!
//! `FOVb`, and the last four bytes of the file point at a `SECd` directory:
//! a version, a count, and entries of `{offset, length, type}`. Five entries
//! on every body measured — `PROP`, `CAMF`, and three images.
//!
//! **There is no TIFF in an X3F**, so unlike a RAF or an MRW there is nothing
//! to hand to the parser. The metadata is in `PROP` as UTF-16 name/value
//! pairs, and the fields it yields are synthesised, the way RW2's sensor
//! borders already are.
//!
//! What `PROP` carries is everything ingest needs, and more than the TIFF
//! dialects give: `CAMMANUF`, `CAMMODEL`, `CAMSERIAL`, `SHUTTER`, `APERTURE`,
//! `ISO`, and **`TIME` as a Unix timestamp** — which is the one thing a raw
//! usually states as a local wall clock and here does not.
//!
//! An image section is `SECi` with a format word: **18 is a JPEG**, and its
//! bytes start right after the 28-byte section header, so the preview is an
//! address like any other. Format 6 is the Foveon mosaic and 3 an
//! uncompressed RGB thumbnail; neither is a picture a viewer can show.

use std::io::{self, SeekFrom};

use crate::error::Error;

const X3F_MAGIC: &[u8] = b"FOVb";
const SECD: &[u8; 4] = b"SECd";
const SECP: &[u8; 4] = b"SECp";
const SECI: &[u8; 4] = b"SECi";

/// `SECi`'s header: magic, version, type, format, columns, rows, row size.
const IMAGE_HEADER: u32 = 28;
/// The image format that means "the bytes here are a JPEG".
const FORMAT_JPEG: u32 = 18;

/// A directory of more than this is not a directory.
const MAX_ENTRIES: u32 = 64;
/// `PROP` is a small block of text; a claim beyond this is not one.
const MAX_PROP: u32 = 1 << 20;
/// Enough of the file header to reach the stated size.
const HEADER_SIZE: usize = 36;
/// The version at which the header stopped stating the picture's size.
const QUATTRO_VERSION: u32 = 4 << 16;
/// A sensor edge beyond this is not one.
const MAX_EDGE: u32 = 1 << 17;

pub fn is_x3f(buf: &[u8]) -> bool {
    buf.starts_with(X3F_MAGIC)
}

/// What an X3F states about itself.
#[derive(Debug, Default)]
pub(crate) struct X3fContents {
    /// `PROP` name/value pairs, in file order.
    pub props: Vec<(String, String)>,
    /// Offset and length of an embedded JPEG, when one is present.
    pub preview: Option<(u64, u32)>,
    /// The embedded JPEG that carries an `Exif` APP1, when one does.
    ///
    /// **The Quattro generation dropped `PROP` entirely** — an sd Quattro or
    /// dp2 Quattro has only `CAMF` and `SPPA`, and `CAMF` is obfuscated. What
    /// it gained is an ordinary Exif block inside the preview, naming the
    /// body and the moment, so the metadata is read the way a RAF's is: from
    /// the JPEG the container happens to hold.
    pub exif_jpeg: Option<(u64, u32)>,
    /// The picture's size, as the FILE HEADER states it.
    ///
    /// **Not the readout, and the difference is every body.** An image
    /// section states its own columns and rows, but those are the sensor's:
    /// a DP1 Merrill reads 4928x3264 and renders 4704x3136, an SD10 reads
    /// 2304x1531 and renders 2268x1512. The header at offset 28 states the
    /// second number, and it is the one exiftool and libraw both report.
    ///
    /// **Version 4 -- the Quattro generation -- does not state it there.**
    /// The field holds something else entirely (1694217600 on an sd
    /// Quattro), so it is read only below that version; a Quattro's preview
    /// carries an ordinary `PixelXDimension` instead, and that agrees.
    pub frame: Option<(u32, u32)>,
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Reads the trailing directory and the sections it names.
pub(crate) fn get_contents<R>(reader: &mut R) -> Result<X3fContents, Error>
where
    R: io::BufRead + io::Seek,
{
    // `FOVb`, a version, a 16-byte identifier, a mark word, then the size.
    let mut header = [0u8; HEADER_SIZE];
    reader
        .read_exact(&mut header)
        .map_err(|_| Error::InvalidFormat("Truncated X3F"))?;
    if !is_x3f(&header) {
        return Err(Error::InvalidFormat("Not an X3F file"));
    }
    let mut out = X3fContents::default();
    let version = le32(&header, 4).unwrap_or(0);
    if version < QUATTRO_VERSION {
        if let (Some(c), Some(r)) = (le32(&header, 28), le32(&header, 32)) {
            if c > 0 && r > 0 && c <= MAX_EDGE && r <= MAX_EDGE {
                out.frame = Some((c, r));
            }
        }
    }
    let end = reader
        .seek(SeekFrom::End(0))
        .map_err(|_| Error::InvalidFormat("X3F is not seekable"))?;
    if end < 8 {
        return Err(Error::InvalidFormat("Truncated X3F"));
    }

    // The last four bytes address the directory.
    reader.seek(SeekFrom::Start(end - 4))?;
    let mut tail = [0u8; 4];
    reader.read_exact(&mut tail)?;
    let dir_at = u64::from(u32::from_le_bytes(tail));
    if dir_at + 12 > end {
        return Err(Error::InvalidFormat("X3F directory is out of range"));
    }

    reader.seek(SeekFrom::Start(dir_at))?;
    let mut head = [0u8; 12];
    reader.read_exact(&mut head)?;
    if &head[..4] != SECD {
        return Err(Error::InvalidFormat("X3F directory is not SECd"));
    }
    let count = le32(&head, 8).ok_or(Error::InvalidFormat("X3F directory"))?;
    if count == 0 || count > MAX_ENTRIES {
        return Err(Error::InvalidFormat("Implausible X3F entry count"));
    }

    let mut table = vec![0u8; count as usize * 12];
    reader
        .read_exact(&mut table)
        .map_err(|_| Error::InvalidFormat("Truncated X3F directory"))?;

    for i in 0..count as usize {
        let base = i * 12;
        let (Some(off), Some(len)) = (le32(&table, base), le32(&table, base + 4)) else {
            continue;
        };
        let typ: [u8; 4] = match table.get(base + 8..base + 12).and_then(|s| s.try_into().ok()) {
            Some(t) => t,
            None => continue,
        };
        if u64::from(off) + u64::from(len) > end {
            continue;
        }
        match &typ {
            b"PROP" => read_props(reader, off, len, &mut out),
            // Both spellings appear: `IMAG` on the SD9, `IMA2` from the SD14
            // on. They are the same section.
            b"IMAG" | b"IMA2" => read_image(reader, off, len, &mut out),
            _ => {}
        }
    }
    Ok(out)
}

/// `SECp`: a header, then a table of `{name, value}` offsets into UTF-16 text.
fn read_props<R: io::BufRead + io::Seek>(reader: &mut R, off: u32, len: u32, out: &mut X3fContents) {
    if len > MAX_PROP {
        return;
    }
    let mut buf = vec![0u8; len as usize];
    if reader.seek(SeekFrom::Start(u64::from(off))).is_err() || reader.read_exact(&mut buf).is_err()
    {
        return;
    }
    if buf.get(..4) != Some(SECP) {
        return;
    }
    let Some(n) = le32(&buf, 8) else { return };
    // The header is 24 bytes; the table follows, and the text after that.
    let table = 24usize;
    let Some(text) = table.checked_add(n as usize * 8) else { return };
    if n > MAX_PROP || text > buf.len() {
        return;
    }
    // Offsets in the table are in UTF-16 units from the start of the text.
    let utf16_at = |start: usize| -> Option<String> {
        let mut at = text.checked_add(start.checked_mul(2)?)?;
        let mut s = String::new();
        loop {
            let c = u16::from_le_bytes(buf.get(at..at + 2)?.try_into().ok()?);
            if c == 0 {
                return Some(s);
            }
            s.push(char::from_u32(u32::from(c))?);
            at += 2;
            // A string running to the end of the block is a malformed table,
            // not a very long name.
            if s.len() > 512 {
                return None;
            }
        }
    };
    for i in 0..n as usize {
        let e = table + i * 8;
        let (Some(no), Some(vo)) = (le32(&buf, e), le32(&buf, e + 4)) else {
            continue;
        };
        if let (Some(k), Some(v)) = (utf16_at(no as usize), utf16_at(vo as usize)) {
            if !k.is_empty() {
                out.props.push((k, v));
            }
        }
    }
}

/// `SECi`: keep it only if the format says the bytes are a JPEG.
fn read_image<R: io::BufRead + io::Seek>(reader: &mut R, off: u32, len: u32, out: &mut X3fContents) {
    if len <= IMAGE_HEADER {
        return;
    }
    let mut head = [0u8; IMAGE_HEADER as usize];
    if reader.seek(SeekFrom::Start(u64::from(off))).is_err() || reader.read_exact(&mut head).is_err()
    {
        return;
    }
    if &head[..4] != SECI {
        return;
    }
    if le32(&head, 12) != Some(FORMAT_JPEG) {
        return;
    }
    let at = u64::from(off) + u64::from(IMAGE_HEADER);
    let size = len - IMAGE_HEADER;
    // Confirm rather than trust: the format word is the file's word about
    // its own bytes, and reporting an address without looking is how a
    // caller extracts something that is not an image.
    let mut lead = [0u8; 4];
    if reader.seek(SeekFrom::Start(at)).is_ok()
        && reader.read_exact(&mut lead).is_ok()
        && lead[..2] == [0xFF, 0xD8]
    {
        // The largest JPEG wins: bodies carry a small one too.
        if out.preview.is_none_or(|(_, prev)| size > prev) {
            out.preview = Some((at, size));
        }
        // `FF E1` straight after the SOI is an APP1, which on these files is
        // the Exif block. Checked rather than assumed, and the largest such
        // JPEG wins for the same reason.
        if lead[2..4] == [0xFF, 0xE1]
            && out.exif_jpeg.is_none_or(|(_, prev)| size > prev)
        {
            out.exif_jpeg = Some((at, size));
        }
    }
}

/// The smallest valid TIFF: a header and one empty IFD.
///
/// An X3F contains no TIFF, and the reader's parser needs one. Everything the
/// file states arrives through [`synthesize`] instead.
pub(crate) const EMPTY_TIFF: [u8; 14] = [
    b'I', b'I', 42, 0, 8, 0, 0, 0, // header: little-endian, magic, IFD at 8
    0, 0, // zero entries
    0, 0, 0, 0, // no next IFD
];

/// `PROP` pairs -> Exif fields.
///
/// **`TIME` is a Unix timestamp**, which is unusual and useful: a raw normally
/// states a local wall clock with nothing saying which zone it is. It is
/// rendered as the UTC wall clock so the repo's naive-UTC rule reproduces the
/// same instant, and that is what exiftool reports for these files too.
pub(crate) fn synthesize(props: &[(String, String)], frame: Option<(u32, u32)>)
                         -> Vec<crate::tiff::Field> {
    use crate::tag::Tag;
    use crate::tiff::{Field, In};
    use crate::value::{Rational, Value};

    let get = |k: &str| {
        props.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    };
    let mut out = Vec::new();
    let mut ascii = |tag: Tag, v: &str| {
        out.push(Field { tag, ifd_num: In::PRIMARY, value: Value::Ascii(vec![v.as_bytes().to_vec()]) });
    };
    if let Some(v) = get("CAMMANUF") {
        ascii(Tag::Make, v);
    }
    if let Some(v) = get("CAMMODEL") {
        ascii(Tag::Model, v);
    }
    if let Some(v) = get("CAMSERIAL") {
        ascii(Tag::BodySerialNumber, v);
    }
    if let Some(t) = get("TIME").and_then(|v| v.parse::<i64>().ok()) {
        if let Some(s) = utc_string(t) {
            ascii(Tag::DateTimeOriginal, &s);
        }
    }
    // A ratio close enough to the decimal to round-trip through a viewer.
    let mut rational = |tag: Tag, v: f64| {
        if v > 0.0 && v.is_finite() {
            let denom: u32 = 1_000_000;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss,
                    reason = "guarded finite and positive; product is bounded below")]
            let num = (v * f64::from(denom)).round() as u64;
            if let Ok(num) = u32::try_from(num) {
                out.push(Field {
                    tag,
                    ifd_num: In::PRIMARY,
                    value: Value::Rational(vec![Rational { num, denom }]),
                });
            }
        }
    };
    if let Some(v) = get("SHUTTER").and_then(|v| v.parse::<f64>().ok()) {
        rational(Tag::ExposureTime, v);
    }
    if let Some(v) = get("APERTURE").and_then(|v| v.parse::<f64>().ok()) {
        rational(Tag::FNumber, v);
    }
    if let Some((w, h)) = frame {
        // **A SUB-IMAGE, not the primary IFD.** Where a preview's Exif is
        // read too, its IFD0 describes a thumbnail, and a consumer reading
        // the ordinary tags would take that for the photograph. A sub-image
        // outranks it in the same precedence a RAF's CFA header and an MRW's
        // PRD block already rely on, and for the same reason.
        for (tag, v) in [(Tag::ImageWidth, w), (Tag::ImageLength, h)] {
            out.push(Field { tag, ifd_num: In::SUB_IMAGE, value: Value::Long(vec![v]) });
        }
    }
    if let Some(v) = get("ISO").and_then(|v| v.parse::<u32>().ok()) {
        out.push(Field {
            tag: Tag::PhotographicSensitivity,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![u16::try_from(v).unwrap_or(u16::MAX)]),
        });
    }
    out
}

/// A Unix timestamp as `"YYYY:MM:DD HH:MM:SS"` in UTC.
///
/// Shared with [`crate::crw`]: a CIFF `CaptureTime` is the same thing in the
/// same encoding, and two copies of a calendar are two chances to differ.
///
/// Days-to-calendar by Howard Hinnant's civil_from_days, which is exact and
/// needs no table; the crate has no date dependency and this is not a reason
/// to add one.
pub(crate) fn utc_string(secs: i64) -> Option<String> {
    if !(0..=253_402_300_799).contains(&secs) {
        return None;
    }
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Some(format!(
        "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
        y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three corpus bodies' own `TIME` values, against what exiftool
    /// reports for the same files — an SD9, an SD14 and a Polaroid x530.
    #[test]
    fn unix_time_becomes_the_utc_wall_clock() {
        assert_eq!(utc_string(1_050_151_205).as_deref(), Some("2003:04:12 12:40:05"));
        assert_eq!(utc_string(1_345_295_521).as_deref(), Some("2012:08:18 13:12:01"));
        assert_eq!(utc_string(1_161_358_478).as_deref(), Some("2006:10:20 15:34:38"));
        assert_eq!(utc_string(0).as_deref(), Some("1970:01:01 00:00:00"));
        // Outside the representable range is absence, not a wrapped date.
        assert_eq!(utc_string(-1), None);
        assert_eq!(utc_string(i64::MAX), None);
    }

    /// Only what the file actually states — a missing property must not
    /// become an empty tag that reads as "the camera said nothing here".
    #[test]
    fn absent_properties_produce_no_fields() {
        assert!(synthesize(&[], None).is_empty());
        let only_junk = vec![("NOTATAG".to_owned(), "x".to_owned()),
                             ("CAMMODEL".to_owned(), String::new())];
        assert!(synthesize(&only_junk, None).is_empty());
    }

    #[test]
    fn the_properties_become_the_expected_tags() {
        let props: Vec<(String, String)> = [
            ("CAMMANUF", "SIGMA"), ("CAMMODEL", "SIGMA SD14"),
            ("CAMSERIAL", "01005322"), ("TIME", "1345295521"),
            ("SHUTTER", "0.001791"), ("APERTURE", "6.44196"), ("ISO", "100"),
        ].iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        let got = synthesize(&props, None);
        let has = |t: crate::tag::Tag| got.iter().any(|f| f.tag == t);
        assert!(has(crate::tag::Tag::Make));
        assert!(has(crate::tag::Tag::Model));
        assert!(has(crate::tag::Tag::BodySerialNumber));
        assert!(has(crate::tag::Tag::DateTimeOriginal));
        assert!(has(crate::tag::Tag::ExposureTime));
        assert!(has(crate::tag::Tag::FNumber));
        assert!(has(crate::tag::Tag::PhotographicSensitivity));
        assert_eq!(got.len(), 7);
    }
}
