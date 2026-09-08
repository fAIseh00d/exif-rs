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

use std::collections::HashMap;

use crate::tag::Tag;
use crate::tiff::{Field, IfdEntry, In, ProvideUnit};
use crate::subimg::{EmbeddedSubImage, EmbeddedSubImageSource};
#[cfg(feature = "make_note")]
use crate::make_note::maker_tag::{MakerNoteField, MakerNoteVendor, MakerTag};
#[cfg(feature = "mpf")]
use crate::mpf::mpf_tag::{MpfField, MpfTag};

/// Where the TIFF header sits in an ordinary JPEG: SOI (2) + APP1 marker (2) +
/// length (2) + `"Exif\0\0"` (6).
///
/// **An assumption, and a documented one**: it holds when APP1 is the first
/// segment, which is the overwhelming majority, but a file with a JFIF APP0
/// ahead of it puts the header further in. Tracking the segment's real
/// position through the JPEG reader is the proper fix.
pub(crate) const JPEG_TIFF_BASE: u64 = 12;

/// A struct that holds the parsed Exif attributes.
///
/// # Examples
/// ```
/// # fn main() { sub(); }
/// # fn sub() -> Option<()> {
/// # use exif::{In, Reader, Tag};
/// # let file = std::fs::File::open("tests/exif.jpg").unwrap();
/// # let exif = Reader::new().read_from_container(
/// #     &mut std::io::BufReader::new(&file)).unwrap();
/// // Get a specific field.
/// let xres = exif.get_field(Tag::XResolution, In::PRIMARY)?;
/// assert_eq!(xres.display_value().with_unit(&exif).to_string(),
///            "72 pixels per inch");
/// // Iterate over all fields.
/// for f in exif.fields() {
///     println!("{} {} {}", f.tag, f.ifd_num, f.display_value());
/// }
/// # Some(()) }
/// ```
pub struct Exif {
    // TIFF data.
    buf: Vec<u8>,
    // Exif fields.  Vec is used to keep the ability to enumerate all fields
    // even if there are duplicates.
    entries: Vec<IfdEntry>,
    // HashMap to the index of the Vec for faster random access.
    entry_map: HashMap<(In, Tag), usize>,
    // True if the TIFF data is little endian.
    little_endian: bool,
    // Where this TIFF header sits IN THE FILE.
    //
    // Every offset inside a TIFF is relative to its header, and that header is
    // not always at the start of the file: it is the file itself for a TIFF or
    // a raw, 12 bytes in for an ordinary JPEG (SOI + APP1 marker + length +
    // "Exif\0\0"), and further still for a RAF, whose Exif belongs to a JPEG
    // embedded at some offset of its own. Without it an offset-valued tag can
    // only be reported relative to nothing.
    tiff_base: u64,
    // MakerNote fields parsed by vendor-specific parser.
    // HashMap for quick access by (vendor, tag_number).
    #[cfg(feature = "make_note")]
    maker_note_fields: HashMap<MakerTag, MakerNoteField>,
    // MakerNote vendor detected from the data, or error if not found.
    #[cfg(feature = "make_note")]
    maker_note_vendor: Result<MakerNoteVendor, crate::Error>,
    // Where the MakerNote's own data begins, relative to the TIFF header.
    //
    // A MakerNote is a container inside a container, and the same rule applies
    // one level deeper: **offsets it states count from ITS start, not the
    // TIFF's.** An Olympus E-M5 II puts `PreviewImageStart` at 48652 with the
    // MakerNote beginning 3572 bytes into the file -- 52224, where the JPEG
    // actually is.
    #[cfg(feature = "make_note")]
    maker_note_offset: u32,
    // Images the CONTAINER addresses that no tag does — a CR3 keeps its
    // thumbnail and preview in boxes, so nothing in the Exif can find them.
    container_images: Vec<(u64, u32)>,
    // MPF fields parsed from APP2 segment.
    // HashMap for quick access by tag number.
    #[cfg(feature = "mpf")]
    mpf_fields: HashMap<MpfTag, MpfField>,
}

