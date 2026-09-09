//
// Copyright (c) 2025 Jinwoo Park (pmnxis@gmail.com).
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

use std::io::{BufRead, Seek};

use crate::error::Error;
#[cfg(feature = "make_note")]
use crate::make_note::maker_tag::{MakerNoteVendor, MakerTag};
#[cfg(feature = "make_note")]
use crate::value::Value;

/// Source of an embedded sub-image (thumbnail or preview)
///
/// This enum identifies where a sub-image was extracted from within the file.
/// Explicit discriminant values ensure compatibility across versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EmbeddedSubImageSource {
    /// TIFF original primary image
    Primary = 0,

    /// IFD1 thumbnail image (always available)
    Thumbnail = 1,

    /// MPF (Multi-Picture Format) embedded image
    #[cfg(feature = "mpf")]
    Mpf = 2,

    /// MakerNote preview image 1 (Olympus: PreviewImageStart 0x0088 / PreviewImageLength 0x0089)
    #[cfg(feature = "make_note")]
    MakerNotePreview1 = 3,

    /// MakerNote preview image 2 (Olympus: PreviewImageStart2 0x1036 / PreviewImageLength2 0x1037)
    #[cfg(feature = "make_note")]
    MakerNotePreview2 = 4,

    /// MakerNote preview image 3 (Olympus: PreviewImageStart3 CameraSettings 0x0101 / PreviewImageLength3 0x0102)
    #[cfg(feature = "make_note")]
    MakerNotePreview3 = 5,

    /// A JPEG addressed by `JPEGInterchangeFormat` in an IFD other than IFD1 —
    /// which is where a raw keeps its real preview.
    ///
    /// **They are all the same tag.** `0x0201`/`0x0202` in IFD1 is what
    /// everyone calls the thumbnail, in IFD0 the preview, in a further chained
    /// IFD or a sub-image the full-size JPEG; the names are labels for the IFD
    /// rather than distinct tags. A Sony ARW carries all three at once — 10 KB
    /// in IFD1, 539 KB in IFD0 and **7.3 MB in IFD2** — so looking only at
    /// IFD1 finds the smallest of them.
    IfdImage = 6,

    /// An image an IFD holds directly, addressed by its `StripOffsets` —
    /// which is how a DNG stores its preview.
    ///
    /// **Not necessarily a JPEG.** A Canon DNG's is an uncompressed RGB
    /// bitmap; a Pixel's is a JPEG. `compression` says which.
    IfdStrip = 7,

    /// An image the CONTAINER addresses rather than any tag — a CR3's `PRVW`
    /// and `THMB` boxes. Its offset is already a file offset.
    ContainerBox = 8,

    /// An image a MakerNote addresses OUTSIDE its own block.
    ///
    /// The pre-2008 Olympus format stores an 11 KB thumbnail as tag `0x0100`
    /// at a TIFF-relative offset, while the MakerNote itself is under a
    /// kilobyte — so the parser, which only ever sees its own bytes, cannot
    /// resolve it. It reports the address and this is what it becomes.
    MakerNoteValue = 9,
}

impl EmbeddedSubImageSource {
    /// Get a human-readable name for the image source
    pub fn name(&self) -> &'static str {
        match self {
            EmbeddedSubImageSource::Primary => "Primary",
            EmbeddedSubImageSource::Thumbnail => "Thumbnail",
            #[cfg(feature = "mpf")]
            EmbeddedSubImageSource::Mpf => "MPF",
            #[cfg(feature = "make_note")]
            EmbeddedSubImageSource::MakerNotePreview1 => "MakerNotePreview1",
            #[cfg(feature = "make_note")]
            EmbeddedSubImageSource::MakerNotePreview2 => "MakerNotePreview2",
            #[cfg(feature = "make_note")]
            EmbeddedSubImageSource::MakerNotePreview3 => "MakerNotePreview3",
            EmbeddedSubImageSource::IfdImage => "IfdImage",
            EmbeddedSubImageSource::IfdStrip => "IfdStrip",
            EmbeddedSubImageSource::ContainerBox => "ContainerBox",
            EmbeddedSubImageSource::MakerNoteValue => "MakerNoteValue",
        }
    }
}

