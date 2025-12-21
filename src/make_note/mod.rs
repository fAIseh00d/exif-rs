//
// Copyright (c) 2025 Jinwoo Park and 2016 KAMADA Ken'ichi.
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

use crate::endian::{Endian, BigEndian, LittleEndian};
use crate::error::Error;
use crate::make_note::maker_tag::MakerNoteVendor;
use crate::tag::Context;
use crate::value::Value;
use crate::value::get_type_info;

// Re-export types from tiff module to avoid duplication
pub use crate::tiff::{IfdEntry, Field, In};

// Import from tiff module
use crate::tag::Tag;
use crate::tiff::{TIFF_BE, TIFF_LE, TIFF_FORTY_TWO};

// MakerNote tag system
pub mod maker_tag;

// Common macro for tag generation
#[macro_use]
mod tag_macro;

// // Common macro for sub-tag generation
// #[macro_use]
// mod subtag_macro;

// Vendor-specific tag definitions
pub mod panasonic;
pub mod nikon;
// pub mod nikon_picture_control;
pub mod sony;
pub mod canon;
pub mod fujifilm;
pub mod olympus;

/// Dummy TIFF header for MakerNote vendors that don't include their own TIFF header.
/// Little-endian TIFF header with IFD offset at 8.
/// Format: [II (little-endian), 42 (TIFF magic), IFD offset (8)]
const DUMMY_TIFF_HEADER: &[u8] = &[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00];

/// Parse MakerNote data with offset correction.
///
/// This function is specifically designed for parsing camera manufacturer-specific
/// MakerNote data where proprietary headers have been removed and replaced with
/// a standard TIFF header.
///
/// # Arguments
/// * `data` - The MakerNote data with TIFF header (after proprietary header removal)
/// * `tiff_offset` - Number of bytes from MakeNote position to original TIFF position
/// * `offset_correction` - Number of bytes removed from original MakerNote position
///
/// # Returns
/// A tuple of (Vec<Field>, bool) where the boolean indicates little-endian byte order.
///
/// # Example
/// ```ignore
/// // Panasonic MakerNote: Remove 12-byte "Panasonic\0\0\0" header
/// let inside = &make_note_data[12..];
/// let mut crafted = Vec::new();
/// crafted.extend(&[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]); // TIFF header
/// crafted.extend_from_slice(inside);
///
/// let (fields, _endian) = parse_make_note(&crafted, 12)?;
/// ```
pub fn parse_make_note(data: &[u8], consider_tiff_offset: bool, tiff_offset: u32, offset_correction: i32) -> Result<(Vec<Field>, bool), Error> {
    let mut parser = MakerNoteParser::with_offset_correction(
        MakerNoteVendor::Unknown, consider_tiff_offset, tiff_offset, offset_correction);

    parser.parse(data)?;
    let (entries, le) = (parser.entries, parser.little_endian);
    Ok((
        entries
            .into_iter()
            .map(|e| e.0.into_field(data, le))
            .collect(),
        le,
    ))
}

/// Parse MakerNote data with vendor detection and offset correction.
///
/// Returns vendor-specific `MakerNoteField` instead of generic `Field`.
///
/// # Arguments
/// * `data` - The raw MakerNote data (before header removal)
/// * `tiff_offset` - Offset from TIFF start to MakerNote start
/// * `make` - Optional Make field from EXIF (e.g., "Canon", "SONY")
///
/// # Returns
/// A tuple of (Vec<MakerNoteField>, MakerNoteVendor, bool) where:
/// - First element is the parsed fields with vendor-specific tags
/// - Second element is the detected vendor
/// - Third element indicates little-endian byte order
///
/// # Example
/// ```ignore
/// use exif::make_note::parse_make_note_with_vendor;
///
/// let (fields, vendor, _endian) = parse_make_note_with_vendor(make_note_data, 0, Some("Canon"))?;
/// for field in fields {
///     println!("{}: {:?}", field.tag, field.value);
/// }
/// ```
// fixed - added offset_of_note: u32
pub fn parse_make_note_with_vendor(
    data: &[u8],
    tiff_offset: u32,
    make: Option<&str>,
) -> Result<
    (
        Vec<maker_tag::MakerNoteField>,
        maker_tag::MakerNoteVendor,
        bool,
    ),
    Error,
