//
// Copyright (c) 2016 KAMADA Ken'ichi.
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

use std::fmt;
use mutate_once::MutOnce;

use crate::endian::{Endian, BigEndian, LittleEndian};
use crate::error::Error;
use crate::tag::{Context, Tag, UnitPiece};
use crate::util::{atou16, ctou32};
use crate::value;
use crate::value::Value;
use crate::value::get_type_info;

// TIFF header magic numbers [EXIF23 4.5.2].
pub(crate) const TIFF_BE: u16 = 0x4d4d;
pub(crate) const TIFF_LE: u16 = 0x4949;
pub(crate) const TIFF_FORTY_TWO: u16 = 0x002a;
pub const TIFF_BE_SIG: [u8; 4] = [0x4d, 0x4d, 0x00, 0x2a];
pub const TIFF_LE_SIG: [u8; 4] = [0x49, 0x49, 0x2a, 0x00];

// Raw dialects: TIFF files whose VERSION WORD is a vendor signature rather
// than 42. The byte order mark, the IFD chain and every entry are ordinary
// TIFF — only the third and fourth bytes differ — so rejecting them on the
// version alone refuses a file this parser can otherwise read completely.
//
// Both vendors write little-endian exclusively, so there is no big-endian
// spelling to accept; a `MM`-flagged file carrying one of these words is not
// something either has been observed to produce.

/// Olympus ORF, the common signature (`IIRO`).
pub(crate) const ORF_RO: u16 = 0x4f52;
/// Olympus ORF as written by later and high-resolution bodies (`IIRS`).
pub(crate) const ORF_RS: u16 = 0x5352;
/// Panasonic RW2/RAW (`IIU\0`) — version word 85.
pub(crate) const RW2_EIGHTY_FIVE: u16 = 0x0055;

/// How many `SubIFDs` pointers are followed. A DNG has one and a NEF two;
/// the cap is a bound on a list the file controls, not a limit anyone meets.
const MAX_SUB_IMAGES: usize = 8;

/// The largest RW2 IFD0 tag number worth capturing (`0x0017`, ISO).
const RW2_TAG_MAX: usize = 0x17;
/// Panasonic's IFD0 tags. A RW2 carries NO `ImageWidth`/`PixelXDimension` and
/// no `PhotographicSensitivity`; these are where the same facts live.
const RW2_SENSOR_TOP: usize = 4;
const RW2_SENSOR_LEFT: usize = 5;
const RW2_SENSOR_BOTTOM: usize = 6;
const RW2_SENSOR_RIGHT: usize = 7;
const RW2_ISO: usize = 0x17;

pub const ORF_RO_SIG: [u8; 4] = [0x49, 0x49, 0x52, 0x4f];
pub const ORF_RS_SIG: [u8; 4] = [0x49, 0x49, 0x52, 0x53];
pub const RW2_LE_SIG: [u8; 4] = [0x49, 0x49, 0x55, 0x00];

// Partially parsed TIFF field (IFD entry).
// Value::Unknown is abused to represent a partially parsed value.
// Such a value must never be exposed to the users of this library.
#[derive(Debug)]
pub struct IfdEntry {
    // When partially parsed, the value is stored as Value::Unknown.
    // Do not leak this field to the outside.
    field: MutOnce<Field>,
}

impl IfdEntry {
    /// Creates a new IfdEntry from a Field.
    /// This is mainly used internally for MakerNote and MPF parsing.
    #[cfg(feature = "make_note")]
    pub(crate) fn from_field(field: Field) -> Self {
        IfdEntry {
            field: MutOnce::from(field),
        }
    }

    /// An entry whose value is ALREADY a real value, not a `Value::Unknown`
    /// waiting for its bytes — a field synthesised from vendor tags rather
    /// than read from an IFD slot.
    ///
    /// It has to be marked fixed on the way in. `into_field` parses any entry
    /// that is not, and `parse_value` PANICS on a value that is already parsed
    /// — so an unfixed synthetic entry takes the whole read down at the point
    /// the caller asks for its fields, far from where it was created.
    pub(crate) fn from_parsed_field(field: Field) -> Self {
        let entry = IfdEntry { field: MutOnce::from(field) };
        let _fix = entry.field.get_ref();
        entry
    }

    pub fn ifd_num_tag(&self) -> (In, Tag) {
        if self.field.is_fixed() {
            let field = self.field.get_ref();
            (field.ifd_num, field.tag)
        } else {
            let field = self.field.get_mut();
            (field.ifd_num, field.tag)
        }
    }

    pub fn ref_field<'a>(&'a self, data: &[u8], le: bool) -> &'a Field {
        self.parse(data, le);
        self.field.get_ref()
    }

    pub(crate) fn into_field(self, data: &[u8], le: bool) -> Field {
        self.parse(data, le);
        self.field.into_inner()
    }

    // Generic parsing for MakerNote entries without tag-specific handling
    #[cfg(feature = "make_note")]
    pub(crate) fn into_field_generic(self, data: &[u8], le: bool) -> Field {
        self.parse_generic(data, le);
        self.field.into_inner()
    }

    fn parse(&self, data: &[u8], le: bool) {
        if !self.field.is_fixed() {
            let mut field = self.field.get_mut();
            if le {
                Self::parse_value::<LittleEndian>(&mut field.value, data);
            } else {
                Self::parse_value::<BigEndian>(&mut field.value, data);
            }
        }
    }

    // Generic parsing without tag-specific handling (for MakerNote)
    #[cfg(feature = "make_note")]
    fn parse_generic(&self, data: &[u8], le: bool) {
        if !self.field.is_fixed() {
            let mut field = self.field.get_mut();
            if le {
                Self::parse_value::<LittleEndian>(&mut field.value, data);
            } else {
                Self::parse_value::<BigEndian>(&mut field.value, data);
            }
        }
    }

    // Converts a partially parsed value into a real one.
    pub(crate) fn parse_value<E>(value: &mut Value, data: &[u8]) where E: Endian {
        match *value {
            Value::Unknown(typ, cnt, ofs) => {
                let (unitlen, parser) = get_type_info::<E>(typ);
                if unitlen != 0 {
                    *value = parser(data, ofs as usize, cnt as usize);
                }
            },
            _ => panic!("value is already parsed"),
        }
    }
}

/// A TIFF/Exif field.
#[derive(Debug, Clone)]
pub struct Field {
    /// The tag of this field.
    pub tag: Tag,
    /// The index of the IFD to which this field belongs.
    pub ifd_num: In,
    /// The value of this field.
    pub value: Value,
}

/// An IFD number.
///
/// The IFDs are indexed from 0.  The 0th IFD is for the primary image
/// and the 1st one is for the thumbnail.  Two associated constants,
/// `In::PRIMARY` and `In::THUMBNAIL`, are defined for them respectively.
///
/// # Examples
/// ```
/// use exif::In;
/// assert_eq!(In::PRIMARY.index(), 0);
/// assert_eq!(In::THUMBNAIL.index(), 1);
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct In(pub u16);

