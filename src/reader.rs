//
// Copyright (c) 2017 KAMADA Ken'ichi.
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

use std::io;
use std::io::Read;

use crate::error::{Error, PartialResult};
use crate::exifimpl::Exif;
use crate::tag::Tag;
use crate::tiff::{Field, IfdEntry, In};
use crate::value::Value;
use crate::isobmff;
use crate::mrw;
use crate::crw;
use crate::x3f;
use crate::jpeg;
use crate::png;
use crate::raf;
use crate::tiff;
use crate::webp;

/// A struct to parse the Exif attributes and
/// create an `Exif` instance that holds the results.
///
/// # Examples
/// ```
/// # use std::fmt::{Display, Formatter, Result};
/// # #[derive(Debug)] struct Error(&'static str);
/// # impl std::error::Error for Error {}
/// # impl Display for Error {
/// #     fn fmt(&self, f: &mut Formatter) -> Result { f.write_str(self.0) }
/// # }
/// # fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
/// use exif::{In, Reader, Tag};
/// let file = std::fs::File::open("tests/exif.jpg")?;
/// let exif = Reader::new()
///     .read_from_container(&mut std::io::BufReader::new(&file))?;
/// let xres = exif.get_field(Tag::XResolution, In::PRIMARY)
///     .ok_or(Error("tests/exif.jpg must have XResolution"))?;
/// assert_eq!(xres.display_value().with_unit(&exif).to_string(),
///            "72 pixels per inch");
/// # Ok(()) }
/// ```
pub struct Reader {
    continue_on_error: bool,
}

impl Reader {
    /// Constructs a new `Reader`.
    pub fn new() -> Self {
        Self {
            continue_on_error: false,
        }
    }

    /// Sets the option to continue parsing on non-fatal errors.
    ///
    /// When this option is enabled, the parser will not stop on non-fatal
    /// errors and returns the results as far as they can be parsed.
    /// In such a case, `read_raw` and `read_from_container`
    /// return `Error::PartialResult`.
    /// The partial result and ignored errors can be obtained by
    /// [`Error::distill_partial_result`] or [`PartialResult::into_inner`].
    ///
    /// Note that a hard error (other than `Error::PartialResult`) may be
    /// returned even if this option is enabled.
    ///
    /// # Examples
    /// ```
    /// # use std::fmt::{Display, Formatter, Result};
    /// # fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    /// use exif::Reader;
    /// let file = std::fs::File::open("tests/exif.jpg")?;
    /// let exif = Reader::new()
    ///     .continue_on_error(true)
    ///     .read_from_container(&mut std::io::BufReader::new(&file))
    ///     .or_else(|e| e.distill_partial_result(|errors| {
    ///         errors.iter().for_each(|e| eprintln!("Warning: {}", e));
    ///     }))?;
    /// # Ok(()) }
    /// ```
    pub fn continue_on_error(&mut self, continue_on_error: bool) -> &mut Self {
        self.continue_on_error = continue_on_error;
        self
    }

    /// Parses the Exif attributes from raw Exif data.
    /// If an error occurred, `exif::Error` is returned.
    pub fn read_raw(&self, data: Vec<u8>) -> Result<Exif, Error> {
        self.read_raw_with_extra(data, Vec::new(), 0, Vec::new())
    }

    /// `read_raw`, plus fields the CONTAINER states that its Exif cannot.
    ///
    /// A RAF is the case this exists for: its Exif is the embedded JPEG's and
    /// describes the JPEG, while the raw image's own size lives in the RAF's
    /// CFA header. The fields are appended as an ordinary sub-image IFD, so a
    /// RAF answers `get_field(ImageWidth, In::SUB_IMAGE)` exactly as a DNG does.
    fn read_raw_with_extra(&self, data: Vec<u8>, extra: Vec<Field>, tiff_base: u64,
                           container_images: Vec<(u64, u32)>)
                           -> Result<Exif, Error> {
        let mut parser = tiff::Parser::new();
        parser.continue_on_error = self.continue_on_error.then(Vec::new);
        parser.parse(&data)?;
        parser.entries.extend(extra.into_iter().map(IfdEntry::from_parsed_field));
        let mut exif = Exif::new_at(data, parser.entries, parser.little_endian, tiff_base);
        exif.set_container_images(container_images);
        match parser.continue_on_error {
            Some(v) if !v.is_empty() =>
                Err(Error::PartialResult(PartialResult::new(exif, v))),
            _ => Ok(exif),
        }
    }

