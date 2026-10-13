//! What the MS/TP codec's fuzz targets share.
//!
//! [`encode`], [`valid_frame`] and [`decoded_data`] are written from the
//! frame layout (Clause 9.3), the CRC definitions (Clause 9.6) and COBS
//! (Clause 9.10), with bit-at-a-time CRCs and a COBS of their own rather than
//! the codec's, and without calling the codec. The targets check the codec
//! against them, so a fault that the codec's encoder and decoder share can't
//! cancel out in a round trip. [`self_test`] pins them to frames written out
//! by hand and to the standard's worked examples.
//!
//! The COBS here works chunk by chunk, as Clause 9.10.1 describes it: the
//! data with a zero appended, cut after each zero. The codec's scans for runs
//! of non-zero octets instead, so the two share no code or structure.
//!
//! [`check_sinks`] encodes one frame through every `FrameSink` the codec
//! implements and checks what each one holds afterwards.

use std::fmt::Debug;

use bacnet_mstp_codec::FrameSink;

/// The two preamble octets.
pub const PREAMBLE: [u8; 2] = [0x55, 0xFF];
/// Octets up to the data field: the preamble, then type, destination,
/// source, two length octets and the header CRC.
pub const HEADER_END: usize = 8;
/// Largest data field of a standard frame.
pub const MAX_DATA: usize = 501;
/// Longest standard frame on the wire.
pub const MAX_WIRE: usize = HEADER_END + MAX_DATA + 2;
/// Smallest and largest Length a COBS-encoded frame's header may declare
/// (Clause 9.5.3).
pub const MIN_COBS_LENGTH: usize = 5;
/// See [`MIN_COBS_LENGTH`].
pub const MAX_COBS_LENGTH: usize = 2043;
/// Longest frame of any kind on the wire: a COBS-encoded one of the largest
/// Length.
pub const MAX_COBS_WIRE: usize = HEADER_END + MAX_COBS_LENGTH + 2;
/// Length of an extended frame's second encoded field, which carries its
/// CRC-32K.
pub const ENCODED_CRC: usize = 5;
/// The octet every fixed sink is filled with before an encode, so that a
/// write outside the frame shows.
pub const SENTINEL: u8 = 0xA5;
/// Capacity of the heapless sink the targets use: a little more than the
/// longest frame, so its prefill decides whether a frame fits.
pub const HEAPLESS_CAPACITY: usize = MAX_COBS_WIRE + 9;

/// The header CRC: CRC-8 with polynomial x^8 + x^7 + 1, shifted out least
/// significant bit first from an all-ones register, and sent complemented.
pub fn header_crc(fields: &[u8]) -> u8 {
    let mut register: u8 = 0xFF;
    for &octet in fields {
        for bit in 0..8 {
            let feedback = (register ^ (octet >> bit)) & 1;
            register >>= 1;
            if feedback != 0 {
                // x^7 + 1, bit-reversed, is again 0x81.
                register ^= 0x81;
            }
        }
    }
    !register
}

/// The data CRC: CRC-16 with polynomial x^16 + x^12 + x^5 + 1, shifted out
/// least significant bit first from an all-ones register, and sent
/// complemented, low octet first.
pub fn data_crc(data: &[u8]) -> u16 {
    let mut register: u16 = 0xFFFF;
    for &octet in data {
        for bit in 0..8 {
            let feedback = (register ^ u16::from(octet >> bit)) & 1;
            register >>= 1;
            if feedback != 0 {
                // x^12 + x^5 + 1, bit-reversed.
                register ^= 0x8408;
            }
        }
    }
    !register
}

