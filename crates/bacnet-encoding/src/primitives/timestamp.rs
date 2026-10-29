//! Bounded timestamp assembly shared by the public buffer and sink encoders.

use bacnet_types::primitives::BACnetTimeStamp;

use crate::sink::{EncodeSink, SinkFull};
use crate::tags::{self, TagClass};

use super::{encode_app_date_to_sink, encode_app_time_to_sink, encode_ctx_unsigned_to_sink};

/// Append a complete bare `BACnetTimeStamp` CHOICE in one sink write.
///
/// This has the wire layout and accepted inputs of `encode_timestamp_choice`.
/// It stages at most twelve output bytes before writing to the caller sink.
/// A known shortage returns the total required and available lengths without
/// calling `write`; an arbitrary sink refusal is returned unchanged. Earlier
/// output, the cursor and unused tail remain unchanged on failure.
///
/// Date/Time fields retain their existing unvalidated behavior. The existing
/// Time alternative remains accepted; no semantic validation is added.
/// The staging bound is not a full-stack bound, and the crate still requires
/// `std` or `alloc` even when the output sink is bounded.
pub fn encode_timestamp_choice_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    timestamp: &BACnetTimeStamp,
) -> Result<usize, SinkFull> {
    let mut encoded = [0; 12];
    let used = stage_choice(&mut &mut encoded[..], timestamp)
        .expect("each timestamp choice fits twelve bytes");
    write_timestamp(sink, &encoded[..used])
}

/// Append an outer context frame and its complete timestamp in one sink write.
///
/// This preserves `encode_timestamp`'s layout, including both nested
/// context-2 frames when a DateTime is enclosed by outer tag 2. It stages at
/// most sixteen output bytes. The [bare encoder's storage contract](encode_timestamp_choice_to_sink)
/// applies to the entire outer frame, not separately to its markers and value.
/// The tag number keeps its existing unvalidated input behavior.
///
/// ```
/// use bacnet_encoding::primitives::encode_timestamp_to_sink;
/// use bacnet_types::primitives::BACnetTimeStamp;
///
/// let mut storage = [0xA5; 7];
/// let mut tail = &mut storage[..];
/// let stamp = BACnetTimeStamp::SequenceNumber(256);
/// assert_eq!(encode_timestamp_to_sink(&mut tail, 15, &stamp)?, 7);
/// assert!(tail.is_empty());
/// assert_eq!(storage, [0xFE, 0x0F, 0x1A, 0x01, 0x00, 0xFF, 0x0F]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_timestamp_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag_number: u8,
    timestamp: &BACnetTimeStamp,
) -> Result<usize, SinkFull> {
    let mut encoded = [0; 16];
    let mut tail = &mut encoded[..];
    let opening = tags::encode_opening_tag_to_sink(&mut tail, tag_number)
        .expect("an opening marker fits the timestamp staging buffer");
    let inner = stage_choice(&mut tail, timestamp)
        .expect("the timestamp choice fits after its opening marker");
    let closing = tags::encode_closing_tag_to_sink(&mut tail, tag_number)
        .expect("the closing marker fits after the timestamp choice");
    let used = opening + inner + closing;
    write_timestamp(sink, &encoded[..used])
}

// These writes target only private staging storage, never the caller sink.
fn stage_choice(tail: &mut &mut [u8], timestamp: &BACnetTimeStamp) -> Result<usize, SinkFull> {
    let before = tail.len();
    match timestamp {
        BACnetTimeStamp::Time(time) => {
            tags::encode_tag_to_sink(tail, 0, TagClass::Context, 4)?;
            tail.write(&time.encode())?;
        }
        BACnetTimeStamp::SequenceNumber(number) => {
            encode_ctx_unsigned_to_sink(tail, 1, u64::from(*number))?;
        }
        BACnetTimeStamp::DateTime { date, time } => {
            tags::encode_opening_tag_to_sink(tail, 2)?;
            encode_app_date_to_sink(tail, date)?;
            encode_app_time_to_sink(tail, time)?;
            tags::encode_closing_tag_to_sink(tail, 2)?;
        }
    }
    Ok(before - tail.len())
}

fn write_timestamp<S: EncodeSink + ?Sized>(
    sink: &mut S,
    encoded: &[u8],
) -> Result<usize, SinkFull> {
    if let Some(available) = sink.remaining_capacity()
        && available < encoded.len()
    {
        return Err(SinkFull {
            needed: encoded.len(),
            available,
        });
    }
    sink.write(encoded)?;
    Ok(encoded.len())
}
