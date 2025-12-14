//
// Panasonic MakerNote Tag definitions
// Based on https://exiftool.org/TagNames/Panasonic.html
//

use super::maker_tag::{MakerTag, MakerNoteVendor};
use crate::value::Value;

/// Display Undefined value as null-terminated string
fn d_undef_as_string(value: &Value) -> String {
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

generate_maker_tags! {
    vendor: Panasonic,
    tags: [
    (ImageQuality, 0x0001, "Image Quality"),
    (FirmwareVersion, 0x0002, "Firmware Version"),
    (WhiteBalance, 0x0003, "White Balance"),
    (FocusMode, 0x0007, "Focus Mode"),
    (AFAreaMode, 0x000f, "AF Area Mode"),
    (ImageStabilization, 0x001a, "Image Stabilization"),
    (MacroMode, 0x001c, "Macro Mode"),
    (ShootingMode, 0x001f, "Shooting Mode"),
    (Audio, 0x0020, "Audio"),
    (DataDump, 0x0021, "Data Dump"),
    (WhiteBalanceBias, 0x0023, "White Balance Bias"),
    (FlashBias, 0x0024, "Flash Bias"),
    (InternalSerialNumber, 0x0025, "Internal Serial Number"),
    (PanasonicExifVersion, 0x0026, "Panasonic Exif Version"),
    (VideoFrameRate, 0x0027, "Video Frame Rate"),
    (ColorEffect, 0x0028, "Color Effect"),
    (TimeSincePowerOn, 0x0029, "Time Since Power On"),
    (BurstMode, 0x002a, "Burst Mode"),
    (SequenceNumber, 0x002b, "Sequence Number"),
    (ContrastMode, 0x002c, "Contrast Mode"),
    (NoiseReduction, 0x002d, "Noise Reduction"),
    (SelfTimer, 0x002e, "Self Timer"),
    (Rotation, 0x0030, "Rotation"),
    (AFAssistLamp, 0x0031, "AF Assist Lamp"),
    (ColorMode, 0x0032, "Color Mode"),
    (BabyAge, 0x0033, "Baby Age"),
    (OpticalZoomMode, 0x0034, "Optical Zoom Mode"),
    (ConversionLens, 0x0035, "Conversion Lens"),
    (TravelDay, 0x0036, "Travel Day"),
    (BatteryLevel, 0x0038, "Battery Level"),
    (Contrast, 0x0039, "Contrast"),
    (WorldTimeLocation, 0x003a, "World Time Location"),
    (TextStamp, 0x003b, "Text Stamp"),
    (ProgramISO, 0x003c, "Program ISO"),
    (AdvancedSceneType, 0x003d, "Advanced Scene Type"),
    (TextStamp2, 0x003e, "Text Stamp 2"),
    (FacesDetected, 0x003f, "Faces Detected"),
    (Saturation, 0x0040, "Saturation"),
    (Sharpness, 0x0041, "Sharpness"),
    (FilmMode, 0x0042, "Film Mode"),
    (JPEGQuality, 0x0043, "JPEG Quality"),
    (ColorTempKelvin, 0x0044, "Color Temp Kelvin"),
    (BracketSettings, 0x0045, "Bracket Settings"),
    (WBShiftAB, 0x0046, "WB Shift AB"),
    (WBShiftGM, 0x0047, "WB Shift GM"),
    (FlashCurtain, 0x0048, "Flash Curtain"),
    (LongExposureNoiseReduction, 0x0049, "Long Exposure Noise Reduction"),
    (PanasonicImageWidth, 0x004b, "Panasonic Image Width"),
    (PanasonicImageHeight, 0x004c, "Panasonic Image Height"),
    (AFPointPosition, 0x004d, "AF Point Position"),
    (FaceDetInfo, 0x004e, "Face Det Info"),
    (LensType, 0x0051, "Lens Type"),
    (LensSerialNumber, 0x0052, "Lens Serial Number"),
    (AccessoryType, 0x0053, "Accessory Type"),
    (AccessorySerialNumber, 0x0054, "Accessory Serial Number"),
    (Transform, 0x0059, "Transform"),
    (IntelligentExposure, 0x005d, "Intelligent Exposure"),
    (LensFirmwareVersion, 0x0060, "Lens Firmware Version"),
    (FaceRecInfo, 0x0061, "Face Rec Info"),
    (FlashWarning, 0x0062, "Flash Warning"),
    (RecognizedFaceFlags, 0x0063, "Recognized Face Flags"),
    (Title, 0x0065, "Title"),
    (BabyName, 0x0066, "Baby Name"),
    (Location, 0x0067, "Location"),
    (Country, 0x0069, "Country"),
    (State, 0x006b, "State"),
    (City, 0x006d, "City"),
    (Landmark, 0x006f, "Landmark"),
    (IntelligentResolution, 0x0070, "Intelligent Resolution"),
    (MergedImages, 0x0076, "Merged Images"),
    (BurstSpeed, 0x0077, "Burst Speed"),
    (IntelligentDRange, 0x0079, "Intelligent D-Range"),
    (ClearRetouch, 0x007c, "Clear Retouch"),
    (City2, 0x0080, "City 2"),
    (ManometerPressure, 0x0086, "Manometer Pressure"),
    (PhotoStyle, 0x0089, "Photo Style"),
    (ShadingCompensation, 0x008a, "Shading Compensation"),
    (WBShiftIntelligentAuto, 0x008b, "WB Shift Intelligent Auto"),
    (AccelerometerZ, 0x008c, "Accelerometer Z"),
    (AccelerometerX, 0x008d, "Accelerometer X"),
    (AccelerometerY, 0x008e, "Accelerometer Y"),
    (CameraOrientation, 0x008f, "Camera Orientation"),
    (RollAngle, 0x0090, "Roll Angle"),
    (PitchAngle, 0x0091, "Pitch Angle"),
    (WBShiftCreativeControl, 0x0092, "WB Shift Creative Control"),
    (SweepPanoramaDirection, 0x0093, "Sweep Panorama Direction"),
    (SweepPanoramaFieldOfView, 0x0094, "Sweep Panorama Field Of View"),
    (TimerRecording, 0x0096, "Timer Recording"),
    (InternalNDFilter, 0x009d, "Internal ND Filter"),
    (HDR, 0x009e, "HDR"),
    (ShutterType, 0x009f, "Shutter Type"),
    (FilterEffect, 0x00a1, "Filter Effect"),
    (ClearRetouchValue, 0x00a3, "Clear Retouch Value"),
    (OutputLUT, 0x00a7, "Output LUT"),
    (TouchAE, 0x00ab, "Touch AE"),
    (MonochromeFilterEffect, 0x00ac, "Monochrome Filter Effect"),
    (HighlightShadow, 0x00ad, "Highlight Shadow"),
    (TimeStamp, 0x00af, "Time Stamp"),
    (VideoBurstResolution, 0x00b3, "Video Burst Resolution"),
    (MultiExposure, 0x00b4, "Multi Exposure"),
    (RedEyeRemoval, 0x00b9, "Red Eye Removal"),
    (VideoBurstMode, 0x00bb, "Video Burst Mode"),
    (DiffractionCorrection, 0x00bc, "Diffraction Correction"),
    (FocusBracket, 0x00bd, "Focus Bracket"),
    (LongExposureNRUsed, 0x00be, "Long Exposure NR Used"),
    (PostFocusMerging, 0x00bf, "Post Focus Merging"),
    (VideoPreburst, 0x00c1, "Video Preburst"),
    (LensTypeMake, 0x00c4, "Lens Type Make"),
    (LensTypeModel, 0x00c5, "Lens Type Model"),
    (SensorType, 0x00ca, "Sensor Type"),
    (ISO, 0x00d1, "ISO"),
    (MonochromeGrainEffect, 0x00d2, "Monochrome Grain Effect"),
    (PhotoStyleName, 0x00d5, "Photo Style Name", d_undef_as_string),
    (NoiseReductionStrength, 0x00d6, "Noise Reduction Strength"),
    (AFAreaSize, 0x00de, "AF Area Size"),
    (LensTypeModel2, 0x00e4, "Lens Type Model 2"),
    (MinimumISO, 0x00e8, "Minimum ISO"),
    (AFSubjectDetection, 0x00e9, "AF Subject Detection"),
    (DynamicRangeBoost, 0x00ee, "Dynamic Range Boost"),
    (LutPrimaryFile, 0x00f1, "Primary LUT File Name", d_undef_as_string),
    (LutPrimaryGain, 0x00f3, "Primary LUT Gain"),
    (LutSecondaryFile, 0x00f4, "Secondary LUT File Name", d_undef_as_string),
    (LutSecondaryGain, 0x00f5, "Secondary LUT Gain"),
    (PrintIM, 0x0e00, "Print IM"),
    (TimeInfo, 0x2003, "Time Info"),
    (MakerNoteVersion, 0x8000, "Maker Note Version"),
    (SceneMode, 0x8001, "Scene Mode"),
    (HighlightWarning, 0x8002, "Highlight Warning"),
    (DarkFocusEnvironment, 0x8003, "Dark Focus Environment"),
    (WBRedLevel, 0x8004, "WB Red Level"),
    (WBGreenLevel, 0x8005, "WB Green Level"),
    (WBBlueLevel, 0x8006, "WB Blue Level"),
    (TextStamp3, 0x8008, "Text Stamp 3"),
    (TextStamp4, 0x8009, "Text Stamp 4"),
    (BabyAge2, 0x8010, "Baby Age 2"),
    (Transform2, 0x8012, "Transform 2"),
    ]
}
