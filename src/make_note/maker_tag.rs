//
// MakerNote Tag definitions
// Based on https://exiv2.org/makernote.html
//

use std::fmt;

use crate::make_note::DUMMY_TIFF_HEADER;

/// Camera manufacturer/vendor identifier for MakerNote data.
///
/// Reference: https://exiv2.org/makernote.html
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum MakerNoteVendor {
    /// Panasonic / Lumix cameras
    /// Header: "Panasonic\0\0\0" (12 bytes)
    /// IFD offset: 12 (relative to MakerNote field start)
    Panasonic,

    /// Nikon cameras (Type 3: E5400/D70/D200)
    /// Header: "Nikon\0" + version + TIFF header (18 bytes total)
    /// IFD offset: relative to TIFF header start
    Nikon,

    /// Canon cameras
    Canon,

    /// Sony cameras
    Sony,

    /// Olympus cameras
    Olympus,

    /// Fujifilm cameras
    /// Uses relative offsets
    Fujifilm,

    /// Unknown or unsupported vendor
    Unknown,
}

impl fmt::Display for MakerNoteVendor {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MakerNoteVendor::Panasonic => write!(f, "Panasonic"),
            MakerNoteVendor::Nikon => write!(f, "Nikon"),
            MakerNoteVendor::Canon => write!(f, "Canon"),
            MakerNoteVendor::Sony => write!(f, "Sony"),
            MakerNoteVendor::Olympus => write!(f, "Olympus"),
            MakerNoteVendor::Fujifilm => write!(f, "Fujifilm"),
            MakerNoteVendor::Unknown => write!(f, "Unknown"),
        }
    }
}

impl MakerNoteVendor {
    /// Detect vendor from MakerNote header data and optional Make field.
    ///
    /// Based on https://exiv2.org/makernote.html header signatures.
    ///
    /// # Arguments
    /// * `data` - The MakerNote data
    /// * `make` - Optional Make field from EXIF (e.g., "Canon", "SONY")
    pub fn from_header(data: &[u8], make: Option<&str>) -> Self {
        // Panasonic: "Panasonic\0\0\0"
        if data.starts_with(b"Panasonic") {
            MakerNoteVendor::Panasonic
        }
        // Nikon: "Nikon\0" + version bytes
        else if data.starts_with(b"Nikon\x00") {
            MakerNoteVendor::Nikon
        }
        // Sony: "SONY DSC "
        else if data.starts_with(b"SONY") {
            MakerNoteVendor::Sony
        }
        // Olympus: "OLYMPUS\0" or "OLYMP\0"
        else if data.starts_with(b"OLYMPUS") || data.starts_with(b"OLYMP\x00") {
            MakerNoteVendor::Olympus
        }
        // Fujifilm
        else if data.starts_with(b"FUJIFILM") {
            MakerNoteVendor::Fujifilm
        }
        // Canon: No header, detect from Make field
        else if let Some(make_str) = make {
            if make_str.starts_with("Canon") {
                MakerNoteVendor::Canon
            } else {
                MakerNoteVendor::Unknown
            }
        } else {
            MakerNoteVendor::Unknown
        }
    }


    /// Returns the proprietary header size (bytes to skip before IFD).
    ///
    /// Based on https://exiv2.org/makernote.html structure definitions.
    pub const fn header_size(&self) -> usize {
        match self {
            MakerNoteVendor::Panasonic => 12, // "Panasonic\0\0\0"
            MakerNoteVendor::Nikon => 10,     // "Nikon\0" + 2 version bytes + 2 padding
            MakerNoteVendor::Fujifilm => 12,  // "FUJIFILM" + 4 bytes
            MakerNoteVendor::Sony => 12,      // "SONY DSC \0\0\0"
            MakerNoteVendor::Canon => 0,
            _ => 0,
        }
    }

    pub const fn offset_correction(&self) -> i32 {
        match self {
            MakerNoteVendor::Panasonic | MakerNoteVendor::Fujifilm | MakerNoteVendor::Sony | MakerNoteVendor::Canon => {
                // Removed header_size bytes, but added DUMMY_TIFF_HEADER bytes
                // Canon: header_size=0, so offset_correction = 0 - 8 = -8
                // tested with Lumix S1R2 (Panasonic), ILCE-7M5 (Sony), EOS R6 Mark III (Canon)
                self.header_size() as i32 - DUMMY_TIFF_HEADER.len() as i32
            }
            MakerNoteVendor::Nikon => {
                // Only removed header, no TIFF header added (Nikon has its own)
                // tested with Nikon Zf
                0
            }
            _ => 0,
        }
    }

    pub const fn consider_tiff_offset(&self) -> bool {
        match self {
            // Casio, Fuji, Olympus, Samsung
            MakerNoteVendor::Fujifilm | MakerNoteVendor::Olympus | MakerNoteVendor::Nikon => {
                false
            },
            // Nikon
            MakerNoteVendor::Panasonic | MakerNoteVendor::Canon | MakerNoteVendor::Sony => {
                true
            },
            // Default
            MakerNoteVendor::Unknown => {
                false
            }
        }
    }
}