/// Information about an embedded sub-image
///
/// This structure provides unified metadata for sub-images from various sources
/// (IFD1 thumbnail, MPF, or MakerNote). The actual image data can be
/// extracted on demand using `extract_data()`.
#[derive(Debug, Clone, Copy)]
pub struct EmbeddedSubImage {
    /// Source of this sub-image
    pub source: EmbeddedSubImageSource,

    /// Size of the image in bytes
    pub length: u32,

    /// Absolute offset from the start of the file
    pub offset: u64,

    /// Which IFD stated it, when one did — `In::PRIMARY` for a preview,
    /// `In::THUMBNAIL` for a thumbnail, `In::SUB_IMAGE`+n for a sub-image.
    pub ifd: Option<crate::tiff::In>,

    /// `Compression` (0x0103) as the file states it: 1 uncompressed, 6 or 7
    /// JPEG. **An embedded image is not always a JPEG** -- a Canon DNG's
    /// preview is a raw RGB bitmap -- so a caller that assumes one will
    /// mis-handle it.
    pub compression: Option<u16>,

    /// `NewSubfileType` (0x00FE), the standard answer to *what is this
    /// image*: bit 0 set means a reduced-resolution version of another image,
    /// i.e. a preview or thumbnail; 0 means the full-resolution one.
    ///
    /// **More reliable than guessing from the other two.** An Olympus ORF
    /// declares its sensor data `BlackIsZero` and `Uncompressed` -- which
    /// describes a perfectly ordinary grayscale bitmap, so neither
    /// `photometric` nor `compression` reveals that it is raw. Its size does,
    /// and so does this.
    pub subfile_type: Option<u32>,

    /// `PhotometricInterpretation` (0x0106): 2 RGB, 6 YCbCr, **32803 CFA**.
    ///
    /// The CFA case is why this is reported. A raw's full-resolution
    /// sub-image is addressed exactly like a preview and is *not a picture* --
    /// it is the undemosaiced sensor mosaic. Showing it to someone expecting a
    /// preview would produce a grey-green grid, so a viewer needs to be able
    /// to tell them apart.
    pub photometric: Option<u16>,

    /// The image's own dimensions, **where the file states them**.
    ///
    /// This is the field that lets a caller tell one embedded image from
    /// another. Without it "the dimensions" of a raw is ambiguous, and picking
    /// the wrong answer is exactly the bug this crate had: a DNG's IFD0 is a
    /// 256x171 thumbnail, so `ImageWidth` there is the thumbnail's size, not
    /// the photograph's. `None` means the file did not say — the JPEG's own
    /// SOF marker would, but reading it is a decode-side concern.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl EmbeddedSubImage {
    /// Create a new EmbeddedSubImage for a primary image
    pub fn new_primary(length: u32, offset: u64) -> Self {
        Self {
            source: EmbeddedSubImageSource::Primary,
            length,
            offset,
            subfile_type: None,
            compression: None,
            photometric: None,
            ifd: None,
            width: None,
            height: None,
        }
    }

    /// Create a new EmbeddedSubImage for a thumbnail
    pub fn new_thumbnail(length: u32, offset: u64) -> Self {
        Self {
            source: EmbeddedSubImageSource::Thumbnail,
            length,
            offset,
            subfile_type: None,
            compression: None,
            photometric: None,
            ifd: None,
            width: None,
            height: None,
        }
    }

    /// Create a new EmbeddedSubImage for an MPF image
    #[cfg(feature = "mpf")]
    pub fn new_mpf(length: u32, offset: u64) -> Self {
        Self {
            source: EmbeddedSubImageSource::Mpf,
            length,
            offset,
            subfile_type: None,
            compression: None,
            photometric: None,
            ifd: None,
            width: None,
            height: None,
        }
    }

    /// Create a new EmbeddedSubImage for a MakerNote preview image
    #[cfg(feature = "make_note")]
    pub fn new_maker_note_preview(
        source: EmbeddedSubImageSource,
        length: u32,
        offset: u64,
    ) -> Self {
        Self {
            source,
            length,
            offset,
            subfile_type: None,
            compression: None,
            photometric: None,
            ifd: None,
            width: None,
            height: None,
        }
    }

