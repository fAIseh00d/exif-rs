//
// Copyright (c) 2020 KAMADA Ken'ichi.
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

use core::convert::{TryFrom, TryInto};
use std::io::{BufRead, ErrorKind, Seek, SeekFrom};

use crate::endian::{Endian, BigEndian};
use crate::error::Error;
use crate::util::{read64, BufReadExt as _, ReadExt as _};

// Checking "mif1" in the compatible brands should be enough, because
// the "heic", "heix", "heim", and "heis" files shall include "mif1"
// among the compatible brands [ISO23008-12 B.4.1] [ISO23008-12 B.4.3].
// Same for "msf1" [ISO23008-12 B.4.2] [ISO23008-12 B.4.4].
static HEIF_BRANDS: &[[u8; 4]] = &[*b"mif1", *b"msf1"];

// Keep upstream's raised ceiling (a recent mirrorless HEIF exceeded 65535);
// `pub(crate)` because the CRX reader below shares it.
pub(crate) const MAX_EXIF_SIZE: usize = 131070;

// Most errors in this file are Error::InvalidFormat.
impl From<&'static str> for Error {
    fn from(err: &'static str) -> Error {
        Error::InvalidFormat(err)
    }
}

pub fn get_exif_attr<R>(reader: &mut R) -> Result<Vec<u8>, Error>
where R: BufRead + Seek {
    let mut parser = Parser::new(reader);
    match parser.parse() {
        Err(Error::Io(ref e)) if e.kind() == ErrorKind::UnexpectedEof =>
            Err("Broken HEIF file".into()),
        Err(e) => Err(e),
        Ok(mut buf) => {
            if buf.len() < 4 {
                return Err("ExifDataBlock too small".into());
            }
            let offset = BigEndian::loadu32(&buf, 0) as usize;
            if buf.len() - 4 < offset {
                return Err("Invalid Exif header offset".into());
            }
            buf.drain(.. 4 + offset);
            Ok(buf)
        },
    }
}

#[derive(Debug)]
struct Parser<R> {
    reader: R,
    // Whether the file type box has been checked.
    ftyp_checked: bool,
    // The item where Exif data is stored.
    item_id: Option<u32>,
    // The location of the item_id.
    item_location: Option<Location>,
}

#[derive(Debug)]
struct Location {
    construction_method: u8,
    // index, offset, length
    extents: Vec<(u64, u64, u64)>,
    base_offset: u64,
}

impl<R> Parser<R> where R: BufRead + Seek {
    fn new(reader: R) -> Self {
        Self {
            reader: reader,
            ftyp_checked: false,
            item_id: None,
            item_location: None,
        }
    }

    fn parse(&mut self) -> Result<Vec<u8>, Error> {
        while let Some((size, boxtype)) = self.read_box_header()? {
            match &boxtype {
                b"ftyp" => {
                    let buf = self.read_file_level_box(size)?;
                    self.parse_ftyp(BoxSplitter::new(&buf))?;
                    self.ftyp_checked = true;
                },
                b"meta" => {
                    if !self.ftyp_checked {
                        return Err("MetaBox found before FileTypeBox".into());
                    }
                    let buf = self.read_file_level_box(size)?;
                    let exif = self.parse_meta(BoxSplitter::new(&buf))?;
                    return Ok(exif);
                },
                _ => self.skip_file_level_box(size)?,
            }
        }
        Err(Error::NotFound("HEIF"))
    }

    // Reads size, type, and largesize,
    // and returns body size and type.
    // If no byte can be read due to EOF, None is returned.
    fn read_box_header(&mut self) -> Result<Option<(u64, [u8; 4])>, Error> {
        if self.reader.is_eof()? {
            return Ok(None);
        }
        let mut buf = [0; 8];
        self.reader.read_exact(&mut buf)?;
        let size = match BigEndian::loadu32(&buf, 0) {
            0 => Some(u64::MAX),
            1 => read64(&mut self.reader)?.checked_sub(16),
            x => u64::from(x).checked_sub(8),
        }.ok_or("Invalid box size")?;
        let boxtype = buf[4..8].try_into().expect("never fails");
        Ok(Some((size, boxtype)))
    }

    fn read_file_level_box(&mut self, size: u64) -> Result<Vec<u8>, Error> {
        let mut buf;
        match size {
            u64::MAX => {
                buf = Vec::new();
                self.reader.read_to_end(&mut buf)?;
            },
            _ => {
                let size = size.try_into()
                    .or(Err("Box is larger than the address space"))?;
                buf = Vec::new();
                self.reader.read_exact_len(&mut buf, size)?;
            },
        }
        Ok(buf)
    }

