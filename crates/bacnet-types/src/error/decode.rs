//! Errors without an allocated message (#1646): [`DecodeError`], which a
//! decoder returns as [`Error::Decoding`](super::Error::Decoding), and
//! [`RangeError`], which a checked constructor returns as
//! [`Error::Range`](super::Error::Range).
//!
//! Each holds `&'static str` labels and up to two numbers, and builds its
//! text only when it is displayed. A firmware image that never displays one
//! links no formatting code for it, and every build, the hosted ones
//! included, reads the same text: the label with each placeholder in it
//! replaced by the next number:
//!
//! - `{}`: the number in decimal;
//! - `{:#x}`: the number in hexadecimal, lower case, without padding;
//! - `{:#04X}`: an octet in hexadecimal, upper case and zero-padded to two
//!   digits, such as `0x0B` (#1803); a wider number prints all its digits;
//! - `{t}`: the name of the application datatype the number is the tag of
//!   (Clause 20.2.1.4), such as `Unsigned` for 2. The names live in the
//!   display code, so one label serves every type.

use core::fmt;

use crate::enums::RejectReason;

/// Which fault a decoder found in the data it refused, so a responder can
/// name it (#1446). [`DecodingKind::reject_reason`] gives the Clause 18.9
/// Reject reason each kind draws when the data was a confirmed request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodingKind {
    /// Contents whose encoding isn't valid for their datatype: a wrong
    /// length, an Unsigned of no octets or with padding a canonical field
    /// refuses, an unknown character set, a BIT STRING's unused-bit count
    /// past 7, and the like. The kind [`Error::decoding`](super::Error::decoding)
    /// makes.
    InvalidEncoding,
    /// A value too large for its field's width, or outside the range the
    /// production or service gives it.
    OutOfRange,
    /// More than the decoder takes: a count of items past its limit, or a tag
    /// length past its sanity bound.
    Overflow,
    /// A tag that doesn't fit where it stands: another number, class or form
    /// than the member there needs, a closing tag that closes no open frame,
    /// a tag header that is itself malformed, or an opening tag nested
    /// deeper than the decoder walks.
    InvalidTag,
    /// The data ends, or the enclosing frame closes, where a required member
    /// or a frame's closing tag is due: a member is missing.
    Missing,
    /// Octets are left after the last member of a value that must fill its
    /// input: arguments beyond those the production has.
    Trailing,
    /// A well-formed encoding the decoder doesn't handle: a CharacterString
    /// in a character set it can't convert (IBM/Microsoft DBCS, JIS X 0208
    /// or UCS-4), or a segmented APDU, or one of a PDU type, that the
    /// allocation-free APDU header doesn't read.
    Unsupported,
}

impl DecodingKind {
    /// The Clause 18.9 Reject reason that names this fault in a confirmed
    /// request:
    ///
    /// | Kind | Reject reason |
    /// |---|---|
    /// | [`InvalidEncoding`](Self::InvalidEncoding) | INVALID_DATA_ENCODING |
    /// | [`OutOfRange`](Self::OutOfRange) | PARAMETER_OUT_OF_RANGE |
    /// | [`Overflow`](Self::Overflow) | BUFFER_OVERFLOW |
    /// | [`InvalidTag`](Self::InvalidTag) | INVALID_TAG |
    /// | [`Missing`](Self::Missing) | MISSING_REQUIRED_PARAMETER |
    /// | [`Trailing`](Self::Trailing) | TOO_MANY_ARGUMENTS |
    /// | [`Unsupported`](Self::Unsupported) | OTHER |
    pub fn reject_reason(self) -> RejectReason {
        match self {
            Self::InvalidEncoding => RejectReason::INVALID_DATA_ENCODING,
            Self::OutOfRange => RejectReason::PARAMETER_OUT_OF_RANGE,
            Self::Overflow => RejectReason::BUFFER_OVERFLOW,
            Self::InvalidTag => RejectReason::INVALID_TAG,
            Self::Missing => RejectReason::MISSING_REQUIRED_PARAMETER,
            Self::Trailing => RejectReason::TOO_MANY_ARGUMENTS,
            Self::Unsupported => RejectReason::OTHER,
        }
    }
}