    /// Parses the Exif attributes from raw Exif data with optional MPF data.
    /// If an error occurred, `exif::Error` is returned.
    #[cfg(feature = "mpf")]
    fn read_raw_with_mpf(&self, exif_data: Vec<u8>, mpf_data: Option<Vec<u8>>, mpf_app2_offset: u64, extra: Vec<Field>, tiff_base: u64, container_images: Vec<(u64, u32)>) -> Result<Exif, Error> {
        let mut parser = tiff::Parser::new();
        parser.continue_on_error = self.continue_on_error.then(|| Vec::new());
        parser.parse(&exif_data)?;
        parser.entries.extend(extra.into_iter().map(IfdEntry::from_parsed_field));

        let mut exif = if let Some(mut mpf_buf) = mpf_data {
            // Parse MPF data and convert offsets using MPF module
            crate::mpf::parse_mpf(&mut mpf_buf, mpf_app2_offset)?;

            // Create Exif with MPF data
            Exif::new_with_mpf(exif_data, parser.entries, parser.little_endian,
                               mpf_buf, mpf_app2_offset, tiff_base)
        } else {
            Exif::new_at(exif_data, parser.entries, parser.little_endian, tiff_base)
        };
        exif.set_container_images(container_images);

        match parser.continue_on_error {
            Some(v) if !v.is_empty() =>
                Err(Error::PartialResult(PartialResult::new(exif, v))),
            _ => Ok(exif),
        }
    }

    /// Parses the Exif attributes from raw Exif data.
    /// If an error occurred, `exif::Error` is returned.
    ///
    /// The buffers are anonymous, so the first is read as IFD0 and every other
    /// one as an Exif IFD. A CR3's boxes go through `read_from_container`,
    /// where each one's name decides how it is read.
    pub fn read_raw_vec(&self, buffers: Vec<Vec<u8>>) -> Result<Exif, Error> {
        let named = buffers
            .into_iter()
            .enumerate()
            .map(|(i, b)| (if i == 0 { *b"CMT1" } else { *b"CMT2" }, b))
            .collect();
        self.read_raw_vec_with(named, Vec::new())
    }

    /// `read_raw_vec`, plus images the CONTAINER addresses that no tag does.
    fn read_raw_vec_with(&self, boxes: Vec<isobmff::crx::CmtBox>, previews: Vec<(u64, u32)>)
                         -> Result<Exif, Error> {
        use crate::tag::Context;

        let mut data = Vec::new();
        let mut parser = tiff::Parser::new();
        parser.continue_on_error = self.continue_on_error.then(|| Vec::new());
        // Join all buffers together
        for (_, buffer) in &boxes {
            data.extend_from_slice(buffer);
        }
        // Each box is a whole TIFF, and only its NAME says which IFD it is.
        #[cfg(feature = "make_note")]
        let mut maker_note = None;
        let mut offset = 0;
        for (name, buffer) in &boxes {
            let range = offset..offset + buffer.len();
            let context = match name {
                b"CMT1" => Some(Context::Tiff),
                b"CMT4" => Some(Context::Gps),
                // CMT3 is the Canon MakerNote, which is not an Exif IFD.
                #[cfg(feature = "make_note")]
                b"CMT3" => None,
                _ => Some(Context::Exif),
            };
            match context {
                Some(ctx) => parser.parse_with_context_offset(&data[range], ctx, offset as u32)?,
                #[cfg(feature = "make_note")]
                None => {
                    let parsed = crate::make_note::parse_tiff_make_note(
                        &data[range], crate::make_note::maker_tag::MakerNoteVendor::Canon);
                    maker_note = Some((offset as u32, parsed));
                }
                #[cfg(not(feature = "make_note"))]
                None => {}
            }
            offset += buffer.len();
        }
        let mut exif = Exif::new(data, parser.entries, parser.little_endian);
        #[cfg(feature = "make_note")]
        if let Some((at, parsed)) = maker_note {
            exif.set_container_maker_note(
                crate::make_note::maker_tag::MakerNoteVendor::Canon, at, parsed);
        }
        exif.set_container_images(previews);
        match parser.continue_on_error {
            Some(v) if !v.is_empty() =>
                Err(Error::PartialResult(PartialResult::new(exif, v))),
            _ => Ok(exif),
        }
    }

