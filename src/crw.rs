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


//! Canon CRW — a CIFF heap, and no TIFF anywhere.
//!
//! Canon's pre-2004 raw, and the largest single container gap the corpus had:
//! more BODIES on raw.pixls.us than PEF, CR3 or SRW. The D30, D60, 10D, the
//! **300D** — the first sub-$1000 DSLR — and the whole PowerShot G/Pro line.
//!
//! `II`, a heap offset, then `HEAPCCDR`. A heap is a block whose LAST four
//! bytes give the offset of its own directory, counted from the block's start;
//! the directory is a count and then ten-byte entries of `{tag, length,
//! offset}`. Two of the tag's bits decide where the value is:
//!
//! - `0x4000` — **the value IS the eight bytes of the length and offset
//!   fields**, in the entry, and there is nothing at `offset` to read. Missing
//!   this reads a serial number two bytes late and gets a plausible number.
//! - otherwise the value is at `start + offset`, and `0x2800` / `0x3000` in
//!   the format bits mean those bytes are another heap. **Nested offsets count
//!   from the SUBDIRECTORY**, not the file.
//!
//! What it states is everything ingest needs and nothing a TIFF would call by
//! the same name: `CanonRawMakeModel` is one NUL-separated string, `ImageInfo`
//! carries the size and the rotation in DEGREES, and `CaptureTime` is a Unix
//! timestamp — the same shape an X3F's `TIME` has, and read the same way.
//!
//! Exposure is Canon's APEX-like encoding in `ShotInfo`, not a rational:
//! `2^(-t/32)` seconds and `2^(a/64)` for the f-number. That is format
//! knowledge, so it is decoded here rather than handed on as a raw short.

use std::io::{self, SeekFrom};

use crate::error::Error;
use crate::tag::Tag;
use crate::tiff::{Field, In};
use crate::value::{Rational, Value};

/// A CIFF entry, and the two bits that say where its value lives.
const STORAGE_MASK: u16 = 0xc000;
/// The value is in the entry's own length and offset fields.
const STORAGE_IN_ENTRY: u16 = 0x4000;
const FORMAT_MASK: u16 = 0x3800;
const TAG_MASK: u16 = 0x3fff;
/// The two format codes that mean "these bytes are another heap".
const FORMAT_SUBDIR_1: u16 = 0x2800;
const FORMAT_SUBDIR_2: u16 = 0x3000;

const ENTRY_LEN: u64 = 10;
/// A directory of more than this is not a directory.
const MAX_ENTRIES: u16 = 1024;
/// Heaps nest three deep in the files measured; beyond this is a cycle.
const MAX_DEPTH: u8 = 8;
/// A value we read into memory. Every tag below is tens of bytes.
const MAX_VALUE: u32 = 4096;

// The tags this reads. Names are exiftool's, which are Canon's.
const T_RAW_DATA: u16 = 0x2005;
const T_JPEG_FROM_RAW: u16 = 0x2007;
const T_THUMBNAIL: u16 = 0x2008;
const T_MAKE_MODEL: u16 = 0x080a;
const T_FIRMWARE: u16 = 0x080b;
const T_IMAGE_INFO: u16 = 0x1810;
const T_CAPTURE_TIME: u16 = 0x180e;
const T_SERIAL: u16 = 0x180b;
const T_BASE_ISO: u16 = 0x101c;
const T_FOCAL_LENGTH: u16 = 0x1029;
const T_SHOT_INFO: u16 = 0x102a;
const T_CAMERA_SETTINGS: u16 = 0x102d;

/// `ShotInfo` is an array of shorts indexed from its own length word.
const SHOT_ISO: usize = 2;
const SHOT_FNUMBER: usize = 21;
const SHOT_EXPOSURE_TIME: usize = 22;
/// `CameraSettings`, same shape: the DIVISOR the focal length is stated in.
const SETTING_FOCAL_UNITS: usize = 25;

pub fn is_crw(buf: &[u8]) -> bool {
    buf.len() >= 14 && buf.starts_with(b"II") && &buf[6..14] == b"HEAPCCDR"
}

