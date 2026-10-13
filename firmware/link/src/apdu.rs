//! The apdu image's scenario.
//!
//! [`run`] drives `bacnet-encoding` and `bacnet-services`, `no_std` with
//! `alloc`, through the exchange of a client that finds a device behind a
//! router and reads one of its values: a Who-Is, the device's I-Am, a
//! ReadProperty request for an analog input's present value, and the
//! ComplexACK that answers it. It returns a [`Summary`] of what it saw.
//!
//! - **Messages:** encode each message's service parameters, then its APDU,
//!   then its NPDU, each into a `BytesMut` of its own; compare the NPDU with
//!   the datagram written by hand for it; and decode it back through all
//!   three layers.
//! - **Hand-written datagrams:** decode each datagram given to [`run`] (the
//!   image and the host test give [`DATAGRAMS`]), copied into a heap buffer
//!   as a receive path would, so a fault that encode and decode share can't
//!   cancel out. Keep what each carries, and decode the ACK's value.
//! - **Rejections:** NPDUs and APDUs the codecs refuse to encode or decode,
//!   with the kind of error each draws, and malformed APDUs and service
//!   parameters the decoders refuse, with the Reject reason the library
//!   maps each error to (`Error::reject_reason`). That is how the library
//!   classifies the fault; a responder sends a Reject only for a confirmed
//!   request, never for the Who-Is or the I-Am.
//!
//! The work lives on the heap: the `BytesMut` and `Bytes` buffers, the ACK's
//! value, and the messages the decoders' errors carry. The image gives its
//! allocator an arena of [`ARENA`] octets.
//!
//! `tests/apdu.rs` runs it natively and compares the whole summary with
//! `expected()`, then runs it again in the image's allocator over an arena
//! of that size. The `apdu` image (`src/bin/apdu.rs`) links it, and its
//! emulation build checks the emulated CPU's summary against the same.

// Only the host tests and the emulation build compare summaries, so the
// link test's images build without it. Unlike the other scenarios' consts,
// a function leaves anonymous constants in the lib's codegen unit even when
// nothing calls it, and they would renumber the local labels in a RISC-V
// image's symbol table.
#[cfg(any(not(target_os = "none"), feature = "emulate"))]
mod expected;

#[cfg(any(not(target_os = "none"), feature = "emulate"))]
pub use expected::expected;

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ComplexAck, ConfirmedRequest, UnconfirmedRequest,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress, NpduDecodeError};
use bacnet_encoding::primitives::{decode_application_value, encode_app_real};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::{DeviceInstanceRange, IAmRequest, WhoIsRequest};
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, RejectReason, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};

/// The heap the apdu image gives its allocator, in octets.
///
/// The scenario's peak use is about 300 octets: `tests/apdu.rs` runs it in
/// the same first-fit allocator over an arena of this size, on a 64-bit
/// host, whose blocks are no smaller than a 32-bit target's, and prints the
/// peak. It fails when the peak passes half the arena, so the arena always
/// holds at least twice the peak: room for fragmentation, and for a
/// scenario that grows a little before the arena has to.
pub const ARENA: usize = 1024;

/// The device the client reads: its instance, the network behind the
/// router it sits on, and its MS/TP station there.
pub const DEVICE: u32 = 1234;
/// See [`DEVICE`].
pub const DEVICE_NETWORK: u16 = 5;
/// See [`DEVICE`].
pub const DEVICE_STATION: u8 = 7;
/// The device's vendor identifier.
pub const VENDOR: u16 = 0x0105;
/// The largest APDU the device takes, as its I-Am says.
pub const DEVICE_MAX_APDU: u32 = 480;
/// The instances the Who-Is asks for.
pub const WHO_IS_LIMITS: (u32, u32) = (1000, 2000);
/// The invoke ID of the ReadProperty request, which the ACK echoes.
pub const INVOKE_ID: u8 = 42;
/// The analog input's present value, which the ACK carries.
pub const PRESENT_VALUE: f32 = 72.5;

/// The messages the scenario encodes, in this order: Who-Is, I-Am,
/// ReadProperty request, ComplexACK.
pub const MESSAGES: usize = 4;