    fn skip_file_level_box(&mut self, size: u64) -> Result<(), Error> {
        match size {
            u64::MAX => self.reader.seek(SeekFrom::End(0))?,
            _ => self.reader.seek(SeekFrom::Current(
                size.try_into().or(Err("Large seek not supported"))?))?,
        };
        Ok(())
    }

    fn parse_ftyp(&mut self, mut boxp: BoxSplitter) -> Result<(), Error> {
        let head = boxp.slice(8)?;
        let _major_brand = &head[0..4];
        let _minor_version = BigEndian::loadu32(head, 4);
        while let Ok(compat_brand) = boxp.array4() {
            if HEIF_BRANDS.contains(&compat_brand) {
                return Ok(());
            }
        }
        Err("No compatible brand recognized in ISO base media file".into())
    }

    fn parse_meta(&mut self, mut boxp: BoxSplitter) -> Result<Vec<u8>, Error> {
        let (version, _flags) = boxp.fullbox_header()?;
        if version != 0 {
            return Err("Unsupported MetaBox".into());
        }
        let mut idat = None;
        let mut iloc = None;
        while !boxp.is_empty() {
            let (boxtype, mut body) = boxp.child_box()?;
            match boxtype {
                b"idat" => idat = Some(body.slice(body.len())?),
                b"iinf" => self.parse_iinf(body)?,
                b"iloc" => iloc = Some(body),
                _ => {},
            }
        }

        self.item_id.ok_or(Error::NotFound("HEIF"))?;
        self.parse_iloc(iloc.ok_or("No ItemLocationBox")?)?;
        let location = self.item_location.as_ref()
            .ok_or("No matching item in ItemLocationBox")?;
        let mut buf = Vec::new();
        match location.construction_method {
            0 => {
                for &(_, off, len) in &location.extents {
                    let off = location.base_offset.checked_add(off)
                        .ok_or("Invalid offset")?;
                    // Seeking beyond the EOF is allowed and
                    // implementation-defined, but the subsequent read
                    // should fail.
                    self.reader.seek(SeekFrom::Start(off))?;
                    match len {
                        0 => { self.reader.read_to_end(&mut buf)?; },
                        _ => {
                            let len = len.try_into()
                                .or(Err("Extent too large"))?;
                            self.reader.read_exact_len(&mut buf, len)?;
                        },
                    }
                    if buf.len() > MAX_EXIF_SIZE {
                        return Err("Exif data too large".into());
                    }
                }
            },
            1 => {
                let idat = idat.ok_or("No ItemDataBox")?;
                for &(_, off, len) in &location.extents {
                    let off = location.base_offset.checked_add(off)
                        .ok_or("Invalid offset")?;
                    let end = off.checked_add(len).ok_or("Invalid length")?;
                    let off = off.try_into().or(Err("Offset too large"))?;
                    let end = end.try_into().or(Err("Length too large"))?;
                    buf.extend_from_slice(match len {
                        0 => idat.get(off..),
                        _ => idat.get(off..end),
                    }.ok_or("Out of ItemDataBox")?);
                    if buf.len() > MAX_EXIF_SIZE {
                        return Err("Exif data too large".into());
                    }
                }
            },
            2 => return Err(Error::NotSupported(
                "Construction by item offset is not supported")),
            _ => return Err("Invalid construction_method".into()),
        }
        Ok(buf)
    }