/// The CRC-32K of an extended frame's Encoded Data, as sent: shifted out
/// least significant bit first from an all-ones register, and complemented.
/// The generator's terms below x^32 (Clause 9.6.3) each set a bit of the
/// right-shifting register, x^n at bit 31 - n.
pub fn crc32k(data: &[u8]) -> u32 {
    const TERMS: [u32; 17] = [0, 1, 2, 4, 6, 7, 10, 11, 15, 16, 17, 19, 20, 26, 28, 29, 30];
    let generator = TERMS.iter().fold(0u32, |acc, term| acc | 1 << (31 - term));
    let mut register = u32::MAX;
    for &octet in data {
        for bit in 0..8 {
            let feedback = (register ^ u32::from(octet >> bit)) & 1;
            register >>= 1;
            if feedback != 0 {
                register ^= generator;
            }
        }
    }
    !register
}

/// COBS (Clause 9.10.1), before the mask: append a zero, cut the data after
/// each zero, and code each chunk as blocks of a code octet and up to 254
/// octets. A chunk ending the data that holds a whole number of full blocks
/// gets no block for the appended zero.
pub fn cobs_encode(data: &[u8]) -> Vec<u8> {
    let mut with_zero = data.to_vec();
    with_zero.push(0);
    let mut out = Vec::new();
    let mut last_body = 0;
    for chunk in with_zero.split_inclusive(|&octet| octet == 0) {
        let mut body = &chunk[..chunk.len() - 1];
        last_body = body.len();
        while body.len() >= 254 {
            out.push(0xFF);
            out.extend_from_slice(&body[..254]);
            body = &body[254..];
        }
        out.push(u8::try_from(body.len() + 1).expect("under 255"));
        out.extend_from_slice(body);
    }
    if last_body != 0 && last_body % 254 == 0 {
        // That last block is the empty one for the appended zero.
        out.pop();
    }
    out
}

/// Undo [`cobs_encode`] the way Clause 9.10.3 does, or `None` when a code is
/// zero or counts past the end. Like it, this takes a zero inside a block and
/// an empty block after a full one that ends the field.
pub fn cobs_decode(field: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut remaining = field.len();
    let mut index = 0;
    while remaining > 0 {
        let code = usize::from(field[index]);
        if code == 0 || code > remaining {
            return None;
        }
        out.extend_from_slice(&field[index + 1..index + code]);
        index += code;
        remaining -= code;
        if remaining > 0 && code != 0xFF {
            out.push(0);
        }
    }
    Some(out)
}

/// Each octet XORed with 0x55: the mask both encoded fields go out with.
pub fn mask(octets: &[u8]) -> Vec<u8> {
    octets.iter().map(|&octet| octet ^ 0x55).collect()
}

/// The two encoded fields of an extended frame carrying `data`: the Encoded
/// Data, then the Encoded CRC-32K, with the CRC-32K of the first's octets as
/// sent, low octet first (Clause 9.10.2).
pub fn encoded_fields(data: &[u8]) -> Vec<u8> {
    let mut fields = mask(&cobs_encode(data));
    let crc = crc32k(&fields).to_le_bytes();
    fields.extend(mask(&cobs_encode(&crc)));
    fields
}

/// Wire length of a frame whose header declares `length`: an extended
/// frame's Length counts its encoded fields less two, and a standard frame
/// has no data CRC after an empty data field, so the same rule fits both.
pub const fn wire_length(length: usize) -> usize {
    if length == 0 {
        HEADER_END
    } else {
        HEADER_END + length + 2
    }
}

/// Whether `raw` is a COBS-encoded frame type (32-127).
pub const fn is_cobs(raw: u8) -> bool {
    matches!(raw, 0x20..=0x7F)
}

/// Whether `raw` is one of the two extended data types (32 and 33), the
/// COBS types the codec encodes and decodes.
pub const fn is_extended(raw: u8) -> bool {
    matches!(raw, 0x20 | 0x21)
}

/// Whether frames of type `raw` are defined without data: types 0 to 2 (the
/// token and the two poll-for-master frames) and 7 (the postponed reply).
pub const fn never_carries_data(raw: u8) -> bool {
    matches!(raw, 0 | 1 | 2 | 7)
}

/// Whether `raw` is a proprietary frame type (128-255), whose data opens
/// with a 2-octet vendor identifier.
pub const fn is_proprietary(raw: u8) -> bool {
    raw >= 0x80
}