impl Exif {
    /// Constructs a new `Exif`.
    pub(crate) fn new(buf: Vec<u8>,
                      entries: Vec<IfdEntry>, little_endian: bool) -> Self {
        Self::new_at(buf, entries, little_endian, 0)
    }

    /// As [`Self::new`], with the file offset of the TIFF header.
    pub(crate) fn new_at(buf: Vec<u8>, entries: Vec<IfdEntry>,
                         little_endian: bool, tiff_base: u64) -> Self {
        let entry_map = entries.iter().enumerate()
            .map(|(i, e)| (e.ifd_num_tag(), i)).collect();

        // Try to parse MakerNote if present
        #[cfg(feature = "make_note")]
        let (maker_note_fields, maker_note_vendor, maker_note_offset) =
            Self::parse_maker_note_internal(&buf, &entries, little_endian);

        Self {
            buf: buf,
            entries: entries,
            entry_map: entry_map,
            little_endian: little_endian,
            tiff_base,
            #[cfg(feature = "make_note")]
            maker_note_fields,
            #[cfg(feature = "make_note")]
            maker_note_vendor,
            #[cfg(feature = "make_note")]
            maker_note_offset,
            container_images: Vec::new(),
            #[cfg(feature = "mpf")]
            mpf_fields: HashMap::new(),
        }
    }

    /// Record images the container addresses directly, in FILE offsets.
    ///
    /// These do not go through [`Self::file_offset`]: a box tree states
    /// absolute positions, not offsets from a TIFF header that a CR3 does not
    /// even have at the file's start.
    pub(crate) fn set_container_images(&mut self, images: Vec<(u64, u32)>) {
        self.container_images = images;
    }

    /// Where an offset stated INSIDE the TIFF lands in the file.
    ///
    /// Offsets in `JPEGInterchangeFormat`, `StripOffsets`, `PreviewImageStart`
    /// and friends are all relative to the TIFF header, so a caller that wants
    /// to seek to one has to be told where that header is.
    #[must_use]
    pub fn file_offset(&self, tiff_offset: u64) -> u64 {
        self.tiff_base.saturating_add(tiff_offset)
    }

    /// The file offset of the TIFF header these fields are relative to.
    #[must_use]
    pub fn tiff_base(&self) -> u64 {
        self.tiff_base
    }

    /// Constructs a new `Exif` with MPF data.
    #[cfg(feature = "mpf")]
    pub(crate) fn new_with_mpf(
        buf: Vec<u8>,
        entries: Vec<IfdEntry>,
        little_endian: bool,
        mpf_buf: Vec<u8>,
        mpf_app2_offset: u64,
        tiff_base: u64,
    ) -> Self {
        let entry_map = entries.iter().enumerate()
            .map(|(i, e)| (e.ifd_num_tag(), i)).collect();

        // Try to parse MakerNote if present
        #[cfg(feature = "make_note")]
        let (maker_note_fields, maker_note_vendor, maker_note_offset) =
            Self::parse_maker_note_internal(&buf, &entries, little_endian);

        // Parse MPF fields
        let mpf_fields = Self::parse_mpf_internal(&mpf_buf, mpf_app2_offset, little_endian);

        Self {
            buf,
            entries,
            entry_map,
            little_endian,
            tiff_base,
            #[cfg(feature = "make_note")]
            maker_note_fields,
            #[cfg(feature = "make_note")]
            maker_note_vendor,
            #[cfg(feature = "make_note")]
            maker_note_offset,
            container_images: Vec::new(),
            mpf_fields,
        }
    }