impl In {
    pub const PRIMARY: In = In(0);
    pub const THUMBNAIL: In = In(1);
    #[cfg(feature = "mpf")]
    pub const MPF: In = In(2);

    /// The first sub-image IFD (`SubIFDs`, tag 0x014A); the next is
    /// `In(SUB_IMAGE.0 + 1)` and so on.
    ///
    /// **Deliberately above every other allocation.** The chained IFDs take
    /// 0..=7 (the chain is capped at 8), and `MPF` squats on 2 — so numbering
    /// sub-images from 0 upward would collide with both. 16 leaves the chain
    /// its whole range and stays clear of `MPF`.
    ///
    /// A DNG's IFD0 is a THUMBNAIL by specification and its real images are
    /// here, which is why `get_field(Tag::ImageWidth, In::PRIMARY)` on a DNG
    /// answers something like 256x171 and is not wrong to do so. For the
    /// dimensions a converter would output, read `DefaultCropSize` from the
    /// sub-image where the format defines it (DNG), or the vendor's own crop
    /// tags from the MakerNote. **Which of those wins is the CONSUMER's
    /// decision** — it differs per vendor and per generation — so this crate
    /// exposes them and picks none.
    pub const SUB_IMAGE: In = In(16);

    /// Returns the IFD number.
    #[inline]
    pub fn index(self) -> u16 {
        self.0
    }
}

impl fmt::Display for In {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            0 => f.pad("primary"),
            1 => f.pad("thumbnail"),
            n => f.pad(&format!("IFD{}", n)),
        }
    }
}

/// Parse the Exif attributes in the TIFF format.
///
/// Returns a Vec of Exif fields and a bool.
/// The boolean value is true if the data is little endian.
/// If an error occurred, `exif::Error` is returned.
pub fn parse_exif(data: &[u8]) -> Result<(Vec<Field>, bool), Error> {
    let mut parser = Parser::new();
    parser.parse(data)?;
    let (entries, le) = (parser.entries, parser.little_endian);
    Ok((entries.into_iter().map(|e| e.into_field(data, le)).collect(), le))
}

#[derive(Debug)]
pub struct Parser {
    pub entries: Vec<IfdEntry>,
    pub little_endian: bool,
    // `Some<Vec>` to enable the option and `None` to disable it.
    pub continue_on_error: Option<Vec<Error>>,
    // Panasonic RW2: the header's version word said 85. See `synthesize_rw2`.
    rw2: bool,
    // RW2 IFD0 SHORTs captured on the way past, keyed by tag number.
    rw2_tags: [Option<u16>; RW2_TAG_MAX + 1],
}

