//! What the B/IP codec's fuzz targets share: inputs that can reach the
//! 16-bit Length field's limit cheaply, and [`check_sinks`], which encodes
//! one message through every `BvllSink` the codec implements and checks
//! what each one holds afterwards.
//!
//! The encode targets build the octets they expect field by field from the
//! Annex J.2 and U.2 layouts, without the codec's own writers or constants.

use std::fmt::Debug;

use arbitrary::Arbitrary;
use bacnet_bip_codec::BvllSink;

/// The BVLC type octet of a B/IP message (Annex J.2).
pub const TYPE_BIP: u8 = 0x81;
/// The BVLC type octet of a B/IPv6 message (Annex U.2).
pub const TYPE_BIP6: u8 = 0x82;
/// The BVLL header: type, function and the two Length octets.
pub const HEADER: usize = 4;
/// The longest message the 16-bit Length field can state, header included.
pub const MAX_LENGTH: usize = 65_535;

/// The octet every fixed sink is filled with before an encode, so that a
/// write outside the message shows.
pub const SENTINEL: u8 = 0xA5;
/// Capacity of the heapless sink: a typical Ethernet MTU's worth.
pub const HEAPLESS_CAPACITY: usize = 1536;

/// An NPDU. A long one is sized so the whole message lands within 8 octets
/// of the largest the Length field can state, on either side.
#[derive(Debug, Arbitrary)]
pub struct Npdu {
    /// The NPDU's octets, or the pattern a long one repeats.
    pub octets: Vec<u8>,
    /// Under 16, a long NPDU, this far from the start of that range. Long
    /// messages are slow to encode six times, so most inputs leave them out.
    pub long: Option<u8>,
}

impl Npdu {
    /// The NPDU's octets, in a message with `before` octets ahead of it
    /// after the header.
    pub fn octets(&self, before: usize) -> Vec<u8> {
        match self.long {
            Some(offset) if offset < 16 => {
                let length = MAX_LENGTH - HEADER - before - 8 + usize::from(offset);
                fill(length, &self.octets)
            }
            _ => self.octets.clone(),
        }
    }
}

/// `length` octets of `pattern` repeated, or a counting pattern when it is
/// empty.
fn fill(length: usize, pattern: &[u8]) -> Vec<u8> {
    if pattern.is_empty() {
        return (0..length)
            .map(|index| (index as u8).wrapping_mul(31))
            .collect();
    }
    pattern.iter().copied().cycle().take(length).collect()
}

/// A message's wire octets: the BVLC type, the function, the Length field
/// counting the whole message, then the payload. The Length field is
/// truncated for a message too long for it, which the codec must refuse.
pub fn bvll_octets(bvlc_type: u8, function: u8, payload: &[u8]) -> Vec<u8> {
    let [high, low] = ((HEADER + payload.len()) as u16).to_be_bytes();
    let mut wire = vec![bvlc_type, function, high, low];
    wire.extend_from_slice(payload);
    wire
}

/// How one encode exercises the sinks: the slices' room relative to the
/// message, what the growable sinks already hold, and how full the heapless
/// sink is.
#[derive(Debug, Arbitrary)]
pub struct SinkKnobs {
    /// The slice sinks get the message's wire length plus this, at least 0.
    pub slack: i8,
    /// The `Vec` and `BytesMut` sinks start with these octets.
    pub prefix: Vec<u8>,
    /// Octets the heapless sink already holds, modulo its capacity plus one.
    pub heapless_used: u16,
}