/// Each message's NPDU, written by hand from the layouts in Clauses 6.2 and
/// 20.1 and the service productions, rather than by the encoder: the network
/// layer's version and control octets, then DNET/DLEN/DADR, SNET/SLEN/SADR
/// and the hop count where the control octet announces them, then the APDU
/// header and the service parameters. Field values whose octets differ
/// catch a swapped byte order.
pub const DATAGRAMS: [&[u8]; MESSAGES] = [
    // Who-Is, to every network (control 0x20: DNET 0xFFFF, DLEN 0, hop count
    // 255). Unconfirmed-Request 0x10, Who-Is 0x08; context [0] 1000 (0x03E8)
    // and [1] 2000 (0x07D0), two octets each.
    &[
        0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08, 0x0A, 0x03, 0xE8, 0x1A, 0x07, 0xD0,
    ],
    // I-Am, as the router passes it on: to every network, with the source
    // the router added (control 0x28: DNET 0xFFFF, DLEN 0; SNET 5, SLEN 1,
    // SADR 7; hop count 254). Unconfirmed-Request 0x10, I-Am 0x00; then
    // application-tagged: object identifier device 1234 (type 8 in the top
    // ten bits: 0x020004D2), Unsigned 480 (0x01E0), Enumerated 3
    // (no segmentation) and Unsigned 261 (0x0105).
    &[
        0x01, 0x28, 0xFF, 0xFF, 0x00, 0x00, 0x05, 0x01, 0x07, 0xFE, 0x10, 0x00, 0xC4, 0x02, 0x00,
        0x04, 0xD2, 0x22, 0x01, 0xE0, 0x91, 0x03, 0x22, 0x01, 0x05,
    ],
    // ReadProperty request to the device (control 0x24: a reply is expected;
    // DNET 5, DLEN 1, DADR 7, hop count 255). Confirmed-Request 0x02 (a
    // segmented reply is accepted), 0x05 (segments unspecified, APDUs up to
    // 1476), invoke ID 42 (0x2A), ReadProperty 0x0C; context [0] analog
    // input 1 (0x00000001) and [1] Present_Value (85, 0x55).
    &[
        0x01, 0x24, 0x00, 0x05, 0x01, 0x07, 0xFF, 0x02, 0x05, 0x2A, 0x0C, 0x0C, 0x00, 0x00, 0x00,
        0x01, 0x19, 0x55,
    ],
    // ComplexACK back from the device, with the source the router added
    // (control 0x08: SNET 5, SLEN 1, SADR 7; no hop count without a DNET).
    // ComplexACK 0x30, invoke ID 42, ReadProperty 0x0C; [0] and [1] as in
    // the request, then [3] opening, application-tagged REAL 72.5
    // (0x42910000) and [3] closing.
    &[
        0x01, 0x08, 0x00, 0x05, 0x01, 0x07, 0x30, 0x2A, 0x0C, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19,
        0x55, 0x3E, 0x44, 0x42, 0x91, 0x00, 0x00, 0x3F,
    ],
];

/// The decoding paths the scenario takes whose error it classifies by the
/// Reject reason the library maps it to.
pub const REJECTIONS: usize = 5;

/// What one datagram decoded to: its network-layer addressing, its APDU's
/// invoke ID (`None` for an unconfirmed request), and its service
/// parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded<P> {
    /// DNET and DADR; an empty DADR is a broadcast.
    pub destination: Option<NpduAddress>,
    /// SNET and SADR, which a router adds.
    pub source: Option<NpduAddress>,
    /// The hop count, which only travels with a destination; 255 otherwise.
    pub hop_count: u8,
    /// Whether the sender expects a reply.
    pub expecting_reply: bool,
    /// The invoke ID of a confirmed request or its ACK.
    pub invoke_id: Option<u8>,
    /// The service parameters.
    pub parameters: P,
}

/// What [`run`] saw, compared whole by the host test.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// NPDUs encoded, one per message.
    pub encoded: u8,
    /// Octets those NPDUs took, in all.
    pub encoded_octets: u16,
    /// NPDUs identical to their hand-written datagram.
    pub golden_matches: u8,
    /// Messages whose NPDU, APDU and service parameters each decoded back
    /// equal to what was encoded.
    pub round_trips: u8,
    /// What the Who-Is datagram decoded to.
    pub who_is: Option<Decoded<WhoIsRequest>>,
    /// What the I-Am datagram decoded to.
    pub i_am: Option<Decoded<IAmRequest>>,
    /// What the ReadProperty request datagram decoded to.
    pub read_property: Option<Decoded<ReadPropertyRequest>>,
    /// What the ComplexACK datagram decoded to.
    pub read_property_ack: Option<Decoded<ReadPropertyACK>>,
    /// The ACK's value, decoded from its application tag.
    pub present_value: Option<PropertyValue>,
    /// Refusals with the error kind each should draw, counted only when it
    /// draws that kind: an NPDU of protocol version 2 on decode
    /// (`NpduDecodeError::Malformed`), and on encode an NPDU to DNET 0 and a
    /// Confirmed-Request offering APDUs of 100 octets, a size the header
    /// can't express (both `Error::Encoding`).
    pub refusals: u8,
    /// How the library classifies each decoding fault: the Reject reason it
    /// maps the error to, `None` where the input was accepted or the error
    /// maps to none. The inputs are a Confirmed-Request that ends after its
    /// invoke ID; a ReadProperty request without its property identifier,
    /// and one with an Unsigned after it; a Who-Is with a low limit only;
    /// and an I-Am whose vendor identifier takes three octets. The last two
    /// are unconfirmed, so no Reject would ever be sent for them.
    pub rejections: [Option<RejectReason>; REJECTIONS],
    /// Calls that failed where the scenario expects success. Zero in a good
    /// run.
    pub errors: u8,
}

