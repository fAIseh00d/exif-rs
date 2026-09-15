//
// Copyright (c) 2026 Ivan Rodnov.
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
//! MakerNote tables whose entries are POSITIONS rather than tags.
//!
//! Some vendors store a block of fixed-width values under one tag, and what
//! each value means is its index: Canon's `ColorInfo` is an int16 array whose
//! element 3 is the colour space, Minolta's camera settings are big-endian
//! int16 blocks whose element 0x25, 0x2F or 0x17 is, depending on the body.
//! The tag alone surfaces the block, which leaves every consumer to know the
//! layout.
//!
//! So each position is also reported as a field of its own, under a
//! sub-vendor named for the table, numbered by index: the same shape an
//! Olympus subdirectory already has (`OlympusCameraSettings:0x0507`), and the
//! way exiftool's binary tables number their entries. Values are the raw
//! integers; nothing is converted, merged or preferred, and the block's own
//! field is kept. Layouts and names follow exiftool (`Canon::ColorInfo`,
//! `Minolta::CameraSettings7D`, `CameraSettings5D`, `CameraSettingsA100`).

use std::collections::HashMap;

use crate::make_note::maker_tag::{MakerNoteField, MakerNoteVendor, MakerTag};
use crate::value::Value;

pub mod canon_color_info {
    use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};
    generate_maker_tags! {
        vendor: CanonColorInfo,
        tags: [
        (Saturation, 0x0001, "Saturation"),
        (ColorTone, 0x0002, "Color Tone"),
        (ColorSpace, 0x0003, "Color Space"),
        ]
    }
}

pub mod minolta_camera_settings_7d {
    use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};
    generate_maker_tags! {
        vendor: MinoltaCameraSettings7D,
        tags: [
        (ColorSpace, 0x0025, "Color Space"),
        ]
    }
}

pub mod minolta_camera_settings_5d {
    use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};
    generate_maker_tags! {
        vendor: MinoltaCameraSettings5D,
        tags: [
        (ColorSpace, 0x002f, "Color Space"),
        ]
    }
}

pub mod minolta_camera_settings_a100 {
    use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};
    generate_maker_tags! {
        vendor: MinoltaCameraSettingsA100,
        tags: [
        (ColorSpace, 0x0017, "Color Space"),
        ]
    }
}

/// Which positional table a MakerNote field is, if any, and the index its
/// entries start at.
///
/// Minolta's 0x0114 is two different tables, told apart only by the body, as
/// exiftool does: the Dynax/Maxxum 5D and Alpha Sweet write one layout, the
/// Sony DSLR-A100 another. Any other body's 0x0114 is left as a block.
fn table_of(tag: MakerTag, model: Option<&str>) -> Option<(MakerNoteVendor, usize)> {
    match (tag.vendor(), tag.number()) {
        // `Canon::ColorInfo` starts at 1: element 0 is not an entry.
        (MakerNoteVendor::Canon, 0x4003) => Some((MakerNoteVendor::CanonColorInfo, 1)),
        (MakerNoteVendor::Minolta, 0x0004) => Some((MakerNoteVendor::MinoltaCameraSettings7D, 0)),
        (MakerNoteVendor::Minolta, 0x0114) => match model {
            Some(m) if ["DYNAX 5D", "MAXXUM 5D", "ALPHA SWEET"].iter().any(|p| m.starts_with(p)) =>
                Some((MakerNoteVendor::MinoltaCameraSettings5D, 0)),
            Some("DSLR-A100") => Some((MakerNoteVendor::MinoltaCameraSettingsA100, 0)),
            _ => None,
        },
        _ => None,
    }
}

/// Add a field for every position of every positional table present.
///
/// `model` is IFD0's `Model`, trimmed; it selects between the two 0x0114
/// layouts and nothing else.
///
/// A block that arrives as bytes is read as big-endian 16-bit values, which
/// is how these tables are written whatever the file's own byte order (exiftool
/// forces it for the same reason: an A100 ARW and an A100 JPEG differ).
/// An array the file declares as SHORT or SSHORT keeps that type, one value
/// per field.
pub(crate) fn expand(fields: &mut HashMap<MakerTag, MakerNoteField>, model: Option<&str>) {
    let mut added = Vec::new();
    for field in fields.values() {
        let Some((vendor, first)) = table_of(field.tag, model) else { continue };
        let values: Vec<Value> = match &field.value {
            Value::Short(v) => v.iter().map(|&x| Value::Short(vec![x])).collect(),
            Value::SShort(v) => v.iter().map(|&x| Value::SShort(vec![x])).collect(),
            Value::Undefined(bytes, _) | Value::Byte(bytes) => bytes
                .chunks_exact(2)
                .map(|c| Value::Short(vec![u16::from_be_bytes([c[0], c[1]])]))
                .collect(),
            _ => continue,
        };
        for (index, value) in values.into_iter().enumerate().skip(first) {
            let Ok(number) = u16::try_from(index) else { break };
            added.push(MakerNoteField::new(MakerTag::new(vendor, number), field.ifd_num, value));
        }
    }
    for field in added {
        fields.insert(field.tag, field);
    }
}