/// Encode one message through every sink and check each: what the call
/// returns, the octets it wrote, and that a refusal left the sink as it was.
///
/// `expected` is the message's wire octets, or the error its checks give,
/// and `wire_length` its length even when refused. `exhausted` builds the
/// error for a fixed sink without room. Returns the octets each successful
/// encode produced, for the caller to decode.
pub fn check_sinks<E>(
    expected: &Result<Vec<u8>, E>,
    wire_length: usize,
    knobs: &SinkKnobs,
    exhausted: impl Fn(usize, usize) -> E,
    encode_into: impl Fn(&mut [u8]) -> Result<usize, E>,
    encode_to_sink: impl Fn(&mut dyn BvllSink) -> Result<usize, E>,
) -> Vec<Vec<u8>>
where
    E: Clone + PartialEq + Debug,
{
    let mut outputs = Vec::new();
    let room = wire_length.saturating_add_signed(isize::from(knobs.slack));
    // A fixed sink with `room` octets free.
    let fixed = |room: usize| match expected {
        Ok(wire) if wire.len() > room => Err(exhausted(wire.len(), room)),
        Ok(wire) => Ok(wire.len()),
        Err(error) => Err(error.clone()),
    };

    // A slice through encode_into: the message at the front, the rest
    // untouched.
    let mut slice = vec![SENTINEL; room];
    let result = encode_into(&mut slice);
    assert_eq!(result, fixed(room), "slice of {room} octets");
    check_fixed(&slice, result.is_ok(), expected, &mut outputs);

    // The same through the sink trait, which advances the slice.
    let mut slice = vec![SENTINEL; room];
    let mut tail: &mut [u8] = &mut slice;
    let result = encode_to_sink(&mut tail);
    let left = tail.len();
    assert_eq!(result, fixed(room), "slice sink of {room} octets");
    assert_eq!(left, room - result.as_ref().map_or(0, |length| *length));
    check_fixed(&slice, result.is_ok(), expected, &mut outputs);

    // The growable sinks keep what they held and append the message; a
    // refused message fails its checks before anything is written.
    let mut growable = |held: &[u8], result: Result<usize, E>| {
        assert_eq!(
            result,
            expected.as_ref().map(Vec::len).map_err(Clone::clone)
        );
        let (prefix, written) = held.split_at(knobs.prefix.len());
        assert_eq!(prefix, knobs.prefix);
        match expected {
            Ok(wire) => {
                assert_eq!(written, &wire[..]);
                outputs.push(written.to_vec());
            }
            Err(_) => assert!(written.is_empty(), "a refused message left octets behind"),
        }
    };
    let mut vec = knobs.prefix.clone();
    let result = encode_to_sink(&mut vec);
    growable(&vec, result);
    let mut bytes = bytes::BytesMut::from(&knobs.prefix[..]);
    let result = encode_to_sink(&mut bytes);
    growable(&bytes, result);

    // A heapless vector with some of its capacity used.
    let used = usize::from(knobs.heapless_used) % (HEAPLESS_CAPACITY + 1);
    let mut heapless = heapless::Vec::<u8, HEAPLESS_CAPACITY>::new();
    heapless
        .resize(used, SENTINEL)
        .expect("prefill fits the capacity");
    let result = encode_to_sink(&mut heapless);
    assert_eq!(
        result,
        fixed(HEAPLESS_CAPACITY - used),
        "heapless, {used} used"
    );
    assert!(heapless[..used].iter().all(|&octet| octet == SENTINEL));
    match (&result, expected) {
        (Ok(_), Ok(wire)) => {
            assert_eq!(heapless[used..], wire[..]);
            outputs.push(heapless[used..].to_vec());
        }
        _ => assert_eq!(heapless.len(), used, "a refused message left octets behind"),
    }
    outputs
}

/// After an encode into a sentinel-filled slice: on success the message is
/// at the front and the rest is untouched, and on refusal nothing changed.
fn check_fixed<E>(
    slice: &[u8],
    accepted: bool,
    expected: &Result<Vec<u8>, E>,
    outputs: &mut Vec<Vec<u8>>,
) {
    let written = match (accepted, expected) {
        (true, Ok(wire)) => {
            assert_eq!(slice[..wire.len()], wire[..]);
            outputs.push(wire.clone());
            wire.len()
        }
        _ => 0,
    };
    assert!(
        slice[written..].iter().all(|&octet| octet == SENTINEL),
        "an encode wrote outside its message, or a refused one wrote anything"
    );
}