    fn parse_iloc(&mut self, mut boxp: BoxSplitter) -> Result<(), Error> {
        let (version, _flags) = boxp.fullbox_header()?;
        let tmp = boxp.uint16().map(usize::from)?;
        let (offset_size, length_size, base_offset_size) =
            (tmp >> 12, tmp >> 8 & 0xf, tmp >> 4 & 0xf);
        let index_size = match version { 1 | 2 => tmp & 0xf, _ => 0 };
        let item_count = match version {
            0 | 1 => boxp.uint16()?.into(),
            2 => boxp.uint32()?,
            _ => return Err("Unsupported ItemLocationBox".into()),
        };
        for _ in 0..item_count {
            let item_id = match version {
                0 | 1 => boxp.uint16()?.into(),
                2 => boxp.uint32()?,
                _ => unreachable!(),
            };
            let construction_method = match version {
                0 => 0,
                1 | 2 => boxp.slice(2).map(|x| x[1] & 0xf)?,
                _ => unreachable!(),
            };
            let data_ref_index = boxp.uint16()?;
            if construction_method == 0 && data_ref_index != 0 {
                return Err(Error::NotSupported(
                    "External data reference is not supported"));
            }
            let base_offset = boxp.size048(base_offset_size)?
                .ok_or("Invalid base_offset_size")?;
            let extent_count = boxp.uint16()?.into();
            if self.item_id == Some(item_id) {
                let mut extents = Vec::with_capacity(extent_count);
                for _ in 0..extent_count {
                    let index = boxp.size048(index_size)?
                        .ok_or("Invalid index_size")?;
                    let offset = boxp.size048(offset_size)?
                        .ok_or("Invalid offset_size")?;
                    let length = boxp.size048(length_size)?
                        .ok_or("Invalid length_size")?;
                    extents.push((index, offset, length));
                }
                self.item_location = Some(Location {
                    construction_method, extents, base_offset });
            } else {
                // (15 + 15 + 15) * u16::MAX never overflows.
                boxp.slice((index_size + offset_size + length_size) *
                           extent_count)?;
            }
        }
        Ok(())
    }

    fn parse_iinf(&mut self, mut boxp: BoxSplitter) -> Result<(), Error> {
        let (version, _flags) = boxp.fullbox_header()?;
        let entry_count = match version {
            0 => boxp.uint16()?.into(),
            _ => boxp.uint32()?,
        };
        for _ in 0..entry_count {
            let (boxtype, body) = boxp.child_box()?;
            if boxtype == b"infe" {
                self.parse_infe(body)?;
            }
        }
        Ok(())
    }

    fn parse_infe(&mut self, mut boxp: BoxSplitter) -> Result<(), Error> {
        let (version, _flags) = boxp.fullbox_header()?;
        let item_id = match version {
            2 => boxp.uint16()?.into(),
            3 => boxp.uint32()?,
            _ => return Err("Unsupported ItemInfoEntry".into()),
        };
        let _item_protection_index = boxp.slice(2)?;
        let item_type = boxp.slice(4)?;
        if item_type == b"Exif" {
            self.item_id = Some(item_id);
        }
        Ok(())
    }
}

pub fn is_heif(buf: &[u8]) -> bool {
    let mut boxp = BoxSplitter::new(buf);
    while let Ok((boxtype, mut body)) = boxp.child_box() {
        if boxtype == b"ftyp" {
            let Ok(_major_brand_minor_version) = body.slice(8) else {
                return false;
            };
            while let Ok(compat_brand) = body.array4() {
                if HEIF_BRANDS.contains(&compat_brand) {
                    return true;
                }
            }
            return false;
        }
    }
    false
}

pub(crate) struct BoxSplitter<'a> {
    inner: &'a [u8],
}

impl<'a> BoxSplitter<'a> {
    pub(crate) fn new(slice: &'a [u8]) -> BoxSplitter<'a> {
        Self { inner: slice }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.inner.len()
    }

    // Returns type and body.
    pub(crate) fn child_box(&mut self) -> Result<(&'a [u8], BoxSplitter<'a>), Error> {
        let size = self.uint32()? as usize;
        let boxtype = self.slice(4)?;
        let body_len = match size {
            0 => Some(self.len()),
            1 => usize::try_from(self.uint64()?)
                .or(Err("Box is larger than the address space"))?
                .checked_sub(16),
            _ => size.checked_sub(8),
        }.ok_or("Invalid box size")?;
        let body = self.slice(body_len)?;
        Ok((boxtype, BoxSplitter::new(body)))
    }

    // Returns 0-, 4-, or 8-byte unsigned integer.
    pub(crate) fn size048(&mut self, size: usize) -> Result<Option<u64>, Error> {
        match size {
            0 => Ok(Some(0)),
            4 => self.uint32().map(u64::from).map(Some),
            8 => self.uint64().map(Some),
            _ => Ok(None),
        }
    }

    // Returns version and flags.
    pub(crate) fn fullbox_header(&mut self) -> Result<(u32, u32), Error> {
        let tmp = self.uint32()?;
        Ok((tmp >> 24, tmp & 0xffffff))
    }

    pub(crate) fn uint16(&mut self) -> Result<u16, Error> {
        self.slice(2).map(|num| BigEndian::loadu16(num, 0))
    }

    pub(crate) fn uint32(&mut self) -> Result<u32, Error> {
        self.slice(4).map(|num| BigEndian::loadu32(num, 0))
    }

    pub(crate) fn uint64(&mut self) -> Result<u64, Error> {
        self.slice(8).map(|num| BigEndian::loadu64(num, 0))
    }

    pub(crate) fn array4(&mut self) -> Result<[u8; 4], Error> {
        self.slice(4).map(|x| x.try_into().expect("never fails"))
    }

    pub(crate) fn slice(&mut self, at: usize) -> Result<&'a [u8], Error> {
        let slice = self.inner.get(..at).ok_or("Box too small")?;
        self.inner = &self.inner[at..];
        Ok(slice)
    }
}