    /// Internal helper to parse MPF data
    #[cfg(feature = "mpf")]
    fn parse_mpf_internal(
        mpf_buf: &[u8],
        _mpf_app2_offset: u64,
        little_endian: bool,
    ) -> HashMap<MpfTag, MpfField> {
        // Parse TIFF structure in MPF buffer
        let mut parser = crate::tiff::Parser::new();
        if parser.parse(mpf_buf).is_err() {
            return HashMap::new();
        }

        // Convert IfdEntries to MpfFields
        let mut mpf_fields = HashMap::new();
        for entry in parser.entries {
            let field = entry.into_field(mpf_buf, little_endian);
            let mpf_tag = MpfTag(field.tag.number());
            mpf_fields.insert(mpf_tag, MpfField::new(mpf_tag, field.value));
        }

        mpf_fields
    }

    /// Internal helper to parse MakerNote data
    #[cfg(feature = "make_note")]
    fn parse_maker_note_internal(
        buf: &[u8],
        entries: &[IfdEntry],
        little_endian: bool,
    ) -> (HashMap<MakerTag, MakerNoteField>, Result<MakerNoteVendor, crate::Error>, u32) {
        // Find MakerNote field
        let maker_note_entry = entries.iter()
            .find(|e| e.ifd_num_tag().1 == Tag::MakerNote);

        let Some(maker_note_entry) = maker_note_entry else {
            return (HashMap::new(), Err(crate::Error::MakerNoteNotFound), 0);
        };

        // Get MakerNote field value
        let field = maker_note_entry.ref_field(buf, little_endian);
        let crate::value::Value::Undefined(ref data, offset) = field.value else {
            return (HashMap::new(), Err(crate::Error::MakerNoteNotFound), 0);
        };

        // Get Make field for vendor detection
        let make = entries.iter()
            .find(|e| e.ifd_num_tag().1 == Tag::Make)
            .and_then(|e| {
                let field = e.ref_field(buf, little_endian);
                if let crate::value::Value::Ascii(ref vec) = field.value {
                    vec.first()
                        .and_then(|s| std::str::from_utf8(s).ok())
                } else {
                    None
                }
            });

        // Parse MakerNote with vendor detection
        #[cfg(feature = "make_note")]
        {
            match crate::make_note::parse_make_note_with_vendor(data, offset, make) {
                Ok((fields, vendor, _le)) => {
                    let map = fields.into_iter()
                        .map(|f| (f.tag, f))
                        .collect();
                    (map, Ok(vendor), offset)
                }
                Err(e) => (HashMap::new(), Err(e), offset)
            }
        }
        #[cfg(not(feature = "make_note"))]
        {
            (HashMap::new(), Err(crate::Error::NotFound("MakerNote parsing disabled")), 0)
        }
    }

    /// Returns the slice that contains the TIFF data.
    #[inline]
    pub fn buf(&self) -> &[u8] {
        &self.buf[..]
    }

    /// Returns ownership of the TIFF data.
    #[inline]
    pub fn take(self) -> Vec<u8> {
        self.buf
    }

    /// Returns an iterator of Exif fields.
    #[inline]
    pub fn fields(&self) -> impl ExactSizeIterator<Item = &Field> {
        self.entries.iter()
            .map(move |e| e.ref_field(&self.buf, self.little_endian))
    }

    /// Returns true if the Exif data (TIFF structure) is in the
    /// little-endian byte order.
    #[inline]
    pub fn little_endian(&self) -> bool {
        self.little_endian
    }

    /// Returns a reference to the Exif field specified by the tag
    /// and the IFD number.
    #[inline]
    pub fn get_field(&self, tag: Tag, ifd_num: In) -> Option<&Field> {
        self.entry_map.get(&(ifd_num, tag))
            .map(|&i| self.entries[i].ref_field(&self.buf, self.little_endian))
    }

    /// Returns a reference to the MPF field specified by the tag.
    ///
    /// Only available when the `mpf` feature is enabled.
    #[cfg(feature = "mpf")]
    #[inline]
    pub fn get_mpf_field(&self, tag: MpfTag) -> Option<&MpfField> {
        self.mpf_fields.get(&tag)
    }

