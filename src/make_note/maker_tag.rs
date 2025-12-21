//
// MakerNote Tag definitions
// Based on https://exiv2.org/makernote.html
//

use std::fmt;

use crate::{Value, make_note::DUMMY_TIFF_HEADER};

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

    /// OM System cameras
    OMSystem,

    /// Fujifilm cameras
    /// Uses relative offsets
    Fujifilm,

    /// Leica cameras
    /// Header: "LEICA\0" + 2-byte version (8 bytes total)
    Leica,

    /// Samsung cameras (NX series)
    /// No proprietary header, starts directly with IFD
    Samsung,

    /// Apple cameras (iPhone, iPad)
    /// No proprietary header, starts directly with IFD
    Apple,

    /// Olympus Equipment subdirectory (0x2010)
    OlympusEquipment,

    /// Olympus CameraSettings subdirectory (0x2020)
    OlympusCameraSettings,

    /// Olympus RawDevelopment subdirectory (0x2030)
    OlympusRawDevelopment,

    /// Olympus ImageProcessing subdirectory (0x2040)
    OlympusImageProcessing,

    /// Olympus FocusInfo subdirectory (0x2050)
    OlympusFocusInfo,

    /// Olympus RawInfo subdirectory (0x3000)
    OlympusRawInfo,

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
            MakerNoteVendor::OMSystem => write!(f, "OM System"),
            MakerNoteVendor::Fujifilm => write!(f, "Fujifilm"),
            MakerNoteVendor::Leica => write!(f, "Leica"),
            MakerNoteVendor::Samsung => write!(f, "Samsung"),
            MakerNoteVendor::Apple => write!(f, "Apple"),
            // olympus specific
            MakerNoteVendor::OlympusEquipment => write!(f, "OlympusEquipment"),
            MakerNoteVendor::OlympusCameraSettings => write!(f, "OlympusCameraSettings"),
            MakerNoteVendor::OlympusRawDevelopment => write!(f, "OlympusRawDevelopment"),
            MakerNoteVendor::OlympusImageProcessing => write!(f, "OlympusImageProcessing"),
            MakerNoteVendor::OlympusFocusInfo => write!(f, "OlympusFocusInfo"),
            MakerNoteVendor::OlympusRawInfo => write!(f, "OlympusRawInfo"),
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
        // OM System: "OM SYSTEM\0\0\0"
        else if data.starts_with(b"OM SYSTEM") {
            MakerNoteVendor::OMSystem
        }
        // Fujifilm
        else if data.starts_with(b"FUJIFILM") {
            MakerNoteVendor::Fujifilm
        }
        // Leica: "LEICA\0"
        else if data.starts_with(b"LEICA\x00") {
            MakerNoteVendor::Leica
        }
        // Apple: "Apple iOS\0"
        else if data.starts_with(b"Apple iOS") {
            MakerNoteVendor::Apple
        }
        // Canon/Samsung: No header, detect from Make field
        else if let Some(make_str) = make {
            if make_str.starts_with("Canon") {
                MakerNoteVendor::Canon
            } else if make_str.starts_with("SAMSUNG") {
                MakerNoteVendor::Samsung
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
            MakerNoteVendor::Canon => 0,      // No header
            MakerNoteVendor::Sony => 12,      // "SONY DSC \0\0\0"
            MakerNoteVendor::Olympus => 12,   // "OLYMPUS\0" + "II/MM" + version (2 bytes)
            MakerNoteVendor::OMSystem => 16,  // "OM SYSTEM\0\0\0" + "II/MM" + version (2 bytes)
            MakerNoteVendor::Fujifilm => 12,  // "FUJIFILM" + 4 bytes
            MakerNoteVendor::Leica => 8,      // "LEICA\0" + version (2 bytes)
            MakerNoteVendor::Samsung => 0,    // No header, starts directly with IFD
            MakerNoteVendor::Apple => 14,     // "Apple iOS\0" (10) + version (2) + "II/MM" (2) = 14 bytes
            // Subdirectories don't have headers (they're already inside parsed data)
            MakerNoteVendor::OlympusEquipment
            | MakerNoteVendor::OlympusCameraSettings
            | MakerNoteVendor::OlympusRawDevelopment
            | MakerNoteVendor::OlympusImageProcessing
            | MakerNoteVendor::OlympusFocusInfo
            | MakerNoteVendor::OlympusRawInfo => 0,
            _ => 0,
        }
    }

    pub const fn offset_correction(&self) -> i32 {
        match self {
            MakerNoteVendor::Panasonic | MakerNoteVendor::Canon | MakerNoteVendor::Sony | MakerNoteVendor::Olympus |
            MakerNoteVendor::OMSystem | MakerNoteVendor::Fujifilm | MakerNoteVendor::Leica | MakerNoteVendor::Samsung |
            &MakerNoteVendor::Apple => {
                // Removed header_size bytes, but added DUMMY_TIFF_HEADER bytes
                // Panasonic: header_size=12, so offset_correction = 12 - 8 = 4
                // Canon: header_size=0, so offset_correction = 0 - 8 = -8
                // Sony: header_size=12, so offset_correction = 12 - 8 = 4
                // Olympus: header_size=12, so offset_correction = 12 - 8 = 4
                // OM System: header_size=16, so offset_correction = 16 - 8 = 8
                // Fujifilm: header_size=12, so offset_correction = 12 - 8 = 4
                // Leica: header_size=8, so offset_correction = 8 - 8 = 0
                // Samsung: header_size=0, so offset_correction = 0 - 8 = -8
                // Apple: header_size=14, so offset_correction = 14 - 8 = 6
                // tested with Lumix S1R2 (Panasonic), ILCE-7M5 (Sony), EOS R6 Mark III (Canon), LEICA Q2 (Leica)
                self.header_size() as i32 - DUMMY_TIFF_HEADER.len() as i32
            }
            MakerNoteVendor::Nikon => {
                // Only removed header, no TIFF header added (Nikon has its own)
                // tested with Nikon Zf
                0
            }
            // Subdirectories don't have headers (they're already inside parsed data)
            MakerNoteVendor::OlympusEquipment
            | MakerNoteVendor::OlympusCameraSettings
            | MakerNoteVendor::OlympusRawDevelopment
            | MakerNoteVendor::OlympusImageProcessing
            | MakerNoteVendor::OlympusFocusInfo
            | MakerNoteVendor::OlympusRawInfo => 0,
            _ => 0,
        }
    }

    pub const fn consider_tiff_offset(&self) -> bool {
        match self {
            // Panasonic, Canon, Sony, Leica use TIFF-relative offsets
            MakerNoteVendor::Panasonic | MakerNoteVendor::Canon | MakerNoteVendor::Sony | MakerNoteVendor::Leica => true,
            // Nikon, Olympus, OM System, Fujifilm, Samsung, Apple - use MakerNote-relative offsets
            MakerNoteVendor::Nikon | MakerNoteVendor::Olympus | MakerNoteVendor::OMSystem |
            MakerNoteVendor::Fujifilm | MakerNoteVendor::Samsung | MakerNoteVendor::Apple => false,
            // Olympus subdirectories use MakerNote-relative offsets (inherited from parent)
            MakerNoteVendor::OlympusEquipment
            | MakerNoteVendor::OlympusCameraSettings
            | MakerNoteVendor::OlympusRawDevelopment
            | MakerNoteVendor::OlympusImageProcessing
            | MakerNoteVendor::OlympusFocusInfo
            | MakerNoteVendor::OlympusRawInfo => false,
            // Default
            MakerNoteVendor::Unknown => false,
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
            MakerNoteVendor::Canon => super::canon::tag_name(self.number),
            MakerNoteVendor::Sony => super::sony::tag_name(self.number),
            MakerNoteVendor::Olympus | MakerNoteVendor::OMSystem => super::olympus::tag_name(self.number),
            MakerNoteVendor::Fujifilm => super::fujifilm::tag_name(self.number),
            MakerNoteVendor::Leica => super::panasonic::tag_name(self.number),
            MakerNoteVendor::Samsung => super::samsung::tag_name(self.number),
            MakerNoteVendor::Apple => super::apple::tag_name(self.number),
            MakerNoteVendor::OlympusEquipment => super::olympus::olympus_equipment_tag_name(self.number),
            MakerNoteVendor::OlympusCameraSettings => super::olympus::olympus_camera_settings_tag_name(self.number),
            MakerNoteVendor::OlympusRawDevelopment => super::olympus::olympus_raw_development_tag_name(self.number),
            MakerNoteVendor::OlympusImageProcessing => super::olympus::olympus_image_processing_tag_name(self.number),
            MakerNoteVendor::OlympusFocusInfo => super::olympus::olympus_focus_info_tag_name(self.number),
            MakerNoteVendor::OlympusRawInfo => super::olympus::olympus_raw_info_tag_name(self.number),
            _ => None,
        }
    }

    /// Returns the tag description if known, otherwise None.
    pub fn description(&self) -> Option<&'static str> {
        match self.vendor {
            MakerNoteVendor::Panasonic => super::panasonic::tag_description(self.number),
            MakerNoteVendor::Nikon => super::nikon::tag_description(self.number),
            MakerNoteVendor::Canon => super::canon::tag_description(self.number),
            MakerNoteVendor::Sony => super::sony::tag_description(self.number),
            MakerNoteVendor::Olympus | MakerNoteVendor::OMSystem => super::olympus::tag_description(self.number),
            MakerNoteVendor::Fujifilm => super::fujifilm::tag_description(self.number),
            MakerNoteVendor::Leica => super::panasonic::tag_description(self.number),
            MakerNoteVendor::Samsung => super::samsung::tag_description(self.number),
            MakerNoteVendor::Apple => super::apple::tag_description(self.number),
            MakerNoteVendor::OlympusEquipment => super::olympus::olympus_equipment_tag_description(self.number),
            MakerNoteVendor::OlympusCameraSettings => super::olympus::olympus_camera_settings_tag_description(self.number),
            MakerNoteVendor::OlympusRawDevelopment => super::olympus::olympus_raw_development_tag_description(self.number),
            MakerNoteVendor::OlympusImageProcessing => super::olympus::olympus_image_processing_tag_description(self.number),
            MakerNoteVendor::OlympusFocusInfo => super::olympus::olympus_focus_info_tag_description(self.number),
            MakerNoteVendor::OlympusRawInfo => super::olympus::olympus_raw_info_tag_description(self.number),
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
            MakerNoteVendor::Canon => super::canon::display_value(tag.number, &value),
            MakerNoteVendor::Sony => super::sony::display_value(tag.number, &value),
            MakerNoteVendor::Olympus | MakerNoteVendor::OMSystem => super::olympus::display_value(tag.number, &value),
            MakerNoteVendor::Fujifilm => super::fujifilm::display_value(tag.number, &value),
            MakerNoteVendor::Leica => super::panasonic::display_value(tag.number, &value),
            MakerNoteVendor::Samsung => super::samsung::display_value(tag.number, &value),
            MakerNoteVendor::Apple => super::apple::display_value(tag.number, &value),
            MakerNoteVendor::OlympusEquipment => super::olympus::olympus_equipment_display_value(tag.number, &value),
            MakerNoteVendor::OlympusCameraSettings => super::olympus::olympus_camera_settings_display_value(tag.number, &value),
            MakerNoteVendor::OlympusRawDevelopment => super::olympus::olympus_raw_development_display_value(tag.number, &value),
            MakerNoteVendor::OlympusImageProcessing => super::olympus::olympus_image_processing_display_value(tag.number, &value),
            MakerNoteVendor::OlympusFocusInfo => super::olympus::olympus_focus_info_display_value(tag.number, &value),
            MakerNoteVendor::OlympusRawInfo => super::olympus::olympus_raw_info_display_value(tag.number, &value),
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

/// Display Undefined value as null-terminated string
pub(crate) fn d_undef_as_string(value: &Value) -> String {
    match value {
        Value::Undefined(bytes, _) => {
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            std::str::from_utf8(&bytes[..end])
                .unwrap_or("<invalid UTF-8>")
                .to_string()
        }
        Value::Ascii(vec) => {
            // For Ascii, take the first non-empty string
            vec.iter()
                .find(|s| !s.is_empty())
                .and_then(|s| std::str::from_utf8(s).ok())
                .unwrap_or("")
                .to_string()
        }
        Value::Byte(bytes) => {
            // For Byte, treat as null-terminated string
            let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
            std::str::from_utf8(&bytes[..end])
                .unwrap_or("<invalid UTF-8>")
                .to_string()
        }
        _ => format!("{:?}", value),
    }
}