impl Parser {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            little_endian: false,
            continue_on_error: None,
            rw2: false,
            rw2_tags: [None; RW2_TAG_MAX + 1],
        }
    }

    pub fn parse(&mut self, data: &[u8]) -> Result<(), Error> {
        self.parse_with_context_offset(data, Context::Tiff, 0)
    }

    pub fn parse_with_context_offset(&mut self, data: &[u8], default_context: Context, base_offset: u32) -> Result<(), Error> {
        // Check the byte order and call the real parser.
        if data.len() < 8 {
            return Err(Error::InvalidFormat("Truncated TIFF header"));
        }
        match BigEndian::loadu16(data, 0) {
            TIFF_BE => {
                self.little_endian = false;
                self.parse_header::<BigEndian>(data, default_context, base_offset)
            },
            TIFF_LE => {
                self.little_endian = true;
                self.parse_header::<LittleEndian>(data, default_context, base_offset)
            },
            _ => Err(Error::InvalidFormat("Invalid TIFF byte order")),
        }
    }

    fn parse_header<E>(&mut self, data: &[u8], ctx: Context, base_offset: u32)
                       -> Result<(), Error> where E: Endian {
        // Parse the rest of the header (42 — or a raw dialect's stand-in for
        // it — and the IFD offset).
        match E::loadu16(data, 2) {
            TIFF_FORTY_TWO | ORF_RO | ORF_RS => {},
            RW2_EIGHTY_FIVE => self.rw2 = true,
            _ => return Err(Error::InvalidFormat("Invalid forty two")),
        }
        let ifd_offset = E::loadu32(data, 4) as usize;
        let r = self.parse_body::<E>(data, ifd_offset, ctx, base_offset)
            .or_else(|e| self.check_error(e));
        if self.rw2 {
            self.synthesize_rw2();
        }
        r
    }

    fn parse_body<E>(&mut self, data: &[u8], mut ifd_offset: usize,
                     ctx: Context, base_offset: u32)
                     -> Result<(), Error> where E: Endian {
        let mut ifd_num_ck = Some(0);
        while ifd_offset != 0 {
            let ifd_num = ifd_num_ck
                .ok_or(Error::InvalidFormat("Too many IFDs"))?;
            // Limit the number of IFDs to defend against resource exhaustion
            // attacks.
            if ifd_num >= 8 {
                return Err(Error::InvalidFormat("Limit the IFD count to 8"));
            }
            ifd_offset = self.parse_ifd::<E>(
                data, ifd_offset, ctx, ifd_num, base_offset)?;
            ifd_num_ck = ifd_num.checked_add(1);
        }
        Ok(())
    }

    // Parse IFD [EXIF23 4.6.2].
    fn parse_ifd<E>(&mut self, data: &[u8],
                    mut offset: usize, ctx: Context, ifd_num: u16,
                    base_offset: u32)
                    -> Result<usize, Error> where E: Endian {
        // Count (the number of the entries).
        if data.len() < offset || data.len() - offset < 2 {
            return Err(Error::InvalidFormat("Truncated IFD count"));
        }
        let count = E::loadu16(data, offset) as usize;
        offset += 2;

        // Array of entries.
        for _ in 0..count {
            if data.len() - offset < 12 {
                return Err(Error::InvalidFormat("Truncated IFD"));
            }
            let entry = Self::parse_ifd_entry::<E>(data, offset);
            offset += 12;
            let (tag, raw_val) = match entry {
                Ok(x) => x,
                Err(e) => {
                    self.check_error(e)?;
                    continue;
                },
            };

            // A value offset is relative to the TIFF header, and that header
            // is not always at the start of `data` — a CR3 hands us the whole
            // file with the `CMTn` box's TIFF somewhere inside it. Shifting
            // here rather than slicing keeps one buffer for the whole parse.
            let val = match raw_val {
                Value::Unknown(t, l, o) => Value::Unknown(t, l, o + base_offset),
                _ => raw_val,
            };
            // Panasonic's sensor geometry and ISO are plain IFD0 SHORTs under
            // vendor tag numbers, so they are captured here — where `data` and
            // the endianness are both in hand — and turned into the standard
            // tags afterwards by `synthesize_rw2`.
            if self.rw2 && ctx == Context::Tiff && ifd_num == 0
                && (tag as usize) <= RW2_TAG_MAX {
                if let Value::Unknown(3, 1, ofs) = val {
                    let ofs = ofs as usize;
                    if data.len() >= ofs + 2 {
                        self.rw2_tags[tag as usize] = Some(E::loadu16(data, ofs));
                    }
                }
            }

            // No infinite recursion will occur because the context is not
            // recursively defined.
            let tag = Tag(ctx, tag);
            // Sub-images are a LIST of pointers, not one, and they are
            // ordinary TIFF IFDs rather than a different context — so they
            // get IFD NUMBERS of their own instead of a child context.
            if tag == Tag::SubIFDs && ifd_num == 0 {
                let mut ptr = val;
                IfdEntry::parse_value::<E>(&mut ptr, data);
                // The same defence the IFD chain has: a pointer list is
                // attacker-controlled, so it is bounded rather than trusted.
                for i in 0..MAX_SUB_IMAGES {
                    let Some(ofs) = ptr.get_uint(i) else { break };
                    let sub = In::SUB_IMAGE.0 + u16::try_from(i).unwrap_or(0);
                    self.parse_ifd::<E>(data, ofs as usize, Context::Tiff, sub,
                                        base_offset)
                        .map(|_next| ())
                        .or_else(|e| self.check_error(e))?;
                }
                continue;
            }

            let child_ctx = match tag {
                Tag::ExifIFDPointer => Context::Exif,
                Tag::GPSInfoIFDPointer => Context::Gps,
                Tag::InteropIFDPointer => Context::Interop,
                _ => {
                    self.entries.push(IfdEntry { field: Field {
                        tag: tag, ifd_num: In(ifd_num), value: val }.into()});
                    continue;
                },
            };
            self.parse_child_ifd::<E>(data, val, child_ctx, ifd_num, base_offset)
                .or_else(|e| self.check_error(e))?;
        }

        // Offset to the next IFD.
        if data.len() - offset < 4 {
            return Err(Error::InvalidFormat("Truncated next IFD offset"));
        }
        let next_ifd_offset = E::loadu32(data, offset);
        Ok(next_ifd_offset as usize)
    }

    fn parse_ifd_entry<E>(data: &[u8], offset: usize)
                          -> Result<(u16, Value), Error> where E: Endian {
        // The size of entry has been checked in parse_ifd().
        let tag = E::loadu16(data, offset);
        let typ = E::loadu16(data, offset + 2);
        let cnt = E::loadu32(data, offset + 4);
        let valofs_at = offset + 8;
        let (unitlen, _parser) = get_type_info::<E>(typ);
        let vallen = unitlen.checked_mul(cnt as usize).ok_or(
            Error::InvalidFormat("Invalid entry count"))?;
        let val = if vallen <= 4 {
            Value::Unknown(typ, cnt, valofs_at as u32)
        } else {
            let ofs = E::loadu32(data, valofs_at) as usize;
            if data.len() < ofs || data.len() - ofs < vallen {
                return Err(Error::InvalidFormat("Truncated field value"));
            }
            Value::Unknown(typ, cnt, ofs as u32)
        };
        Ok((tag, val))
    }

    fn parse_child_ifd<E>(&mut self, data: &[u8],
                          mut pointer: Value, ctx: Context, ifd_num: u16, base_offset: u32)
                          -> Result<(), Error> where E: Endian {
        // The pointer is not yet parsed, so do it here.
        IfdEntry::parse_value::<E>(&mut pointer, data);

        // A pointer field has type == LONG and count == 1, so the
        // value (IFD offset) must be embedded in the "value offset"
        // element of the field.
        let ofs = pointer.get_uint(0).ok_or(
            Error::InvalidFormat("Invalid pointer"))? as usize;
        match self.parse_ifd::<E>(data, ofs, ctx, ifd_num, base_offset)? {
            0 => Ok(()),
            _ => Err(Error::InvalidFormat("Unexpected next IFD")),
        }
    }

    /// Turn Panasonic's IFD0 vendor tags into the standard ones.
    ///
    /// **A RW2 carries no `ImageWidth`, no `PixelXDimension` and no
    /// `PhotographicSensitivity`** — measured across five bodies (FZ45, GX7,
    /// GH5, G9, S5), every one of them opens and parses and then answers
    /// nothing at all for size or ISO. The facts are there under vendor tag
    /// numbers, so a consumer either grows Panasonic-specific knowledge or
    /// keeps a second raw library around for one manufacturer.
    ///
    /// * **Size** is the active area: `SensorRightBorder - SensorLeftBorder`
    ///   by `SensorBottomBorder - SensorTopBorder` (tags 4-7). Measured
    ///   against libraw on those five bodies it lands within **0.23-0.47 %** —
    ///   e.g. 6000x4000 against libraw's 6024x4016 on a DC-S5. It is not
    ///   libraw's number and does not try to be: libraw reports its own
    ///   post-crop geometry. `SensorWidth`/`SensorHeight` (tags 2-3) are the
    ///   whole readout INCLUDING the masked border, and drift up to 2.5 %.
    /// * **ISO** is tag `0x0017`, which matched exiftool exactly on all five
    ///   (160, 200, 400, 100, 125).
    ///
    /// Written as `In::PRIMARY` entries so an ordinary `get_field` finds them,
    /// and **only when the header said RW2**. Tags 2-7 are unassigned in
    /// baseline TIFF, so nothing standard is being overwritten — but they are
    /// free for any other dialect to use for anything, so the version word
    /// gates it rather than the numbers being "probably safe".
    ///
    /// Naming these as Panasonic tags instead was considered and does not
    /// work: this crate names a tag by `(Context, number)` with no room for a
    /// dialect, so `Tag(Tiff, 4)` would be labelled `SensorTopBorder` in every
    /// TIFF ever read. Deriving the standard tags, gated, is the more honest
    /// of the two options actually available.
    fn synthesize_rw2(&mut self) {
        let t = |i: usize| self.rw2_tags.get(i).copied().flatten().map(u32::from);
        let mut add = |tag: Tag, v: u32| {
            self.entries.push(IfdEntry::from_parsed_field(Field {
                tag, ifd_num: In::PRIMARY, value: Value::Long(vec![v]),
            }));
        };
        if let (Some(l), Some(r)) = (t(RW2_SENSOR_LEFT), t(RW2_SENSOR_RIGHT)) {
            if r > l {
                add(Tag::ImageWidth, r - l);
            }
        }
        if let (Some(top), Some(b)) = (t(RW2_SENSOR_TOP), t(RW2_SENSOR_BOTTOM)) {
            if b > top {
                add(Tag::ImageLength, b - top);
            }
        }
        if let Some(iso) = t(RW2_ISO) {
            if iso > 0 {
                add(Tag::PhotographicSensitivity, iso);
            }
        }
    }

    fn check_error(&mut self, err: Error) -> Result<(), Error> {
        match self.continue_on_error {
            Some(ref mut v) => Ok(v.push(err)),
            None => Err(err),
        }
    }
}