/// A tag for MakerNote data, vendor-specific.
///
/// Unlike standard EXIF tags, MakerNote tags are manufacturer-specific.
/// The same tag number has completely different meanings for different vendors.
///
/// # Examples
/// ```ignore
/// use exif::make_note::{MakerTag, MakerNoteVendor};
///
/// // Panasonic tag 0x0001 is "ImageQuality"
/// let panasonic_tag = MakerTag::new(MakerNoteVendor::Panasonic, 0x0001);
///
/// // Nikon tag 0x0001 is "MakerNoteVersion" (completely different!)
/// let nikon_tag = MakerTag::new(MakerNoteVendor::Nikon, 0x0001);
///
/// assert_ne!(panasonic_tag, nikon_tag);
/// ```
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MakerTag {
    /// The camera vendor/manufacturer
    pub vendor: MakerNoteVendor,
    /// The tag number (manufacturer-specific meaning)
    pub number: u16,
}

impl MakerTag {
    /// Creates a new MakerTag.
    pub const fn new(vendor: MakerNoteVendor, number: u16) -> Self {
        MakerTag { vendor, number }
    }

    /// Returns the vendor.
    #[inline]
    pub fn vendor(self) -> MakerNoteVendor {
        self.vendor
    }

    /// Returns the tag number.
    #[inline]
    pub fn number(self) -> u16 {
        self.number
    }

    /// Returns the tag name if known, otherwise None.
    pub fn name(&self) -> Option<&'static str> {
        match self.vendor {
            MakerNoteVendor::Panasonic => super::panasonic::tag_name(self.number),
            MakerNoteVendor::Nikon => super::nikon::tag_name(self.number),
            MakerNoteVendor::Sony => super::sony::tag_name(self.number),
            MakerNoteVendor::Canon => super::canon::tag_name(self.number),
            MakerNoteVendor::Fujifilm => super::fujifilm::tag_name(self.number),
            MakerNoteVendor::Olympus => super::olympus::tag_name(self.number),
            _ => None,
        }
    }

    /// Returns the tag description if known, otherwise None.
    pub fn description(&self) -> Option<&'static str> {
        match self.vendor {
            MakerNoteVendor::Panasonic => super::panasonic::tag_description(self.number),
            MakerNoteVendor::Nikon => super::nikon::tag_description(self.number),
            MakerNoteVendor::Sony => super::sony::tag_description(self.number),
            MakerNoteVendor::Canon => super::canon::tag_description(self.number),
            MakerNoteVendor::Fujifilm => super::fujifilm::tag_description(self.number),
            MakerNoteVendor::Olympus => super::olympus::tag_description(self.number),
            _ => None,
        }
    }
}

impl fmt::Display for MakerTag {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{}:{}", self.vendor, name),
            None => write!(f, "{}:0x{:04X}", self.vendor, self.number),
        }
    }
}

/// A field in MakerNote data with vendor-specific tag.
///
/// Similar to the standard `Field` but uses `MakerTag` instead of `Tag`.
#[derive(Debug, Clone)]
pub struct MakerNoteField {
    /// The tag representing the meaning of this field
    pub tag: MakerTag,
    /// IFD number (usually 0 for MakerNote)
    pub ifd_num: super::In,
    /// The value of the field
    pub value: crate::value::Value,
    /// Custom display string (if vendor-specific display is needed)
    pub custom_display: Option<String>,
}

impl MakerNoteField {
    /// Creates a new MakerNoteField.
    pub fn new(tag: MakerTag, ifd_num: super::In, value: crate::value::Value) -> Self {
        // Try to get vendor-specific custom display
        let custom_display = match tag.vendor {
            MakerNoteVendor::Panasonic => super::panasonic::display_value(tag.number, &value),
            MakerNoteVendor::Nikon => super::nikon::display_value(tag.number, &value),
            MakerNoteVendor::Sony => super::sony::display_value(tag.number, &value),
            MakerNoteVendor::Canon => super::canon::display_value(tag.number, &value),
            MakerNoteVendor::Fujifilm => super::fujifilm::display_value(tag.number, &value),
            MakerNoteVendor::Olympus => super::olympus::display_value(tag.number, &value),
            _ => None,
        };

        MakerNoteField { tag, ifd_num, value, custom_display }
    }

    /// Returns a display value for this field.
    pub fn display_value(&self) -> MakerNoteDisplayValue<'_> {
        MakerNoteDisplayValue { field: self }
    }
}

/// Helper struct for displaying MakerNoteField values
pub struct MakerNoteDisplayValue<'a> {
    field: &'a MakerNoteField,
}

impl fmt::Display for MakerNoteDisplayValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(ref custom_str) = self.field.custom_display {
            write!(f, "{}", custom_str)
        } else {
            // Fall back to default display
            let standard_tag = crate::tag::Tag(crate::tag::Context::Tiff, self.field.tag.number);
            self.field.value.display_as(standard_tag).fmt(f)
        }
    }
}