impl Summary {
    const fn new() -> Self {
        Self {
            encoded: 0,
            encoded_octets: 0,
            golden_matches: 0,
            round_trips: 0,
            who_is: None,
            i_am: None,
            read_property: None,
            read_property_ack: None,
            present_value: None,
            refusals: 0,
            rejections: [None; REJECTIONS],
            errors: 0,
        }
    }

    /// Count a call that failed where the scenario expects success.
    fn error(&mut self) {
        self.errors = self.errors.saturating_add(1);
    }
}

/// Run the whole scenario, decoding `datagrams` as the hand-written
/// messages: [`DATAGRAMS`], passed in so an image can hide them from the
/// optimizer.
///
/// It never panics on a failed call: it counts the failure in
/// [`Summary::errors`] and carries on.
pub fn run(datagrams: &[&[u8]; MESSAGES]) -> Summary {
    let mut summary = Summary::new();
    let [who_is, i_am, read_property, ack] = *datagrams;

    round_trip(who_is_request(), who_is, &mut summary);
    round_trip(i_am_request(), i_am, &mut summary);
    round_trip(read_property_request(), read_property, &mut summary);
    round_trip(read_property_ack(), ack, &mut summary);

    summary.who_is = receive(who_is, WHO_IS);
    summary.i_am = receive(i_am, I_AM);
    summary.read_property = receive(read_property, READ_PROPERTY);
    summary.read_property_ack = receive(ack, READ_PROPERTY_ACK);
    summary.present_value = summary.read_property_ack.as_ref().and_then(|ack| {
        decode_application_value(&ack.parameters.property_value, 0)
            .ok()
            .map(|(value, _)| value)
    });

    summary.refusals = refusals(who_is);
    summary.rejections = rejections();
    summary
}

/// Service parameters the scenario carries.
trait Parameters: Sized + PartialEq {
    /// Encode them into `buf`.
    fn encode(&self, buf: &mut BytesMut);
    /// Decode them from service-request or service-ACK octets.
    fn decode(data: &[u8]) -> Result<Self, Error>;
}

macro_rules! parameters {
    ($($service:ty),+) => {$(
        impl Parameters for $service {
            fn encode(&self, buf: &mut BytesMut) {
                <$service>::encode(self, buf);
            }
            fn decode(data: &[u8]) -> Result<Self, Error> {
                <$service>::decode(data)
            }
        }
    )+};
}

parameters!(
    WhoIsRequest,
    IAmRequest,
    ReadPropertyRequest,
    ReadPropertyACK
);

/// Which APDU carries a message's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    /// An Unconfirmed-Request for this service.
    Unconfirmed(UnconfirmedServiceChoice),
    /// A Confirmed-Request for this service, unsegmented, accepting a
    /// segmented reply of APDUs up to 1476 octets.
    Request(ConfirmedServiceChoice),
    /// An unsegmented ComplexACK for this service.
    Ack(ConfirmedServiceChoice),
}

const WHO_IS: Header = Header::Unconfirmed(UnconfirmedServiceChoice::WHO_IS);
const I_AM: Header = Header::Unconfirmed(UnconfirmedServiceChoice::I_AM);
const READ_PROPERTY: Header = Header::Request(ConfirmedServiceChoice::READ_PROPERTY);
const READ_PROPERTY_ACK: Header = Header::Ack(ConfirmedServiceChoice::READ_PROPERTY);