pub mod crx {
    use std::io::{BufRead, ErrorKind, Seek, SeekFrom};

    use super::BoxSplitter;

    use crate::endian::{BigEndian, Endian};
    use crate::error::Error;
    use crate::util::{read64, BufReadExt as _, ReadExt as _};

    // Canon CR3 uses isobmff container format
    static CANON_FORMATS: &[[u8; 4]] = &[*b"crx "];
    static CANON_UUID:&[u8; 16] = &[0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];

    /// Where a CR3 keeps its embedded JPEGs, as `(file offset, length)`.
    ///
    /// Canon stores three images and none is addressed by a TIFF tag, so
    /// nothing in the Exif can find them:
    ///
    /// * **`THMB`** — a 160x120 thumbnail, ~9.7 KB;
    /// * **`PRVW`** — a 1620x1080 preview, ~332 KB;
    /// * a full-size JPEG carried as a **track sample** in `mdat` — 2.4 MB
    ///   and 6000x4000 on an R6 Mark II, against PRVW's 1620x1080 — found
    ///   through the track's sample tables.
    ///
    /// Both boxes carry a 16-byte header before the JPEG, and their layouts
    /// differ — `PRVW` states width and height where `THMB` states a length —
    /// so neither is parsed. **The size comes from the box**: `box - 8 - 16`,
    /// which matches exiftool exactly on both (332512 - 24 = 332488;
    /// 9712 - 24 = 9688). Dimensions come from the JPEG's own frame header,
    /// which describes the image actually present.
    ///
    /// The walk descends only into CONTAINER boxes. Recursing generally would
    /// mean walking `mdat`, which is the entire 30 MB raw.

/// The JPEG inside a `PRVW`/`THMB` box: its offset and length.
///
/// Canon puts it 24 bytes in -- an 8-byte box header plus 16 of their own --
/// and the length that follows from the box size matches exiftool exactly
/// (332512 - 24 = 332488; 9712 - 24 = 9688). Confirmed across 341 corpus CR3s
/// from 11 bodies, from a PowerShot SX70 to an R5 Mark II.
///
/// **The two bytes there are still read.** It is a layout this code does not
/// parse, and reporting an offset without looking at it is how a caller ends
/// up extracting 300 KB of Canon's header and calling it a preview -- silently,
/// because nothing downstream can tell. A box whose JPEG is not where the
/// layout says is skipped rather than guessed at: searching for an `SOI`
/// instead would be a code path no file has ever taken, which is its own risk.
fn jpeg_in_box<R: BufRead + Seek>(reader: &mut R, at: u64, size: u64) -> Option<(u64, u32)> {
    /// The 8-byte box header plus Canon's own 16.
    const JPEG_AT: u64 = 24;

    if size <= JPEG_AT {
        return None;
    }
    let mut soi = [0u8; 2];
    reader.seek(SeekFrom::Start(at + JPEG_AT)).ok()?;
    reader.read_exact(&mut soi).ok()?;
    if soi != [0xFF, 0xD8] {
        return None;
    }
    u32::try_from(size - JPEG_AT).ok().map(|len| (at + JPEG_AT, len))
}