    /// The image's dimensions, reading the JPEG's own header when the file
    /// did not state them.
    ///
    /// [`Self::width`] and [`Self::height`] carry what the CONTAINER said, and
    /// most formats say nothing: a Sony ARW addresses three JPEGs and sizes
    /// none of them. Those dimensions are in each JPEG's `SOF` marker, so this
    /// walks its segment chain — **a few short reads near the start of the
    /// image, never the whole thing.** A 7.3 MB preview costs the same as a
    /// 10 KB thumbnail.
    ///
    /// This is what makes an embedded image identifiable rather than merely
    /// present. Without it a caller choosing "one big enough for this screen"
    /// has only the byte length, and compression makes that a poor proxy — a
    /// detailed thumbnail can outweigh a smooth preview of ten times the area.
    ///
    /// Returns the stated dimensions unread when they are already known.
    pub fn dimensions<R>(&self, reader: &mut R) -> Result<(u32, u32), Error>
    where
        R: BufRead + Seek,
    {
        if let (Some(w), Some(h)) = (self.width, self.height) {
            return Ok((w, h));
        }
        use std::io::SeekFrom;
        reader.seek(SeekFrom::Start(self.offset))?;
        let mut head = [0u8; 4];
        reader.read_exact(&mut head)?;
        if !is_jpeg_start(&head) {
            return Err(Error::InvalidFormat("Embedded image is not a JPEG"));
        }
        reader.seek(SeekFrom::Start(self.offset + 2))?;
        // Walk the marker chain to a frame header. Bounded by the image's own
        // declared length so a malformed chain cannot read past it.
        let mut pos: u64 = 2;
        let mut byte = [0u8; 1];
        while pos + 4 <= u64::from(self.length) {
            reader.read_exact(&mut byte)?;
            pos += 1;
            if byte[0] != 0xFF {
                return Err(Error::InvalidFormat("Broken JPEG segment chain"));
            }
            // **Any number of 0xFF fill bytes may precede a marker**, so the
            // id is the first byte after them that is not itself 0xFF. Taking
            // the byte straight after the first 0xFF reads a pad as the marker
            // and the real marker as a segment length, and the walk desyncs.
            loop {
                reader.read_exact(&mut byte)?;
                pos += 1;
                if byte[0] != 0xFF {
                    break;
                }
            }
            let id = byte[0];
            // **Standalone markers carry no length field.** Reading two bytes
            // after one takes image content for a segment length.
            if id == 0x01 || (0xD0..=0xD8).contains(&id) {
                continue;
            }
            if id == 0xD9 {
                return Err(Error::NotFound("no frame header in embedded image"));
            }
            // **Past the start of scan there are no more segments.** Entropy
            // data is not a marker chain, and walking it as one can match a
            // plausible frame header inside compressed bytes -- a confident
            // wrong size, which is worse than an error.
            if id == 0xDA {
                return Err(Error::NotFound("no frame header before scan"));
            }
            // SOF0..SOF15 carry the size; C4/C8/CC are not frame headers.
            let is_sof = (0xC0..=0xCF).contains(&id)
                && !matches!(id, 0xC4 | 0xC8 | 0xCC);
            let mut len_be = [0u8; 2];
            reader.read_exact(&mut len_be)?;
            pos += 2;
            let seg_len = u64::from(u16::from_be_bytes(len_be));
            if seg_len < 2 {
                return Err(Error::InvalidFormat("Broken JPEG segment length"));
            }
            if is_sof {
                // precision(1) height(2) width(2)
                let mut sof = [0u8; 5];
                reader.read_exact(&mut sof)?;
                let h = u32::from(u16::from_be_bytes([sof[1], sof[2]]));
                let w = u32::from(u16::from_be_bytes([sof[3], sof[4]]));
                return (w > 0 && h > 0)
                    .then_some((w, h))
                    .ok_or(Error::InvalidFormat("Zero-sized JPEG frame"));
            }
            reader.seek(SeekFrom::Current(seg_len as i64 - 2))?;
            pos += seg_len - 2;
        }
        Err(Error::NotFound("no frame header in embedded image"))
    }

