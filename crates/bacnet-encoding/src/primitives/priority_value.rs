//! The date-time choice of `BACnetPriorityValue` (Clause 21), the element a
//! DateTime Value or DateTime Pattern Value object keeps in each set slot of
//! its Priority_Array.
//!
//! Every other choice of that production is one application-tagged
//! primitive, which the generic value codecs already carry. The date-time
//! choice is constructed: context tag 1 opens it, the application-tagged
//! Date and Time follow, and a closing tag 1 ends it (#2098). Sending the
//! Date and Time bare would make a whole-array read two values per set slot,
//! so a reader could no longer count sixteen slots.

use bacnet_types::error::{DecodeError, Error};
use bacnet_types::primitives::{Date, Time};
#[cfg(feature = "alloc")]
use bytes::BytesMut;

use crate::constructed::tagged::{self, misplaced_kind};
use crate::tags::{self, app_tag};

/// The context tag of the date-time choice of `BACnetPriorityValue`.
pub const PRIORITY_DATE_TIME_TAG: u8 = 1;

/// Append one Priority_Array element holding a date-time: opening tag 1, the
/// application-tagged Date and Time, closing tag 1 (twelve octets).
///
/// ```
/// use bacnet_encoding::primitives::encode_priority_date_time;
/// use bacnet_types::primitives::{Date, Time};
/// use bytes::BytesMut;
///
/// let date = Date { year: 126, month: 10, day: 25, day_of_week: 7 };
/// let time = Time { hour: 7, minute: 30, second: 0, hundredths: 0 };
/// let mut buf = BytesMut::new();
/// encode_priority_date_time(&mut buf, &date, &time);
/// assert_eq!(buf.len(), 12);
/// assert_eq!((buf[0], buf[11]), (0x1E, 0x1F));
/// ```
#[cfg(feature = "alloc")]
pub fn encode_priority_date_time(buf: &mut BytesMut, date: &Date, time: &Time) {
    tags::encode_opening_tag(buf, PRIORITY_DATE_TIME_TAG);
    super::encode_app_date(buf, date);
    super::encode_app_time(buf, time);
    tags::encode_closing_tag(buf, PRIORITY_DATE_TIME_TAG);
}

/// Decode the date-time choice of `BACnetPriorityValue` at `offset`,
/// returning the Date, the Time and the offset past the closing tag.
///
/// Anything else at `offset` (another choice, a bare Date) is an error, so a
/// caller can try this first and fall back to the application-tagged
/// choices. Errors follow [`constructed::tagged`](crate::constructed::tagged).
pub fn decode_priority_date_time(
    data: &[u8],
    offset: usize,
) -> Result<((Date, Time), usize), Error> {
    const WHAT: &str = "BACnetPriorityValue datetime";
    let (tag, after) = tags::decode_tag(data, offset)?;
    if !tag.is_opening_tag(PRIORITY_DATE_TIME_TAG) {
        return Err(DecodeError::new(
            misplaced_kind(data, offset, &tag, Some(PRIORITY_DATE_TIME_TAG)),
            offset,
            "expected opening tag 1 for a BACnetPriorityValue datetime",
        )
        .into());
    }
    let (date, date_end) = tagged::decode_app_fixed(data, after, app_tag::DATE, 4, WHAT)?;
    let (time, time_end) = tagged::decode_app_fixed(data, date_end, app_tag::TIME, 4, WHAT)?;
    let end = tagged::expect_closing(data, time_end, PRIORITY_DATE_TIME_TAG, WHAT)?;
    Ok(((Date::decode(date)?, Time::decode(time)?), end))
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;

    fn sample() -> (Date, Time) {
        (
            Date {
                year: 126,
                month: 10,
                day: 25,
                day_of_week: 7,
            },
            Time {
                hour: 7,
                minute: 30,
                second: 15,
                hundredths: 0,
            },
        )
    }

    #[test]
    fn the_datetime_choice_is_wrapped_in_context_tag_one() {
        let (date, time) = sample();
        let mut buf = BytesMut::new();
        encode_priority_date_time(&mut buf, &date, &time);
        assert_eq!(
            &buf[..],
            &[
                0x1E, // opening tag 1
                0xA4, 126, 10, 25, 7, // application Date
                0xB4, 7, 30, 15, 0,    // application Time
                0x1F, // closing tag 1
            ][..]
        );
    }

    #[test]
    fn the_datetime_choice_round_trips() {
        let (date, time) = sample();
        let mut buf = BytesMut::new();
        encode_priority_date_time(&mut buf, &date, &time);
        buf.extend_from_slice(&[0x00]); // a following NULL slot
        let ((d, t), end) = decode_priority_date_time(&buf, 0).unwrap();
        assert_eq!((d, t), (date, time));
        assert_eq!(end, 12);
    }

    #[test]
    fn a_bare_date_time_or_other_tag_is_not_the_datetime_choice() {
        let (date, time) = sample();
        let mut bare = BytesMut::new();
        super::super::encode_app_date(&mut bare, &date);
        super::super::encode_app_time(&mut bare, &time);
        assert!(decode_priority_date_time(&bare, 0).is_err());
        // Opening tag 0 (constructed-value) is a different choice.
        assert!(decode_priority_date_time(&[0x0E, 0x0F], 0).is_err());
        // Unclosed or truncated.
        let mut cut = BytesMut::new();
        encode_priority_date_time(&mut cut, &date, &time);
        assert!(decode_priority_date_time(&cut[..11], 0).is_err());
        assert!(decode_priority_date_time(&cut[..4], 0).is_err());
    }
}