    pub fn preview_boxes<R>(reader: &mut R) -> Result<Vec<(u64, u32)>, Error>
    where
        R: BufRead + Seek,
    {
        const MAX_DEPTH: u32 = 4;

        fn walk<R: BufRead + Seek>(
            reader: &mut R, start: u64, end: u64, depth: u32, out: &mut Vec<(u64, u32)>,
        ) -> Result<(), Error> {
            if depth > MAX_DEPTH {
                return Ok(());
            }
            let mut at = start;
            while at + 8 <= end {
                reader.seek(SeekFrom::Start(at))?;
                let mut head = [0u8; 8];
                if reader.read_exact(&mut head).is_err() {
                    return Ok(());
                }
                let size = u64::from(BigEndian::loadu32(&head, 0));
                let boxtype: [u8; 4] = head[4..8].try_into().expect("never fails");
                // A size of 0 means "to the end"; 1 means a 64-bit size
                // follows. Neither is expected around these boxes, and
                // guessing would risk a runaway walk.
                if size < 8 || at + size > end {
                    return Ok(());
                }
                match &boxtype {
                    b"PRVW" | b"THMB" => {
                        // **Verified, not assumed.** 24 is where these two
                        // bodies put their JPEG on every corpus CR3, and it
                        // matches exiftool -- but it is a layout we do not
                        // parse, so it is checked rather than trusted. If the
                        // SOI is not there the box is searched for one, and a
                        // box with no JPEG in it is skipped instead of
                        // reported as an image that is not there.
                        if let Some((ofs, len)) = jpeg_in_box(reader, at, size) {
                            out.push((ofs, len));
                        }
                    }
                    b"uuid" => {
                        // A `uuid` box carries a 16-byte identifier, and what
                        // follows depends on WHICH uuid. Canon's preview
                        // container puts 8 bytes of its own ahead of its
                        // children; the metadata one does not. Getting this
                        // wrong lands mid-box and finds nothing, which is how
                        // PRVW stayed hidden while THMB was found.
                        const CANON_PREVIEW_UUID: [u8; 16] = [
                            0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88,
                            0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16,
                        ];
                        let mut id = [0u8; 16];
                        reader.seek(SeekFrom::Start(at + 8))?;
                        let extra = if reader.read_exact(&mut id).is_ok()
                            && id == CANON_PREVIEW_UUID { 8 } else { 0 };
                        walk(reader, at + 8 + 16 + extra, at + size, depth + 1, out)?;
                    }
                    b"moov" | b"udta" | b"trak" | b"mdia" | b"minf" => {
                        walk(reader, at + 8, at + size, depth + 1, out)?;
                    }
                    // A track's sample sits where its chunk-offset table says,
                    // with the size its sample-size table gives. Canon carries
                    // the full-size JPEG this way, so it is reachable by no
                    // tag and by no box name -- only by the tables.
                    b"stbl" => {
                        if let Some(img) = sample_of(reader, at + 8, at + size)? {
                            out.push(img);
                        }
                    }
                    _ => {}
                }
                at += size;
            }
            Ok(())
        }

        /// The single sample a `stbl` describes, when it is a JPEG.
        ///
        /// Only a ONE-sample track is read. A track with several samples is a
        /// sequence rather than an embedded still, and `stsc` would have to be
        /// honoured to place any but the first. Canon writes one sample per
        /// track.
        ///
        /// **Whether it is a JPEG is decided by looking**, not by the sample
        /// description: every CR3 track declares `CRAW` in its `stsd`,
        /// including the one holding an ordinary JPEG, so the format code
        /// cannot distinguish them. The SOI can.
        fn sample_of<R: BufRead + Seek>(
            reader: &mut R, start: u64, end: u64,
        ) -> Result<Option<(u64, u32)>, Error> {
            let (mut offset, mut size) = (None, None);
            let mut at = start;
            while at + 8 <= end {
                reader.seek(SeekFrom::Start(at))?;
                let mut head = [0u8; 8];
                if reader.read_exact(&mut head).is_err() {
                    break;
                }
                let boxsize = u64::from(BigEndian::loadu32(&head, 0));
                if boxsize < 8 || at + boxsize > end {
                    break;
                }
                let mut body = vec![0u8; (boxsize as usize - 8).min(64)];
                if reader.read_exact(&mut body).is_ok() {
                    match &head[4..8] {
                        // version/flags, sample_size, sample_count, [entries]
                        b"stsz" if body.len() >= 16 => {
                            let stated = BigEndian::loadu32(&body, 4);
                            let count = BigEndian::loadu32(&body, 8);
                            if count == 1 {
                                size = Some(if stated == 0 {
                                    BigEndian::loadu32(&body, 12)
                                } else {
                                    stated
                                });
                            }
                        }
                        // version/flags, entry_count, [64-bit entries]
                        b"co64" if body.len() >= 16
                            && BigEndian::loadu32(&body, 4) == 1 =>
                        {
                            offset = Some(BigEndian::loadu64(&body, 8));
                        }
                        b"stco" if body.len() >= 12
                            && BigEndian::loadu32(&body, 4) == 1 =>
                        {
                            offset = Some(u64::from(BigEndian::loadu32(&body, 8)));
                        }
                        _ => {}
                    }
                }
                at += boxsize;
            }
            let (Some(offset), Some(size)) = (offset, size) else {
                return Ok(None);
            };
            if size == 0 {
                return Ok(None);
            }
            reader.seek(SeekFrom::Start(offset))?;
            let mut soi = [0u8; 2];
            if reader.read_exact(&mut soi).is_err() || soi != [0xFF, 0xD8] {
                return Ok(None);
            }
            Ok(Some((offset, size)))
        }

        let end = reader.seek(SeekFrom::End(0))?;
        let mut out = Vec::new();
        walk(reader, 0, end, 0, &mut out)?;
        Ok(out)
    }

