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


//! Minolta / Konica Minolta MakerNote tags.
//!
//! **A bare IFD with no signature at all.** Where Olympus writes `OLYMPUS\0`
//! and Panasonic writes `Panasonic\0\0\0`, a Minolta block starts on its entry
//! count — the first entry is `MakerNoteVersion`, whose value is the ASCII
//! `MLT0` that looks like a header and is not one. So the vendor can only be
//! recognised from `Make`, and four spellings appear across the 13 corpus
//! bodies: `MINOLTA`, `Minolta Co., Ltd.`, `KONICA MINOLTA` and
//! `Konica Minolta Camera, Inc.`
//!
//! **Olympus inherited this format**, which is why the Olympus table already
//! names `MinoltaCameraSettings` at 0x0001 and 0x0003 — and why the preview
//! is at the same 0x0088/0x0089 in both.
//!
//! Names follow exiftool's Minolta table, which is the reference.

use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};

generate_maker_tags! {
    vendor: Minolta,
    tags: [
    (MakerNoteVersion, 0x0000, "Maker Note Version"),
    (MinoltaCameraSettingsOld, 0x0001, "Minolta Camera Settings Old"),
    (MinoltaCameraSettings, 0x0003, "Minolta Camera Settings"),
    (MinoltaCameraSettings7D, 0x0004, "Minolta Camera Settings 7D"),
    (ImageStabilizationData, 0x0018, "Image Stabilization Data"),
    (WBInfoA100, 0x0020, "White Balance Info A100"),
    (CompressedImageSize, 0x0040, "Compressed Image Size"),
    (PreviewImage, 0x0081, "Preview Image"),
    (PreviewImageStart, 0x0088, "Preview Image Start"),
    (PreviewImageLength, 0x0089, "Preview Image Length"),
    (SceneMode, 0x0100, "Scene Mode"),
    (ColorMode, 0x0101, "Color Mode"),
    (MinoltaQuality, 0x0102, "Minolta Quality"),
    (FlashExposureComp, 0x0104, "Flash Exposure Compensation"),
    (Teleconverter, 0x0105, "Teleconverter"),
    (ImageStabilization, 0x0107, "Image Stabilization"),
    (RawAndJpgRecording, 0x0109, "Raw and JPEG Recording"),
    (ZoneMatching, 0x010a, "Zone Matching"),
    (ColorTemperature, 0x010b, "Color Temperature"),
    (LensType, 0x010c, "Lens Type"),
    (ColorCompensationFilter, 0x0111, "Color Compensation Filter"),
    (WhiteBalanceFineTune, 0x0112, "White Balance Fine Tune"),
    (ImageStabilizationA100, 0x0113, "Image Stabilization A100"),
    (CameraSettings, 0x0114, "Camera Settings"),
    (WhiteBalance, 0x0115, "White Balance"),
    (PrintIM, 0x0e00, "Print Image Matching"),
    (MinoltaCameraSettings2, 0x0f00, "Minolta Camera Settings 2"),
    ]
}

/// Which byte order the block's bare IFD is in.
///
/// **There is no header to read it from**, and the container's order is not
/// passed down here — so it comes from the block itself. The first entry's
/// TYPE settles it: a TIFF type is 1..=13, so one reading of those two bytes
/// is a small number and the other is that number times 256. The two cannot
/// both be valid, which is what makes it a decision rather than a preference.
///
/// Every corpus MRW is big-endian, as its `\0TTW` TIFF is.
pub(crate) fn detect_minolta_byte_order(data: &[u8]) -> bool {
    // `[count:u16][tag:u16][type:u16][count:u32][value:u32]`
    let Some(t) = data.get(4..6) else { return true };
    let (be, le) = (u16::from_be_bytes([t[0], t[1]]), u16::from_le_bytes([t[0], t[1]]));
    if (1..=13).contains(&be) {
        return false;
    }
    if (1..=13).contains(&le) {
        return true;
    }
    // Neither reads as a type. Nothing here to decide on.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Dynax 7D's own first 14 bytes: 19 entries, then
    /// `MakerNoteVersion` as UNDEFINED[4] holding `MLT0`.
    #[test]
    fn the_entry_type_settles_the_byte_order() {
        let be = [0x00, 0x13, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00,
                  0x00, 0x04, b'M', b'L', b'T', b'0'];
        assert!(!detect_minolta_byte_order(&be));
        // The same entry written the other way round.
        let le = [0x13, 0x00, 0x00, 0x00, 0x07, 0x00, 0x04, 0x00,
                  0x00, 0x00, b'M', b'L', b'T', b'0'];
        assert!(detect_minolta_byte_order(&le));
    }
}