> {
    use maker_tag::{MakerNoteField, MakerNoteVendor, MakerTag};

    // Step 1: Detect vendor from header and Make field
    let vendor = MakerNoteVendor::from_header(data, make);
    let header_size = vendor.header_size();

    // Step 2: Skip proprietary header
    if data.len() < header_size {
        return Err(Error::InvalidFormat("MakerNote data too short for vendor header"));
    }
    let inside = &data[header_size..];

    // Step 3: Create buffer with TIFF header if needed
    let crafted;
    let parse_data = match vendor {
        MakerNoteVendor::Nikon => {
            // Nikon Type 3 already has TIFF header after the proprietary header
            inside
        }
        MakerNoteVendor::Panasonic | MakerNoteVendor::Fujifilm | MakerNoteVendor::Sony | MakerNoteVendor::Canon |
        MakerNoteVendor::Leica | MakerNoteVendor::Olympus | MakerNoteVendor::OMSystem => {
            // Need to add TIFF header (Canon has no header at all, offsets are relative to TIFF start)
            crafted = {
                let mut buf = Vec::new();
                buf.extend(DUMMY_TIFF_HEADER);
                buf.extend_from_slice(inside);
                buf
            };
            &crafted[..]
        }
        _ => {
            // Unknown vendor - try parsing as-is
            data
        }
    };

    // Step 4: Parse with offset correction
    let offset_correction = vendor.offset_correction();

    let consider_tiff_offset = vendor.consider_tiff_offset();

    let mut parser = MakerNoteParser::with_offset_correction(
        vendor,
        consider_tiff_offset,
        tiff_offset,
        offset_correction,
    );

    parser.parse(parse_data)?;
    let (entries, le) = (parser.entries, parser.little_endian);

    // Step 5: Convert to MakerNoteField with vendor-specific tags
    let maker_fields = entries
        .into_iter()
        .map(|(entry, entry_vendor)| {
            // Use generic parsing to avoid tag-specific handling (e.g., JPEGInterchangeFormat)
            let field = entry.into_field_generic(parse_data, le);
            MakerNoteField::new(
                MakerTag::new(entry_vendor, field.tag.1),
                field.ifd_num,
                field.value,
            )
        })
        .collect();

    Ok((maker_fields, vendor, le))
}

#[derive(Debug)]
pub struct MakerNoteParser {
    pub vendor: MakerNoteVendor,
    pub entries: Vec<(IfdEntry, MakerNoteVendor)>, // (entry, vendor at time of parsing)
    pub little_endian: bool,
    // `Some<Vec>` to enable the option and `None` to disable it.
    pub continue_on_error: Option<Vec<Error>>,

    pub consider_tiff_offset: bool,

    pub tiff_offset: u32,

    /// Offset correction for MakerNote parsing.
    /// This value is subtracted from all offset references in IFD entries.
    ///
    /// Example: If original MakerNote data started at offset 1000 in main EXIF,
    /// but we removed a 12-byte proprietary header and added an 8-byte TIFF header,
    /// then offset_correction = 12 (bytes removed from original position)
    pub offset_correction: i32,
}

impl MakerNoteParser {
    pub fn new() -> Self {
        Self {
            vendor: MakerNoteVendor::Unknown,
            entries: Vec::new(),
            little_endian: false,
            continue_on_error: None,
            consider_tiff_offset: false,
            tiff_offset: 0,
            offset_correction: 0,
        }
    }

    pub fn with_offset_correction(vendor: MakerNoteVendor, consider_tiff_offset: bool, tiff_offset: u32, offset_correction: i32) -> Self {
        Self {
            vendor,
            entries: Vec::new(),
            little_endian: false,
            continue_on_error: None,
            consider_tiff_offset,
            tiff_offset,
            offset_correction,
        }
    }

    pub fn parse(&mut self, data: &[u8]) -> Result<(), Error> {
        // Check the byte order and call the real parser.
        if data.len() < 8 {
            return Err(Error::InvalidFormat("Truncated TIFF header"));
        }
        match BigEndian::loadu16(data, 0) {
            TIFF_BE => {
                self.little_endian = false;
                self.parse_header::<BigEndian>(data)
            },
            TIFF_LE => {
                self.little_endian = true;
                self.parse_header::<LittleEndian>(data)
            },
            _ => Err(Error::InvalidFormat("Invalid TIFF byte order")),
        }
    }

