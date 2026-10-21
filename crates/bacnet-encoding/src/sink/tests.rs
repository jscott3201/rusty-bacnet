//! `EncodeSink::write_parts` on each shipped adapter: a value that fits is
//! appended part by part in order, and one that doesn't leaves the sink as
//! it was (#1614).

use super::*;

const PARTS: [&[u8]; 3] = [&[1, 2], &[], &[3, 4, 5]];

#[test]
fn slice_appends_every_part_in_order() {
    let mut storage = [0xA5; 7];
    let mut tail = &mut storage[..];
    tail.write(&[9]).unwrap();
    tail.write_parts(&PARTS).unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(storage, [9, 1, 2, 3, 4, 5, 0xA5]);
}

#[test]
fn slice_refuses_a_value_too_long_without_writing_any_part() {
    for room in 0..5 {
        let mut storage = [0xA5; 6];
        let (_, last) = storage.split_at_mut(6 - room);
        let mut tail = last;
        let refused = tail.write_parts(&PARTS).unwrap_err();
        assert_eq!(
            refused,
            SinkFull {
                needed: 5,
                available: room
            }
        );
        // Neither the cursor nor any octet moved, the first part's included.
        assert_eq!(tail.len(), room);
        assert_eq!(storage, [0xA5; 6]);
    }
}

#[test]
fn an_empty_value_writes_nothing() {
    let mut storage = [0xA5; 0];
    let mut tail = &mut storage[..];
    tail.write_parts(&[]).unwrap();
    tail.write_parts(&[&[], &[]]).unwrap();
    assert!(tail.is_empty());
}

#[cfg(feature = "heapless")]
#[test]
fn heapless_appends_or_refuses_whole() {
    let mut output = heapless::Vec::<u8, 6>::new();
    output.write(&[9]).unwrap();
    output.write_parts(&PARTS).unwrap();
    assert_eq!(output[..], [9, 1, 2, 3, 4, 5]);

    let mut output = heapless::Vec::<u8, 6>::new();
    output.write(&[9, 8]).unwrap();
    let refused = output.write_parts(&PARTS).unwrap_err();
    assert_eq!(
        refused,
        SinkFull {
            needed: 5,
            available: 4
        }
    );
    assert_eq!(output[..], [9, 8]);
}

#[cfg(feature = "alloc")]
#[test]
fn growable_adapters_append_every_part_in_order() {
    let mut vec = alloc::vec![9];
    vec.write_parts(&PARTS).unwrap();
    assert_eq!(vec, [9, 1, 2, 3, 4, 5]);

    let mut bytes = bytes::BytesMut::from(&[9][..]);
    bytes.write_parts(&PARTS).unwrap();
    assert_eq!(bytes[..], [9, 1, 2, 3, 4, 5]);
}

/// Advertises room it then refuses after the first write, as a quota it
/// doesn't advertise would.
struct RefusesAfterOneWrite {
    output: [u8; 8],
    len: usize,
    writes: usize,
}

impl EncodeSink for RefusesAfterOneWrite {
    fn remaining_capacity(&self) -> Option<usize> {
        Some(self.output.len() - self.len)
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), SinkFull> {
        self.writes += 1;
        if self.writes > 1 {
            return Err(SinkFull {
                needed: bytes.len(),
                available: 0,
            });
        }
        self.output[self.len..self.len + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
        Ok(())
    }
}

/// The contract's caveat: the provided method relies on `write` honoring the
/// capacity a sink advertises. A sink that doesn't keeps the earlier parts,
/// which is why such a sink must override `write_parts`.
#[test]
fn a_sink_refusing_despite_its_advertised_room_keeps_earlier_parts() {
    let mut sink = RefusesAfterOneWrite {
        output: [0; 8],
        len: 0,
        writes: 0,
    };
    assert!(sink.write_parts(&[&[1, 2], &[3]]).is_err());
    assert_eq!(sink.output[..sink.len], [1, 2]);
}