    #[allow(unused)]
    pub fn get_exif_attr<R>(reader: &mut R) -> Result<Vec<u8>, Error>
    where
        R: BufRead + Seek,
    {
        Ok(get_exif_attr_vec(reader)?.first().cloned().unwrap_or_else(|| Vec::new()))
    }

    pub fn get_exif_attr_vec<R>(reader: &mut R) -> Result<Vec<Vec<u8>>, Error>
    where
        R: BufRead + Seek,
    {
        let mut parser = Parser::new(reader);
        match parser.parse() {
            Err(Error::Io(ref e)) if e.kind() == ErrorKind::UnexpectedEof => Err("Broken CR3 file".into()),
            Err(e) => Err(e),
            // We can only parse the first segment
            Ok(buf) => Ok(buf)
        }
    }

    #[derive(Debug)]
    struct Parser<R> {
        reader: R,
        // Whether the file type box has been checked.
        ftyp_checked: bool,
    }

    impl<R> Parser<R>
    where
        R: BufRead + Seek
    {
        fn new(reader: R) -> Self { Self { reader, ftyp_checked: false } }

        /// Extracts the Exif attributes from raw Exif data for CR3 the result are 4 non-contiguous buffer segments
        /// If an error occurred, `exif::Error` is returned.
        fn parse(&mut self) -> Result<Vec<Vec<u8>>, Error> {
            while let Some((size, boxtype)) = self.read_box_header()? {
                match &boxtype {
                    b"ftyp" => {
                        let buf = self.read_file_level_box(size)?;
                        self.parse_ftyp(BoxSplitter::new(&buf))?;
                        self.ftyp_checked = true;
                    }
                    b"moov" => {
                        if !self.ftyp_checked {
                            return Err("moov found before FileTypeBox".into());
                        }
                        let buf = self.read_file_level_box(size)?;
                        let mut out_buf = Vec::new();
                        self.parse_moov(&mut out_buf, BoxSplitter::new(&buf))?;
                        if !out_buf.is_empty() {
                            return Ok(out_buf);
                        }
                    }
                    _ => {
                        self.skip_file_level_box(size)?;
                    }
                }
            }
            Err(Error::NotFound("CR3"))
        }

        fn parse_moov(&mut self, out_data: &mut Vec<Vec<u8>>, mut split: BoxSplitter) -> Result<(), Error> {
            //let indent = indent + 1;
            while let Ok((boxtype, mut boxbody)) = split.child_box() {
                let size = boxbody.len();
                match boxtype {
                    b"uuid" => {
                        if boxbody.slice(16)? == CANON_UUID {
                            self.parse_moov(out_data, boxbody)?;
                        }
                    }
                    b"CMT1" | b"CMT2" | b"CMT3" | b"CMT4" => {
                        out_data.push(Vec::from(boxbody.slice(size)?));
                    }
                    _ => {
                        boxbody.slice(size)?;
                    }
                }
            }
            Ok(())
        }

        // Reads size, type, and largesize,
        // and returns body size and type.
        // If no byte can be read due to EOF, None is returned.
        fn read_box_header(&mut self) -> Result<Option<(u64, [u8; 4])>, Error> {
            if self.reader.is_eof()? {
                return Ok(None);
            }
            let mut buf = [0; 8];
            self.reader.read_exact(&mut buf)?;
            let size = match BigEndian::loadu32(&buf, 0) {
                0 => Some(std::u64::MAX),
                1 => read64(&mut self.reader)?.checked_sub(16),
                x => u64::from(x).checked_sub(8),
            }
                .ok_or("Invalid box size")?;
            let boxtype = buf[4..8].try_into().expect("never fails");
            Ok(Some((size, boxtype)))
        }

        fn read_file_level_box(&mut self, size: u64) -> Result<Vec<u8>, Error> {
            let mut buf;
            match size {
                std::u64::MAX => {
                    buf = Vec::new();
                    self.reader.read_to_end(&mut buf)?;
                }
                _ => {
                    let size = size.try_into().or(Err("Box is larger than the address space"))?;
                    buf = Vec::new();
                    self.reader.read_exact_len(&mut buf, size)?;
                }
            }
            Ok(buf)
        }

        fn skip_file_level_box(&mut self, size: u64) -> Result<(), Error> {
            match size {
                std::u64::MAX => self.reader.seek(SeekFrom::End(0))?,
                _ => self.reader.seek(SeekFrom::Current(size.try_into().or(Err("Large seek not supported"))?))?,
            };
            Ok(())
        }

        fn parse_ftyp(&mut self, mut boxp: BoxSplitter) -> Result<(), Error> {
            let head = boxp.slice(8)?;
            let _major_brand = &head[0..4];
            let _minor_version = BigEndian::loadu32(&head, 4);
            while let Ok(compat_brand) = boxp.array4() {
                if CANON_FORMATS.contains(&compat_brand) {
                    return Ok(());
                }
            }
            Err("No compatible brand recognized in ISO base media file".into())
        }
    }