/// The Length field of a frame of type `raw` carrying `data`.
pub fn length_field(raw: u8, data: &[u8]) -> usize {
    if is_cobs(raw) {
        encoded_fields(data).len() - 2
    } else {
        data.len()
    }
}

/// The wire octets of a frame, built field by field with no checks: a COBS
/// type gets both encoded fields, any other type its data and data CRC. The
/// Length must fit in 16 bits.
pub fn encode(raw_type: u8, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let body = if is_cobs(raw_type) {
        encoded_fields(data)
    } else if data.is_empty() {
        Vec::new()
    } else {
        let mut body = data.to_vec();
        body.extend_from_slice(&data_crc(data).to_le_bytes());
        body
    };
    let length = length_field(raw_type, data);
    let [high, low] = u16::try_from(length)
        .expect("the Length fits its field")
        .to_be_bytes();
    let fields = [raw_type, destination, source, high, low];
    let mut wire = Vec::with_capacity(HEADER_END + body.len());
    wire.extend_from_slice(&PREAMBLE);
    wire.extend_from_slice(&fields);
    wire.push(header_crc(&fields));
    wire.extend_from_slice(&body);
    wire
}

/// A frame header read from the octets after a preamble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header {
    /// Fewer than the eight octets of preamble and header.
    Incomplete,
    /// The header CRC doesn't match.
    BadCrc,
    /// The header CRC matches.
    Good {
        /// Frame type octet.
        frame_type: u8,
        /// Destination address.
        destination: u8,
        /// Source address.
        source: u8,
        /// The Length the header declares.
        length: usize,
    },
}

impl Header {
    /// Read the header of the candidate at the start of `octets`, which the
    /// caller has checked starts with the preamble.
    pub fn read(octets: &[u8]) -> Self {
        let Some(header) = octets.get(..HEADER_END) else {
            return Self::Incomplete;
        };
        if header_crc(&header[2..7]) != header[7] {
            return Self::BadCrc;
        }
        Self::Good {
            frame_type: header[2],
            destination: header[3],
            source: header[4],
            length: usize::from(u16::from_be_bytes([header[5], header[6]])),
        }
    }

    /// Whether the header passes the checks of Clause 9.5.8: its CRC, a
    /// source other than 255, and a Length within its type's bounds, at most
    /// 501 for a standard type and 5 to 2043 for a COBS one.
    pub fn passes(self) -> bool {
        match self {
            Self::Good {
                frame_type,
                source,
                length,
                ..
            } if source != 0xFF => {
                if is_cobs(frame_type) {
                    (MIN_COBS_LENGTH..=MAX_COBS_LENGTH).contains(&length)
                } else {
                    length <= MAX_DATA
                }
            }
            _ => false,
        }
    }

    /// Whether a frame with this header can be valid: it passes, and it is
    /// not a reserved COBS type (34-127).
    pub fn decodable(self) -> bool {
        self.decodable_with(false)
    }

    /// [`Self::decodable`] for a decoder that checks the reserved COBS
    /// types when `reserved_cobs` is set: then any header that passes can
    /// start a valid frame, since a receiver with COBS support checks every
    /// COBS type's encoded fields (Clause 9.5.4.9).
    pub fn decodable_with(self, reserved_cobs: bool) -> bool {
        matches!(self, Self::Good { frame_type, .. }
            if self.passes()
                && (!is_cobs(frame_type) || is_extended(frame_type) || reserved_cobs))
    }

    /// The frame's wire length, when the header passes.
    pub fn wire_length(self) -> Option<usize> {
        match self {
            Self::Good { length, .. } if self.passes() => Some(wire_length(length)),
            _ => None,
        }
    }
}

/// The wire length and data of the valid frame at the start of `octets`, or
/// `None` when there isn't one there whole. Like the codec's decoder it
/// accepts data on a type defined without any, a proprietary frame too
/// short for its vendor identifier, and the encodings Clause 9.10.3 takes
/// that a strict encoder doesn't send.
pub fn decode(octets: &[u8]) -> Option<(usize, Vec<u8>)> {
    decode_with(octets, false)
}