impl Header {
    /// The APDU carrying `parameters`.
    fn apdu(self, parameters: Bytes) -> Apdu {
        match self {
            Self::Unconfirmed(service_choice) => Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice,
                service_request: parameters,
            }),
            Self::Request(service_choice) => Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: true,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: INVOKE_ID,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_request: parameters,
            }),
            Self::Ack(service_choice) => Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: INVOKE_ID,
                sequence_number: None,
                proposed_window_size: None,
                service_choice,
                service_ack: parameters,
            }),
        }
    }

    /// The invoke ID and the parameters of `apdu`, when it is this kind of
    /// APDU for this service.
    fn open(self, apdu: Apdu) -> Option<(Option<u8>, Bytes)> {
        match (self, apdu) {
            (Self::Unconfirmed(service), Apdu::UnconfirmedRequest(pdu))
                if pdu.service_choice == service =>
            {
                Some((None, pdu.service_request))
            }
            (Self::Request(service), Apdu::ConfirmedRequest(pdu))
                if pdu.service_choice == service =>
            {
                Some((Some(pdu.invoke_id), pdu.service_request))
            }
            (Self::Ack(service), Apdu::ComplexAck(pdu)) if pdu.service_choice == service => {
                Some((Some(pdu.invoke_id), pdu.service_ack))
            }
            _ => None,
        }
    }
}

/// A message to encode: its NPDU's addressing, its APDU, and its parameters.
struct Message<P> {
    destination: Option<NpduAddress>,
    source: Option<NpduAddress>,
    hop_count: u8,
    expecting_reply: bool,
    header: Header,
    parameters: P,
}

impl<P> Message<P> {
    /// The NPDU carrying `apdu`.
    fn npdu(&self, apdu: Bytes) -> Npdu {
        Npdu {
            expecting_reply: self.expecting_reply,
            destination: self.destination.clone(),
            source: self.source.clone(),
            hop_count: self.hop_count,
            payload: apdu,
            ..Npdu::default()
        }
    }
}

/// Every network: DNET 0xFFFF with an empty DADR.
fn global_broadcast() -> NpduAddress {
    NpduAddress {
        network: 0xFFFF,
        mac_address: MacAddr::new(),
    }
}

/// The device's address behind the router.
fn device_address() -> NpduAddress {
    NpduAddress {
        network: DEVICE_NETWORK,
        mac_address: MacAddr::from_slice(&[DEVICE_STATION]),
    }
}

/// The analog input the client reads.
fn analog_input() -> Result<ObjectIdentifier, Error> {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1)
}

fn who_is_request() -> Result<Message<WhoIsRequest>, Error> {
    let (low, high) = WHO_IS_LIMITS;
    Ok(Message {
        destination: Some(global_broadcast()),
        source: None,
        hop_count: 255,
        expecting_reply: false,
        header: WHO_IS,
        parameters: WhoIsRequest {
            range: Some(DeviceInstanceRange::new(low, high)?),
        },
    })
}

fn i_am_request() -> Result<Message<IAmRequest>, Error> {
    Ok(Message {
        destination: Some(global_broadcast()),
        source: Some(device_address()),
        hop_count: 254,
        expecting_reply: false,
        header: I_AM,
        parameters: IAmRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, DEVICE)?,
            max_apdu_length: DEVICE_MAX_APDU,
            segmentation_supported: Segmentation::NONE,
            vendor_id: VENDOR,
        },
    })
}

fn read_property_request() -> Result<Message<ReadPropertyRequest>, Error> {
    Ok(Message {
        destination: Some(device_address()),
        source: None,
        hop_count: 255,
        expecting_reply: true,
        header: READ_PROPERTY,
        parameters: ReadPropertyRequest {
            object_identifier: analog_input()?,
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        },
    })
}

fn read_property_ack() -> Result<Message<ReadPropertyACK>, Error> {
    let mut value = BytesMut::new();
    encode_app_real(&mut value, PRESENT_VALUE);
    Ok(Message {
        destination: None,
        source: Some(device_address()),
        hop_count: 255,
        expecting_reply: false,
        header: READ_PROPERTY_ACK,
        parameters: ReadPropertyACK {
            object_identifier: analog_input()?,
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: value.to_vec(),
        },
    })
}