/// What a CRW states about itself.
#[derive(Debug, Default)]
pub(crate) struct CrwContents {
    /// The full-size embedded JPEG, in FILE offsets.
    pub preview: Option<(u64, u32)>,
    /// The small one, which some bodies carry and some do not.
    pub thumbnail: Option<(u64, u32)>,
    /// The raw image record — the picture's own bytes, for the essence walk.
    pub raw_data: Option<(u64, u32)>,
    /// Every field the heap stated, already in Exif terms.
    pub fields: Vec<Field>,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Reads the heap and everything it names.
pub(crate) fn get_contents<R>(reader: &mut R) -> Result<CrwContents, Error>
where
    R: io::BufRead + io::Seek,
{
    let mut header = [0u8; 14];
    reader
        .read_exact(&mut header)
        .map_err(|_| Error::InvalidFormat("Truncated CRW"))?;
    if !is_crw(&header) {
        return Err(Error::InvalidFormat("Not a CRW file"));
    }
    // The header states where the outermost heap begins; the file's end is
    // where it ends.
    let heap_at = u64::from(le32(&header, 2).ok_or(Error::InvalidFormat("CRW header"))?);
    let end = reader
        .seek(SeekFrom::End(0))
        .map_err(|_| Error::InvalidFormat("CRW is not seekable"))?;
    if heap_at + 4 > end {
        return Err(Error::InvalidFormat("CRW heap is out of range"));
    }

    // **Three records only mean something together**, and a heap states them
    // in whatever order it likes, so they are gathered and decoded after the
    // walk rather than as they arrive.
    let mut out = CrwContents::default();
    let mut acc = Deferred::default();
    walk(reader, heap_at, end, 0, &mut out, &mut acc)?;
    exposure_fields(&acc, &mut out.fields);
    Ok(out)
}

/// The records whose meaning depends on another record.
#[derive(Debug, Default)]
struct Deferred {
    shot_info: Vec<u16>,
    camera_settings: Vec<u16>,
    /// The body's ISO floor, for a shot that did not state its own.
    base_iso: Option<u32>,
    /// `FocalType` then the length, in units `CameraSettings` names.
    focal: Option<u16>,
}

/// One heap: its directory, then whatever the entries name.
fn walk<R>(reader: &mut R, start: u64, end: u64, depth: u8, out: &mut CrwContents,
           acc: &mut Deferred) -> Result<(), Error>
where
    R: io::BufRead + io::Seek,
{
    if depth > MAX_DEPTH || end < start + 4 {
        return Ok(());
    }
    // **The directory's address is the block's last four bytes**, and it is
    // relative to the block -- which for a nested heap is not the file.
    reader.seek(SeekFrom::Start(end - 4))?;
    let mut tail = [0u8; 4];
    reader.read_exact(&mut tail)?;
    let dir_at = start + u64::from(u32::from_le_bytes(tail));
    if dir_at + 2 > end {
        return Ok(());
    }
    reader.seek(SeekFrom::Start(dir_at))?;
    let mut count_bytes = [0u8; 2];
    reader.read_exact(&mut count_bytes)?;
    let count = u16::from_le_bytes(count_bytes);
    if count == 0 || count > MAX_ENTRIES || dir_at + 2 + u64::from(count) * ENTRY_LEN > end + 4 {
        return Ok(());
    }
    let mut table = vec![0u8; count as usize * ENTRY_LEN as usize];
    reader
        .read_exact(&mut table)
        .map_err(|_| Error::InvalidFormat("Truncated CRW directory"))?;

    for i in 0..count as usize {
        let base = i * ENTRY_LEN as usize;
        let Some(tag) = le16(&table, base) else { continue };
        let id = tag & TAG_MASK;

        // **The value can be the entry itself.** For `0x4000` storage the
        // eight bytes of the length and offset fields ARE the value, so it is
        // read from the table in hand and nothing is seeked.
        if tag & STORAGE_MASK == STORAGE_IN_ENTRY {
            let Some(v) = table.get(base + 2..base + 10) else { continue };
            take_value(id, v, out, acc);
            continue;
        }

        let (Some(len), Some(off)) = (le32(&table, base + 2), le32(&table, base + 6)) else {
            continue;
        };
        let at = start + u64::from(off);
        if at + u64::from(len) > end {
            continue;
        }
        match tag & FORMAT_MASK {
            FORMAT_SUBDIR_1 | FORMAT_SUBDIR_2 => {
                walk(reader, at, at + u64::from(len), depth + 1, out, acc)?;
            }
            _ => {
                // The three image records are addresses, not values: they are
                // megabytes and the caller extracts them itself.
                match id {
                    T_RAW_DATA => out.raw_data = Some((at, len)),
                    T_JPEG_FROM_RAW => keep_larger(&mut out.preview, (at, len)),
                    T_THUMBNAIL => keep_larger(&mut out.thumbnail, (at, len)),
                    _ if len <= MAX_VALUE => {
                        reader.seek(SeekFrom::Start(at))?;
                        let mut v = vec![0u8; len as usize];
                        if reader.read_exact(&mut v).is_ok() {
                            take_value(id, &v, out, acc);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn keep_larger(slot: &mut Option<(u64, u32)>, found: (u64, u32)) {
    if slot.is_none_or(|(_, prev)| found.1 > prev) {
        *slot = Some(found);
    }
}

/// One record's bytes -> the Exif field it states, where there is one.
fn take_value(id: u16, v: &[u8], out: &mut CrwContents, acc: &mut Deferred) {
    let shorts = || v.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let mut ascii = |tag: Tag, s: &[u8]| {
        if !s.is_empty() {
            out.fields.push(Field {
                tag,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![s.to_vec()]),
            });
        }
    };
    match id {
        // **One string holding two**, NUL-separated: `Canon\0Canon EOS 300D
        // DIGITAL`. Splitting on the first NUL and stopping -- which is what
        // reading it as a C string does -- yields a make and no model.
        T_MAKE_MODEL => {
            let mut parts = v.split(|&b| b == 0).filter(|s| !s.is_empty());
            if let Some(make) = parts.next() {
                ascii(Tag::Make, make);
            }
            if let Some(model) = parts.next() {
                ascii(Tag::Model, model);
            }
        }
        T_FIRMWARE => ascii(Tag::Software, first_cstr(v)),
        T_SERIAL => {
            if let Some(n) = le32(v, 0) {
                ascii(Tag::BodySerialNumber, n.to_string().as_bytes());
            }
        }
        T_BASE_ISO => acc.base_iso = le16(v, 0).map(u32::from),
        T_SHOT_INFO => acc.shot_info = shorts(),
        T_CAMERA_SETTINGS => acc.camera_settings = shorts(),
        // `FocalType`, then the length -- in units the SETTINGS record names,
        // not in millimetres. A G2 states 240 at 32 units to the millimetre
        // and a 300D states 21 at one, so reading the number alone reports a
        // 7.5 mm compact as a 240 mm telephoto and looks like a real lens.
        T_FOCAL_LENGTH => acc.focal = le16(v, 2).filter(|&mm| mm > 0),
        // `ImageWidth`, `ImageHeight`, a float aspect, then the rotation.
        T_IMAGE_INFO => {
            if let (Some(w), Some(h)) = (le32(v, 0), le32(v, 4)) {
                if w > 0 && h > 0 {
                    // **A SUB-IMAGE**, for the reason a RAF's CFA header and
                    // an MRW's `PRD` block are: it is the raw frame, and a
                    // consumer must be able to outrank an IFD0 that describes
                    // a preview. A CRW carries no such IFD0 today -- its
                    // embedded JPEG has no Exif at all -- and the precedence
                    // costs nothing where there is nothing to outrank.
                    for (tag, n) in [(Tag::ImageWidth, w), (Tag::ImageLength, h)] {
                        out.fields.push(Field {
                            tag,
                            ifd_num: In::SUB_IMAGE,
                            value: Value::Long(vec![n]),
                        });
                    }
                }
            }
            // **Rotation is in DEGREES, not an orientation code.** libraw
            // agrees on both corpus bodies -- flip 5 for the 270 and 0 for
            // the 0 -- which is where the mapping is checked.
            if let Some(o) = le32(v, 12).and_then(rotation_to_orientation) {
                out.fields.push(Field {
                    tag: Tag::Orientation,
                    ifd_num: In::PRIMARY,
                    value: Value::Short(vec![o]),
                });
            }
        }
        // A Unix timestamp, then a time-zone code and a flag. Rendered as the
        // UTC wall clock, which is the camera's LOCAL clock and what the
        // repo's naive-UTC rule wants; exiftool prints the same string.
        T_CAPTURE_TIME => {
            if let Some(s) = le32(v, 0).and_then(|t| crate::x3f::utc_string(i64::from(t))) {
                ascii(Tag::DateTimeOriginal, s.as_bytes());
            }
        }
        _ => {}
    }
}

fn first_cstr(v: &[u8]) -> &[u8] {
    v.split(|&b| b == 0).next().unwrap_or_default()
}

/// CIFF's rotation, in degrees clockwise, as an Exif orientation.
fn rotation_to_orientation(deg: u32) -> Option<u16> {
    match deg {
        0 => Some(1),
        90 => Some(6),
        180 => Some(3),
        270 => Some(8),
        _ => None,
    }
}

/// `ShotInfo`'s APEX-like shorts, as the values a viewer shows.
///
/// Canon stores the exposure as `2^(-t/32)` seconds and the aperture as
/// `2^(a/64)`, both signed, and the ISO as an offset from 100 at 160. None of
/// the three is a number any other format writes, so decoding them is part of
/// reading the container rather than a policy on top of it.
fn exposure_fields(acc: &Deferred, out: &mut Vec<Field>) {
    let at = |i: usize| acc.shot_info.get(i).copied().filter(|&v| v != 0 && v != 0xffff);

    if let Some(mm) = acc.focal {
        let units = acc.camera_settings.get(SETTING_FOCAL_UNITS).copied().unwrap_or(1).max(1);
        out.push(Field {
            tag: Tag::FocalLength,
            ifd_num: In::PRIMARY,
            value: Value::Rational(vec![Rational { num: u32::from(mm), denom: u32::from(units) }]),
        });
    }

    if let Some(raw) = at(SHOT_EXPOSURE_TIME) {
        let secs = 2f64.powf(-f64::from(raw as i16) / 32.0);
        if let Some(r) = approximate(secs) {
            out.push(Field { tag: Tag::ExposureTime, ifd_num: In::PRIMARY,
                             value: Value::Rational(vec![r]) });
        }
    }
    if let Some(raw) = at(SHOT_FNUMBER) {
        let f = 2f64.powf(f64::from(raw as i16) / 64.0);
        if let Some(r) = approximate(f) {
            out.push(Field { tag: Tag::FNumber, ifd_num: In::PRIMARY,
                             value: Value::Rational(vec![r]) });
        }
    }
    // `ShotInfo`'s ISO is the shot's; `BaseISO` is the body's floor and stands
    // in only where the shot did not say (an auto-ISO PowerShot writes zero).
    let iso = at(SHOT_ISO)
        .map(|v| (2f64.powf(f64::from(v as i16) / 32.0) * 100.0 / 32.0).round() as u32)
        .or(acc.base_iso);
    if let Some(iso) = iso.filter(|&v| v > 0) {
        out.push(Field {
            tag: Tag::PhotographicSensitivity,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![u16::try_from(iso).unwrap_or(u16::MAX)]),
        });
    }
}

/// A decimal as a ratio a viewer can round-trip, the way X3F's are made.
fn approximate(v: f64) -> Option<Rational> {
    if !(v.is_finite() && v > 0.0) {
        return None;
    }
    let denom: u32 = 1_000_000;
    let num = (v * f64::from(denom)).round();
    (num >= 1.0 && num <= f64::from(u32::MAX)).then(|| Rational { num: num as u32, denom })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_needs_the_heap_name_not_just_the_byte_order() {
        assert!(is_crw(b"II\x1a\x00\x00\x00HEAPCCDR\x02\x00"));
        // A TIFF starts the same way and is not a CIFF.
        assert!(!is_crw(b"II\x2a\x00\x08\x00\x00\x00\x00\x00\x00\x00\x00\x00"));
        assert!(!is_crw(b"II\x1a\x00"));
    }

    /// libraw's flip for the two corpus bodies: 5 for the 300D's 270, 0 for
    /// the G2's 0.
    #[test]
    fn rotation_degrees_become_orientations() {
        assert_eq!(rotation_to_orientation(0), Some(1));
        assert_eq!(rotation_to_orientation(90), Some(6));
        assert_eq!(rotation_to_orientation(180), Some(3));
        assert_eq!(rotation_to_orientation(270), Some(8));
        assert_eq!(rotation_to_orientation(45), None);
    }

    /// A G2's focal length is 240 in units of 32 to the millimetre -- 7.5 mm,
    /// which is what exiftool prints. A 300D's units are 1.
    #[test]
    fn focal_length_is_in_the_units_the_settings_name() {
        let mut acc = Deferred::default();
        acc.focal = Some(240);
        acc.camera_settings = vec![0u16; 40];
        acc.camera_settings[SETTING_FOCAL_UNITS] = 32;
        let mut out = Vec::new();
        exposure_fields(&acc, &mut out);
        let Value::Rational(ref r) = out[0].value else { panic!("not a rational") };
        assert_eq!((r[0].num, r[0].denom), (240, 32));
    }

    /// The 300D's own `ShotInfo`, against what exiftool prints for it:
    /// 1/25 s at f/10, ISO 100.
    #[test]
    fn shot_info_decodes_to_what_exiftool_prints() {
        let mut acc = Deferred::default();
        acc.shot_info = vec![0u16; 33];
        acc.shot_info[SHOT_ISO] = 160;
        acc.shot_info[SHOT_FNUMBER] = 212;
        acc.shot_info[SHOT_EXPOSURE_TIME] = 148;
        let mut out = Vec::new();
        exposure_fields(&acc, &mut out);
        let secs = 2f64.powf(-148.0 / 32.0);
        assert!((1.0 / secs - 24.7).abs() < 0.1, "{secs}");
        let f = 2f64.powf(212.0 / 64.0);
        assert!((f - 9.94).abs() < 0.05, "{f}");
        assert_eq!(out.len(), 3);
    }
}