/// [`decode`] for a decoder that checks the reserved COBS types when
/// `reserved_cobs` is set: one of those is then valid when its encoded
/// fields are, as for Types 32 and 33.
pub fn decode_with(octets: &[u8], reserved_cobs: bool) -> Option<(usize, Vec<u8>)> {
    if !octets.starts_with(&PREAMBLE) {
        return None;
    }
    let header = Header::read(octets);
    let Header::Good {
        frame_type, length, ..
    } = header
    else {
        return None;
    };
    if !header.decodable_with(reserved_cobs) {
        return None;
    }
    let total = wire_length(length);
    let body = octets.get(HEADER_END..total)?;
    let data = if is_cobs(frame_type) {
        let (encoded, field) = body.split_at(body.len() - ENCODED_CRC);
        let data = cobs_decode(&mask(encoded))?;
        let crc = cobs_decode(&mask(field))?;
        if crc != crc32k(encoded).to_le_bytes() {
            return None;
        }
        data
    } else {
        if length != 0 && data_crc(&body[..length]).to_le_bytes() != body[length..] {
            return None;
        }
        body[..length].to_vec()
    };
    Some((total, data))
}

/// The wire length of the valid frame at the start of `octets`; see
/// [`decode`].
pub fn valid_frame(octets: &[u8]) -> Option<usize> {
    decode(octets).map(|(total, _)| total)
}

/// The data of the valid frame at the start of `octets`, decoded; see
/// [`decode`].
pub fn decoded_data(octets: &[u8]) -> Option<Vec<u8>> {
    decode(octets).map(|(_, data)| data)
}

/// `length` octets of `pattern` repeated, or a counting pattern when it is
/// empty. Lets a short input ask for a long data field.
pub fn fill(length: usize, pattern: &[u8]) -> Vec<u8> {
    if pattern.is_empty() {
        return (0..length)
            .map(|index| (index as u8).wrapping_mul(31))
            .collect();
    }
    pattern.iter().copied().cycle().take(length).collect()
}