    pub fn is_crx(buf: &[u8]) -> bool {
        let mut boxp = BoxSplitter::new(buf);
        while let Ok((boxtype, mut body)) = boxp.child_box() {
            if boxtype == b"ftyp" {
                let _major_brand_minor_version = if body.slice(8).is_err() {
                    return false;
                };
                while let Ok(compat_brand) = body.array4() {
                    if CANON_FORMATS.contains(&compat_brand) {
                        return true;
                    }
                }
                return false;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use super::*;

    #[test]
    fn extract() {
        let file = std::fs::File::open("tests/exif.heic").unwrap();
        let buf = get_exif_attr(
            &mut std::io::BufReader::new(&file)).unwrap();
        assert_eq!(buf.len(), 79);
        assert!(buf.starts_with(b"MM\x00\x2a"));
        assert!(buf.ends_with(b"xif\0"));
    }

    #[test]
    fn unknown_before_ftyp() {
        let data =
            b"\0\0\0\x09XXXXx\
              \0\0\0\x14ftypmif1\0\0\0\0mif1\
              \0\0\0\x57meta\0\0\0\0\
                  \0\0\0\x18iloc\x01\0\0\0\0\0\0\x01\x1e\x1d\0\x01\0\0\0\x01\
                  \0\0\0\x22iinf\0\0\0\0\0\x01\
                      \0\0\0\x14infe\x02\0\0\0\x1e\x1d\0\0Exif\
                  \0\0\0\x11idat\0\0\0\x01xabcd";
        assert!(is_heif(data));
        let exif = get_exif_attr(&mut Cursor::new(&data[..])).unwrap();
        assert_eq!(exif, b"abcd");
    }

    #[test]
    fn bad_exif_data_block() {
        let data =
            b"\0\0\0\x14ftypmif1\0\0\0\0mif1\
              \0\0\0\x52meta\0\0\0\0\
                  \0\0\0\x18iloc\x01\0\0\0\0\0\0\x01\x1e\x1d\0\x01\0\0\0\x01\
                  \0\0\0\x22iinf\0\0\0\0\0\x01\
                      \0\0\0\x14infe\x02\0\0\0\x1e\x1d\0\0Exif\
                  \0\0\0\x0cidat\0\0\0\x01";
        assert_err_pat!(get_exif_attr(&mut Cursor::new(&data[..])),
                        Error::InvalidFormat("Invalid Exif header offset"));

        let data =
            b"\0\0\0\x14ftypmif1\0\0\0\0mif1\
              \0\0\0\x51meta\0\0\0\0\
                  \0\0\0\x18iloc\x01\0\0\0\0\0\0\x01\x1e\x1d\0\x01\0\0\0\x01\
                  \0\0\0\x22iinf\0\0\0\0\0\x01\
                      \0\0\0\x14infe\x02\0\0\0\x1e\x1d\0\0Exif\
                  \0\0\0\x0bidat\0\0\0";
        assert_err_pat!(get_exif_attr(&mut Cursor::new(&data[..])),
                        Error::InvalidFormat("ExifDataBlock too small"));
    }

    #[test]
    fn parser_box_header() {
        // size
        let mut p = Parser::new(Cursor::new(b"\0\0\0\x08abcd"));
        assert_eq!(p.read_box_header().unwrap(), Some((0, *b"abcd")));
        let mut p = Parser::new(Cursor::new(b"\0\0\0\x08abc"));
        assert_err_pat!(p.read_box_header(), Error::Io(_));
        let mut p = Parser::new(Cursor::new(b"\0\0\0\x07abcd"));
        assert_err_pat!(p.read_box_header(), Error::InvalidFormat(_));
        // max size
        let mut p = Parser::new(Cursor::new(b"\xff\xff\xff\xffabcd"));
        assert_eq!(p.read_box_header().unwrap(),
                   Some((0xffffffff - 8, *b"abcd")));
        // to the end of the file
        let mut p = Parser::new(Cursor::new(b"\0\0\0\0abcd"));
        assert_eq!(p.read_box_header().unwrap(),
                   Some((u64::MAX, *b"abcd")));
        // largesize
        let mut p = Parser::new(Cursor::new(
            b"\0\0\0\x01abcd\0\0\0\0\0\0\0\x10"));
        assert_eq!(p.read_box_header().unwrap(), Some((0, *b"abcd")));
        let mut p = Parser::new(Cursor::new(
            b"\0\0\0\x01abcd\0\0\0\0\0\0\0"));
        assert_err_pat!(p.read_box_header(), Error::Io(_));
        let mut p = Parser::new(Cursor::new(
            b"\0\0\0\x01abcd\0\0\0\0\0\0\0\x0f"));
        assert_err_pat!(p.read_box_header(), Error::InvalidFormat(_));
        // max largesize
        let mut p = Parser::new(Cursor::new(
            b"\0\0\0\x01abcd\xff\xff\xff\xff\xff\xff\xff\xff"));
        assert_eq!(p.read_box_header().unwrap(),
                   Some((u64::MAX.wrapping_sub(16), *b"abcd")));
    }

    #[test]
    fn is_heif_test() {
        // HEIF (with any coding format)
        assert!(is_heif(b"\0\0\0\x14ftypmif1\0\0\0\0mif1"));
        // HEIC
        assert!(is_heif(b"\0\0\0\x18ftypheic\0\0\0\0heicmif1"));
        // HEIC image sequence
        assert!(is_heif(b"\0\0\0\x18ftyphevc\0\0\0\0msf1hevc"));
        // unknown major brand but compatible with HEIF
        assert!(is_heif(b"\0\0\0\x18ftypXXXX\0\0\0\0XXXXmif1"));
        // incomplete brand (OK to ignore?)
        assert!(is_heif(b"\0\0\0\x15ftypmif1\0\0\0\0mif1h"));
        assert!(is_heif(b"\0\0\0\x16ftypmif1\0\0\0\0mif1he"));
        assert!(is_heif(b"\0\0\0\x17ftypmif1\0\0\0\0mif1hei"));
        // ISO base media file but not a HEIF
        assert!(!is_heif(b"\0\0\0\x14ftypmp41\0\0\0\0mp41"));
        // missing compatible brands (what should we do?)
        assert!(!is_heif(b"\0\0\0\x10ftypmif1\0\0\0\0"));
        // truncated box
        let mut data: &[u8] = b"\0\0\0\x14ftypmif1\0\0\0\0mif1";
        while let Some((_, rest)) = data.split_last() {
            data = rest;
            assert!(!is_heif(data));
        }
        // short box size
        assert!(!is_heif(b"\0\0\0\x13ftypmif1\0\0\0\0mif1"));
    }

    #[test]
    fn box_splitter() {
        let buf = b"0123456789abcdef";
        let mut boxp = BoxSplitter::new(buf);
        assert_err_pat!(boxp.slice(17), Error::InvalidFormat(_));
        assert_eq!(boxp.slice(16).unwrap(), buf);
        assert_err_pat!(boxp.slice(std::usize::MAX), Error::InvalidFormat(_));

        let mut boxp = BoxSplitter::new(buf);
        assert_eq!(boxp.slice(1).unwrap(), b"0");
        assert_eq!(boxp.uint16().unwrap(), 0x3132);
        assert_eq!(boxp.uint32().unwrap(), 0x33343536);
        assert_eq!(boxp.uint64().unwrap(), 0x3738396162636465);
    }
}
