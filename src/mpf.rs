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

use std::io::{BufRead, ErrorKind, Seek, SeekFrom};

use crate::endian::{BigEndian, Endian, LittleEndian};
use crate::error::Error;
use crate::jpeg::marker;
use crate::tiff::{TIFF_BE, TIFF_LE};
use crate::util::{read16, read8};

// MPF identifier code "MPF\0". [CIPA DC-007-2009]
const MPF_ID: [u8; 4] = [0x4d, 0x50, 0x46, 0x00];

/// Information about an individual image in the MPF
#[derive(Debug, Clone, Copy)]
pub struct MpImage {
    /// Type of the MP image (e.g., Baseline MP Primary Image, Large Thumbnail)
    pub r#type: u32,
    /// Size of the individual image in bytes
    pub length: u32,
    /// Absolute offset of the individual image from the start of the file
    /// (This is the actual file offset, ready to use for seeking)
    pub offset: u64,
}

impl MpImage {
    /// Extract the image data from a reader
    ///
    /// # Arguments
    /// * `reader` - A reader positioned at the start of the JPEG file
    ///
    /// # Returns
    /// The raw image data as a Vec<u8>
    pub fn extract_data<R>(&self, reader: &mut R) -> Result<Vec<u8>, Error>
    where
        R: BufRead + Seek,
    {
        extract_image_data(reader, self.offset, self.length as usize)
    }
}

/// Multi-Picture Format (MPF) information extracted from a JPEG file
#[derive(Debug, Clone)]
pub struct MpfInfo {
    /// Version of MPF format
    pub version: String,
    /// Total number of images in the MPF
    pub number_of_images: u32,
    /// Individual images in the MPF
    pub images: Vec<MpImage>,
}

impl MpfInfo {
    /// Get the large preview image metadata (typically MPImage2)
    /// Returns the first non-primary image with a reasonable size (> 50KB)
    pub fn get_preview_image(&self) -> Option<&MpImage> {
        self.images
            .iter()
            .skip(1) // Skip first image (primary JPEG)
            .find(|img| img.length > 50_000)
    }

    /// Get the primary image metadata (MPImage1)
    pub fn get_primary_image(&self) -> Option<&MpImage> {
        self.images.first()
    }
}

/// Get the MPF information from a JPEG file.
/// Returns metadata only - actual image data can be extracted using `MpImage::extract_data()`.
///
/// # Examples
/// ```no_run
/// use std::fs::File;
/// use std::io::BufReader;
/// use exif::get_mpf_info;
///
/// let file = File::open("image.jpg").unwrap();
/// let mut reader = BufReader::new(file);
///
/// if let Some(mpf) = get_mpf_info(&mut reader).unwrap() {
///     if let Some(preview) = mpf.get_preview_image() {
///         // Extract preview data only when needed
///         let data = preview.extract_data(&mut reader).unwrap();
///         println!("Preview size: {} bytes", data.len());
///     }
/// }
/// ```
pub fn get_mpf_info<R>(reader: &mut R) -> Result<Option<MpfInfo>, Error>
where
    R: BufRead + Seek,
{
    match get_mpf_info_sub(reader) {
        Err(Error::Io(ref e)) if e.kind() == ErrorKind::UnexpectedEof => {
            Err(Error::InvalidFormat("Broken JPEG file"))
        }
        r => r,
    }
}

fn get_mpf_info_sub<R>(reader: &mut R) -> Result<Option<MpfInfo>, Error>
where
    R: BufRead + Seek,
{
    // Read SOI marker
    let mut soi = [0u8; 2];
    reader.read_exact(&mut soi)?;
    if soi != [marker::P, marker::SOI] {
        return Err(Error::InvalidFormat("Not a JPEG file"));
    }

    let mut mpf_data: Option<Vec<u8>> = None;
    let mut mpf_app2_position: Option<u64> = None;

    // First pass: find MPF APP2 segment
    loop {
        // Save current position (before reading marker)
        let pos = reader.seek(SeekFrom::Current(0))?;

        // Find a marker prefix
        reader.read_until(marker::P, &mut Vec::new())?;

        // Get a marker code
        let mut code;
        loop {
            code = read8(reader)?;
            if code != marker::P {
                break;
            }
        }

        // Continue or return early on stand-alone markers
        match code {
            marker::Z | marker::TEM | marker::RST0..=marker::RST7 => continue,
            marker::SOI => return Err(Error::InvalidFormat("Unexpected SOI")),
            marker::EOI => break,
            _ => {}
        }

        // Read marker segments
        let len = read16(reader)?
            .checked_sub(2)
            .ok_or(Error::InvalidFormat("Invalid segment length"))?;
        let mut seg = vec![0; len.into()];
        reader.read_exact(&mut seg)?;

        if code == marker::APP2 && seg.starts_with(&MPF_ID) {
            // Calculate the position where MPF TIFF data starts
            // = APP2 marker (2 bytes: 0xFF 0xE2) + length field (2 bytes) + MPF ID (4 bytes)
            // The offset stored in MPEntry is relative to the MPF TIFF data start
            mpf_app2_position = Some(pos + 2 + 2 + 4); // marker + length + MPF_ID
            seg.drain(..MPF_ID.len());
            mpf_data = Some(seg);
            // Continue to find end position
        }

        if code == marker::SOS {
            // Skipping the scan data is handled in the main loop
        }
    }

    // If no MPF data found, return None
    let mpf_seg = match mpf_data {
        Some(data) => data,
        None => return Ok(None),
    };

    let app2_pos = mpf_app2_position.ok_or(Error::InvalidFormat("MPF APP2 position not found"))?;

    // Parse MPF data (TIFF-like structure)
    parse_mpf_segment(&mpf_seg, reader, app2_pos)
}