    /// Returns information about embedded sub-images (thumbnails and previews).
    ///
    /// This method returns metadata for sub-images embedded in the Exif data:
    /// - IFD1 thumbnail (JPEG format, typically 10-20 KB)
    /// - MakerNote preview images (if `make_note` feature is enabled and vendor supports it)
    /// - MPF (Multi-Picture Format) images (if `mpf` feature is enabled and data exists)
    ///
    /// The returned offsets are relative to the start of the TIFF data (Exif segment),
    /// not the file start. For JPEG files, you need to account for the APP1 marker offset.
    ///
    /// # Examples
    /// ```no_run
    /// # use exif::Reader;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let file = std::fs::File::open("image.jpg")?;
    /// let exif = Reader::new().read_from_container(
    ///     &mut std::io::BufReader::new(&file))?;
    ///
    /// for img_info in exif.thumbnails() {
    ///     println!("Source: {}, Size: {} bytes, Offset: {}",
    ///              img_info.source.name(), img_info.length, img_info.offset);
    /// }
    /// # Ok(()) }
    /// ```
    /// Every image embedded in the file, with where it is and what it is.
    ///
    /// **Renamed from `thumbnails`, which had stopped being true.** The name
    /// fitted when only IFD1 was read; it now returns a Sony ARW's 7.3 MB
    /// full-resolution JPEG and, on a Nikon NEF, the undemosaiced sensor
    /// mosaic. Calling those thumbnails would mislead exactly the caller who
    /// most needs to tell them apart.
    ///
    /// Sources, none of which a single tag would find: `JPEGInterchangeFormat`
    /// in any IFD, an IFD's own `StripOffsets`, a vendor MakerNote, MPF, and
    /// the container's boxes (a CR3's `PRVW` and `THMB`).
    ///
    /// **This enumerates and chooses nothing.** Which image a caller wants —
    /// biggest, big enough for a screen, or the full-resolution one — differs
    /// per product, so [`EmbeddedSubImage`] reports the facts to choose by:
    /// dimensions, `subfile_type`, `compression` and `photometric`.
    pub fn embedded_images(&self) -> Vec<EmbeddedSubImage> {
        let mut images = Vec::new();

        // **Every IFD, not just IFD1.** `JPEGInterchangeFormat` (0x0201) and
        // its length (0x0202) are ONE tag pair that appears in several IFDs,
        // and the familiar names are labels for the IFD rather than different
        // tags: IFD1 is what everyone calls the thumbnail, IFD0 the preview, a
        // further chained IFD or a sub-image the full-size JPEG. A Sony ARW
        // carries all three at once -- 10 KB in IFD1, 539 KB in IFD0 and
        // **7.3 MB in IFD2** -- so reading IFD1 alone finds the smallest.
        //
        // Each one is reported with the IFD that stated it and with that IFD's
        // own dimensions where present, because "which of these is the
        // photograph" is a question the caller has to be able to answer.
        let chained = (0..8u16).map(In);
        let sub_images = (0..8u16).map(|i| In(In::SUB_IMAGE.0 + i));
        for ifd in chained.chain(sub_images) {
            let u = |tag| self.get_field(tag, ifd).and_then(|f| f.value.get_uint(0));
            let u16_of = |tag| u(tag).and_then(|v| u16::try_from(v).ok());

            // An IFD that IS an image addresses itself with StripOffsets,
            // which is how a DNG stores its preview -- 1280x964 JPEG on a
            // Pixel, 256x171 uncompressed RGB on an Adobe conversion.
            //
            // Only a SINGLE strip is reported. A multi-strip image is not one
            // contiguous run of bytes, so an offset and a length cannot
            // describe it, and answering with the first strip would be a
            // plausible-looking lie. Previews are written as one strip in
            // practice (`RowsPerStrip` covers the full height).
            //
            // Dimensions here come from the IFD and are AUTHORITATIVE, unlike
            // the JPEGInterchangeFormat case: this IFD is the image, rather
            // than merely pointing at one.
            if let (Some(strip), Some(count)) =
                (self.get_field(Tag::StripOffsets, ifd),
                 self.get_field(Tag::StripByteCounts, ifd))
            {
                let single = strip.value.get_uint(1).is_none()
                    && count.value.get_uint(1).is_none();
                if let (true, Some(offset), Some(length)) =
                    (single, strip.value.get_uint(0), count.value.get_uint(0))
                {
                    // **`0xFFFFFFFF` is a sentinel, not an address.** Panasonic
                    // writes it in a RW2's StripOffsets and states the real
                    // position in its own `RawDataOffset` (0x0118) instead.
                    // Reporting it would hand a caller an offset four
                    // gigabytes into a file that is not that long.
                    if length > 0 && offset != u32::MAX {
                        images.push(EmbeddedSubImage {
                            source: EmbeddedSubImageSource::IfdStrip,
                            length,
                            offset: self.file_offset(u64::from(offset)),
                            subfile_type: u(Tag::NewSubfileType),
                            compression: u16_of(Tag::Compression),
                            photometric: u16_of(Tag::PhotometricInterpretation),
                            ifd: Some(ifd),
                            width: u(Tag::ImageWidth),
                            height: u(Tag::ImageLength),
                        });
                    }
                }
            }

            let (Some(offset_field), Some(length_field)) = (
                self.get_field(Tag::JPEGInterchangeFormat, ifd),
                self.get_field(Tag::JPEGInterchangeFormatLength, ifd),
            ) else {
                continue;
            };
            let (Some(tiff_offset), Some(length)) =
                (offset_field.value.get_uint(0), length_field.value.get_uint(0))
            else {
                continue;
            };
            if length == 0 {
                continue;
            }
            // **The IFD's own dimensions are NOT this image's.** An IFD
            // describes ITS image, and the JPEG it addresses is a different
            // one: IFD0 of a Sony ARW measures 6192x4128 -- the raw -- while
            // the preview it points at is 1616x1080. Only IFD1 is genuinely
            // about its thumbnail, and even there the JPEG's own header is the
            // better authority. So nothing is stated here; `dimensions()`
            // reads the SOF, which is about the image actually present.
            images.push(EmbeddedSubImage {
                source: if ifd == In::THUMBNAIL {
                    EmbeddedSubImageSource::Thumbnail
                } else {
                    EmbeddedSubImageSource::IfdImage
                },
                length,
                offset: self.file_offset(u64::from(tiff_offset)),
                subfile_type: None,
                compression: None,
                photometric: None,
                ifd: Some(ifd),
                width: None,
                height: None,
            });
        }

        // Panasonic does not ADDRESS its preview at all -- it stores the JPEG
        // as the VALUE of an IFD0 tag, so there is no offset tag to follow and
        // `StripOffsets` holds a 0xFFFFFFFF sentinel. Every RW2 body checked
        // carries one, 335 KB to 926 KB, 1920x1280 on a DC-S5.
        //
        // The value's own position in the TIFF is the image's position, since
        // a value this size is stored out of line.
        const PANASONIC_JPG_FROM_RAW: u16 = 0x002e;
        if let Some(f) = self.fields().find(|f| {
            f.tag.number() == PANASONIC_JPG_FROM_RAW && f.ifd_num == In::PRIMARY
        }) {
            if let crate::value::Value::Undefined(ref data, offset) = f.value {
                // Verify rather than assume: 0x002E means nothing in baseline
                // TIFF, so another dialect is free to use it for anything.
                if data.starts_with(&[0xFF, 0xD8]) {
                    if let Ok(length) = u32::try_from(data.len()) {
                        images.push(EmbeddedSubImage {
                            source: EmbeddedSubImageSource::IfdStrip,
                            length,
                            offset: self.file_offset(u64::from(offset)),
                            subfile_type: None,
                            compression: None,
                            photometric: None,
                            ifd: Some(In::PRIMARY),
                            width: None,
                            height: None,
                        });
                    }
                }
            }
        }

        for &(offset, length) in &self.container_images {
            images.push(EmbeddedSubImage {
                source: EmbeddedSubImageSource::ContainerBox,
                length,
                offset,
                subfile_type: None,
                compression: None,
                photometric: None,
                ifd: None,
                width: None,
                height: None,
            });
        }

        // Try to get MakerNote preview images
        #[cfg(feature = "make_note")]
        {
            let maker_note_images = crate::subimg::extract_maker_note_preview_info(
                &self.maker_note_fields,
                &self.maker_note_vendor,
            );
            images.extend(maker_note_images);

            // Olympus keeps its full-size preview in the CameraSettings
            // subdirectory rather than the MakerNote's top level -- 975 KB on
            // an E-M5 II, against an 8 KB thumbnail. `extract_maker_note_
            // preview_info` reads the two top-level variants and left this one
            // as a TODO because the subdirectory was not parsed then; it is
            // now, so the fields are simply there to be read.
            //
            // **Its offsets count from the MakerNote's own start**, not the
            // TIFF header: 48652 stated, with the MakerNote 3572 bytes into
            // the file, is 52224 -- which is where the SOI actually is.
            const OLYMPUS_CS_PREVIEW_START: u16 = 0x0101;
            const OLYMPUS_CS_PREVIEW_LENGTH: u16 = 0x0102;
            let cs = |number: u16| {
                self.maker_note_fields.values().find(|f| {
                    f.tag.number() == number
                        && f.tag.vendor() == MakerNoteVendor::OlympusCameraSettings
                })
            };
            if let (Some(start), Some(len)) =
                (cs(OLYMPUS_CS_PREVIEW_START), cs(OLYMPUS_CS_PREVIEW_LENGTH))
            {
                if let (Some(start), Some(len)) =
                    (start.value.get_uint(0), len.value.get_uint(0))
                {
                    if len > 0 {
                        images.push(EmbeddedSubImage {
                            source: EmbeddedSubImageSource::MakerNotePreview3,
                            length: len,
                            offset: self.file_offset(u64::from(
                                self.maker_note_offset.saturating_add(start))),
                            subfile_type: None,
                            compression: None,
                            photometric: None,
                            ifd: None,
                            width: None,
                            height: None,
                        });
                    }
                }
            }
        }

        // Try to get MPF images from MPF fields
        #[cfg(feature = "mpf")]
        {
            // MPEntry tag (0xb002) contains image metadata with absolute offsets
            if let Some(mp_entry_field) = self.get_mpf_field(MpfTag::MPEntry) {
                if let crate::value::Value::Undefined(ref data, ..) = mp_entry_field.value {
                    // Parse MPEntry data using the MPF parser
                    let entries = crate::mpf::mpf_tag::parse_mp_entry(data, self.little_endian);

                    for entry in entries.iter() {
                        if entry.image_size > 0 {
                            // Determine image classification from ImageAttr field
                            // - is_primary(): BaselinePrimary type WITHOUT representative flag (e.g., RAW/TIFF data)
                            // - is_representative(): Has representative flag (e.g., JPEG preview)
                            // - Others: MPF images (panorama, multi-frame, etc.)
                            if !entry.is_primary() && (entry.is_representative() || entry.is_thumbnail()) {
                                // Representative preview or other MPF images
                                images.push(EmbeddedSubImage::new_mpf(entry.image_size, entry.image_data_offset));
                            }
                        }
                    }
                }
            }
        }

        images
    }

