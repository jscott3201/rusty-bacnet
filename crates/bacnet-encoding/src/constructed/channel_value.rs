//! BACnetChannelValue (Clause 21, as Addendum 135-2020ca extends it): the
//! value a Channel object's Present_Value holds and a WriteGroup change-list
//! entry carries.
//!
//! The production is an untagged CHOICE. Every alternative but three is a
//! primitive with its own application tag. The three constructed ones each
//! sit between an opening and a closing context tag: a BACnetLightingCommand
//! in `[0]`, and the addendum's BACnetxyColor in `[1]` and
//! BACnetColorCommand in `[2]`, each read with its own codec. The codec here
//! only finds where one value ends and which alternative it is, so callers
//! can keep the octets as they arrived.

use bacnet_types::error::Error;

use super::color_command::{self, decode_xy_color};
use super::lighting_command;
use super::tagged::expect_closing;
use crate::primitives;
use crate::tags::{self, TagClass};

/// A constructed alternative of a BACnetChannelValue: which context tag
/// frames it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructedChannelValue {
    /// `[0]`: a BACnetLightingCommand.
    LightingCommand,
    /// `[1]`: a BACnetxyColor (Addendum 135-2020ca).
    XyColor,
    /// `[2]`: a BACnetColorCommand (Addendum 135-2020ca).
    ColorCommand,
}

impl ConstructedChannelValue {
    /// The alternative context tag `tag` frames, if any.
    fn of_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::LightingCommand),
            1 => Some(Self::XyColor),
            2 => Some(Self::ColorCommand),
            _ => None,
        }
    }
}

/// Return the offset just past the single BACnetChannelValue starting at
/// `offset`.
///
/// The value is one well-formed application-tagged primitive of any character
/// set, or one of the constructed alternatives:
///
/// - `[0]` a BACnetLightingCommand. It must decode (see
///   [`decode_lighting_command`]) with no field too wide for its type, and
///   its priority, when present, must be 1 to 16; its levels, fade time and
///   operation aren't range-checked.
/// - `[1]` a BACnetxyColor: two application-tagged REALs (see
///   [`decode_xy_color`]), of any value.
/// - `[2]` a BACnetColorCommand. It must decode (see
///   [`decode_color_command`]) with no field too wide for its type; its
///   operation and fields aren't range-checked, as the colour object that
///   takes it checks them.
///
/// Each must close right after its last field. Contents that run past the
/// end of `data` fail with [`Error::BufferTooShort`]; any other malformed
/// value with [`Error::Decoding`], including a field of the wrong length
/// that the data also cuts short, and a field too wide for its type.
///
/// [`decode_lighting_command`]: super::decode_lighting_command
/// [`decode_color_command`]: super::decode_color_command
pub fn channel_value_end(data: &[u8], offset: usize) -> Result<usize, Error> {
    if offset >= data.len() {
        return Err(Error::decoding(offset, "missing channel value"));
    }
    let (tag, pos) = tags::decode_tag(data, offset)?;
    if tag.class == TagClass::Application {
        return primitives::validate_application_value(data, offset);
    }
    let kind = tag
        .is_opening
        .then(|| ConstructedChannelValue::of_tag(tag.number))
        .flatten()
        .ok_or_else(|| {
            Error::decoding(
                offset,
                "a channel value is an application-tagged primitive, or a lighting command, xy \
                 color or color command in context tag 0, 1 or 2",
            )
        })?;
    match kind {
        ConstructedChannelValue::LightingCommand => {
            let (command, end, oversized) = lighting_command::decode_fields(data, pos)?;
            let after = expect_closing(data, end, 0, "lighting command")?;
            if let Some(field) = oversized {
                return Err(Error::decoding(offset, format!("lighting command {field}")));
            }
            if let Some(priority) = command.priority.filter(|p| !(1..=16).contains(p)) {
                return Err(Error::out_of_range(
                    offset,
                    format!("lighting command priority {priority} out of range 1-16"),
                ));
            }
            Ok(after)
        }
        ConstructedChannelValue::XyColor => {
            let (_, end) = decode_xy_color(data, pos)?;
            expect_closing(data, end, 1, "xy color")
        }
        ConstructedChannelValue::ColorCommand => {
            let (_, end, oversized) = color_command::decode_fields(data, pos)?;
            let after = expect_closing(data, end, 2, "color command")?;
            if let Some(field) = oversized {
                return Err(Error::decoding(offset, format!("color command {field}")));
            }
            Ok(after)
        }
    }
}

/// Which constructed alternative `data` is, when it is exactly one
/// well-formed constructed BACnetChannelValue; `None` for a primitive,
/// anything malformed, or octets left over.
pub fn constructed_channel_value(data: &[u8]) -> Option<ConstructedChannelValue> {
    let (tag, _) = tags::decode_tag(data, 0).ok()?;
    if tag.class != TagClass::Context || !tag.is_opening {
        return None;
    }
    let kind = ConstructedChannelValue::of_tag(tag.number)?;
    (channel_value_end(data, 0).ok()? == data.len()).then_some(kind)
}
