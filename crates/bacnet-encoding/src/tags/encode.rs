#[cfg(feature = "alloc")]
use bytes::BytesMut;

use super::TagClass;
use crate::sink::{EncodeSink, SinkFull};

/// Encode a tag header into the buffer.
///
/// For application tags, `tag_number` identifies the datatype (0-12).
/// For context tags, `tag_number` is the field identifier (0-254).
/// This function preserves the unvalidated input behavior of the original
/// `BytesMut` encoder; it does not enforce those semantic ranges.
#[cfg(feature = "alloc")]
pub fn encode_tag(buf: &mut BytesMut, tag_number: u8, class: TagClass, length: u32) {
    encode_tag_to_sink(buf, tag_number, class, length)
        .expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-specific opening tag into a growable buffer.
#[cfg(feature = "alloc")]
pub fn encode_opening_tag(buf: &mut BytesMut, tag_number: u8) {
    encode_opening_tag_to_sink(buf, tag_number).expect("the BytesMut sink cannot return SinkFull");
}

/// Encode a context-specific closing tag into a growable buffer.
#[cfg(feature = "alloc")]
pub fn encode_closing_tag(buf: &mut BytesMut, tag_number: u8) {
    encode_closing_tag_to_sink(buf, tag_number).expect("the BytesMut sink cannot return SinkFull");
}

/// Append one tag header and return its byte count.
///
/// The complete header occupies at most seven stack bytes and is appended in
/// one [`EncodeSink::write`]. If advertised capacity is too small, no write is
/// attempted. A write's own error is returned unchanged. In either case the
/// sink contract preserves all previous output and leaves no partial header.
///
/// Like `encode_tag`, this only assembles a header: it does not validate tag
/// numbers or datatype-specific lengths, and it does not append the contents.
/// For an application Boolean, `length` carries its value (0 or 1).
///
/// ```
/// use bacnet_encoding::tags::{encode_tag_to_sink, TagClass};
///
/// let mut storage = [0; 4];
/// let mut tail = &mut storage[..];
/// assert_eq!(encode_tag_to_sink(&mut tail, 2, TagClass::Application, 254)?, 4);
/// assert!(tail.is_empty());
/// assert_eq!(storage, [0x25, 0xFE, 0x00, 0xFE]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_tag_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag_number: u8,
    class: TagClass,
    length: u32,
) -> Result<usize, SinkFull> {
    let mut header = [0; 7];
    let tag_nibble = if tag_number <= 14 {
        tag_number << 4
    } else {
        0xF0
    };
    let lvt = if length <= 4 { length as u8 } else { 5 };
    header[0] = tag_nibble | ((class as u8) << 3) | lvt;
    let mut used = 1;
    if tag_number > 14 {
        header[used] = tag_number;
        used += 1;
    }
    if length > 4 {
        if length <= 253 {
            header[used] = length as u8;
            used += 1;
        } else if length <= 65535 {
            header[used] = 254;
            header[used + 1..used + 3].copy_from_slice(&(length as u16).to_be_bytes());
            used += 3;
        } else {
            header[used] = 255;
            header[used + 1..used + 5].copy_from_slice(&length.to_be_bytes());
            used += 5;
        }
    }
    write_header(sink, &header[..used])
}

/// Append a context opening marker and return its one- or two-byte length.
///
/// Uses one atomic sink write, with the same capacity and error contract as
/// [`encode_tag_to_sink`]. No tag-number validation is added.
///
/// ```
/// use bacnet_encoding::tags::encode_opening_tag_to_sink;
///
/// let mut storage = [0; 2];
/// let mut tail = &mut storage[..];
/// assert_eq!(encode_opening_tag_to_sink(&mut tail, 15)?, 2);
/// assert_eq!(storage, [0xFE, 0x0F]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_opening_tag_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag_number: u8,
) -> Result<usize, SinkFull> {
    encode_marker(sink, tag_number, 0x0E)
}

/// Append a context closing marker and return its one- or two-byte length.
///
/// Uses one atomic sink write, with the same capacity and error contract as
/// [`encode_tag_to_sink`]. No tag-number validation is added.
///
/// ```
/// use bacnet_encoding::tags::encode_closing_tag_to_sink;
///
/// let mut storage = [0; 2];
/// let mut tail = &mut storage[..];
/// assert_eq!(encode_closing_tag_to_sink(&mut tail, 254)?, 2);
/// assert_eq!(storage, [0xFF, 0xFE]);
/// # Ok::<(), bacnet_encoding::sink::SinkFull>(())
/// ```
pub fn encode_closing_tag_to_sink<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag_number: u8,
) -> Result<usize, SinkFull> {
    encode_marker(sink, tag_number, 0x0F)
}

fn encode_marker<S: EncodeSink + ?Sized>(
    sink: &mut S,
    tag_number: u8,
    marker: u8,
) -> Result<usize, SinkFull> {
    if tag_number <= 14 {
        write_header(sink, &[(tag_number << 4) | marker])
    } else {
        write_header(sink, &[0xF0 | marker, tag_number])
    }
}

fn write_header<S: EncodeSink + ?Sized>(sink: &mut S, header: &[u8]) -> Result<usize, SinkFull> {
    if let Some(available) = sink.remaining_capacity() {
        if available < header.len() {
            return Err(SinkFull {
                needed: header.len(),
                available,
            });
        }
    }
    sink.write(header)?;
    Ok(header.len())
}