    /// Every embedded image.
    ///
    /// Kept so existing callers still build; it never returned only
    /// thumbnails once more than IFD1 was read.
    #[deprecated(note = "renamed to `embedded_images`, which is what it returns")]
    pub fn thumbnails(&self) -> Vec<EmbeddedSubImage> {
        self.embedded_images()
    }

    /// Returns metadata for the primary image described in the MPF (APP2) segment.
    ///
    /// This method extracts information about the primary image entry from the
    /// MPF (Multi-Picture Format) data, if present. The MPF `MPEntry` tag contains
    /// image metadata including absolute offsets and lengths for each embedded image.
    ///
    /// The returned image corresponds to the MPF entry marked as the primary image.
    /// Note that this is distinct from the Exif primary image (APP1) and represents
    /// the MPF-defined main image when multiple images are present.
    ///
    /// The returned offset is relative to the start of the JPEG file (SOI),
    /// as defined by the MPF specification.
    ///
    /// # Examples
    /// ```no_run
    /// # use exif::Reader;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let file = std::fs::File::open("image.jpg")?;
    /// let exif = Reader::new().read_from_container(
    ///     &mut std::io::BufReader::new(&file))?;
    ///
    /// if let Some(img) = exif.primary_image() {
    ///     println!(
    ///         "Primary MPF image: size={} bytes, offset={}",
    ///         img.length, img.offset
    ///     );
    /// }
    /// # Ok(()) }
    /// ```
    #[cfg(feature = "mpf")]
    pub fn primary_image(&self) -> Option<EmbeddedSubImage> {
        self.get_mpf_field(MpfTag::MPEntry)
            .and_then(|field| {
                if let crate::value::Value::Undefined(ref data, ..) = field.value {
                    crate::mpf::mpf_tag::parse_mp_entry(data, self.little_endian)
                        .iter()
                        .find(|e| e.is_primary())
                        .map(|e| {
                            EmbeddedSubImage::new_primary(
                                e.image_size,
                                e.image_data_offset,
                            )
                        })
                } else {
                    None
                }
            })
    }