fn parse_mpf_segment<R>(
    data: &[u8],
    reader: &mut R,
    app2_position: u64,
) -> Result<Option<MpfInfo>, Error>
where
    R: BufRead + Seek,
{
    if data.len() < 8 {
        return Err(Error::InvalidFormat("MPF segment too short"));
    }

    // Check byte order (first 2 bytes).
    match BigEndian::loadu16(data, 0) {
        TIFF_BE => parse_mpf_segment_endian::<BigEndian, R>(data, reader, app2_position),
        TIFF_LE => parse_mpf_segment_endian::<LittleEndian, R>(data, reader, app2_position),
        _ => Err(Error::InvalidFormat("Invalid MPF byte order")),
    }
}

fn parse_mpf_segment_endian<E, R>(
    data: &[u8],
    _reader: &mut R,
    app2_position: u64,
) -> Result<Option<MpfInfo>, Error>
where
    E: Endian,
    R: BufRead + Seek,
{
    // Read IFD offset (4 bytes at offset 4)
    let ifd_offset = E::loadu32(data, 4) as usize;

    if ifd_offset >= data.len() {
        return Err(Error::InvalidFormat("Invalid MPF IFD offset"));
    }

    // Read number of entries (2 bytes at ifd_offset)
    let num_entries = E::loadu16(data, ifd_offset) as usize;

    let mut version: Option<String> = None;
    let mut number_of_images: Option<u32> = None;
    let mut mp_entry_data: Option<Vec<u8>> = None;

    // Parse IFD entries
    for i in 0..num_entries {
        let entry_offset = ifd_offset + 2 + i * 12;
        if entry_offset + 12 > data.len() {
            break;
        }

        let tag = E::loadu16(data, entry_offset);
        let field_type = E::loadu16(data, entry_offset + 2);
        let count = E::loadu32(data, entry_offset + 4) as usize;
        let value_offset = entry_offset + 8;

        match tag {
            0xb000 => {
                // MPFVersion
                if field_type == 7 && count >= 4 {
                    // UNDEFINED
                    let ver_bytes = &data[value_offset..value_offset + 4];
                    version = Some(format!(
                        "{}.{}",
                        (ver_bytes[0] as char).to_string() + &(ver_bytes[1] as char).to_string(),
                        (ver_bytes[2] as char).to_string() + &(ver_bytes[3] as char).to_string()
                    ));
                }
            }
            0xb001 => {
                // NumberOfImages
                if field_type == 4 {
                    // LONG
                    number_of_images = Some(E::loadu32(data, value_offset));
                }
            }
            0xb002 => {
                // MPEntry
                if field_type == 7 {
                    // UNDEFINED
                    let data_offset = if count > 4 {
                        E::loadu32(data, value_offset) as usize
                    } else {
                        value_offset
                    };

                    if data_offset + count <= data.len() {
                        mp_entry_data = Some(data[data_offset..data_offset + count].to_vec());
                    }
                }
            }
            _ => {}
        }
    }

    let version = version.ok_or(Error::InvalidFormat("MPFVersion not found"))?;
    let number_of_images =
        number_of_images.ok_or(Error::InvalidFormat("NumberOfImages not found"))?;
    let mp_entry_data = mp_entry_data.ok_or(Error::InvalidFormat("MPEntry not found"))?;

    // Parse MP Entry (each entry is 16 bytes)
    let entry_size = 16;
    let num_entries_from_data = mp_entry_data.len() / entry_size;

    if num_entries_from_data != number_of_images as usize {
        return Err(Error::InvalidFormat("MPEntry count mismatch"));
    }

    let mut images = Vec::new();

    for i in 0..num_entries_from_data {
        let entry_start = i * entry_size;

        // Parse each MP Entry fields
        let image_type = E::loadu32(&mp_entry_data, entry_start + 0);
        let image_length = E::loadu32(&mp_entry_data, entry_start + 4);
        let image_start = E::loadu32(&mp_entry_data, entry_start + 8);

        // Calculate absolute offset from file start
        // MPImageStart in MPEntry is relative to the MPF TIFF data start position
        let offset = app2_position + image_start as u64;

        images.push(MpImage {
            r#type: image_type,
            length: image_length,
            offset,
        });
    }

    Ok(Some(MpfInfo {
        version,
        number_of_images,
        images,
    }))
}

fn extract_image_data<R>(reader: &mut R, offset: u64, length: usize) -> Result<Vec<u8>, Error>
where
    R: BufRead + Seek,
{
    // Seek to the image start position
    reader.seek(SeekFrom::Start(offset))?;

    // Read the image data
    let mut data = vec![0u8; length];
    reader.read_exact(&mut data)?;

    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpf_identifier() {
        assert_eq!(&MPF_ID, b"MPF\0");
    }
}
