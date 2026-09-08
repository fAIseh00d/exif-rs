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
    pub fn read_raw_vec(&self, buffers: Vec<Vec<u8>>) -> Result<Exif, Error> {
        self.read_raw_vec_with(buffers, Vec::new())
    }

    /// `read_raw_vec`, plus images the CONTAINER addresses that no tag does.
    fn read_raw_vec_with(&self, buffers: Vec<Vec<u8>>, previews: Vec<(u64, u32)>)
                         -> Result<Exif, Error> {
        let mut data = Vec::new();
        let mut parser = tiff::Parser::new();
        parser.continue_on_error = self.continue_on_error.then(|| Vec::new());
        // Join all buffers together
        for buffer in &buffers {
            data.extend_from_slice(buffer);
        }
        let mut offset = 0;
        for (idx, buffer) in buffers.iter().enumerate() {
            let default_context = match idx {
                0 => crate::tag::Context::Tiff,
                _ => crate::tag::Context::Exif,
            };
            parser.parse_with_context_offset(&data[offset..offset + buffer.len()], default_context, offset as u32)?;
            offset += buffer.len();
        }
        let mut exif = Exif::new(data, parser.entries, parser.little_endian);
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
        } else if webp::is_webp(&buf) {
            buf = webp::get_exif_attr(&mut buf.chain(reader))?;
        } else {
            return Err(Error::InvalidFormat("Unknown image format"));
        }

        #[cfg(feature = "mpf")]
        {
            self.read_raw_with_mpf(buf, mpf_data, mpf_app2_offset,
                                   raf_sub_image(raf_raw_image), tiff_base,
                                   container_images)
        }
        #[cfg(not(feature = "mpf"))]
        {
            self.read_raw_with_extra(buf, raf_sub_image(raf_raw_image), tiff_base,
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
}

/// The RAF's raw image, as ordinary sub-image fields.
fn raf_sub_image(raw: Option<raf::RafRawImage>) -> Vec<Field> {
    raw.map_or_else(Vec::new, |r| vec![
        Field { tag: Tag::ImageWidth, ifd_num: In::SUB_IMAGE,
                value: Value::Long(vec![r.width]) },
        Field { tag: Tag::ImageLength, ifd_num: In::SUB_IMAGE,
                value: Value::Long(vec![r.height]) },
    ])
}