    /// Returns the detected MakerNote vendor.
    ///
    /// Returns `Ok(vendor)` if a MakerNote was found and the vendor was detected,
    /// or `Err(Error::MakerNoteNotFound)` if no MakerNote field exists,
    /// or another error if parsing failed.
    #[inline]
    #[cfg(feature = "make_note")]
    pub fn maker_note_vendor(&self) -> Result<MakerNoteVendor, &crate::Error> {
        self.maker_note_vendor.as_ref().copied()
    }

    /// Returns a reference to a MakerNote field by tag.
    ///
    /// # Arguments
    /// * `tag` - The MakerTag identifying the vendor-specific field
    ///
    /// # Examples
    /// ```
    /// # use exif::{Reader, make_note::maker_tag::MakerTag};
    /// # fn main() { let _ = example(); }
    /// # fn example() -> Option<()> {
    /// # let file = std::fs::File::open("tests/exif.jpg").unwrap();
    /// # let exif = Reader::new().read_from_container(
    /// #     &mut std::io::BufReader::new(&file)).unwrap();
    /// // Get a specific MakerNote field
    /// let vendor = exif.maker_note_vendor().ok()?;
    /// let tag = MakerTag::new(vendor, 0x0002);
    /// let field = exif.get_maker_note_field(&tag)?;
    /// println!("{}: {}", field.tag, field.display_value());
    /// # Some(()) }
    /// ```
    #[inline]
    #[cfg(feature = "make_note")]
    pub fn get_maker_note_field(&self, tag: &MakerTag) -> Option<&MakerNoteField> {
        self.maker_note_fields.get(tag)
    }

