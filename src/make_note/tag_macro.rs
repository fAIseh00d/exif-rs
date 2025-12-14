//
// Common macro for generating vendor-specific MakerNote tags
//

/// Generates vendor-specific MakerNote tag constants and lookup functions.
///
/// This macro creates:
/// - A `tags` module with tag constants
/// - A `tag_name()` function for looking up tag names
/// - A `tag_description()` function for looking up tag descriptions
/// - A `display_value()` function for custom value formatting
///
/// # Example
/// ```ignore
/// use crate::make_note::maker_tag::{MakerTag, MakerNoteVendor};
///
/// generate_maker_tags! {
///     vendor: Panasonic,
///     tags: [
///         (ImageQuality, 0x0001, "Image Quality"),
///         (PhotoStyleName, 0x00d5, "Photo Style Name", d_undef_as_string),
///     ]
/// }
/// ```
#[macro_export]
macro_rules! generate_maker_tags {
    // With custom display function
    (
        vendor: $vendor:ident,
        tags: [
            $(
                ($name:ident, $num:expr, $desc:expr $(, $dispfn:ident)?)
            ),+ $(,)?
        ]
    ) => {
        pub mod tags {
            use super::{MakerTag, MakerNoteVendor};

            $(
                #[allow(non_upper_case_globals)]
                pub const $name: MakerTag = MakerTag::new(MakerNoteVendor::$vendor, $num);
            )+
        }

        pub(crate) fn tag_name(number: u16) -> Option<&'static str> {
            match number {
                $(
                    $num => Some(stringify!($name)),
                )+
                _ => None,
            }
        }

        pub(crate) fn tag_description(number: u16) -> Option<&'static str> {
            match number {
                $(
                    $num => Some($desc),
                )+
                _ => None,
            }
        }

        /// Display value for a tag. Returns None to use default display.
        #[allow(unused_variables)]
        pub(crate) fn display_value(number: u16, value: &crate::value::Value) -> Option<String> {
            match number {
                $(
                    $num => {
                        $(
                            return Some($dispfn(value));
                        )?
                        #[allow(unreachable_code)]
                        None
                    }
                )+
                _ => None,
            }
        }
    }
}