pub fn is_tiff(buf: &[u8]) -> bool {
    buf.starts_with(&TIFF_BE_SIG) || buf.starts_with(&TIFF_LE_SIG) ||
        buf.starts_with(&ORF_RO_SIG) || buf.starts_with(&ORF_RS_SIG) ||
        buf.starts_with(&RW2_LE_SIG)
}

/// A struct used to parse a DateTime field.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use exif::DateTime;
/// let dt = DateTime::from_ascii(b"2016:05:04 03:02:01")?;
/// assert_eq!(dt.year, 2016);
/// assert_eq!(dt.to_string(), "2016-05-04 03:02:01");
/// # Ok(()) }
/// ```
#[derive(Debug)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    /// The subsecond data in nanoseconds.  If the Exif attribute has
    /// more sigfinicant digits, they are rounded down.
    pub nanosecond: Option<u32>,
    /// The offset of the time zone in minutes.
    pub offset: Option<i16>,
}

impl DateTime {
    /// Parse an ASCII data of a DateTime field.  The range of a number
    /// is not validated, so, for example, 13 may be returned as the month.
    ///
    /// If the value is blank, `Error::BlankValue` is returned.
    pub fn from_ascii(data: &[u8]) -> Result<DateTime, Error> {
        if data == b"    :  :     :  :  " || data == b"                   " {
            return Err(Error::BlankValue("DateTime is blank"));
        } else if data.len() < 19 {
            return Err(Error::InvalidFormat("DateTime too short"));
        } else if !(data[4] == b':' && data[7] == b':' && data[10] == b' ' &&
                    data[13] == b':' && data[16] == b':') {
            return Err(Error::InvalidFormat("Invalid DateTime delimiter"));
        }
        Ok(DateTime {
            year: atou16(&data[0..4])?,
            month: atou16(&data[5..7])? as u8,
            day: atou16(&data[8..10])? as u8,
            hour: atou16(&data[11..13])? as u8,
            minute: atou16(&data[14..16])? as u8,
            second: atou16(&data[17..19])? as u8,
            nanosecond: None,
            offset: None,
        })
    }

    /// Parses an SubsecTime-like field.
    pub fn parse_subsec(&mut self, data: &[u8]) -> Result<(), Error> {
        let mut subsec = 0;
        let mut ndigits = 0;
        for &c in data {
            if c == b' ' {
                break;
            }
            subsec = subsec * 10 + ctou32(c)?;
            ndigits += 1;
            if ndigits >= 9 {
                break;
            }
        }
        if ndigits == 0 {
            self.nanosecond = None;
        } else {
            for _ in ndigits..9 {
                subsec *= 10;
            }
            self.nanosecond = Some(subsec);
        }
        Ok(())
    }

    /// Parses an OffsetTime-like field.
    pub fn parse_offset(&mut self, data: &[u8]) -> Result<(), Error> {
        if data == b"   :  " || data == b"      " {
            return Err(Error::BlankValue("OffsetTime is blank"));
        } else if data.len() < 6 {
            return Err(Error::InvalidFormat("OffsetTime too short"));
        } else if data[3] != b':' {
            return Err(Error::InvalidFormat("Invalid OffsetTime delimiter"));
        }
        let hour = atou16(&data[1..3])?;
        let min = atou16(&data[4..6])?;
        let offset = (hour * 60 + min) as i16;
        self.offset = Some(match data[0] {
            b'+' => offset,
            b'-' => -offset,
            _ => return Err(Error::InvalidFormat("Invalid OffsetTime sign")),
        });
        Ok(())
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
               self.year, self.month, self.day,
               self.hour, self.minute, self.second)
    }
}

impl Field {
    /// Returns an object that implements `std::fmt::Display` for
    /// printing the value of this field in a tag-specific format.
    ///
    /// To print the value with the unit, call `with_unit` method on the
    /// returned object.  It takes a parameter, which is either `()`,
    /// `&Field`, or `&Exif`, that provides the unit information.
    /// If the unit does not depend on another field, `()` can be used.
    /// Otherwise, `&Field` or `&Exif` should be used.
    ///
    /// # Examples
    ///
    /// ```
    /// use exif::{Field, In, Tag, Value};
    ///
    /// let xres = Field {
    ///     tag: Tag::XResolution,
    ///     ifd_num: In::PRIMARY,
    ///     value: Value::Rational(vec![(72, 1).into()]),
    /// };
    /// let resunit = Field {
    ///     tag: Tag::ResolutionUnit,
    ///     ifd_num: In::PRIMARY,
    ///     value: Value::Short(vec![3]),
    /// };
    /// assert_eq!(xres.display_value().to_string(), "72");
    /// assert_eq!(resunit.display_value().to_string(), "cm");
    /// // The unit of XResolution is indicated by ResolutionUnit.
    /// assert_eq!(xres.display_value().with_unit(&resunit).to_string(),
    ///            "72 pixels per cm");
    /// // If ResolutionUnit is not given, the default value is used.
    /// assert_eq!(xres.display_value().with_unit(()).to_string(),
    ///            "72 pixels per inch");
    /// assert_eq!(xres.display_value().with_unit(&xres).to_string(),
    ///            "72 pixels per inch");
    ///
    /// let flen = Field {
    ///     tag: Tag::FocalLengthIn35mmFilm,
    ///     ifd_num: In::PRIMARY,
    ///     value: Value::Short(vec![24]),
    /// };
    /// // The unit of the focal length is always mm, so the argument
    /// // has nothing to do with the result.
    /// assert_eq!(flen.display_value().with_unit(()).to_string(),
    ///            "24 mm");
    /// assert_eq!(flen.display_value().with_unit(&resunit).to_string(),
    ///            "24 mm");
    /// ```
    #[inline]
    pub fn display_value(&self) -> DisplayValue<'_> {
        DisplayValue {
            tag: self.tag,
            ifd_num: self.ifd_num,
            value_display: self.value.display_as(self.tag),
        }
    }
}

/// Helper struct for printing a value in a tag-specific format.
pub struct DisplayValue<'a> {
    pub(crate) tag: Tag,
    pub(crate) ifd_num: In,
    pub(crate) value_display: value::Display<'a>,
}

impl<'a> DisplayValue<'a> {
    #[inline]
    pub fn with_unit<T>(&self, unit_provider: T)
                        -> DisplayValueUnit<'a, T> where T: ProvideUnit<'a> {
        DisplayValueUnit {
            ifd_num: self.ifd_num,
            value_display: self.value_display,
            unit: self.tag.unit(),
            unit_provider: unit_provider,
        }
    }
}

impl fmt::Display for DisplayValue<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.value_display.fmt(f)
    }
}