    /// Extract the image data from a reader
    ///
    /// # Arguments
    /// * `reader` - A reader positioned at the start of the file
    ///
    /// # Returns
    /// The raw image data as a Vec<u8>
    pub fn extract_data<R>(&self, reader: &mut R) -> Result<Vec<u8>, Error>
    where
        R: BufRead + Seek,
    {
        use std::io::{Read, SeekFrom};

        // Seek to the image start position
        reader.seek(SeekFrom::Start(self.offset))?;

        // **`length` is stated by the file, so it is not a size to trust.** A
        // malformed or hostile header asks for up to 4 GB before a single byte
        // is read, and `vec![0; n]` commits to all of it up front. Growing the
        // buffer from the reader instead costs only what is really there, and
        // a header longer than the file becomes an error rather than a
        // four-gigabyte allocation.
        let mut data = Vec::new();
        reader.by_ref().take(u64::from(self.length)).read_to_end(&mut data)?;
        if data.len() as u64 != u64::from(self.length) {
            return Err(Error::InvalidFormat("Embedded image is truncated"));
        }

        // **Restore the SOI byte a Minolta body zeroed**, and only that one.
        // The bytes are already ours -- a copy the caller decodes -- and
        // handing back an image that cannot open, when the file says it is a
        // JPEG and the very next marker confirms it, reports a broken address
        // rather than a working one. exiftool does the same repair.
        //
        // Deliberately narrow: `?? D8 FF ??` and nothing else, so a file that
        // really is not a JPEG is still returned untouched.
        if data.first() != Some(&0xFF) && is_jpeg_start(&data) {
            data[0] = 0xFF;
        }

        Ok(data)
    }

    /// Extract and save the image data to a file
    ///
    /// # Arguments
    /// * `reader` - A reader positioned at the start of the file
    /// * `path` - The file path to save the image data
    ///
    /// # Returns
    /// The number of bytes written
    ///
    /// # Examples
    /// ```no_run
    /// # use exif::Reader;
    /// # use std::fs::File;
    /// # use std::io::BufReader;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let file = File::open("image.jpg")?;
    /// let mut reader = BufReader::new(&file);
    /// let exif = Reader::new().read_from_container(&mut reader)?;
    ///
    /// for img in exif.embedded_images() {
    ///     let mut file = File::open("image.jpg")?;
    ///     let mut reader = BufReader::new(&file);
    ///     img.save_to_file(&mut reader, format!("thumbnail_{}.jpg", img.source.name()))?;
    /// }
    /// # Ok(()) }
    /// ```
    pub fn save_to_file<R, P>(&self, reader: &mut R, path: P) -> Result<usize, Error>
    where
        R: BufRead + Seek,
        P: AsRef<std::path::Path>,
    {
        let data = self.extract_data(reader)?;
        std::fs::write(path, &data)?;
        Ok(data.len())
    }
}

/// Helper function to extract preview image from MakerNote tag pairs
#[cfg(feature = "make_note")]
/// Is this the start of a JPEG, INCLUDING the one Minolta writes?
///
/// **A Minolta body clobbers the SOI's first byte.** Measured across the 13
/// corpus MRWs: 12 state a MakerNote preview and only ONE -- a DiMAGE A200 --
/// begins `FF D8`. The DSLRs (Dynax and Maxxum 7D, Dynax 5D, Alpha-7 and
/// Alpha Sweet) begin `02 D8` and the DiMAGE compacts `00 D8`, with a valid
/// `FF DB` or `FF C4` immediately after in every case. exiftool patches the
/// byte back to `FF` on the way out, which is what makes its extracted
/// previews decode where a byte-exact copy does not.
///
/// So the shape is checked rather than the first byte: `?? D8` followed by a
/// real marker introducer. Anything else is not a JPEG and is refused as
/// before.
fn is_jpeg_start(head: &[u8]) -> bool {
    // `.get(..4)`, because a bare slice pattern matches a slice of exactly
    // that length -- so testing a whole image against it silently never
    // matches, and the repair below quietly does nothing.
    matches!(head.get(..4), Some([_, 0xD8, 0xFF, _]))
}

fn try_extract_preview(
    maker_note_fields: &std::collections::HashMap<
        MakerTag,
        crate::make_note::maker_tag::MakerNoteField,
    >,
    start_tag: &MakerTag,
    length_tag: &MakerTag,
    source: EmbeddedSubImageSource,
) -> Option<EmbeddedSubImage> {
    let start_field = maker_note_fields.get(start_tag)?;
    let length_field = maker_note_fields.get(length_tag)?;

    if let (Value::Long(ref start_val), Value::Long(ref length_val)) =
        (&start_field.value, &length_field.value)
    {
        if !start_val.is_empty() && !length_val.is_empty() {
            let offset = start_val[0] as u64;
            let length = length_val[0];
            if length > 0 {
                return Some(EmbeddedSubImage::new_maker_note_preview(
                    source, length, offset,
                ));
            }
        }
    }
    None
}