    fn parse_header<E>(&mut self, data: &[u8])
                       -> Result<(), Error> where E: Endian {
        // Parse the rest of the header (42 and the IFD offset).
        if E::loadu16(data, 2) != TIFF_FORTY_TWO {
            return Err(Error::InvalidFormat("Invalid forty two"));
        }
        let ifd_offset = E::loadu32(data, 4) as usize;
        
        // log::info!("parse_header, ifd_offset={ifd_offset};");
        self.parse_body::<E>(data, ifd_offset)
            .or_else(|e| self.check_error(e))
    }

    fn parse_body<E>(&mut self, data: &[u8], mut ifd_offset: usize)
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
                data, ifd_offset, Context::Tiff, ifd_num)?;
            ifd_num_ck = ifd_num.checked_add(1);
        }
        Ok(())
    }

    // Parse IFD [EXIF23 4.6.2].
    fn parse_ifd<E>(&mut self, data: &[u8],
                    mut offset: usize, ctx: Context, ifd_num: u16)
                    -> Result<usize, Error> where E: Endian {
        // Count (the number of the entries).
        if data.len() < offset || data.len() - offset < 2 {
            return Err(Error::InvalidFormat("Truncated IFD count"));
        }

        let count = E::loadu16(data, offset) as usize;

        let tiff_correction = self.consider_tiff_offset as i32 * self.tiff_offset as i32;
        offset += 2;

        // Array of entries.
        for _ in 0..count {
            if data.len() - offset < 12 {
                return Err(Error::InvalidFormat("Truncated IFD"));
            }
            let entry = Self::parse_ifd_entry::<E>(data, offset, tiff_correction, self.offset_correction);
            offset += 12;
            let (tag, val) = match entry {
                Ok(x) => x,
                Err(e) => {
                    self.check_error(e)?;
                    continue;
                },
            };

            // No infinite recursion will occur because the context is not
            // recursively defined.
            let tag = Tag(ctx, tag);
            let child_ctx = match (tag, self.vendor) {
                (Tag::ExifIFDPointer, _) => Context::Exif,
                (Tag::GPSInfoIFDPointer, _) => Context::Gps,
                (Tag::InteropIFDPointer, _) => Context::Interop,
                (_, MakerNoteVendor::Olympus | MakerNoteVendor::OMSystem) => {
                    if olympus::is_olympus_subdir(tag) && ctx == Context::Tiff {
                        // Determine subdirectory vendor based on tag number
                        let subdir_vendor = olympus::get_subdir_vendor(tag, self.vendor);

                        // Save current vendor and switch to subdirectory vendor
                        let saved_vendor = self.vendor;
                        self.vendor = subdir_vendor;

                        let result = olympus::parse_olympus_subdir::<E, _>(
                                data, val, tag, self.offset_correction, ifd_num,
                                |d, o, i| self.parse_ifd::<E>(d, o, Context::Tiff, i),
                        );

                        // Restore original vendor
                        self.vendor = saved_vendor;

                        result.or_else(|e| self.check_error(e))?;
                        continue;
                    }

                    self.entries.push((IfdEntry::from_field(Field {
                        tag: tag,
                        ifd_num: In(ifd_num),
                        value: val,
                    }), self.vendor));
                    continue;
                }
                _ => {
                    self.entries.push((IfdEntry::from_field(Field {
                        tag: tag, ifd_num: In(ifd_num), value: val }), self.vendor));
                    continue;
                },
            };
            // log::info!("> child_ctx : {:?}", child_ctx);
            self.parse_child_ifd::<E>(data, val, child_ctx, ifd_num)
                .or_else(|e| self.check_error(e))?;
        }

        // Offset to the next IFD.
        // log::debug!("After processing all {} entries, checking for next IFD at offset {} (0x{:X}), data.len()={}", count, offset, offset, data.len());
        if data.len() < offset {
            // log::warn!("Offset {} exceeds data length {}, assuming no next IFD", offset, data.len());
            return Ok(0);
        }
        if data.len() - offset < 4 {
            // log::warn!("Not enough space for next IFD offset at {}, remaining bytes: {}", offset, data.len() - offset);
            return Ok(0);
        }
        let next_ifd_offset = E::loadu32(data, offset);
        // log::debug!("next_ifd_offset = {} (0x{:X}) at offset {} (0x{:X})", next_ifd_offset, next_ifd_offset, offset, offset);

        // Validate next IFD offset
        if next_ifd_offset != 0 {
            // Check if the offset is reasonable (within data bounds)
            if next_ifd_offset as usize >= data.len() {
                // log::warn!("Invalid next IFD offset {} (0x{:X}) exceeds data length {}, treating as 0",
                //     next_ifd_offset, next_ifd_offset, data.len());
                return Ok(0);
            }
            // log::info!("Found next IFD at offset {} (0x{:X})", next_ifd_offset, next_ifd_offset);
        } else {
            // log::debug!("No next IFD (offset = 0)");
        }

        Ok(next_ifd_offset as usize)
    }

    fn parse_ifd_entry<E>(data: &[u8], offset: usize, tiff_correction: i32, offset_correction: i32)
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
            let raw_ofs = E::loadu32(data, valofs_at) as i32;
            // Apply offset correction: subtract the correction value from the raw offset
            let signed_corrected_ofs = raw_ofs - offset_correction - tiff_correction;
            let corrected_ofs = signed_corrected_ofs as usize;

            if data.len() < corrected_ofs || data.len() - corrected_ofs < vallen || signed_corrected_ofs.is_negative() {
                return Err(Error::InvalidFormat("Truncated field value"));
            }

            Value::Unknown(typ, cnt, corrected_ofs as u32)
        };


        Ok((tag, val))
    }

    fn parse_child_ifd<E>(&mut self, data: &[u8],
                          mut pointer: Value, ctx: Context, ifd_num: u16)
                          -> Result<(), Error> where E: Endian {
        // The pointer is not yet parsed, so do it here.
        IfdEntry::parse_value::<E>(&mut pointer, data);

        // A pointer field has type == LONG and count == 1, so the
        // value (IFD offset) must be embedded in the "value offset"
        // element of the field.
        let ofs = pointer.get_uint(0).ok_or(
            Error::InvalidFormat("Invalid pointer"))? as usize;
        match self.parse_ifd::<E>(data, ofs, ctx, ifd_num)? {
            0 => Ok(()),
            _ => Err(Error::InvalidFormat("Unexpected next IFD")),
        }   
    }

    fn check_error(&mut self, err: Error) -> Result<(), Error> {
        match self.continue_on_error {
            Some(ref mut v) => Ok(v.push(err)),
            None => Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Before the error is returned, the IFD is parsed multiple times
    // as the 0th, 1st, ..., and n-th IFDs.
    #[test]
    fn inf_loop_by_next() {
        let data = b"MM\0\x2a\0\0\0\x08\
                     \0\x01\x01\0\0\x03\0\0\0\x01\0\x14\0\0\0\0\0\x08";
        assert_err_pat!(parse_make_note(data, false, 0, 0),
                        Error::InvalidFormat("Limit the IFD count to 8"));
    }

    #[test]
    fn inf_loop_by_exif_next() {
        let data = b"MM\x00\x2a\x00\x00\x00\x08\
                     \x00\x01\x87\x69\x00\x04\x00\x00\x00\x01\x00\x00\x00\x1a\
                     \x00\x00\x00\x00\
                     \x00\x01\x90\x00\x00\x07\x00\x00\x00\x040231\
                     \x00\x00\x00\x08";
        assert_err_pat!(parse_make_note(data, false, 0, 0),
                        Error::InvalidFormat("Unexpected next IFD"));
    }

    #[test]
    fn unknown_field() {
        let data = b"MM\0\x2a\0\0\0\x08\
                     \0\x01\x01\0\xff\xff\0\0\0\x01\0\x14\0\0\0\0\0\0";
        let (v, _le) = parse_make_note(data, false, 0, 0).unwrap();
        assert_eq!(v.len(), 1);
        assert_pat!(v[0].value, Value::Unknown(0xffff, 1, 0x12));
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
                let mut parser = MakerNoteParser::new();
                assert_err_pat!(parser.parse(data), $first_error);
                let mut parser = MakerNoteParser::new();
                parser.continue_on_error = Some(Vec::new());
                parser.parse(data).unwrap();
                assert_eq!(parser.little_endian, false);
                let mut entries = parser.entries.iter();
                $(
                    assert_pat!(entries.next().unwrap()
                                       .0.ref_field(data, parser.little_endian),
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