/// Helper struct for printing a value with its unit.
pub struct DisplayValueUnit<'a, T> where T: ProvideUnit<'a> {
    ifd_num: In,
    value_display: value::Display<'a>,
    unit: Option<&'static [UnitPiece]>,
    unit_provider: T,
}

impl<'a, T> fmt::Display for DisplayValueUnit<'a, T> where T: ProvideUnit<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(unit) = self.unit {
            assert!(!unit.is_empty());
            for piece in unit {
                match *piece {
                    UnitPiece::Value => self.value_display.fmt(f),
                    UnitPiece::Str(s) => f.write_str(s),
                    UnitPiece::Tag(tag) =>
                        if let Some(x) = self.unit_provider.get_field(
                                tag, self.ifd_num) {
                            x.value.display_as(tag).fmt(f)
                        } else if let Some(x) = tag.default_value() {
                            x.display_as(tag).fmt(f)
                        } else {
                            write!(f, "[{} missing]", tag)
                        },
                }?
            }
            Ok(())
        } else {
            self.value_display.fmt(f)
        }
    }
}

pub trait ProvideUnit<'a>: Copy {
    fn get_field(self, tag: Tag, ifd_num: In) -> Option<&'a Field>;
}

impl<'a> ProvideUnit<'a> for () {
    fn get_field(self, _tag: Tag, _ifd_num: In) -> Option<&'a Field> {
        None
    }
}

