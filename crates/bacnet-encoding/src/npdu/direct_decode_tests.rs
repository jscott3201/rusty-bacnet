//! The direct decode against the view (#2334). [`decode_npdu`] reads a frame
//! the view accepts without the view, so these tests compare it, result for
//! result, with a decode written on the view alone, as `decode_npdu` was
//! before: over valid frames of every shape, each with every header bit
//! flipped, every control octet, every value of each length octet, each
//! address at every length and the reserved network numbers, over the
//! encoding fuzz crate's seeds behind every control octet, and over
//! generated frames, and over every cut of each of those.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::*;
use proptest::prelude::*;
use proptest::sample::Index;

/// `decode_npdu` as it was before #2334: the view's fields, owned, or the
/// view's refusal. It shares no code with `decode_npdu`, so a change to the
/// direct path or to its fallback shows as a difference.
fn through_view(frame: &Bytes) -> Result<Npdu, NpduDecodeError> {
    let owned = |address: view::Address<'_>| NpduAddress {
        network: address.network,
        mac_address: MacAddr::from_slice(address.mac),
    };
    match view::Npdu::decode(frame) {
        Ok(decoded) => Ok(Npdu {
            expecting_reply: decoded.expecting_reply,
            priority: decoded.priority,
            destination: decoded.destination.map(owned),
            source: decoded.source.map(owned),
            hop_count: decoded.hop_count,
            message_type: decoded.message_type,
            vendor_id: decoded.vendor_id,
            payload: frame.slice(decoded.payload_offset..),
        }),
        Err(view::DecodeError::AddressTooLong {
            field,
            length,
            dnet,
            source,
        }) => Err(NpduDecodeError::AddressTooLong {
            field,
            length,
            dnet,
            source: source.map(owned),
        }),
        Err(view::DecodeError::Malformed(fault)) => Err(NpduDecodeError::Malformed(fault)),
    }
}

/// How many inputs a test compared, and which of the view's outcomes they
/// drew.
#[derive(Default)]
struct Tally {
    inputs: usize,
    outcomes: BTreeSet<&'static str>,
}

/// Every outcome [`outcome`] names: accepted, and each refusal the view has.
const OUTCOMES: [&str; 9] = [
    "accepted",
    "truncated",
    "version",
    "reserved network",
    "empty source",
    "long DADR, no source behind it",
    "long DADR, a source behind it",
    "long SADR, no DNET",
    "long SADR, a DNET",
];

fn outcome(result: &Result<Npdu, NpduDecodeError>) -> &'static str {
    use view::AddressField::{Destination, Source};
    match result {
        Ok(_) => "accepted",
        Err(NpduDecodeError::Malformed(view::Malformed::Truncated)) => "truncated",
        Err(NpduDecodeError::Malformed(view::Malformed::Version)) => "version",
        Err(NpduDecodeError::Malformed(view::Malformed::ReservedNetwork)) => "reserved network",
        Err(NpduDecodeError::Malformed(view::Malformed::EmptySource)) => "empty source",
        Err(NpduDecodeError::Malformed(_)) => "a fault this test doesn't know",
        Err(NpduDecodeError::AddressTooLong {
            field,
            dnet,
            source,
            ..
        }) => match (field, dnet.is_some(), source.is_some()) {
            (Destination, _, false) => "long DADR, no source behind it",
            (Destination, _, true) => "long DADR, a source behind it",
            (Source, false, _) => "long SADR, no DNET",
            (Source, true, _) => "long SADR, a DNET",
        },
    }
}

impl Tally {
    /// Holds `decode_npdu` to the view's result for `frame`, and the direct
    /// path to exactly the frames the view accepts.
    fn compare(&mut self, frame: Bytes) {
        let expected = through_view(&frame);
        assert_eq!(
            accepted_header(&frame).is_some(),
            expected.is_ok(),
            "the direct path and the view disagree on whether to accept {frame:02x?}"
        );
        assert_eq!(decode_npdu(frame.clone()), expected, "frame {frame:02x?}");
        self.inputs += 1;
        self.outcomes.insert(outcome(&expected));
    }

    /// [`compare`](Self::compare) for `frame` and every cut of it.
    fn compare_cuts(&mut self, frame: &[u8]) {
        let whole = Bytes::copy_from_slice(frame);
        for end in 0..=whole.len() {
            self.compare(whole.slice(..end));
        }
    }
}

