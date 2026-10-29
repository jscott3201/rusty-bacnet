//! Sink encoders for the variable-length primitives: CharacterString (UTF-8),
//! OCTET STRING and BIT STRING (#1614).
//!
//! Each stages its tag header and leading octet in a few bytes on the stack
//! and appends them with the caller's payload in one
//! [`EncodeSink::write_parts`] call, so a refusal leaves the sink as it was.
//! The octets are those of the `BytesMut` encoders: the header of Clause
//! 20.2.1, then for a CharacterString the character-set octet X'00' and the
//! UTF-8 octets (Clause 20.2.9), for a BIT STRING the unused-bit count and the
//! data octets (Clause 20.2.10), and for an OCTET STRING the octets alone
//! (Clause 20.2.8).
//!
//! A value whose contents are longer than a header's 32-bit length can say is
//! refused with [`SinkFull`], whose `needed` is its encoded length and
//! `available` the sink's room, or `u32::MAX` for a sink that advertises
//! none. The CharacterString `BytesMut` encoders, and the contents-only BIT
//! STRING one, delegate to these, a too-long CharacterString being a static
//! [`Error::Encode`](bacnet_types::error::Error::Encode). The tagged OCTET
//! and BIT STRING `BytesMut` encoders keep their own code: they write such a
//! value with its length saturated instead.
//! Only a host with more than 4 GiB in one value can meet either.

use super::charset;
use crate::sink::{EncodeSink, SinkFull};
use crate::tags::{TagClass, app_tag, encode_tag_to_sink};

/// The longest tag header [`encode_tag_to_sink`] writes.
const MAX_HEADER: usize = 7;

/// Append the header for `tag`/`class`, then `lead` and `payload`, as one
/// value.
fn write_tagged<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    class: TagClass,
    lead: &[u8],
    payload: &[u8],
) -> Result<usize, SinkFull> {
    let contents = lead.len().saturating_add(payload.len());
    let length = header_length(sink, contents)?;
    let mut header = [0; MAX_HEADER];
    let mut tail = &mut header[..];
    let header_len = encode_tag_to_sink(&mut tail, tag, class, length)
        .expect("seven octets hold any tag header");
    sink.write_parts(&[&header[..header_len], lead, payload])?;
    Ok(header_len + contents)
}

/// The header length for a value of `contents` octets bound for `sink`, or
/// the refusal of one too long for a 32-bit length. It takes the length
/// alone, so a test can reach the refusal without a 4 GiB value.
fn header_length<S: EncodeSink + ?Sized>(sink: &S, contents: usize) -> Result<u32, SinkFull> {
    u32::try_from(contents).map_err(|_| SinkFull {
        needed: contents.saturating_add(MAX_HEADER),
        available: sink.remaining_capacity().unwrap_or(u32::MAX as usize),
    })
}

/// Append `lead` and `payload` as one value, without a tag.
fn write_contents<S: EncodeSink + ?Sized>(
    sink: &mut S,
    lead: u8,
    payload: &[u8],
) -> Result<usize, SinkFull> {
    sink.write_parts(&[&[lead], payload])?;
    Ok(1 + payload.len())
}

/// Append a UTF-8 CharacterString's contents, without a tag: the
/// character-set octet X'00', then the string's octets. Returns the octets
/// appended.
pub fn encode_character_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: &str,
) -> Result<usize, SinkFull> {
    write_contents(sink, charset::UTF8, value.as_bytes())
}

/// Append an application-tagged UTF-8 CharacterString. Returns the octets
/// appended.
///
/// ```
/// use bacnet_encoding::primitives::encode_app_character_string_to_sink;
///
/// let mut storage = [0; 5];
/// let mut tail = &mut storage[..];
/// assert_eq!(encode_app_character_string_to_sink(&mut tail, "AV")?, 4);
/// // Four more octets don't fit the one left: nothing is written.
/// assert!(encode_app_character_string_to_sink(&mut tail, "AV").is_err());
/// assert_eq!(tail.len(), 1);
/// assert_eq!(storage, [0x73, 0x00, b'A', b'V', 0]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_app_character_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    value: &str,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::CHARACTER_STRING,
        TagClass::Application,
        &[charset::UTF8],
        value.as_bytes(),
    )
}

/// Append a UTF-8 CharacterString under context tag `tag`. Returns the
/// octets appended.
pub fn encode_ctx_character_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    value: &str,
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        tag,
        TagClass::Context,
        &[charset::UTF8],
        value.as_bytes(),
    )
}

/// Append an application-tagged OCTET STRING. Returns the octets appended.
pub fn encode_app_octet_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    data: &[u8],
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::OCTET_STRING,
        TagClass::Application,
        &[],
        data,
    )
}

/// Append an OCTET STRING under context tag `tag`. Returns the octets
/// appended.
pub fn encode_ctx_octet_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    data: &[u8],
) -> Result<usize, SinkFull> {
    write_tagged(sink, tag, TagClass::Context, &[], data)
}

/// Append a BIT STRING's contents, without a tag: the unused-bit count, then
/// the data octets. Returns the octets appended. Like the `BytesMut`
/// encoder, it doesn't check the count.
pub fn encode_bit_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    unused_bits: u8,
    data: &[u8],
) -> Result<usize, SinkFull> {
    write_contents(sink, unused_bits, data)
}

/// Append an application-tagged BIT STRING. Returns the octets appended.
pub fn encode_app_bit_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    unused_bits: u8,
    data: &[u8],
) -> Result<usize, SinkFull> {
    write_tagged(
        sink,
        app_tag::BIT_STRING,
        TagClass::Application,
        &[unused_bits],
        data,
    )
}

/// Append a BIT STRING under context tag `tag`. Returns the octets appended.
pub fn encode_ctx_bit_string_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag: u8,
    unused_bits: u8,
    data: &[u8],
) -> Result<usize, SinkFull> {
    write_tagged(sink, tag, TagClass::Context, &[unused_bits], data)
}

#[cfg(test)]
mod tests;