/// A decoder's refusal, without an allocated message (#1646).
///
/// It displays as `decoding error at offset {offset}: {context}: {label}`,
/// leaving out `{context}: ` when the context is empty, and with each `{}`
/// in the label replaced by the next of [`values`](Self::values) in decimal,
/// each `{:#x}` in hexadecimal, or each `{:#04X}` as an upper-case,
/// zero-padded octet such as `0x0B`. An [`index`](Self::index) follows the
/// context as `[{index}]`.
///
/// ```
/// use bacnet_types::error::{DecodeError, DecodingKind, Error};
///
/// let refused = DecodeError::new(DecodingKind::Trailing, 9, "{} trailing byte(s)")
///     .within("IAm")
///     .with_value(2);
/// assert_eq!(refused.to_string(), "decoding error at offset 9: IAm: 2 trailing byte(s)");
/// assert_eq!(Error::from(refused).reject_reason(), Some(DecodingKind::Trailing.reject_reason()));
/// ```
///
/// It is `#[non_exhaustive]`: build one with [`new`](Self::new) and the
/// builder methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct DecodeError {
    /// Byte offset where the decoder found the fault.
    pub offset: usize,
    /// Which kind of fault it is.
    pub kind: DecodingKind,
    /// The production or member being decoded, such as `"WhoIs low limit"`;
    /// empty when the decoder names none.
    pub context: &'static str,
    /// What is wrong, such as `"expected context tag [{}]"`.
    pub label: &'static str,
    /// The numbers the label's placeholders stand for, in order; a slot with
    /// no placeholder is 0.
    pub values: [u64; 2],
    /// The zero-based position of the list item the fault is in, such as a
    /// file record, when the decoder names one (#1776). It sits in the
    /// struct's padding, so the error is no larger for it.
    pub index: Option<u16>,
}

impl DecodeError {
    /// A fault of `kind` at `offset`, described by `label`.
    pub const fn new(kind: DecodingKind, offset: usize, label: &'static str) -> Self {
        Self {
            offset,
            kind,
            context: "",
            label,
            values: [0; 2],
            index: None,
        }
    }

    /// The same fault, met decoding `context`.
    #[must_use]
    pub const fn within(mut self, context: &'static str) -> Self {
        self.context = context;
        self
    }

    /// The same fault, in the list item at `index`.
    #[must_use]
    pub const fn at_item(mut self, index: u16) -> Self {
        self.index = Some(index);
        self
    }

    /// The same fault, with the number the label's one placeholder stands
    /// for.
    #[must_use]
    pub const fn with_value(mut self, value: u64) -> Self {
        self.values = [value, 0];
        self
    }

    /// The same fault, with the numbers the label's two placeholders stand
    /// for.
    #[must_use]
    pub const fn with_values(mut self, first: u64, second: u64) -> Self {
        self.values = [first, second];
        self
    }

    /// The context and label with their numbers, without the
    /// `decoding error at offset {offset}: ` prefix.
    pub fn message(&self) -> impl fmt::Display + '_ {
        DecodeText(self)
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decoding error at offset {}: ", self.offset)?;
        fmt::Display::fmt(&self.message(), f)
    }
}

impl core::error::Error for DecodeError {}

/// [`DecodeError::message`].
struct DecodeText<'a>(&'a DecodeError);

impl fmt::Display for DecodeText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = self.0;
        if !error.context.is_empty() || error.index.is_some() {
            f.write_str(error.context)?;
            if let Some(index) = error.index {
                write!(f, "[{index}]")?;
            }
            f.write_str(": ")?;
        }
        write_label(f, error.label, error.values)
    }
}

/// A value a checked constructor refused for lying outside the range its
/// type takes, such as an object instance past 4,194,303 (#1646).
///
/// Like [`DecodeError`] it carries a static label and up to two numbers,
/// formatted when displayed. It displays as `value out of range: {label}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RangeError {
    /// What is out of range, such as `"instance number {} exceeds max 4194303"`.
    pub label: &'static str,
    /// The numbers the label's placeholders stand for, in order; a slot with
    /// no placeholder is 0.
    pub values: [u64; 2],
}

impl RangeError {
    /// A refusal described by `label`.
    pub const fn new(label: &'static str) -> Self {
        Self {
            label,
            values: [0; 2],
        }
    }

    /// The same refusal, with the number the label's one placeholder stands
    /// for.
    #[must_use]
    pub const fn with_value(mut self, value: u64) -> Self {
        self.values = [value, 0];
        self
    }

    /// The same refusal, with the numbers the label's two placeholders stand
    /// for.
    #[must_use]
    pub const fn with_values(mut self, first: u64, second: u64) -> Self {
        self.values = [first, second];
        self
    }

    /// The label with its numbers, without the `value out of range: ` prefix.
    pub fn message(&self) -> impl fmt::Display + '_ {
        Message(self)
    }
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("value out of range: ")?;
        write_label(f, self.label, self.values)
    }
}

impl core::error::Error for RangeError {}

/// [`RangeError::message`].
struct Message<'a>(&'a RangeError);

impl fmt::Display for Message<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_label(f, self.0.label, self.0.values)
    }
}

/// The name of the application datatype tag `number` stands for (Clause
/// 20.2.1.4), as the decoders' labels call it.
fn datatype_name(number: u64) -> &'static str {
    const NAMES: [&str; 13] = [
        "NULL",
        "BOOLEAN",
        "Unsigned",
        "INTEGER",
        "REAL",
        "Double",
        "OCTET STRING",
        "CharacterString",
        "BIT STRING",
        "ENUMERATED",
        "Date",
        "Time",
        "BACnetObjectIdentifier",
    ];
    usize::try_from(number)
        .ok()
        .and_then(|number| NAMES.get(number))
        .copied()
        .unwrap_or("value of a reserved type")
}