/// An address as it lies in a frame: the length octet need not be the
/// number of MAC octets that follow it.
#[derive(Clone, Copy)]
struct Field {
    network: u16,
    length: u8,
    held: usize,
}

impl Field {
    /// An address whose length octet tells the truth.
    const fn whole(network: u16, length: u8) -> Option<Self> {
        Some(Self {
            network,
            length,
            held: length as usize,
        })
    }
}

/// The fields of one frame.
#[derive(Clone, Copy)]
struct Shape {
    /// The control octet's priority and expecting-reply bits.
    low_bits: u8,
    destination: Option<Field>,
    source: Option<Field>,
    message_type: Option<u8>,
    payload: &'static [u8],
}

/// A frame laid out from a [`Shape`], with where its fields are.
struct Laid {
    frame: Vec<u8>,
    /// Where the header ends.
    header: usize,
    /// Where each address's length octet is: DLEN, then SLEN.
    lengths: Vec<usize>,
}

impl Shape {
    fn lay(&self) -> Laid {
        let mut control = self.low_bits & 0x07;
        let mut frame = vec![view::VERSION, 0];
        let mut lengths = Vec::new();
        for (bit, field, first) in [
            (DESTINATION, self.destination, 0xD0u8),
            (SOURCE, self.source, 0x50),
        ] {
            let Some(field) = field else { continue };
            control |= bit;
            frame.extend_from_slice(&field.network.to_be_bytes());
            lengths.push(frame.len());
            frame.push(field.length);
            frame.extend((0..field.held).map(|i| first.wrapping_add(i as u8)));
        }
        if self.destination.is_some() {
            frame.push(200); // hop count
        }
        if let Some(kind) = self.message_type {
            control |= NETWORK_MESSAGE;
            frame.push(kind);
            if kind >= 0x80 {
                frame.extend_from_slice(&999u16.to_be_bytes());
            }
        }
        frame[1] = control;
        let header = frame.len();
        frame.extend_from_slice(self.payload);
        Laid {
            frame,
            header,
            lengths,
        }
    }
}

/// Valid frames of every shape: no destination, a broadcast or an address
/// of each kind of length; the same for the source; an APDU, a standard
/// message or a proprietary one. The priority, the expecting-reply bit and
/// the payload vary from one to the next.
fn valid_shapes() -> Vec<Shape> {
    let longest = MAX_MAC_LEN as u8;
    let destinations = [
        None,
        Field::whole(1, 0),
        Field::whole(0xFFFF, 0),
        Field::whole(1000, 1),
        Field::whole(0xFFFE, 6),
        Field::whole(2000, longest),
    ];
    let sources = [
        None,
        Field::whole(1, 1),
        Field::whole(0xFFFE, 6),
        Field::whole(3000, longest),
    ];
    let mut shapes = Vec::new();
    for destination in destinations {
        for source in sources {
            for message_type in [None, Some(0x01), Some(0x80)] {
                let n = shapes.len();
                shapes.push(Shape {
                    low_bits: (n % 8) as u8,
                    destination,
                    source,
                    message_type,
                    payload: if n % 2 == 0 { &[0x10, 0x08] } else { &[] },
                });
            }
        }
    }
    shapes
}