impl<'a> ProvideUnit<'a> for &'a Field {
    fn get_field(self, tag: Tag, ifd_num: In) -> Option<&'a Field> {
        Some(self).filter(|x| x.tag == tag && x.ifd_num == ifd_num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal little-endian RW2: header with version word 85, then one IFD
    /// holding the four sensor borders and the ISO tag as SHORTs.
    fn rw2(entries: &[(u16, u16)]) -> Vec<u8> {
        let mut f = vec![0x49, 0x49];                       // "II"
        f.extend_from_slice(&RW2_EIGHTY_FIVE.to_le_bytes());
        f.extend_from_slice(&8u32.to_le_bytes());           // IFD0 at 8
        f.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for &(tag, value) in entries {
            f.extend_from_slice(&tag.to_le_bytes());
            f.extend_from_slice(&3u16.to_le_bytes());       // SHORT
            f.extend_from_slice(&1u32.to_le_bytes());       // count 1
            f.extend_from_slice(&value.to_le_bytes());
            f.extend_from_slice(&[0, 0]);                   // pad to 4
        }
        f.extend_from_slice(&0u32.to_le_bytes());           // no next IFD
        f
    }

    fn field_of(fields: &[Field], tag: Tag) -> Option<u32> {
        fields.iter().find(|f| f.tag == tag)?.value.get_uint(0)
    }

    /// A RW2 states its size and ISO ONLY in Panasonic's own IFD0 tags, so
    /// without this the format parses completely and answers nothing for
    /// either.
    #[test]
    fn rw2_size_and_iso_come_from_the_vendor_tags() {
        let f = rw2(&[(4, 10), (5, 18), (6, 4010), (7, 6018), (0x17, 100)]);
        let (fields, _le) = parse_exif(&f).unwrap();
        // The ACTIVE AREA: right - left, bottom - top.
        assert_eq!(field_of(&fields, Tag::ImageWidth), Some(6000));
        assert_eq!(field_of(&fields, Tag::ImageLength), Some(4000));
        assert_eq!(field_of(&fields, Tag::PhotographicSensitivity), Some(100));
    }

    /// The substitution must be gated on the RW2 version word: tags 2-7 mean
    /// nothing in baseline TIFF but are free for another dialect to claim.
    #[test]
    fn a_plain_tiff_gets_no_synthesised_fields() {
        let mut f = rw2(&[(4, 10), (5, 18), (6, 4010), (7, 6018), (0x17, 100)]);
        f[2..4].copy_from_slice(&TIFF_FORTY_TWO.to_le_bytes());
        let (fields, _le) = parse_exif(&f).unwrap();
        assert_eq!(field_of(&fields, Tag::ImageWidth), None);
        assert_eq!(field_of(&fields, Tag::PhotographicSensitivity), None);
    }

    /// Borders that do not describe a positive area are not a size.
    #[test]
    fn rw2_borders_that_make_no_sense_are_ignored() {
        let f = rw2(&[(4, 4010), (5, 6018), (6, 10), (7, 18)]);
        let (fields, _le) = parse_exif(&f).unwrap();
        assert_eq!(field_of(&fields, Tag::ImageWidth), None);
        assert_eq!(field_of(&fields, Tag::ImageLength), None);
    }

    /// A DNG-shaped TIFF: IFD0 is a thumbnail carrying a `SubIFDs` pointer,
    /// and the sub-image holds the real size plus `DefaultCropSize`.
    fn dng(crop: Option<(u16, u16)>) -> Vec<u8> {
        // Layout: header(8) | IFD0 | SubIFD | (no trailing data)
        let ifd0_at = 8usize;
        let ifd0_entries = 3usize;
        let sub_at = ifd0_at + 2 + ifd0_entries * 12 + 4;
        let mut f = vec![0x49, 0x49];
        f.extend_from_slice(&TIFF_FORTY_TWO.to_le_bytes());
        f.extend_from_slice(&(ifd0_at as u32).to_le_bytes());

        let short = |f: &mut Vec<u8>, tag: u16, v: u16| {
            f.extend_from_slice(&tag.to_le_bytes());
            f.extend_from_slice(&3u16.to_le_bytes());
            f.extend_from_slice(&1u32.to_le_bytes());
            f.extend_from_slice(&v.to_le_bytes());
            f.extend_from_slice(&[0, 0]);
        };
        // IFD0: a 256x171 thumbnail plus the sub-image pointer.
        f.extend_from_slice(&(ifd0_entries as u16).to_le_bytes());
        short(&mut f, 0x0100, 256);
        short(&mut f, 0x0101, 171);
        f.extend_from_slice(&0x014au16.to_le_bytes());
        f.extend_from_slice(&4u16.to_le_bytes());          // LONG
        f.extend_from_slice(&1u32.to_le_bytes());
        f.extend_from_slice(&(sub_at as u32).to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());          // no next IFD
        assert_eq!(f.len(), sub_at);

        // Sub-image: the full readout, optionally with DefaultCropSize.
        let n = if crop.is_some() { 3u16 } else { 2 };
        f.extend_from_slice(&n.to_le_bytes());
        short(&mut f, 0x0100, 6188);
        short(&mut f, 0x0101, 4120);
        if let Some((w, h)) = crop {
            f.extend_from_slice(&0xc620u16.to_le_bytes());
            f.extend_from_slice(&3u16.to_le_bytes());      // SHORT
            f.extend_from_slice(&2u32.to_le_bytes());
            f.extend_from_slice(&w.to_le_bytes());
            f.extend_from_slice(&h.to_le_bytes());
        }
        f.extend_from_slice(&0u32.to_le_bytes());
        f
    }

    /// **IFD0 of a DNG is a thumbnail by specification**, so the primary IFD
    /// keeps answering 256 — the sub-image is reachable separately rather than
    /// overwriting it.
    #[test]
    fn sub_images_get_their_own_ifd_number() {
        let (fields, _le) = parse_exif(&dng(None)).unwrap();
        let at = |tag: Tag, ifd: In| fields.iter()
            .find(|f| f.tag == tag && f.ifd_num == ifd)
            .and_then(|f| f.value.get_uint(0));
        assert_eq!(at(Tag::ImageWidth, In::PRIMARY), Some(256));
        assert_eq!(at(Tag::ImageWidth, In::SUB_IMAGE), Some(6188));
        assert_eq!(at(Tag::ImageLength, In::SUB_IMAGE), Some(4120));
    }

    /// **The same tag in three IFDs is three different images.** IFD1's
    /// `JPEGInterchangeFormat` is the thumbnail everyone reads; IFD0's is the
    /// preview and a further IFD's the full-size JPEG, and a Sony ARW carries
    /// all three. Reading only IFD1 finds the smallest of them — 10 KB where
    /// a 7.3 MB full-size JPEG is sitting in IFD2.
    #[test]
    fn every_ifd_that_addresses_a_jpeg_is_reported() {
        // IFD0 -> preview at 900 (60 bytes), IFD1 -> thumbnail at 700 (30).
        let mut f = vec![0x49, 0x49];
        f.extend_from_slice(&TIFF_FORTY_TWO.to_le_bytes());
        f.extend_from_slice(&8u32.to_le_bytes());
        let long = |f: &mut Vec<u8>, tag: u16, v: u32| {
            f.extend_from_slice(&tag.to_le_bytes());
            f.extend_from_slice(&4u16.to_le_bytes());
            f.extend_from_slice(&1u32.to_le_bytes());
            f.extend_from_slice(&v.to_le_bytes());
        };
        let ifd1_at = 8 + 2 + 2 * 12 + 4;
        f.extend_from_slice(&2u16.to_le_bytes());
        long(&mut f, 0x0201, 900);
        long(&mut f, 0x0202, 60);
        f.extend_from_slice(&(ifd1_at as u32).to_le_bytes());
        assert_eq!(f.len(), ifd1_at);
        f.extend_from_slice(&2u16.to_le_bytes());
        long(&mut f, 0x0201, 700);
        long(&mut f, 0x0202, 30);
        f.extend_from_slice(&0u32.to_le_bytes());

        let exif = crate::Reader::new().read_raw(f).unwrap();
        let mut found: Vec<(u64, u32)> =
            exif.thumbnails().iter().map(|i| (i.offset, i.length)).collect();
        found.sort_unstable();
        assert_eq!(found, vec![(700, 30), (900, 60)],
                   "both IFDs address an image; only one was reported");
    }

    /// A TIFF that IS an image addresses itself with `StripOffsets`, which is
    /// how a DNG stores its preview — and unlike the `JPEGInterchangeFormat`
    /// case, the IFD's own dimensions describe it, because the IFD is it.
    #[test]
    fn a_strip_addressed_image_is_reported_with_its_own_size() {
        let f = strip_tiff(132940, 131328, false);
        let exif = crate::Reader::new().read_raw(f).unwrap();
        let imgs = exif.thumbnails();
        assert_eq!(imgs.len(), 1, "the strip image was not found");
        assert_eq!((imgs[0].offset, imgs[0].length), (132940, 131328));
        assert_eq!((imgs[0].width, imgs[0].height), (Some(256), Some(171)));
        assert_eq!(imgs[0].subfile_type, Some(1), "bit 0 marks it a preview");
    }

    /// **`0xFFFFFFFF` is a sentinel, not an address.** Panasonic writes it in
    /// a RW2's `StripOffsets` and puts the real position in its own tag, so
    /// reporting it hands the caller an offset four gigabytes into a much
    /// smaller file.
    #[test]
    fn a_sentinel_strip_offset_is_not_an_image() {
        let f = strip_tiff(u32::MAX, 43_180_032, false);
        let exif = crate::Reader::new().read_raw(f).unwrap();
        assert!(exif.thumbnails().is_empty());
    }

    /// A multi-strip image is not one contiguous run, so an offset and a
    /// length cannot describe it; answering with the first strip would be a
    /// plausible-looking lie.
    #[test]
    fn a_multi_strip_image_is_not_reported() {
        let f = strip_tiff(132940, 131328, true);
        let exif = crate::Reader::new().read_raw(f).unwrap();
        assert!(exif.thumbnails().is_empty());
    }

    /// One IFD holding a 256x171 image in `strips` strips.
    fn strip_tiff(offset: u32, count: u32, multi: bool) -> Vec<u8> {
        let entries: u16 = 5;
        let ifd_at = 8usize;
        let array_at = ifd_at + 2 + entries as usize * 12 + 4;
        let mut f = vec![0x49, 0x49];
        f.extend_from_slice(&TIFF_FORTY_TWO.to_le_bytes());
        f.extend_from_slice(&(ifd_at as u32).to_le_bytes());
        f.extend_from_slice(&entries.to_le_bytes());
        let mut put = |f: &mut Vec<u8>, tag: u16, typ: u16, cnt: u32, val: u32| {
            f.extend_from_slice(&tag.to_le_bytes());
            f.extend_from_slice(&typ.to_le_bytes());
            f.extend_from_slice(&cnt.to_le_bytes());
            f.extend_from_slice(&val.to_le_bytes());
        };
        put(&mut f, 0x00fe, 4, 1, 1);          // NewSubfileType: reduced
        put(&mut f, 0x0100, 4, 1, 256);        // ImageWidth
        put(&mut f, 0x0101, 4, 1, 171);        // ImageLength
        if multi {
            // Two strips, so the values live out of line.
            put(&mut f, 0x0111, 4, 2, array_at as u32);
            put(&mut f, 0x0117, 4, 2, (array_at + 8) as u32);
        } else {
            put(&mut f, 0x0111, 4, 1, offset);
            put(&mut f, 0x0117, 4, 1, count);
        }
        f.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(f.len(), array_at);
        for v in [offset, offset.wrapping_add(1), count, count] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        f
    }

    /// The sub-image base sits above the chained IFDs (capped at 8) and above
    /// `In::MPF`, so nothing it writes can land on an existing allocation.
    #[test]
    fn the_sub_image_base_cannot_collide() {
        assert!(In::SUB_IMAGE.index() >= 8);
        assert_ne!(In::SUB_IMAGE, In::PRIMARY);
        assert_ne!(In::SUB_IMAGE, In::THUMBNAIL);
        #[cfg(feature = "mpf")]
        assert_ne!(In::SUB_IMAGE, In::MPF);
    }

    #[test]
    fn in_convert() {
        assert_eq!(In::PRIMARY.index(), 0);
        assert_eq!(In::THUMBNAIL.index(), 1);
        assert_eq!(In(2).index(), 2);
        assert_eq!(In(65535).index(), 65535);
        assert_eq!(In::PRIMARY, In(0));
    }

    #[test]
    fn in_display() {
        assert_eq!(format!("{:10}", In::PRIMARY), "primary   ");
        assert_eq!(format!("{:>10}", In::THUMBNAIL), " thumbnail");
        assert_eq!(format!("{:10}", In(2)), "IFD2      ");
        assert_eq!(format!("{:^10}", In(65535)), " IFD65535 ");
    }

    #[test]
    fn truncated() {
        let mut data =
            b"MM\0\x2a\0\0\0\x08\
              \0\x01\x01\0\0\x03\0\0\0\x01\0\x14\0\0\0\0\0\0".to_vec();
        parse_exif(&data).unwrap();
        while let Some(_) = data.pop() {
            parse_exif(&data).unwrap_err();
        }
    }

    // Before the error is returned, the IFD is parsed multiple times
    // as the 0th, 1st, ..., and n-th IFDs.
    #[test]
    fn inf_loop_by_next() {
        let data = b"MM\0\x2a\0\0\0\x08\
                     \0\x01\x01\0\0\x03\0\0\0\x01\0\x14\0\0\0\0\0\x08";
        assert_err_pat!(parse_exif(data),
                        Error::InvalidFormat("Limit the IFD count to 8"));
    }

    #[test]
    fn inf_loop_by_exif_next() {
        let data = b"MM\x00\x2a\x00\x00\x00\x08\
                     \x00\x01\x87\x69\x00\x04\x00\x00\x00\x01\x00\x00\x00\x1a\
                     \x00\x00\x00\x00\
                     \x00\x01\x90\x00\x00\x07\x00\x00\x00\x040231\
                     \x00\x00\x00\x08";
        assert_err_pat!(parse_exif(data),
                        Error::InvalidFormat("Unexpected next IFD"));
    }

    #[test]
    fn unknown_field() {
        let data = b"MM\0\x2a\0\0\0\x08\
                     \0\x01\x01\0\xff\xff\0\0\0\x01\0\x14\0\0\0\0\0\0";
        let (v, _le) = parse_exif(data).unwrap();
        assert_eq!(v.len(), 1);
        assert_pat!(v[0].value, Value::Unknown(0xffff, 1, 0x12));
    }

    #[test]
    fn parse_ifd_entry() {
        // BYTE (type == 1)
        let data = b"\x02\x03\x00\x01\0\0\0\x04ABCD";
        assert_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 0).unwrap(),
                    (0x0203, Value::Unknown(1, 4, 8)));
        let data = b"\x02\x03\x00\x01\0\0\0\x05\0\0\0\x0cABCDE";
        assert_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 0).unwrap(),
                    (0x0203, Value::Unknown(1, 5, 12)));
        let data = b"\x02\x03\x00\x01\0\0\0\x05\0\0\0\x0cABCD";
        assert_err_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 0),
                        Error::InvalidFormat("Truncated field value"));

        // SHORT (type == 3)
        let data = b"X\x04\x05\x00\x03\0\0\0\x02ABCD";
        assert_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 1).unwrap(),
                    (0x0405, Value::Unknown(3, 2, 9)));
        let data = b"X\x04\x05\x00\x03\0\0\0\x03\0\0\0\x0eXABCDEF";
        assert_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 1).unwrap(),
                    (0x0405, Value::Unknown(3, 3, 14)));
        let data = b"X\x04\x05\x00\x03\0\0\0\x03\0\0\0\x0eXABCDE";
        assert_err_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 1),
                        Error::InvalidFormat("Truncated field value"));

        // Really unknown
        let data = b"X\x01\x02\x03\x04\x05\x06\x07\x08ABCD";
        assert_pat!(Parser::parse_ifd_entry::<BigEndian>(data, 1).unwrap(),
                    (0x0102, Value::Unknown(0x0304, 0x05060708, 9)));
    }

    #[test]
    fn date_time() {
        let mut dt = DateTime::from_ascii(b"2016:05:04 03:02:01").unwrap();
        assert_eq!(dt.year, 2016);
        assert_eq!(dt.to_string(), "2016-05-04 03:02:01");

        dt.parse_subsec(b"987").unwrap();
        assert_eq!(dt.nanosecond.unwrap(), 987000000);
        dt.parse_subsec(b"000987").unwrap();
        assert_eq!(dt.nanosecond.unwrap(), 987000);
        dt.parse_subsec(b"987654321").unwrap();
        assert_eq!(dt.nanosecond.unwrap(), 987654321);
        dt.parse_subsec(b"9876543219").unwrap();
        assert_eq!(dt.nanosecond.unwrap(), 987654321);
        dt.parse_subsec(b"130   ").unwrap();
        assert_eq!(dt.nanosecond.unwrap(), 130000000);
        dt.parse_subsec(b"0").unwrap();
        assert_eq!(dt.nanosecond.unwrap(), 0);
        dt.parse_subsec(b"").unwrap();
        assert!(dt.nanosecond.is_none());
        dt.parse_subsec(b" ").unwrap();
        assert!(dt.nanosecond.is_none());

        dt.parse_offset(b"+00:00").unwrap();
        assert_eq!(dt.offset.unwrap(), 0);
        dt.parse_offset(b"+01:23").unwrap();
        assert_eq!(dt.offset.unwrap(), 83);
        dt.parse_offset(b"+99:99").unwrap();
        assert_eq!(dt.offset.unwrap(), 6039);
        dt.parse_offset(b"-01:23").unwrap();
        assert_eq!(dt.offset.unwrap(), -83);
        dt.parse_offset(b"-99:99").unwrap();
        assert_eq!(dt.offset.unwrap(), -6039);
        assert_err_pat!(dt.parse_offset(b"   :  "), Error::BlankValue(_));
        assert_err_pat!(dt.parse_offset(b"      "), Error::BlankValue(_));
    }

    #[test]
    fn display_value_with_unit() {
        let cm = Field {
            tag: Tag::ResolutionUnit,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![3]),
        };
        let cm_tn = Field {
            tag: Tag::ResolutionUnit,
            ifd_num: In::THUMBNAIL,
            value: Value::Short(vec![3]),
        };
        // No unit.
        let exifver = Field {
            tag: Tag::ExifVersion,
            ifd_num: In::PRIMARY,
            value: Value::Undefined(b"0231".to_vec(), 0),
        };
        assert_eq!(exifver.display_value().to_string(),
                   "2.31");
        assert_eq!(exifver.display_value().with_unit(()).to_string(),
                   "2.31");
        assert_eq!(exifver.display_value().with_unit(&cm).to_string(),
                   "2.31");
        // Fixed string.
        let width = Field {
            tag: Tag::ImageWidth,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![257]),
        };
        assert_eq!(width.display_value().to_string(),
                   "257");
        assert_eq!(width.display_value().with_unit(()).to_string(),
                   "257 pixels");
        assert_eq!(width.display_value().with_unit(&cm).to_string(),
                   "257 pixels");
        // Unit tag (with a non-default value).
        // Unit tag is missing but the default is specified.
        let xres = Field {
            tag: Tag::XResolution,
            ifd_num: In::PRIMARY,
            value: Value::Rational(vec![(300, 1).into()]),
        };
        assert_eq!(xres.display_value().to_string(),
                   "300");
        assert_eq!(xres.display_value().with_unit(()).to_string(),
                   "300 pixels per inch");
        assert_eq!(xres.display_value().with_unit(&cm).to_string(),
                   "300 pixels per cm");
        assert_eq!(xres.display_value().with_unit(&cm_tn).to_string(),
                   "300 pixels per inch");
        // Unit tag is missing and the default is not specified.
        let gpslat = Field {
            tag: Tag::GPSLatitude,
            ifd_num: In::PRIMARY,
            value: Value::Rational(vec![
                (10, 1).into(), (0, 1).into(), (1, 10).into()]),
        };
        assert_eq!(gpslat.display_value().to_string(),
                   "10 deg 0 min 0.1 sec");
        assert_eq!(gpslat.display_value().with_unit(()).to_string(),
                   "10 deg 0 min 0.1 sec [GPSLatitudeRef missing]");
        assert_eq!(gpslat.display_value().with_unit(&cm).to_string(),
                   "10 deg 0 min 0.1 sec [GPSLatitudeRef missing]");
    }

    #[test]
    fn no_borrow_no_move() {
        let resunit = Field {
            tag: Tag::ResolutionUnit,
            ifd_num: In::PRIMARY,
            value: Value::Short(vec![3]),
        };
        // This fails to compile with "temporary value dropped while
        // borrowed" error if with_unit() borrows self.
        let d = resunit.display_value().with_unit(());
        assert_eq!(d.to_string(), "cm");
        // This fails to compile if with_unit() moves self.
        let d1 = resunit.display_value();
        let d2 = d1.with_unit(());
        assert_eq!(d1.to_string(), "cm");
        assert_eq!(d2.to_string(), "cm");
    }

    #[test]
    fn continue_on_error() {
        macro_rules! define_test {
            {
                data: $data:expr,
                fields: [$($fields:pat),*],
                errors: [$first_error:pat $(, $rest_errors:pat)*]
            } => {
                let data = $data;
                let mut parser = Parser::new();
                assert_err_pat!(parser.parse(data), $first_error);
                let mut parser = Parser::new();
                parser.continue_on_error = Some(Vec::new());
                parser.parse(data).unwrap();
                assert_eq!(parser.little_endian, false);
                let mut entries = parser.entries.iter();
                $(
                    assert_pat!(entries.next().unwrap()
                                       .ref_field(data, parser.little_endian),
                                $fields);
                )*
                assert_pat!(entries.next(), None);
                let mut errors =
                    parser.continue_on_error.as_ref().unwrap().iter();
                assert_pat!(errors.next().unwrap(), $first_error);
                $(
                    assert_pat!(errors.next().unwrap(), $rest_errors);
                )*
                assert_pat!(errors.next(), None);
            }
        }
        // 0th IFD is missing.
        define_test! {
            data: b"MM\0\x2a\0\0\0\x08",
            fields: [],
            errors: [Error::InvalidFormat("Truncated IFD count")]
        }
        // 2nd entry is truncated.
        define_test! {
            data: b"MM\0\x2a\0\0\0\x08\
                    \0\x02\x01\x00\0\x03\0\0\0\x01\0\x14\0\0\
                          \x01\x01\0\x03\0\0\0\x01\0\x15\0",
            fields: [Field { tag: Tag::ImageWidth, ifd_num: In(0),
                             value: Value::Short(_) }],
            errors: [Error::InvalidFormat("Truncated IFD")]
        }
        // 1st entry broken.
        define_test! {
            data: b"MM\0\x2a\0\0\0\x08\
                    \0\x02\x01\x00\0\x03\0\0\0\x03\0\0\0\x21\
                          \x01\x01\0\x03\0\0\0\x01\0\x15\0\0\
                          \0\0\0\0",
            fields: [Field { tag: Tag::ImageLength, ifd_num: In(0),
                             value: Value::Short(_) }],
            errors: [Error::InvalidFormat("Truncated field value")]
        }
        // Exif IFD has non-zero next IFD offset.
        // Top-level next IFD is also broken.
        define_test! {
            data: b"MM\0\x2a\0\0\0\x08\
                    \0\x02\x87\x69\0\x04\0\0\0\x01\0\0\0\x26\
                          \xfd\xe8\0\x09\0\0\0\x01\xfe\xdc\xba\x98\
                          \xff\xff\xff\xff\
                    \0\x01\x90\x00\0\x07\0\0\0\x04\x00\x02\x03\x02\
                          \0\0\0\x01",
            fields: [Field { tag: Tag::ExifVersion, ifd_num: In(0),
                             value: Value::Undefined(_, _) },
                     Field { tag: Tag(Context::Tiff, 65000), ifd_num: In(0),
                             value: Value::SLong(_) }],
            errors: [Error::InvalidFormat("Unexpected next IFD"),
                     Error::InvalidFormat("Truncated IFD count")]
        }
        // Exif IFD pointer has a bad type.
        define_test! {
            data: b"MM\0\x2a\0\0\0\x08\
                    \0\x02\x87\x69\0\x09\0\0\0\x01\0\0\0\x26\
                          \xfd\xe8\0\x06\0\0\0\x03\xfe\xdc\xba\x98\
                          \0\0\0\0\
                    \0\x01\x90\x00\0\x07\0\0\0\x04\x00\x02\x03\x02\
                          \0\0\0\x01",
            fields: [Field { tag: Tag(Context::Tiff, 65000), ifd_num: In(0),
                             value: Value::SByte(_) }],
            errors: [Error::InvalidFormat("Invalid pointer")]
        }
        // Exif IFD pointer is empty.
        define_test! {
            data: b"MM\0\x2a\0\0\0\x08\
                    \0\x02\x87\x69\0\x04\0\0\0\x00\0\0\0\x26\
                          \xfd\xe8\0\x08\0\0\0\x02\xfe\xdc\xba\x98\
                          \0\0\0\0\
                    \0\x01\x90\x00\0\x07\0\0\0\x04\x00\x02\x03\x02\
                          \0\0\0\x01",
            fields: [Field { tag: Tag(Context::Tiff, 65000), ifd_num: In(0),
                             value: Value::SShort(_) }],
            errors: [Error::InvalidFormat("Invalid pointer")]
        }
    }
}