/// Encode `message` layer by layer, compare it with `datagram`, and decode
/// it back.
fn round_trip<P: Parameters>(
    message: Result<Message<P>, Error>,
    datagram: &[u8],
    summary: &mut Summary,
) {
    let Ok(message) = message else {
        summary.error();
        return;
    };
    let mut parameters = BytesMut::new();
    message.parameters.encode(&mut parameters);
    let apdu = message.header.apdu(parameters.freeze());
    let mut payload = BytesMut::new();
    if encode_apdu(&mut payload, &apdu).is_err() {
        summary.error();
        return;
    }
    let npdu = message.npdu(payload.freeze());
    let mut wire = BytesMut::new();
    if encode_npdu(&mut wire, &npdu).is_err() {
        summary.error();
        return;
    }
    summary.encoded += 1;
    summary.encoded_octets = summary
        .encoded_octets
        .saturating_add(u16::try_from(wire.len()).unwrap_or(u16::MAX));
    if wire[..] == *datagram {
        summary.golden_matches += 1;
    } else {
        summary.error();
    }

    let Ok(decoded) = decode_npdu(wire.freeze()) else {
        summary.error();
        return;
    };
    let Ok(decoded_apdu) = decode_apdu(decoded.payload.clone()) else {
        summary.error();
        return;
    };
    let parameters = message
        .header
        .open(decoded_apdu.clone())
        .and_then(|(_, octets)| P::decode(&octets).ok());
    if decoded == npdu && decoded_apdu == apdu && parameters.as_ref() == Some(&message.parameters) {
        summary.round_trips += 1;
    } else {
        summary.error();
    }
}

/// Decode `datagram` as a message `header` carries, from a heap copy.
fn receive<P: Parameters>(datagram: &[u8], header: Header) -> Option<Decoded<P>> {
    let npdu = decode_npdu(Bytes::copy_from_slice(datagram)).ok()?;
    let (invoke_id, octets) = header.open(decode_apdu(npdu.payload).ok()?)?;
    Some(Decoded {
        destination: npdu.destination,
        source: npdu.source,
        hop_count: npdu.hop_count,
        expecting_reply: npdu.expecting_reply,
        invoke_id,
        parameters: P::decode(&octets).ok()?,
    })
}

/// Take the refusal paths that have no Reject reason, `who_is` being the
/// Who-Is datagram, and count those refused with the error kind each
/// should draw.
fn refusals(who_is: &[u8]) -> u8 {
    let mut version_2 = BytesMut::from(who_is);
    if let Some(version) = version_2.first_mut() {
        *version = 2;
    }
    let version_refused = matches!(
        decode_npdu(version_2.freeze()),
        Err(NpduDecodeError::Malformed(_))
    );
    let to_network_0 = Npdu {
        destination: Some(NpduAddress {
            network: 0,
            mac_address: MacAddr::new(),
        }),
        ..Npdu::default()
    };
    let network_0_refused = matches!(
        encode_npdu(&mut BytesMut::new(), &to_network_0),
        Err(Error::Encoding(_))
    );
    // `validate_max_apdu_length` refuses the size, as an encoding error,
    // before the encoder writes the header.
    let odd_size = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 100,
        invoke_id: INVOKE_ID,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::new(),
    });
    let mut header = BytesMut::new();
    let odd_size_refused =
        matches!(encode_apdu(&mut header, &odd_size), Err(Error::Encoding(_))) && header.is_empty();
    u8::from(version_refused) + u8::from(network_0_refused) + u8::from(odd_size_refused)
}

/// Take each decoding fault and return the Reject reason the library maps
/// its error to, `None` where a call unexpectedly succeeded or its error
/// maps to none.
fn rejections() -> [Option<RejectReason>; REJECTIONS] {
    // A Confirmed-Request that ends after its invoke ID.
    let cut_apdu = Bytes::from_static(&[0x02, 0x05, 0x2A]);
    // A ReadProperty request with only its object identifier.
    let no_property = [0x0C, 0x00, 0x00, 0x00, 0x01];
    // A ReadProperty request followed by an application-tagged Unsigned 0.
    let extra_argument = [0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x21, 0x00];
    // A Who-Is with its low limit only.
    let low_limit_only = [0x0A, 0x03, 0xE8];
    // An I-Am whose vendor identifier is 65536 (0x010000).
    let wide_vendor = [
        0xC4, 0x02, 0x00, 0x04, 0xD2, 0x22, 0x01, 0xE0, 0x91, 0x03, 0x23, 0x01, 0x00, 0x00,
    ];
    [
        decode_apdu(cut_apdu).err(),
        ReadPropertyRequest::decode(&no_property).err(),
        ReadPropertyRequest::decode(&extra_argument).err(),
        WhoIsRequest::decode(&low_limit_only).err(),
        IAmRequest::decode(&wide_vendor).err(),
    ]
    .map(|error| error.and_then(|error| error.reject_reason()))
}