#[test]
fn direct_decode_matches_the_view_on_valid_frames_and_their_mutations() {
    let mut tally = Tally::default();
    for shape in valid_shapes() {
        let laid = shape.lay();
        let decoded = view::Npdu::decode(&laid.frame).expect("a valid frame");
        assert_eq!(decoded.payload_offset, laid.header);
        tally.compare_cuts(&laid.frame);

        // Every header bit.
        for at in 0..laid.header {
            for bit in 0..8 {
                let mut frame = laid.frame.clone();
                frame[at] ^= 1 << bit;
                tally.compare_cuts(&frame);
            }
        }
        // Every control octet over the same octets.
        for control in 0..=u8::MAX {
            let mut frame = laid.frame.clone();
            frame[1] = control;
            tally.compare_cuts(&frame);
        }
        for &at in &laid.lengths {
            // Every value of the length octet over the same octets.
            for length in 0..=u8::MAX {
                let mut frame = laid.frame.clone();
                frame[at] = length;
                tally.compare_cuts(&frame);
            }
            // The reserved network numbers.
            for network in [0u16, 0xFFFF] {
                let mut frame = laid.frame.clone();
                frame[at - 2..at].copy_from_slice(&network.to_be_bytes());
                tally.compare_cuts(&frame);
            }
        }
        // Each address at every length up to past the bound, all its octets
        // present, so what follows an over-long one is still in place.
        for length in 0..=MAX_MAC_LEN as u8 + 8 {
            for field in 0..2 {
                let mut resized = shape;
                let address = match field {
                    0 => &mut resized.destination,
                    _ => &mut resized.source,
                };
                let Some(address) = address else { continue };
                (address.length, address.held) = (length, usize::from(length));
                tally.compare_cuts(&resized.lay().frame);
            }
        }
    }
    println!("{} inputs", tally.inputs);
    assert!(tally.inputs > 1_000_000, "only {} inputs", tally.inputs);
    // Every outcome the view has was compared, so none agrees untested.
    assert_eq!(tally.outcomes, BTreeSet::from(OUTCOMES));
}

/// There is no NPDU fuzz target, so its seeds are the other targets': each
/// file of `fuzz/seeds/` as a frame, and behind every control octet, where
/// its octets are read as addresses, a hop count and a message type.
#[test]
fn direct_decode_matches_the_view_on_the_fuzz_seeds() {
    let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds");
    let (mut files, mut tally) = (0, Tally::default());
    for target in fs::read_dir(&seeds).expect("the fuzz crate's seeds") {
        for seed in fs::read_dir(target.unwrap().path()).unwrap() {
            let octets = fs::read(seed.unwrap().path()).unwrap();
            files += 1;
            tally.compare_cuts(&octets);
            for control in 0..=u8::MAX {
                let mut frame = vec![view::VERSION, control];
                frame.extend_from_slice(&octets);
                tally.compare_cuts(&frame);
            }
        }
    }
    println!("{files} seeds, {} inputs", tally.inputs);
    assert!(files > 300, "only {files} seeds under {}", seeds.display());
}

/// A network number, weighted toward the reserved and boundary ones.
fn network() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(0u16),
        Just(1),
        Just(0xFFFE),
        Just(0xFFFF),
        any::<u16>()
    ]
}

/// A length octet and the octets behind it, usually agreeing.
fn address() -> impl Strategy<Value = (u16, u8, Vec<u8>)> {
    (
        network(),
        prop::collection::vec(any::<u8>(), 0..=MAX_MAC_LEN + 2),
        prop_oneof![3 => Just(None), 1 => any::<u8>().prop_map(Some)],
    )
        .prop_map(|(network, mac, length)| {
            let length = length.unwrap_or(mac.len() as u8);
            (network, length, mac)
        })
}

/// Frames laid out field by field from any control octet, with any value in
/// any field, sometimes cut short.
fn frame() -> impl Strategy<Value = Vec<u8>> {
    (
        prop_oneof![9 => Just(view::VERSION), 1 => any::<u8>()],
        any::<u8>(),
        address(),
        address(),
        any::<u8>(),
        any::<u8>(),
        prop::collection::vec(any::<u8>(), 0..8),
        prop_oneof![3 => Just(None), 1 => any::<Index>().prop_map(Some)],
    )
        .prop_map(
            |(version, control, destination, source, hops, kind, tail, cut)| {
                let mut frame = vec![version, control];
                for (bit, (network, length, mac)) in [(DESTINATION, destination), (SOURCE, source)]
                {
                    if control & bit != 0 {
                        frame.extend_from_slice(&network.to_be_bytes());
                        frame.push(length);
                        frame.extend_from_slice(&mac);
                    }
                }
                if control & DESTINATION != 0 {
                    frame.push(hops);
                }
                if control & NETWORK_MESSAGE != 0 {
                    frame.push(kind);
                }
                frame.extend_from_slice(&tail);
                if let Some(cut) = cut {
                    frame.truncate(cut.index(frame.len() + 1));
                }
                frame
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    #[test]
    fn direct_decode_matches_the_view_on_generated_frames(frame in frame()) {
        Tally::default().compare_cuts(&frame);
    }

    #[test]
    fn direct_decode_matches_the_view_on_any_octets(
        octets in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        Tally::default().compare_cuts(&octets);
    }
}
