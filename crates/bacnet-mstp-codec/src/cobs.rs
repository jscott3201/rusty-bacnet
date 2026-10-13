//! The two encoded fields of an extended frame (Clause 9.10), Encoded Data
//! then Encoded CRC-32K: how the codec writes them, and how it checks and
//! decodes them on receipt.
//!
//! COBS takes every zero octet out of a field. It splits the octets into code
//! blocks: a code octet, then the data octets up to the next zero, at most
//! 254 of them. The code is one more than that count. Every code below 0xFF
//! also stands for one zero after its octets; at the end of the field that
//! zero is a "phantom" one the encoder adds and the decoder drops. A block of
//! 254 octets, code 0xFF, stands for no zero, and when one ends the field no
//! block follows for the phantom zero (Clause 9.10.1, Annex T). Each octet
//! of both fields then goes out XORed with 0x55, so neither can hold the
//! first preamble octet.

use crate::crc::crc32k_update;
use crate::{FrameSink, SinkFull};

/// What each octet of both encoded fields is XORed with on the wire: the
/// first preamble octet (Clause 9.10.2).
const MASK: u8 = 0x55;
/// Most data octets one code block holds: the block with code 0xFF.
const MAX_RUN: usize = 254;
/// Length of the second encoded field: COBS adds one octet to the four CRC
/// octets.
pub(crate) const ENCODED_CRC_LENGTH: usize = 5;
/// The Length field of an extended frame counts its two encoded fields less
/// these two octets, so a receiver without COBS skips the right number of
/// octets (Clause 9.3).
pub(crate) const LENGTH_ADJUSTMENT: usize = 2;

/// Call `block` with each code block of `data` in order, as its code and its
/// data octets, unmasked.
fn for_each_block(data: &[u8], mut block: impl FnMut(u8, &[u8])) {
    let mut rest = data;
    loop {
        let run = rest
            .iter()
            .take(MAX_RUN)
            .take_while(|&&octet| octet != 0)
            .count();
        let (octets, tail) = rest.split_at(run);
        // At most 254, so the code fits an octet.
        block(run as u8 + 1, octets);
        rest = tail;
        if run == MAX_RUN {
            if rest.is_empty() {
                return;
            }
        } else {
            // The block stands for the zero that follows its octets, or at
            // the end for the phantom zero, after which nothing is left.
            match rest.split_first() {
                Some((_, after_zero)) => rest = after_zero,
                None => return,
            }
        }
    }
}

/// The Length field of an extended frame carrying `data`: its Encoded Data
/// field, plus the Encoded CRC-32K field, less two (Clause 9.3).
pub(crate) fn frame_length(data: &[u8]) -> usize {
    let mut encoded = 0;
    for_each_block(data, |_, octets| encoded += 1 + octets.len());
    encoded + ENCODED_CRC_LENGTH - LENGTH_ADJUSTMENT
}

/// Octets [`write_fields`] gathers before each write to the sink.
const CHUNK: usize = 32;

/// Masked octets on their way to a sink, a chunk at a time, with the CRC-32K
/// of everything written so far.
struct Output<'s, S: FrameSink + ?Sized> {
    sink: &'s mut S,
    chunk: [u8; CHUNK],
    used: usize,
    crc: u32,
    result: Result<(), SinkFull>,
}

impl<S: FrameSink + ?Sized> Output<'_, S> {
    /// Add the masked code blocks of `data`.
    fn blocks(&mut self, data: &[u8]) {
        for_each_block(data, |code, octets| {
            self.push(code);
            for &octet in octets {
                self.push(octet);
            }
        });
    }

    fn push(&mut self, octet: u8) {
        if self.used == CHUNK {
            self.flush();
        }
        self.chunk[self.used] = octet ^ MASK;
        self.used += 1;
    }

    /// Write what is gathered. After a failed write, nothing more is.
    fn flush(&mut self) {
        let chunk = &self.chunk[..self.used];
        self.crc = crc32k_update(self.crc, chunk);
        if self.result.is_ok() {
            self.result = self.sink.write(chunk);
        }
        self.used = 0;
    }
}

/// Write both encoded fields for `data` to `sink`.
///
/// The CRC-32K covers the Encoded Data field as it goes on the wire; its
/// complement goes into the second field low octet first (Clause 9.10.2).
pub(crate) fn write_fields<S: FrameSink + ?Sized>(
    data: &[u8],
    sink: &mut S,
) -> Result<(), SinkFull> {
    let mut output = Output {
        sink,
        chunk: [0; CHUNK],
        used: 0,
        crc: u32::MAX,
        result: Ok(()),
    };
    output.blocks(data);
    output.flush();
    let crc = !output.crc;
    output.blocks(&crc.to_le_bytes());
    output.flush();
    output.result
}

/// The length of the data that `field`, one masked encoded field, decodes
/// to. `None` when a code octet is zero or claims more octets than the field
/// has left, the two faults Clause 9.10.3 checks for.
///
/// Like the procedure there, it accepts what a strict encoder never sends: a
/// data octet that decodes to zero, or a final empty block after a full one.
fn decoded_length(field: &[u8]) -> Option<usize> {
    let mut read = 0;
    let mut length = 0;
    while let Some(&octet) = field.get(read) {
        let code = usize::from(octet ^ MASK);
        if code == 0 || code > field.len() - read {
            return None;
        }
        read += code;
        length += code - 1;
        if code != 0xFF && read < field.len() {
            length += 1;
        }
    }
    Some(length)
}

/// Decode `field`, which [`decoded_length`] accepted, over itself and return
/// the length of the decoded data at its start. Each octet is written no
/// later than it is read, so nothing is overwritten before it is used.
fn decode_in_place(field: &mut [u8]) -> usize {
    let (mut read, mut write) = (0, 0);
    while read < field.len() {
        let code = usize::from(field[read] ^ MASK);
        read += 1;
        let end = (read + code.saturating_sub(1)).min(field.len());
        while read < end {
            field[write] = field[read] ^ MASK;
            read += 1;
            write += 1;
        }
        if code != 0xFF && read < field.len() {
            field[write] = 0;
            write += 1;
        }
    }
    write
}

/// Check an extended frame's encoded fields, `fields`: the Encoded Data,
/// then the five octets of the Encoded CRC-32K. Return the length of the
/// data they carry, or `None` when either field breaks COBS's rules or the
/// CRC-32K doesn't match. Clause 9.10.3 treats both as a bad CRC.
pub(crate) fn check_fields(fields: &[u8]) -> Option<usize> {
    let split = fields.len().checked_sub(ENCODED_CRC_LENGTH)?;
    let (data, crc_field) = fields.split_at(split);
    let length = decoded_length(data)?;
    let mut crc = [0u8; ENCODED_CRC_LENGTH];
    crc.copy_from_slice(crc_field);
    // Five octets that pass always decode to four.
    decoded_length(&crc)?;
    decode_in_place(&mut crc);
    let received = u32::from_le_bytes([crc[0], crc[1], crc[2], crc[3]]);
    (received == !crc32k_update(u32::MAX, data)).then_some(length)
}

/// Decode the Encoded Data field at the start of `fields`, which
/// [`check_fields`] accepted, over itself, and return the length of the data
/// now at its start.
pub(crate) fn decode_data_in_place(fields: &mut [u8]) -> usize {
    let encoded = fields.len() - ENCODED_CRC_LENGTH;
    decode_in_place(&mut fields[..encoded])
}