/// Check the encoder, decoder and CRCs here against frames written out by
/// hand and the standard's worked examples:
///
/// - a Token and a Data Not Expecting Reply from slave 200, from the codec's
///   documentation;
/// - the data CRC of Annex G.2 (01 22 30 goes out with 10 BD) and the
///   CRC-32K of Annex G.3.1 (the same octets, complement 7C22A5BE);
/// - the COBS encoding of Annex T.1 ("Hello World", a newline and a zero);
/// - the Who-Has frame of Annex T.3: its header, Length 512 and header CRC
///   4E, its first encoded octets, and its Encoded CRC-32K;
/// - COBS runs at the 254-octet block limit, worked out by hand.
pub fn self_test() {
    let token = [0x55, 0xFF, 0x00, 0x10, 0x05, 0x00, 0x00, 0x8C];
    let data = [
        0x55, 0xFF, 0x06, 0x01, 0xC8, 0x00, 0x02, 0xEF, 0x01, 0x00, 0x9F, 0x16,
    ];
    assert_eq!(encode(0x00, 0x10, 0x05, &[]), token);
    assert_eq!(encode(0x06, 0x01, 0xC8, &[0x01, 0x00]), data);
    assert_eq!(valid_frame(&token), Some(token.len()));
    assert_eq!(decode(&data), Some((data.len(), vec![0x01, 0x00])));
    assert_eq!(valid_frame(&data[..data.len() - 1]), None);
    assert_eq!(data_crc(&[0x01, 0x22, 0x30]).to_le_bytes(), [0x10, 0xBD]);
    assert_eq!(crc32k(&[0x01, 0x22, 0x30]), 0x7C22_A5BE);

    let hello = b"Hello World\n\0";
    let mut cobs = vec![0x0D];
    cobs.extend_from_slice(&hello[..12]);
    cobs.push(0x01);
    assert_eq!(cobs_encode(hello), cobs);
    assert_eq!(mask(&cobs)[..4], [0x58, 0x1D, 0x30, 0x39]);
    assert_eq!(cobs_decode(&cobs).as_deref(), Some(&hello[..]));

    let mut npdu = vec![
        0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x07, 0x3D, 0xFE, 0x01, 0xEF, 0x00,
    ];
    npdu.extend((b'A'..=b'Z').flat_map(|letter| [letter; 19]));
    let who_has = encode(0x21, 0xFF, 0x01, &npdu);
    assert_eq!(who_has.len(), 522);
    assert_eq!(
        who_has[..8],
        [0x55, 0xFF, 0x21, 0xFF, 0x01, 0x02, 0x00, 0x4E]
    );
    assert_eq!(who_has[8..12], [0x50, 0x54, 0x75, 0xAA]);
    assert_eq!(who_has[517..], [0x50, 0xF9, 0xA1, 0xD6, 0xE8]);
    assert_eq!(decode(&who_has), Some((522, npdu.clone())));
    // The same encoded fields under a reserved COBS type are valid only to a
    // decoder that checks reserved types.
    let reserved = encode(0x22, 0xFF, 0x01, &npdu);
    assert_eq!(reserved[8..], who_has[8..]);
    assert_eq!(decode(&reserved), None);
    assert_eq!(decode_with(&reserved, true), Some((522, npdu)));

    let run = [0x11u8; 254];
    assert_eq!(cobs_encode(&run), [&[0xFF][..], &run].concat());
    assert_eq!(
        cobs_encode(&[&run[..], &[0x00]].concat()),
        [&[0xFF][..], &run, &[0x01, 0x01]].concat()
    );
    assert_eq!(
        cobs_encode(&[0x11; 255]),
        [&[0xFF][..], &run, &[0x02, 0x11]].concat()
    );
    assert_eq!(cobs_encode(&[]), [0x01]);
    assert_eq!(cobs_decode(&[0x00]), None);
    assert_eq!(cobs_decode(&[0x03, 0x11]), None);
}

/// How one encode exercises the sinks: the slice's room relative to the
/// frame, what the growable sinks already hold, and how full the heapless
/// sink is.
#[derive(Debug, arbitrary::Arbitrary)]
pub struct SinkKnobs {
    /// The slice sinks get the frame's wire length plus this, at least 0.
    pub slack: i8,
    /// The `Vec` and `BytesMut` sinks start with these octets.
    pub prefix: Vec<u8>,
    /// Octets the heapless sink already holds, modulo its capacity plus one.
    pub heapless_used: u16,
}

/// Encode one frame through every sink and check each: what the call
/// returns, the octets it wrote, and that a refusal left the sink as it was.
///
/// `expected` is the frame's wire octets, or the error its checks give,
/// and `wire_length` its length even when refused. `exhausted` builds the
/// error for a fixed sink without room. Returns the octets each successful
/// encode produced, for the caller to decode.
pub fn check_sinks<E>(
    expected: &Result<Vec<u8>, E>,
    wire_length: usize,
    knobs: &SinkKnobs,
    exhausted: impl Fn(usize, usize) -> E,
    encode_into: impl Fn(&mut [u8]) -> Result<usize, E>,
    encode_to_sink: impl Fn(&mut dyn FrameSink) -> Result<usize, E>,
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

    // A slice through encode_into: the frame at the front, the rest untouched.
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

    // The growable sinks keep what they held and append the frame; a refused
    // frame fails its checks before anything is written.
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
            Err(_) => assert!(written.is_empty(), "a refused frame left octets behind"),
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
        _ => assert_eq!(heapless.len(), used, "a refused frame left octets behind"),
    }
    outputs
}

/// After an encode into a sentinel-filled slice: on success the frame is at
/// the front and the rest is untouched, and on refusal nothing changed.
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
        "an encode wrote outside its frame, or a refused one wrote anything"
    );
}