    /// Returns an iterator over all parsed MakerNote fields.
    ///
    /// # Examples
    /// ```
    /// # fn main() { sub(); }
    /// # fn sub() -> Option<()> {
    /// # use exif::Reader;
    /// # let file = std::fs::File::open("tests/exif.jpg").unwrap();
    /// # let exif = Reader::new().read_from_container(
    /// #     &mut std::io::BufReader::new(&file)).unwrap();
    /// // Iterate over all MakerNote fields
    /// for field in exif.maker_note_fields() {
    ///     println!("{}: {}", field.tag, field.display_value());
    /// }
    /// # Some(()) }
    /// ```
    #[inline]
    #[cfg(feature = "make_note")]
    pub fn maker_note_fields(&self) -> impl Iterator<Item = &MakerNoteField> {
        self.maker_note_fields.values()
    }
}

impl<'a> ProvideUnit<'a> for &'a Exif {
    fn get_field(self, tag: Tag, ifd_num: In) -> Option<&'a Field> {
        self.get_field(tag, ifd_num)
    }
}

#[cfg(test)]
mod tests {
    use crate::value::Value;
    use std::fs::File;
    use std::io::BufReader;
    use crate::reader::Reader;
    
/// Where the TIFF header sits in an ordinary JPEG: SOI (2) + APP1 marker (2) +
/// length (2) + `"Exif\0\0"` (6).
///
/// **An assumption, and a documented one**: it holds when APP1 is the first
/// segment, which is the overwhelming majority, but a file with a JFIF APP0
/// ahead of it puts the header further in. Tracking the segment's real
/// position through the JPEG reader is the proper fix.
pub(crate) const JPEG_TIFF_BASE: u64 = 12;
    use super::*;