    /// Reads an image file and parses the Exif attributes in it.
    /// If an error occurred, `exif::Error` is returned.
    ///
    /// Supported formats are:
    /// - TIFF and some RAW image formats based on it
    /// - JPEG
    /// - HEIF and coding-specific variations including HEIC and AVIF
    /// - PNG
    /// - WebP
    /// - Fujifilm RAF (the Exif comes from its embedded JPEG)
    ///
    /// This method is provided for the convenience even though
    /// parsing containers is basically out of the scope of this library.
    pub fn read_from_container<R>(&self, reader: &mut R) -> Result<Exif, Error>
    where
        R: io::BufRead + io::Seek,
    {
        let mut buf = Vec::new();
        reader.by_ref().take(4096).read_to_end(&mut buf)?;

        // A RAF's Exif comes from its embedded JPEG and therefore describes
        // the JPEG. What the RAW image measures is stated only in the RAF's
        // own CFA header, so it is carried out separately and attached below.
        let mut raf_raw_image: Option<raf::RafRawImage> = None;
        let mut mrw_raw_image: Option<mrw::MrwRawImage> = None;
        let mut x3f_fields: Vec<Field> = Vec::new();
        // Images the container addresses directly, in FILE offsets.
        let mut container_images: Vec<(u64, u32)> = Vec::new();
        // Where the TIFF header these fields describe sits in the file. It is
        // a property of the CONTAINER, so each branch below states it.
        let mut tiff_base: u64 = 0;

        #[cfg(feature = "mpf")]
        let mut mpf_data: Option<Vec<u8>> = None;
        #[cfg(feature = "mpf")]
        let mut mpf_app2_offset: u64 = 0;

        if tiff::is_tiff(&buf) {
            // The buffer IS the file, so offsets are already file offsets.
            reader.read_to_end(&mut buf)?;
        } else if jpeg::is_jpeg(&buf) {
            #[cfg(feature = "mpf")]
            {
                let segments = jpeg::get_exif_and_mpf_sub(&mut buf.chain(reader))?;
                buf = segments.exif_data;
                mpf_data = segments.mpf_data;
                mpf_app2_offset = segments.mpf_app2_offset;
            }
            #[cfg(not(feature = "mpf"))]
            {
                buf = jpeg::get_exif_attr(&mut buf.chain(reader))?;
            }
            tiff_base = crate::exifimpl::JPEG_TIFF_BASE;
        } else if png::is_png(&buf) {
            buf = png::get_exif_attr(&mut buf.chain(reader))?;
        } else if isobmff::is_heif(&buf) {
            reader.seek(io::SeekFrom::Start(0))?;
            buf = isobmff::get_exif_attr(reader)?;
        } else if isobmff::crx::is_crx(&buf) {
            reader.seek(io::SeekFrom::Start(0))?;
            // A CR3's embedded JPEGs are in boxes, addressed by nothing in the
            // Exif -- so they are collected here, where the file is still in
            // hand, and attached below.
            let previews = isobmff::crx::preview_boxes(reader).unwrap_or_default();
            reader.seek(io::SeekFrom::Start(0))?;
            let buf_vec = isobmff::crx::get_exif_attr_vec(reader)?;
            return self.read_raw_vec_with(buf_vec, previews);
        } else if raf::is_raf(&buf) {
            reader.seek(io::SeekFrom::Start(0))?;
            let raf = raf::get_exif_and_raw_image(reader)?;
            buf = raf.exif;
            raf_raw_image = raf.raw_image;
            // The Exif belongs to a JPEG embedded partway into the file, so
            // its offsets need the JPEG's position as well as the JPEG's own
            // header size.
            tiff_base = raf.jpeg_offset + crate::exifimpl::JPEG_TIFF_BASE;
            // The embedded JPEG is the body's full-size rendering, addressed
            // by the RAF header and by no tag at all -- without this a Fuji
            // raw offers only its 8.8 KB IFD1 thumbnail.
            container_images.push((raf.jpeg_offset, raf.jpeg_length));
        } else if mrw::is_mrw(&buf) {
            reader.seek(io::SeekFrom::Start(0))?;
            let parsed = mrw::get_exif_attr(reader)?;
            buf = parsed.exif;
            mrw_raw_image = parsed.raw_image;
            // The TIFF sits inside a block partway into the file, so every
            // offset in it counts from where that block begins.
            tiff_base = parsed.tiff_offset;
        } else if crw::is_crw(&buf) {
            reader.seek(io::SeekFrom::Start(0))?;
            let c = crw::get_contents(reader)?;
            for img in [c.preview, c.thumbnail].into_iter().flatten() {
                container_images.push(img);
            }
            // **A CIFF holds no TIFF and its preview holds no Exif** -- the
            // embedded JPEG starts `FF D8 FF DB`, a quantisation table, with
            // no APP1 anywhere. So the parser is given the smallest valid
            // TIFF and every field arrives synthesised, as an X3F's do.
            x3f_fields = c.fields;
            buf = x3f::EMPTY_TIFF.to_vec();
        } else if x3f::is_x3f(&buf) {
            reader.seek(io::SeekFrom::Start(0))?;
            let x = x3f::get_contents(reader)?;
            if let Some(p) = x.preview {
                container_images.push(p);
            }
            x3f_fields = x3f::synthesize(&x.props, x.frame);
            // **The Quattro generation dropped `PROP`.** Its preview carries
            // an ordinary Exif block instead, so it is read the way a RAF's
            // is -- the offsets inside it count from the JPEG's own header.
            // Older bodies have no such block and rely on the synthesised
            // fields alone, which is why both paths exist.
            match x.exif_jpeg {
                Some((at, _)) => {
                    reader.seek(io::SeekFrom::Start(at))?;
                    match jpeg::get_exif_attr(reader) {
                        Ok(tiff) => {
                            buf = tiff;
                            tiff_base = at + crate::exifimpl::JPEG_TIFF_BASE;
                        }
                        // A JPEG that claims an APP1 and has no usable Exif
                        // is not a reason to lose the rest of the file.
                        Err(_) => buf = x3f::EMPTY_TIFF.to_vec(),
                    }
                }
                // An X3F holds no TIFF at all, so the parser is given the
                // smallest valid one and every field arrives synthesised.
                None => buf = x3f::EMPTY_TIFF.to_vec(),
            }
        } else if webp::is_webp(&buf) {
            buf = webp::get_exif_attr(&mut buf.chain(reader))?;
        } else {
            return Err(Error::InvalidFormat("Unknown image format"));
        }

        #[cfg(feature = "mpf")]
        {
            self.read_raw_with_mpf(buf, mpf_data, mpf_app2_offset,
                                   { let mut extra = raf_sub_image(raf_raw_image); extra.extend(mrw_sub_image(mrw_raw_image)); extra.extend(x3f_fields); extra }, tiff_base,
                                   container_images)
        }
        #[cfg(not(feature = "mpf"))]
        {
            self.read_raw_with_extra(buf, { let mut extra = raf_sub_image(raf_raw_image); extra.extend(mrw_sub_image(mrw_raw_image)); extra.extend(x3f_fields); extra }, tiff_base,
                                     container_images)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::BufReader;
    use crate::tag::{Context, Tag};
    use crate::tiff::{Field, In};
    use crate::value::Value;
    use super::*;

    #[test]
    fn yaminabe() {
        let file = File::open("tests/yaminabe.tif").unwrap();
        let be = Reader::new().read_from_container(
            &mut BufReader::new(&file)).unwrap();
        let file = File::open("tests/yaminale.tif").unwrap();
        let le = Reader::new().read_from_container(
            &mut BufReader::new(&file)).unwrap();
        assert!(!be.little_endian());
        assert!(le.little_endian());
        for exif in &[be, le] {
            assert_eq!(exif.fields().len(), 26);
            let f = exif.get_field(Tag::ImageWidth, In(0)).unwrap();
            assert_eq!(f.display_value().to_string(), "17");
            let f = exif.get_field(Tag::Humidity, In(0)).unwrap();
            assert_eq!(f.display_value().to_string(), "65");
            let f = exif.get_field(Tag(Context::Tiff, 65000), In(0)).unwrap();
            match f.value {
                Value::Float(ref v) => assert_eq!(v[0], std::f32::MIN),
                _ => panic!(),
            }
            let f = exif.get_field(Tag(Context::Tiff, 65001), In(0)).unwrap();
            match f.value {
                Value::Double(ref v) => assert_eq!(v[0], std::f64::MIN),
                _ => panic!(),
            }
        }
    }

    #[test]
    fn heif() {
        let file = std::fs::File::open("tests/exif.heic").unwrap();
        let exif = Reader::new().read_from_container(
            &mut std::io::BufReader::new(&file)).unwrap();
        assert_eq!(exif.fields().len(), 2);
        let exifver = exif.get_field(Tag::ExifVersion, In::PRIMARY).unwrap();
        assert_eq!(exifver.display_value().to_string(), "2.31");
    }

    #[test]
    fn png() {
        let file = std::fs::File::open("tests/exif.png").unwrap();
        let exif = Reader::new().read_from_container(
            &mut std::io::BufReader::new(&file)).unwrap();
        assert_eq!(exif.fields().len(), 6);
        let exifver = exif.get_field(Tag::ExifVersion, In::PRIMARY).unwrap();
        assert_eq!(exifver.display_value().to_string(), "2.32");
    }

    #[test]
    fn webp() {
        let file = std::fs::File::open("tests/exif.webp").unwrap();
        let exif = Reader::new().read_from_container(
            &mut std::io::BufReader::new(&file)).unwrap();
        assert_eq!(exif.fields().len(), 6);
        let exifver = exif.get_field(Tag::ExifVersion, In::PRIMARY).unwrap();
        assert_eq!(exifver.display_value().to_string(), "2.32");
        let desc = exif.get_field(Tag::ImageDescription, In::PRIMARY).unwrap();
        assert_eq!(desc.display_value().to_string(), "\"WebP test\"");
    }

    #[test]
    fn continue_on_error() {
        let data = b"MM\0\x2a\0\0\0\x08\
                     \0\x02\x01\x00\0\x03\0\0\0\x01\0\x14\0\0\
                           \x01\x01\0\x03\0\0\0\x01\0\x15\0";
        let result = Reader::new()
            .continue_on_error(true)
            .read_raw(data.to_vec());
        if let Err(Error::PartialResult(partial)) = result {
            let (exif, errors) = partial.into_inner();
            assert_pat!(exif.fields().collect::<Vec<_>>().as_slice(),
                        [Field { tag: Tag::ImageWidth, ifd_num: In(0),
                                 value: Value::Short(_) }]);
            assert_pat!(&errors[..], [Error::InvalidFormat("Truncated IFD")]);
        } else {
            panic!("partial result expected");
        }
    }

    /// A little-endian TIFF holding one IFD of SHORT entries, each one value.
    fn tiff_le(entries: &[(u16, u16)]) -> Vec<u8> {
        let mut t = b"II\x2a\0\x08\0\0\0".to_vec();
        t.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for &(tag, value) in entries {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&3u16.to_le_bytes());
            t.extend_from_slice(&1u32.to_le_bytes());
            t.extend_from_slice(&value.to_le_bytes());
            t.extend_from_slice(&[0, 0]);
        }
        t.extend_from_slice(&[0, 0, 0, 0]);
        t
    }

    /// **A CR3's boxes are read by name**: `CMT3` is the Canon MakerNote and
    /// `CMT4` the GPS IFD, and neither is an Exif IFD. Read as one, Canon's
    /// `ColorSpace` (0x00B4) became an unnamed Exif tag with no vendor, and
    /// GPSVersionID (0x0000) an unnamed Exif tag too.
    #[cfg(feature = "make_note")]
    #[test]
    fn cr3_boxes_by_name() {
        use crate::make_note::maker_tag::{MakerNoteVendor, MakerTag};

        let boxes = vec![
            (*b"CMT1", tiff_le(&[(0x0112, 1)])),   // Orientation
            (*b"CMT2", tiff_le(&[(0xa001, 0xffff)])), // ColorSpace
            (*b"CMT3", tiff_le(&[(0x00b4, 2)])),   // Canon ColorSpace
            (*b"CMT4", tiff_le(&[(0x0005, 0)])),   // GPSAltitudeRef
        ];
        let exif = Reader::new().read_raw_vec_with(boxes, Vec::new()).unwrap();

        assert_eq!(exif.maker_note_vendor().ok(), Some(MakerNoteVendor::Canon));
        let colour = exif
            .get_maker_note_field(&MakerTag::new(MakerNoteVendor::Canon, 0x00b4))
            .expect("Canon ColorSpace as a maker-note field");
        assert_eq!(colour.value.get_uint(0), Some(2));
        assert!(exif.fields().all(|f| f.tag.number() != 0x00b4),
                "the MakerNote must not also arrive as plain Exif");

        assert_eq!(exif.get_field(Tag::Orientation, In::PRIMARY)
                       .and_then(|f| f.value.get_uint(0)), Some(1));
        assert_eq!(exif.get_field(Tag::ColorSpace, In::PRIMARY)
                       .and_then(|f| f.value.get_uint(0)), Some(0xffff));
        assert!(exif.get_field(Tag::GPSAltitudeRef, In::PRIMARY).is_some());
    }

    /// **An `AOC\0` Pentax MakerNote**: a 6-byte header, and value offsets
    /// counted from the TIFF header. Read as a `PENTAX \0II` block (10 bytes,
    /// MakerNote-relative) the IFD started four bytes late and every field
    /// after it was noise.
    #[cfg(feature = "make_note")]
    #[test]
    fn pentax_aoc_maker_note() {
        use crate::make_note::maker_tag::{MakerNoteVendor, MakerTag};

        let mut t = Vec::new();
        t.extend_from_slice(b"MM\0\x2a\0\0\0\x08");
        // IFD0 at 8: Make (out of line at 38) and the Exif IFD pointer (46).
        t.extend_from_slice(&[0, 2]);
        t.extend_from_slice(&[0x01, 0x0f, 0, 2, 0, 0, 0, 7, 0, 0, 0, 38]);
        t.extend_from_slice(&[0x87, 0x69, 0, 4, 0, 0, 0, 1, 0, 0, 0, 46]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        t.extend_from_slice(b"PENTAX\0\0"); // 38..46, padded
        // Exif IFD at 46: MakerNote, 42 bytes at 64.
        t.extend_from_slice(&[0, 1]);
        t.extend_from_slice(&[0x92, 0x7c, 0, 7, 0, 0, 0, 42, 0, 0, 0, 64]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 64);
        // MakerNote at 64: "AOC\0MM", then its IFD.
        t.extend_from_slice(b"AOC\0MM");
        t.extend_from_slice(&[0, 2]);
        // 0x0037 ColorSpace = 1, inline.
        t.extend_from_slice(&[0x00, 0x37, 0, 3, 0, 0, 0, 1, 0, 1, 0, 0]);
        // 0x0002, three SHORTs out of line at TIFF offset 100.
        t.extend_from_slice(&[0x00, 0x02, 0, 3, 0, 0, 0, 3, 0, 0, 0, 100]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 100);
        t.extend_from_slice(&[0, 10, 0, 20, 0, 30]);

        let exif = Reader::new().read_raw(t).unwrap();
        assert_eq!(exif.maker_note_vendor().ok(), Some(MakerNoteVendor::Pentax));
        let field = |n| exif.get_maker_note_field(&MakerTag::new(MakerNoteVendor::Pentax, n));
        assert_eq!(field(0x0037).and_then(|f| f.value.get_uint(0)), Some(1));
        let size = field(0x0002).expect("the out-of-line value resolves");
        assert_eq!((0..3).map(|i| size.value.get_uint(i)).collect::<Vec<_>>(),
                   vec![Some(10), Some(20), Some(30)]);
        assert_eq!(exif.maker_note_fields().count(), 2);
    }

    /// **An old `OLYMP\0` MakerNote**: offsets counted from the TIFF header,
    /// and each sub-directory an UNDEFINED block that is the IFD itself. The
    /// shape an E-1 writes; read as MakerNote-relative pointers, the E-1 lost
    /// every one of its MakerNote fields.
    #[cfg(feature = "make_note")]
    #[test]
    fn olympus_old_format_maker_note() {
        use crate::make_note::maker_tag::{MakerNoteVendor, MakerTag};

        let mut t = Vec::new();
        t.extend_from_slice(b"II\x2a\0\x08\0\0\0");
        // IFD0 at 8: Make (out of line at 38) and the Exif IFD pointer (46).
        t.extend_from_slice(&[2, 0]);
        t.extend_from_slice(&[0x0f, 0x01, 2, 0, 8, 0, 0, 0, 38, 0, 0, 0]);
        t.extend_from_slice(&[0x69, 0x87, 4, 0, 1, 0, 0, 0, 46, 0, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        t.extend_from_slice(b"OLYMPUS\0"); // 38..46
        // Exif IFD at 46: MakerNote, 122 bytes at 64.
        t.extend_from_slice(&[1, 0]);
        t.extend_from_slice(&[0x7c, 0x92, 7, 0, 122, 0, 0, 0, 64, 0, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 64);
        // MakerNote at 64: "OLYMP\0" + version, then its IFD.
        t.extend_from_slice(b"OLYMP\0\x01\0");
        t.extend_from_slice(&[3, 0]);
        // 0x0200, three LONGs out of line at TIFF offset 114.
        t.extend_from_slice(&[0x00, 0x02, 4, 0, 3, 0, 0, 0, 114, 0, 0, 0]);
        // 0x2010 Equipment, an 18-byte UNDEFINED block at TIFF offset 126.
        t.extend_from_slice(&[0x10, 0x20, 7, 0, 18, 0, 0, 0, 126, 0, 0, 0]);
        // 0x2020 CameraSettings, a 42-byte UNDEFINED block at TIFF offset 144.
        t.extend_from_slice(&[0x20, 0x20, 7, 0, 42, 0, 0, 0, 144, 0, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 114);
        t.extend_from_slice(&[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
        // Equipment: 0x0100 CameraType2, six ASCII bytes OUTSIDE the MakerNote.
        t.extend_from_slice(&[1, 0]);
        t.extend_from_slice(&[0x00, 0x01, 2, 0, 6, 0, 0, 0, 0x88, 0x13, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 144);
        // CameraSettings: 0x0507 ColorSpace = 1, and a preview at TIFF offset
        // 200, 4 bytes long.
        t.extend_from_slice(&[3, 0]);
        t.extend_from_slice(&[0x07, 0x05, 3, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
        t.extend_from_slice(&[0x01, 0x01, 4, 0, 1, 0, 0, 0, 200, 0, 0, 0]);
        t.extend_from_slice(&[0x02, 0x01, 4, 0, 1, 0, 0, 0, 4, 0, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 186);
        t.resize(200, 0);
        t.extend_from_slice(&[0xff, 0xd8, 0xff, 0xd9]);

        let exif = Reader::new().read_raw(t).unwrap();
        assert_eq!(exif.maker_note_vendor().ok(), Some(MakerNoteVendor::Olympus));
        let mode = exif
            .get_maker_note_field(&MakerTag::new(MakerNoteVendor::Olympus, 0x0200))
            .expect("the out-of-line value resolves from the TIFF header");
        assert_eq!((0..3).map(|i| mode.value.get_uint(i)).collect::<Vec<_>>(),
                   vec![Some(1), Some(2), Some(3)]);
        let colour = exif
            .get_maker_note_field(&MakerTag::new(MakerNoteVendor::OlympusCameraSettings, 0x0507))
            .expect("the CameraSettings block is parsed as an IFD");
        assert_eq!(colour.value.get_uint(0), Some(1));

        use crate::subimg::EmbeddedSubImageSource;
        let images = exif.embedded_images();
        assert!(images.iter().any(|i| i.source == EmbeddedSubImageSource::MakerNotePreview3
                                    && i.offset == 200 && i.length == 4),
                "the preview address counts from the TIFF header: {images:?}");
        assert!(images.iter().all(|i| i.source != EmbeddedSubImageSource::MakerNoteValue),
                "Equipment's 0x0100 is CameraType2, not the thumbnail: {images:?}");
    }

    /// A little-endian TIFF whose IFD0 holds `Make` and a `DNGPrivateData`
    /// BYTE value of `private`.
    #[cfg(feature = "make_note")]
    fn dng_with_private(make: &[u8], private: &[u8]) -> Vec<u8> {
        let make_at = 38u32;
        let private_at = make_at + make.len() as u32;
        let mut t = b"II\x2a\0\x08\0\0\0".to_vec();
        t.extend_from_slice(&[2, 0]);
        t.extend_from_slice(&[0x0f, 0x01, 2, 0]);
        t.extend_from_slice(&(make.len() as u32).to_le_bytes());
        t.extend_from_slice(&make_at.to_le_bytes());
        t.extend_from_slice(&[0x34, 0xc6, 1, 0]);
        t.extend_from_slice(&(private.len() as u32).to_le_bytes());
        t.extend_from_slice(&private_at.to_le_bytes());
        t.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(t.len(), 38);
        t.extend_from_slice(make);
        t.extend_from_slice(private);
        t
    }

    /// **A DNG converter's MakerNote copy**: `DNGPrivateData` = `Adobe\0` and
    /// named blocks; `MakN` states the original byte order and the MakerNote's
    /// offset in the original file, which its value offsets still count from.
    #[cfg(feature = "make_note")]
    #[test]
    fn dng_adobe_makn_maker_note() {
        use crate::make_note::maker_tag::{MakerNoteVendor, MakerTag};

        // A big-endian Canon MakerNote that sat at offset 1000 of the original:
        // 0x00B4 ColorSpace = 2 inline, 0x0002 three SHORTs at original 1030.
        let mut note = vec![0, 2];
        note.extend_from_slice(&[0x00, 0xb4, 0, 3, 0, 0, 0, 1, 0, 2, 0, 0]);
        note.extend_from_slice(&[0x00, 0x02, 0, 3, 0, 0, 0, 3, 0, 0, 0x04, 0x06]);
        note.extend_from_slice(&[0, 0, 0, 0]);
        note.extend_from_slice(&[0, 10, 0, 20, 0, 30]);
        let mut makn = b"MM".to_vec();
        makn.extend_from_slice(&1000u32.to_be_bytes());
        makn.extend_from_slice(&note);

        let mut private = b"Adobe\0".to_vec();
        private.extend_from_slice(b"Junk");
        private.extend_from_slice(&4u32.to_be_bytes());
        private.extend_from_slice(&[9, 9, 9, 9]);
        private.extend_from_slice(b"MakN");
        private.extend_from_slice(&(makn.len() as u32).to_be_bytes());
        private.extend_from_slice(&makn);

        let exif = Reader::new().read_raw(dng_with_private(b"Canon\0", &private)).unwrap();
        assert_eq!(exif.maker_note_vendor().ok(), Some(MakerNoteVendor::Canon));
        let field = |n| exif.get_maker_note_field(&MakerTag::new(MakerNoteVendor::Canon, n));
        assert_eq!(field(0x00b4).and_then(|f| f.value.get_uint(0)), Some(2),
                   "read in the byte order MakN states");
        let values = field(0x0002).expect("offsets corrected by the original position");
        assert_eq!((0..3).map(|i| values.value.get_uint(i)).collect::<Vec<_>>(),
                   vec![Some(10), Some(20), Some(30)]);
    }

    /// **An in-camera DNG's MakerNote** is the vendor block itself, header
    /// first. Big-endian `PENTAX \0MM` also pins the byte-order table: its
    /// signature carries a space, and without it the block was read
    /// little-endian (1 came back as 256).
    #[cfg(feature = "make_note")]
    #[test]
    fn dng_in_camera_pentax_maker_note() {
        use crate::make_note::maker_tag::{MakerNoteVendor, MakerTag};

        let mut private = b"PENTAX \0MM".to_vec();
        private.extend_from_slice(&[0, 1]);
        private.extend_from_slice(&[0x00, 0x37, 0, 3, 0, 0, 0, 1, 0, 1, 0, 0]);
        private.extend_from_slice(&[0, 0, 0, 0]);

        let exif = Reader::new().read_raw(dng_with_private(b"PENTAX\0", &private)).unwrap();
        assert_eq!(exif.maker_note_vendor().ok(), Some(MakerNoteVendor::Pentax));
        let colour = exif
            .get_maker_note_field(&MakerTag::new(MakerNoteVendor::Pentax, 0x0037))
            .expect("Pentax ColorSpace");
        assert_eq!(colour.value.get_uint(0), Some(1));
    }
}

/// The RAF's raw image, as ordinary sub-image fields.
/// `PRD`'s geometry, in the same shape -- see [`raf_sub_image`].
fn mrw_sub_image(raw: Option<mrw::MrwRawImage>) -> Vec<Field> {
    raw.map_or_else(Vec::new, |r| vec![
        Field { tag: Tag::ImageWidth, ifd_num: In::SUB_IMAGE,
                value: Value::Long(vec![r.width]) },
        Field { tag: Tag::ImageLength, ifd_num: In::SUB_IMAGE,
                value: Value::Long(vec![r.height]) },
    ])
}

fn raf_sub_image(raw: Option<raf::RafRawImage>) -> Vec<Field> {
    raw.map_or_else(Vec::new, |r| vec![
        Field { tag: Tag::ImageWidth, ifd_num: In::SUB_IMAGE,
                value: Value::Long(vec![r.width]) },
        Field { tag: Tag::ImageLength, ifd_num: In::SUB_IMAGE,
                value: Value::Long(vec![r.height]) },
    ])
}
