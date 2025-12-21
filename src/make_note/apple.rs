//
// Apple MakerNote Tag definitions
// Based on iPhoto information with iPhone 17 Pro
//

use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};

/// Detect byte order from Apple MakerNote header.
///
/// Apple MakerNote structure:
/// - "Apple iOS\0" (10 bytes)
/// - Version (2 bytes)
/// - Byte order marker: "MM" or "II" (2 bytes)
/// - IFD entry count and data follow
///
/// # Arguments
/// * `data` - The full MakerNote data starting with "Apple iOS"
///
/// # Returns
/// `true` if little-endian (II), `false` if big-endian (MM)
pub(crate) fn detect_apple_byte_order(data: &[u8]) -> bool {
    // Need at least 14 bytes: "Apple iOS\0" (10) + version (2) + byte order (2)
    if data.len() < 14 {
        return false; // Default to big-endian if not enough data
    }

    // Check byte order marker at offset 12-13
    // "II" = 0x4949 = little-endian
    // "MM" = 0x4D4D = big-endian
    if data[12] == b'I' && data[13] == b'I' {
        true // Little-endian
    } else if data[12] == b'M' && data[13] == b'M' {
        false // Big-endian
    } else {
        // Invalid byte order marker, default to big-endian (most common for Apple)
        false
    }
}

generate_maker_tags! {
    vendor: Apple,
    tags: [
    (MakerNoteVersion, 0x0001, "Maker Note Version"),
    (AEMatrix, 0x0002, "AE Matrix"),
    (RunTime, 0x0003, "Run Time"),
    (AEStable, 0x0004, "AE Stable"),
    (AETarget, 0x0005, "AE Target"),
    (AEAverage, 0x0006, "AE Average"),
    (AFStable, 0x0007, "AF Stable"),
    (AccelerationVector, 0x0008, "Acceleration Vector"),
    (FocusDistanceRange, 0x000c, "Focus Distance Range"),
    (Apple_0x000d, 0x000d, "Apple 0x000d"),
    (Apple_0x000e, 0x000e, "Apple 0x000e"),
    (Apple_0x0010, 0x0010, "Apple 0x0010"),
    (ImageCaptureType, 0x0014, "Image Capture Type"),
    (Apple_0x0016, 0x0016, "Apple 0x0016"),
    (LivePhotoVideoIndex, 0x0017, "Live Photo Video Index"),
    (ImageProcessingFlags, 0x0019, "Image Processing Flags"),
    (QualityHint, 0x001a, "Quality Hint"),
    (LuminanceNoiseAmplitude, 0x001d, "Luminance Noise Amplitude"),
    (PhotosAppFeatureFlags, 0x001f, "Photos App Feature Flags"),
    (ImageCaptureRequestID, 0x0020, "Image Capture Request ID"),
    (HDRHeadroom, 0x0021, "HDR Headroom"),
    (AFPerformance, 0x0023, "AF Performance"),
    (SceneFlags, 0x0025, "Scene Flags"),
    (SignalToNoiseRatioType, 0x0026, "Signal To Noise Ratio Type"),
    (SignalToNoiseRatio, 0x0027, "Signal To Noise Ratio"),
    (PhotoIdentifier, 0x002b, "Photo Identifier"),
    (ColorTemperature, 0x002f, "Color Temperature"),
    (CameraType, 0x0033, "Camera Type"),
    (FocusPosition, 0x0034, "Focus Position"),
    (HDRGain, 0x0035, "HDR Gain"),
    (Apple_0x0036, 0x0036, "Apple 0x0036"),
    (Apple_0x0037, 0x0037, "Apple 0x0037"),
    (AFMeasuredDepth, 0x0038, "AF Measured Depth"),
    (Apple_0x0039, 0x0039, "Apple 0x0039"),
    (Apple_0x003a, 0x003a, "Apple 0x003a"),
    (Apple_0x003b, 0x003b, "Apple 0x003b"),
    (Apple_0x003c, 0x003c, "Apple 0x003c"),
    (AFConfidence, 0x003d, "AF Confidence"),
    (GreenGhostMitigationStatus, 0x003e, "Green Ghost Mitigation Status"),
    (SemanticStyleRenderingVer, 0x003f, "Semantic Style Rendering Ver"),
    (SemanticStylePreset, 0x0040, "Semantic Style Preset"),
    (Apple_0x0043, 0x0043, "Apple 0x0043"),
    (Apple_0x0044, 0x0044, "Apple 0x0044"),
    (Apple_0x0045, 0x0045, "Apple 0x0045"),
    (Apple_0x0046, 0x0046, "Apple 0x0046"),
    (Apple_0x0048, 0x0048, "Apple 0x0048"),
    (Apple_0x0049, 0x0049, "Apple 0x0049"),
    (Apple_0x004a, 0x004a, "Apple 0x004a"),
    (Apple_0x004d, 0x004d, "Apple 0x004d"),
    (Apple_0x004e, 0x004e, "Apple 0x004e"),
    (Apple_0x004f, 0x004f, "Apple 0x004f"),
    (Apple_0x0052, 0x0052, "Apple 0x0052"),
    (Apple_0x0053, 0x0053, "Apple 0x0053"),
    (Apple_0x0054, 0x0054, "Apple 0x0054"),
    (Apple_0x0055, 0x0055, "Apple 0x0055"),
    (Apple_0x0058, 0x0058, "Apple 0x0058"),
    (Apple_0x005a, 0x005a, "Apple 0x005a"),
    ]
}