    #[test]
    fn get_field() {
        let file = File::open("tests/yaminabe.tif").unwrap();
        let exif = Reader::new().read_from_container(
            &mut BufReader::new(&file)).unwrap();
        match exif.get_field(Tag::ImageDescription, In(0)).unwrap().value {
            Value::Ascii(ref vec) => assert_eq!(vec, &[b"Test image"]),
            ref v => panic!("wrong variant {:?}", v)
        }
        match exif.get_field(Tag::ImageDescription, In(1)).unwrap().value {
            Value::Ascii(ref vec) => assert_eq!(vec, &[b"Test thumbnail"]),
            ref v => panic!("wrong variant {:?}", v)
        }
        match exif.get_field(Tag::ImageDescription, In(2)).unwrap().value {
            Value::Ascii(ref vec) => assert_eq!(vec, &[b"Test 2nd IFD"]),
            ref v => panic!("wrong variant {:?}", v)
        }
    }

    #[test]
    fn display_value_with_unit() {
        let file = File::open("tests/yaminabe.tif").unwrap();
        let exif = Reader::new().read_from_container(
            &mut BufReader::new(&file)).unwrap();
        // No unit.
        let exifver = exif.get_field(Tag::ExifVersion, In::PRIMARY).unwrap();
        assert_eq!(exifver.display_value().with_unit(&exif).to_string(),
                   "2.31");
        // Fixed string.
        let width = exif.get_field(Tag::ImageWidth, In::PRIMARY).unwrap();
        assert_eq!(width.display_value().with_unit(&exif).to_string(),
                   "17 pixels");
        // Unit tag (with a non-default value).
        let gpsalt = exif.get_field(Tag::GPSAltitude, In::PRIMARY).unwrap();
        assert_eq!(gpsalt.display_value().with_unit(&exif).to_string(),
                   "0.5 meters below sea level");
        // Unit tag is missing but the default is specified.
        let xres = exif.get_field(Tag::XResolution, In::PRIMARY).unwrap();
        assert_eq!(xres.display_value().with_unit(&exif).to_string(),
                   "72 pixels per inch");
        // Unit tag is missing and the default is not specified.
        let gpslat = exif.get_field(Tag::GPSLatitude, In::PRIMARY).unwrap();
        assert_eq!(gpslat.display_value().with_unit(&exif).to_string(),
                   "10 deg 0 min 0 sec [GPSLatitudeRef missing]");
    }
}