/// Write `label` with its placeholders filled from `values` in order (see
/// the module docs). A placeholder past the second is written as it stands.
pub(super) fn write_label(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    values: [u64; 2],
) -> fmt::Result {
    let mut rest = label;
    let mut next = values.iter();
    while let Some(at) = rest.find('{') {
        let (before, from) = rest.split_at(at);
        f.write_str(before)?;
        let placeholder = ["{}", "{:#x}", "{:#04X}", "{t}"]
            .into_iter()
            .find(|placeholder| from.starts_with(placeholder));
        match placeholder.zip(next.clone().next()) {
            Some((placeholder, &value)) => {
                next.next();
                match placeholder {
                    "{:#x}" => write!(f, "{value:#x}")?,
                    "{:#04X}" => write!(f, "{value:#04X}")?,
                    "{t}" => f.write_str(datatype_name(value))?,
                    _ => write!(f, "{value}")?,
                }
                rest = &from[placeholder.len()..];
            }
            None => {
                let placeholder = placeholder.unwrap_or("{");
                f.write_str(placeholder)?;
                rest = &from[placeholder.len()..];
            }
        }
    }
    f.write_str(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_fill_their_placeholders_in_order() {
        let display = |label, values: [u64; 2]| {
            let mut refused = DecodeError::new(DecodingKind::InvalidTag, 3, label);
            refused.values = values;
            refused.to_string()
        };
        assert_eq!(
            display("closing tag {} does not match opening tag {}", [4, 2]),
            "decoding error at offset 3: closing tag 4 does not match opening tag 2"
        );
        assert_eq!(
            display("unknown PDU type nibble: {:#x}", [0xF, 0]),
            "decoding error at offset 3: unknown PDU type nibble: 0xf"
        );
        // An octet pads to two upper-case digits; a wider value keeps them all.
        assert_eq!(
            display("type {:#04X}, function {:#04X}", [0x0B, 0x1FF]),
            "decoding error at offset 3: type 0x0B, function 0x1FF"
        );
        assert_eq!(
            display("no placeholder", [7, 8]),
            "decoding error at offset 3: no placeholder"
        );
        assert_eq!(
            display("expected application-tagged {t}, not {t}", [2, 12]),
            "decoding error at offset 3: expected application-tagged Unsigned, \
             not BACnetObjectIdentifier"
        );
        assert_eq!(
            display("{t} and {t}", [0, 13]),
            "decoding error at offset 3: NULL and value of a reserved type"
        );
        // Braces that aren't a placeholder, and a third placeholder, as written.
        assert_eq!(
            display("{a} {} {} {}", [1, 2]),
            "decoding error at offset 3: {a} 1 2 {}"
        );
        assert_eq!(
            DecodeError::new(DecodingKind::OutOfRange, 0, "[{}] value {} exceeds u32")
                .within("WhoIs low limit")
                .with_values(0, 1 << 32)
                .to_string(),
            "decoding error at offset 0: WhoIs low limit: [0] value 4294967296 exceeds u32"
        );
        assert_eq!(
            DecodeError::new(DecodingKind::InvalidTag, 7, "expected {t}")
                .within("record data")
                .at_item(1)
                .with_value(6)
                .to_string(),
            "decoding error at offset 7: record data[1]: expected OCTET STRING"
        );
    }

    #[test]
    fn range_errors_display_their_label() {
        let refused =
            RangeError::new("low limit {} is above its high limit {}").with_values(20, 10);
        assert_eq!(
            refused.to_string(),
            "value out of range: low limit 20 is above its high limit 10"
        );
        assert_eq!(
            refused.message().to_string(),
            "low limit 20 is above its high limit 10"
        );
    }

    /// The errors stay small enough to return by value on a firmware stack.
    #[test]
    fn errors_stay_small() {
        use core::mem::size_of;
        // 64 octets on a 64-bit host, the error type's tag in DecodeError's
        // niche; less on a 32-bit target, whose pointers are half the size.
        assert!(size_of::<DecodeError>() <= 64);
        assert!(size_of::<super::super::Error>() <= 64);
        assert!(size_of::<RangeError>() <= 32);
        assert!(size_of::<super::super::EncodeError>() <= 56);
        assert!(size_of::<super::super::SegmentationError>() <= 24);
    }
}

// On a 32-bit target the error is 48 octets: the other variants fit beside
// DecodeError's niche, so the enum needs no tag of its own. A variant that
// outgrows that space adds one, which the firmware decoders pay for
// (#1802), so check-no-std's builds refuse it here.
#[cfg(target_pointer_width = "32")]
const _: () = assert!(core::mem::size_of::<super::Error>() == 48);