/// Extract preview image info from MakerNote data
///
/// Currently supports:
/// - Olympus: PreviewImageStart/PreviewImageLength tags (3 variants)
/// - Minolta: the same two tags, which Olympus inherited from it
///
/// **The offsets are TIFF-relative and are returned that way.** The caller
/// adds the TIFF header's own position, which is zero for an ORF and 48 or
/// 140 for an MRW -- where the TIFF lives inside a `\0TTW` block.
#[cfg(feature = "make_note")]
pub(crate) fn extract_maker_note_preview_info(
    maker_note_fields: &std::collections::HashMap<
        MakerTag,
        crate::make_note::maker_tag::MakerNoteField,
    >,
    maker_note_vendor: &Result<MakerNoteVendor, Error>,
) -> Vec<EmbeddedSubImage> {
    let mut images = Vec::new();
    let vendor = match maker_note_vendor.as_ref() {
        Ok(v) => v,
        Err(_) => return images,
    };

    match vendor {
        MakerNoteVendor::Olympus => {
            use crate::make_note::olympus;

            // Olympus has multiple preview image locations:
            // 1. PreviewImageStart (0x0088) + PreviewImageLength (0x0089)
            // 2. PreviewImageStart2 (0x1036) + PreviewImageLength2 (0x1037)
            // 3. PreviewImageStart3 (CameraSettings 0x0101) + PreviewImageLength3 (CameraSettings 0x0102)

            // Try PreviewImageStart/Length
            if let Some(img) = try_extract_preview(
                maker_note_fields,
                &olympus::tags::PreviewImageStart,
                &olympus::tags::PreviewImageLength,
                EmbeddedSubImageSource::MakerNotePreview1,
            ) {
                images.push(img);
            }

            // Try PreviewImageStart2/Length2
            if let Some(img) = try_extract_preview(
                maker_note_fields,
                &olympus::tags::PreviewImageStart2,
                &olympus::tags::PreviewImageLength2,
                EmbeddedSubImageSource::MakerNotePreview2,
            ) {
                images.push(img);
            }

            // TODO: PreviewImageStart3/Length3 from CameraSettings subdirectory (0x2020)
            // This requires parsing the CameraSettings IFD structure
        }
        // **Minolta states its preview exactly where Olympus does** -- 0x0088
        // and 0x0089 -- because Olympus inherited the format. It is the only
        // embedded image an MRW addresses: the container itself points at
        // nothing, so without this a Minolta raw offers no picture at all.
        MakerNoteVendor::Minolta => {
            use crate::make_note::minolta;

            if let Some(img) = try_extract_preview(
                maker_note_fields,
                &minolta::tags::PreviewImageStart,
                &minolta::tags::PreviewImageLength,
                EmbeddedSubImageSource::MakerNotePreview1,
            ) {
                images.push(img);
            }
        }
        _ => {}
    }

    images
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal JPEG: SOI, a padding segment, then a frame header stating
    /// `h x w`. The padding is there because a reader that assumes SOF comes
    /// first passes without walking the chain.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8];
        b.extend_from_slice(&[0xFF, 0xE0, 0x00, 0x08, 1, 2, 3, 4, 5, 6]); // APP0
        b.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 8]);                // SOF0
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        b.extend_from_slice(&[0xFF, 0xD9]);
        b
    }

    fn at(data: Vec<u8>, offset: u64) -> (Vec<u8>, EmbeddedSubImage) {
        let mut file = vec![0u8; offset as usize];
        let len = data.len() as u32;
        file.extend_from_slice(&data);
        (file, EmbeddedSubImage::new_thumbnail(len, offset))
    }

    /// The dimensions come from the JPEG's own header, because the container
    /// usually does not state them -- a Sony ARW addresses three JPEGs and
    /// sizes none of them.
    #[test]
    fn dimensions_come_from_the_frame_header() {
        let (file, img) = at(jpeg(1616, 1080), 64);
        let mut r = std::io::Cursor::new(file);
        assert_eq!(img.dimensions(&mut r).unwrap(), (1616, 1080));
    }

    /// A Dynax 7D's own first bytes, and a DiMAGE 7's: `02 D8` and `00 D8`,
    /// each followed by a real marker. A DiMAGE A200 writes an ordinary
    /// `FF D8`, and a slice that is not a JPEG at all stays refused.
    #[test]
    fn a_clobbered_soi_is_still_a_jpeg_start() {
        assert!(is_jpeg_start(&[0x02, 0xD8, 0xFF, 0xDB]));
        assert!(is_jpeg_start(&[0x00, 0xD8, 0xFF, 0xDB]));
        assert!(is_jpeg_start(&[0xFF, 0xD8, 0xFF, 0xC4]));
        assert!(!is_jpeg_start(&[0x00, 0x00, 0x00, 0x00]));
        assert!(!is_jpeg_start(&[0xFF, 0xD8]));
        // **Longer than four bytes still matches.** A bare slice pattern
        // would not, and that is exactly how the repair came to do nothing.
        assert!(is_jpeg_start(&[0x02, 0xD8, 0xFF, 0xDB, 0x00, 0x84]));
    }

    /// Stated dimensions win and cost no read at all.
    #[test]
    fn a_stated_size_is_not_re_read() {
        let (_file, mut img) = at(jpeg(1616, 1080), 0);
        img.width = Some(4);
        img.height = Some(5);
        // An empty reader proves nothing was read.
        let mut r = std::io::Cursor::new(Vec::new());
        assert_eq!(img.dimensions(&mut r).unwrap(), (4, 5));
    }

    /// A stated length longer than the file is a lie, not an allocation.
    #[test]
    fn a_length_past_the_end_is_refused_not_allocated() {
        let (file, mut img) = at(jpeg(64, 48), 0);
        img.length = u32::MAX; // the whole point: 4 GB if believed
        let mut r = std::io::Cursor::new(file);
        assert!(img.extract_data(&mut r).is_err());
    }

    /// **Fill bytes are legal and must not desync the walk.** JPEG allows any
    /// number of `0xFF` bytes before a marker, so `FF FF C0` is a frame header
    /// with one pad byte. Reading the pad as the marker and the next two bytes
    /// as a segment length walks off into the image.
    #[test]
    fn fill_bytes_before_a_marker_are_skipped() {
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xFF, 0xFF];
        data.extend_from_slice(&[0xC0, 0x00, 0x11, 8]);
        data.extend_from_slice(&300u16.to_be_bytes());
        data.extend_from_slice(&400u16.to_be_bytes());
        data.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1, 0xFF, 0xD9]);
        let (file, img) = at(data, 0);
        let mut r = std::io::Cursor::new(file);
        assert_eq!(img.dimensions(&mut r).unwrap(), (400, 300));
    }

    /// **Past the start of scan there are no more segments.** Entropy-coded
    /// data is not a marker chain, and reading it as one can match `FFC0` in
    /// compressed bytes and return a frame header that does not exist --
    /// a wrong size reported confidently, which is worse than an error.
    #[test]
    fn scan_data_is_not_parsed_as_segments() {
        let mut data = vec![0xFF, 0xD8];
        data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0]);
        // Entropy bytes that look exactly like a 64x64 frame header.
        data.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 8, 0, 64, 0, 64]);
        data.extend_from_slice(&[0xFF, 0xD9]);
        let (file, img) = at(data, 0);
        let mut r = std::io::Cursor::new(file);
        assert!(img.dimensions(&mut r).is_err(),
                "scan data must not yield dimensions");
    }

    /// The segment chain is attacker-controlled, so it is bounded by the
    /// image's declared length rather than walked until something matches.
    #[test]
    fn a_chain_that_never_ends_is_refused() {
        // A segment claiming to be longer than the image, and no frame header.
        let mut data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0xFF, 0xFF];
        data.resize(64, 0);
        let (file, img) = at(data, 0);
        let mut r = std::io::Cursor::new(file);
        assert!(img.dimensions(&mut r).is_err());
    }

    #[test]
    fn something_that_is_not_a_jpeg_is_refused() {
        let (file, img) = at(vec![0x00; 32], 0);
        let mut r = std::io::Cursor::new(file);
        assert!(img.dimensions(&mut r).is_err());
    }
}
