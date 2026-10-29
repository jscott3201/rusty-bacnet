//! The device image's application: a fixed Device object and one Analog
//! Input, served with no allocator (#1771).
//!
//! [`answer`] takes one received NPDU and writes the NPDU that answers it,
//! if any. It reads the NPDU through `bacnet-npdu`'s borrowed view (#1770),
//! the APDU through the allocation-free header of `bacnet-encoding`, and the
//! service parameters through the Who-Is and ReadProperty decoders of
//! `bacnet-services`, all built without `alloc` (#1614). It writes the
//! answer the same way, through the encoders' sink entry points, into the
//! caller's [`Scratch`] and output buffer. Nothing here knows about MS/TP:
//! the scenario in the parent module carries the answer to the link.
//!
//! What it serves:
//!
//! - **Who-Is** (Clause 16.10): a Who-Is with no range, or a range that
//!   holds [`DEVICE_INSTANCE`], is answered with an I-Am broadcast to every
//!   network, which reaches a requester behind a router too (16.10.4). Other
//!   unconfirmed requests are ignored.
//! - **ReadProperty** (Clause 15.5): the properties in `device_property`
//!   and `analog_input_property`. Object_List is the one array. The
//!   failures get the Error class and code Clause 15.5.1.3.1 lists for them:
//!   an unknown object, an unknown property, an array index on a property
//!   that isn't an array, and an index past the array's end. The Device
//!   instance 4194303 stands for this device (15.5.2).
//! - **Everything else confirmed:** a request whose parameters don't decode
//!   is rejected with the reason the decoder's error maps to, another
//!   confirmed service is rejected as unrecognized (Clause 20.1.8), and a
//!   segmented request is aborted, since this device takes none (Clause
//!   20.1.9).
//!
//! The objects are a demonstration, not a conformant Device: a product
//! adds the properties Clause 12.11 requires that are missing here, such as
//! Property_List, and its own identity.

use bacnet_encoding::apdu::header::ApduHeader;
use bacnet_encoding::primitives::{
    encode_app_bit_string_to_sink, encode_app_boolean_to_sink, encode_app_character_string_to_sink,
    encode_app_enumerated_to_sink, encode_app_object_id_to_sink, encode_app_real_to_sink,
    encode_app_unsigned_to_sink,
};
use bacnet_encoding::sink::SinkFull;
use bacnet_npdu::{Address, Npdu};
use bacnet_services::read_property::{ReadPropertyAckRef, ReadPropertyRequest};
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, DeviceStatus, EngineeringUnits, ErrorClass, ErrorCode,
    EventState, ObjectType, PduType, PropertyIdentifier, RejectReason, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;

/// The Device object's instance.
pub const DEVICE_INSTANCE: u32 = 1234;
/// The Analog Input's instance.
pub const ANALOG_INPUT_INSTANCE: u32 = 1;
/// The Device's Object_Name.
pub const DEVICE_NAME: &str = "rusty-device";
/// The Analog Input's Object_Name.
pub const ANALOG_INPUT_NAME: &str = "supply-temp";
/// The Analog Input's Present_Value, in degrees Fahrenheit.
pub const PRESENT_VALUE: f32 = 72.5;
/// The Device's Vendor_Identifier, 0 as the hosted server's default is: a
/// product puts the identifier ASHRAE assigned its vendor here.
pub const VENDOR_ID: u16 = 0;
/// The largest APDU the device takes: what an MS/TP frame carries.
pub const MAX_APDU: u16 = 480;
/// The MS/TP port's Max_Master and Max_Info_Frames, which the Device
/// reports for an MS/TP master (Clause 12.11). The scenario configures the
/// core with them.
pub const MAX_MASTER: u8 = 2;
/// See [`MAX_MASTER`].
pub const MAX_INFO_FRAMES: u8 = 1;

/// Octets for an answer's APDU, before its NPCI goes in front. Every
/// answer here is under 50 octets, the smallest APDU a client may take, so
/// none has to be checked against the requester's size. A product with
/// longer values checks each ACK against the request's max-APDU field and
/// aborts one that doesn't fit.
pub const APDU_SCRATCH: usize = 64;
/// Octets for one property value.
pub const VALUE_SCRATCH: usize = 24;
/// Octets for an answer's NPDU: the APDU, plus the longest NPCI a reply
/// carries, with a DNET, a DADR as long as any data link's and a hop count.
pub const ANSWER_LENGTH: usize = APDU_SCRATCH + 6 + bacnet_npdu::MAX_MAC_LEN;

/// The working buffers [`answer`] writes into, besides its output.
pub struct Scratch {
    apdu: [u8; APDU_SCRATCH],
    value: [u8; VALUE_SCRATCH],
}

impl Scratch {
    /// Zeroed buffers.
    pub const fn new() -> Self {
        Self {
            apdu: [0; APDU_SCRATCH],
            value: [0; VALUE_SCRATCH],
        }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

/// What [`answer`] wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Nothing: the NPDU needs no answer, or isn't one the device reads.
    Nothing,
    /// An answer of this many octets for the requester, as the reply to its
    /// confirmed request.
    Reply(usize),
    /// An I-Am of this many octets, to broadcast.
    Broadcast(usize),
}

/// The identifiers of the two objects, in Object_List's order.
#[derive(Clone, Copy)]
struct Objects {
    device: ObjectIdentifier,
    analog_input: ObjectIdentifier,
}

impl Objects {
    /// Both identifiers. Their types and instances are in range, so this
    /// never fails, but the constructor is checked, and a failure here
    /// leaves the device silent rather than linking a panic's formatting.
    fn new() -> Option<Self> {
        Some(Self {
            device: ObjectIdentifier::new(ObjectType::DEVICE, DEVICE_INSTANCE).ok()?,
            analog_input: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, ANALOG_INPUT_INSTANCE)
                .ok()?,
        })
    }
}

/// Answer `npdu`, received from the link, into `out`, with `scratch` for
/// the APDU and the property values.
///
/// Network-layer messages, NPDUs for another network and anything
/// malformed get [`Answer::Nothing`]: a device that isn't a router drops
/// them. So does a confirmed request sent to every network (or to a
/// network's broadcast address): only unconfirmed services travel by
/// broadcast (Clauses 5.4.5.1 and 6.3), as the hosted network layer also
/// enforces. A confirmed request in a link-layer broadcast frame is the
/// link's to drop, since only it sees the frame's destination. An answer
/// that doesn't fit `out`, which [`ANSWER_LENGTH`] octets always hold, is
/// dropped too.
pub fn answer(npdu: &[u8], scratch: &mut Scratch, out: &mut [u8]) -> Answer {
    let (Ok(request), Some(objects)) = (Npdu::decode(npdu), Objects::new()) else {
        return Answer::Nothing;
    };
    let for_us = request
        .destination
        .is_none_or(|destination| request.is_global() && destination.mac.is_empty());
    if request.is_network_message() || !for_us {
        return Answer::Nothing;
    }
    // Any destination left here is a broadcast: DNET 0xFFFF with no DADR.
    let unconfirmed = request
        .payload
        .first()
        .is_some_and(|&first| PduType::from_raw(first >> 4) == PduType::UNCONFIRMED_REQUEST);
    if request.destination.is_some() && !unconfirmed {
        return Answer::Nothing;
    }
    let Scratch { apdu, value } = scratch;
    let mut tail = &mut apdu[..];
    let broadcast = match serve(objects, request.payload, &mut tail, value) {
        Ok(Served::Reply) => false,
        Ok(Served::IAm) => true,
        Ok(Served::Nothing) | Err(SinkFull { .. }) => return Answer::Nothing,
    };
    let written = APDU_SCRATCH - tail.len();
    let mut reply = Npdu::apdu(&apdu[..written]);
    // An I-Am goes to every network; a reply goes back where the request
    // came from, through the router that relayed it if it came from afar.
    reply.destination = if broadcast {
        Some(Address {
            network: bacnet_npdu::GLOBAL_NETWORK,
            mac: &[],
        })
    } else {
        request.source
    };
    match reply.encode_into(out) {
        Ok(length) if broadcast => Answer::Broadcast(length),
        Ok(length) => Answer::Reply(length),
        Err(_) => Answer::Nothing,
    }
}

/// What [`serve`] wrote into its sink.
enum Served {
    Nothing,
    Reply,
    IAm,
}

/// Serve one APDU, writing the answering APDU into `out`.
fn serve(
    objects: Objects,
    apdu: &[u8],
    out: &mut &mut [u8],
    value: &mut [u8],
) -> Result<Served, SinkFull> {
    let Ok(header) = ApduHeader::decode(apdu) else {
        return refuse_segmented(apdu, out);
    };
    match header {
        (ApduHeader::UnconfirmedRequest(service), parameters)
            if service == UnconfirmedServiceChoice::WHO_IS =>
        {
            who_is(objects, parameters, out)
        }
        (ApduHeader::ConfirmedRequest(request), parameters) => {
            let invoke_id = request.invoke_id;
            if request.service_choice != ConfirmedServiceChoice::READ_PROPERTY {
                reject(out, invoke_id, RejectReason::UNRECOGNIZED_SERVICE)?;
                return Ok(Served::Reply);
            }
            match ReadPropertyRequest::decode(parameters) {
                Ok(read) => read_property(objects, &read, invoke_id, out, value)?,
                Err(error) => {
                    let reason = error.reject_reason().unwrap_or(RejectReason::OTHER);
                    reject(out, invoke_id, reason)?;
                }
            }
            Ok(Served::Reply)
        }
        _ => Ok(Served::Nothing),
    }
}

/// Abort a segmented Confirmed-Request, which the header refuses to read:
/// this device takes none. Anything else the header refuses is dropped.
fn refuse_segmented(apdu: &[u8], out: &mut &mut [u8]) -> Result<Served, SinkFull> {
    match *apdu {
        [first, _, invoke_id, ..]
            if PduType::from_raw(first >> 4) == PduType::CONFIRMED_REQUEST && first & 0x08 != 0 =>
        {
            ApduHeader::Abort {
                server: true,
                invoke_id,
                reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            }
            .encode_to_sink(out)?;
            Ok(Served::Reply)
        }
        _ => Ok(Served::Nothing),
    }
}

/// Answer a Who-Is that names this device, or every device, with an I-Am.
fn who_is(objects: Objects, parameters: &[u8], out: &mut &mut [u8]) -> Result<Served, SinkFull> {
    let Ok(who_is) = WhoIsRequest::decode(parameters) else {
        return Ok(Served::Nothing);
    };
    if who_is
        .range
        .is_some_and(|range| !range.contains(DEVICE_INSTANCE))
    {
        return Ok(Served::Nothing);
    }
    let i_am = IAmRequest {
        object_identifier: objects.device,
        max_apdu_length: u32::from(MAX_APDU),
        segmentation_supported: Segmentation::NONE,
        vendor_id: VENDOR_ID,
    };
    ApduHeader::UnconfirmedRequest(UnconfirmedServiceChoice::I_AM).encode_to_sink(out)?;
    i_am.encode_to_sink(out)?;
    Ok(Served::IAm)
}

/// Write a Reject.
fn reject(out: &mut &mut [u8], invoke_id: u8, reason: RejectReason) -> Result<usize, SinkFull> {
    ApduHeader::Reject { invoke_id, reason }.encode_to_sink(out)
}

/// Answer a ReadProperty with its value, or with the Error that says why
/// not.
fn read_property(
    objects: Objects,
    read: &ReadPropertyRequest,
    invoke_id: u8,
    out: &mut &mut [u8],
    value: &mut [u8],
) -> Result<(), SinkFull> {
    let service_choice = ConfirmedServiceChoice::READ_PROPERTY;
    let requested = read.object_identifier;
    // ALL, REQUIRED and OPTIONAL aren't properties, so they fall through to
    // UNKNOWN_PROPERTY below: a deliberate simplification for this
    // demonstration, which serves single properties only.
    // The Device instance 4194303 stands for whichever device receives the
    // request (Clause 15.5.2).
    let object_identifier = if requested.object_type() == ObjectType::DEVICE
        && requested.instance_number() == ObjectIdentifier::WILDCARD_INSTANCE
    {
        objects.device
    } else {
        requested
    };
    let capacity = value.len();
    let mut tail = &mut value[..];
    let found = read_value(
        objects,
        object_identifier,
        read.property_identifier,
        read.property_array_index,
        &mut tail,
    );
    let written = capacity - tail.len();
    match found {
        Ok(()) => {
            ApduHeader::ComplexAck {
                invoke_id,
                service_choice,
            }
            .encode_to_sink(out)?;
            ReadPropertyAckRef {
                object_identifier,
                property_identifier: read.property_identifier,
                property_array_index: read.property_array_index,
                property_value: &value[..written],
            }
            .encode_to_sink(out)?;
        }
        Err(Refusal::Error(class, code)) => {
            ApduHeader::Error {
                invoke_id,
                service_choice,
            }
            .encode_to_sink(out)?;
            encode_app_enumerated_to_sink(out, u32::from(class.to_raw()))?;
            encode_app_enumerated_to_sink(out, u32::from(code.to_raw()))?;
        }
        // A value too long for the scratch: none of this device's are.
        Err(Refusal::Full(full)) => return Err(full),
    }
    Ok(())
}

/// Why a property's value wasn't written.
enum Refusal {
    /// The Error class and code to answer with.
    Error(ErrorClass, ErrorCode),
    /// The value didn't fit its scratch.
    Full(SinkFull),
}

impl From<SinkFull> for Refusal {
    fn from(full: SinkFull) -> Self {
        Self::Full(full)
    }
}

/// The Error for a property that doesn't exist (Clause 15.5.1.3.1).
const UNKNOWN_PROPERTY: Refusal = Refusal::Error(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY);

/// Write the value of `property` of `object`, or of its element `index`.
fn read_value(
    objects: Objects,
    object_identifier: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    out: &mut &mut [u8],
) -> Result<(), Refusal> {
    if object_identifier == objects.device {
        if property == PropertyIdentifier::OBJECT_LIST {
            return object_list(objects, index, out);
        }
        device_property(objects, property, out)?;
    } else if object_identifier == objects.analog_input {
        analog_input_property(objects, property, out)?;
    } else {
        return Err(Refusal::Error(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ));
    }
    // The property exists, and it isn't an array.
    match index {
        Some(_) => Err(Refusal::Error(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        )),
        None => Ok(()),
    }
}

/// Write Object_List, an array: the whole of it, its length at index 0,
/// or one element.
fn object_list(objects: Objects, index: Option<u32>, out: &mut &mut [u8]) -> Result<(), Refusal> {
    let objects = [objects.device, objects.analog_input];
    match index {
        None => {
            for object in &objects {
                encode_app_object_id_to_sink(out, object)?;
            }
        }
        Some(0) => {
            encode_app_unsigned_to_sink(out, objects.len() as u64)?;
        }
        Some(index) => {
            let element = usize::try_from(index)
                .ok()
                .and_then(|at| objects.get(at - 1))
                .ok_or(Refusal::Error(
                    ErrorClass::PROPERTY,
                    ErrorCode::INVALID_ARRAY_INDEX,
                ))?;
            encode_app_object_id_to_sink(out, element)?;
        }
    }
    Ok(())
}

/// Protocol_Services_Supported: the 49 bits of the 2020 list (7 octets, 7
/// bits unused), with the two this device executes set, readProperty (12)
/// and who-Is (34), each at its bit from the high bit of the first octet
/// (Clause 20.2.10).
const SERVICES_SUPPORTED: [u8; 7] = [0x00, 0x08, 0x00, 0x00, 0x20, 0x00, 0x00];
/// Protocol_Object_Types_Supported: analog-input (0) and device (8), in 9
/// bits, as the hosted Device object trims the list to the highest type it
/// has.
const OBJECT_TYPES_SUPPORTED: [u8; 2] = [0x80, 0x80];

/// Write the value of a Device property other than Object_List.
fn device_property(
    objects: Objects,
    property: PropertyIdentifier,
    out: &mut &mut [u8],
) -> Result<(), Refusal> {
    type P = PropertyIdentifier;
    match property {
        P::OBJECT_IDENTIFIER => encode_app_object_id_to_sink(out, &objects.device)?,
        P::OBJECT_NAME => encode_app_character_string_to_sink(out, DEVICE_NAME)?,
        P::OBJECT_TYPE => enumerated(out, ObjectType::DEVICE.to_raw())?,
        P::SYSTEM_STATUS => enumerated(out, DeviceStatus::OPERATIONAL.to_raw())?,
        P::VENDOR_NAME => encode_app_character_string_to_sink(out, "Rusty BACnet")?,
        P::VENDOR_IDENTIFIER => encode_app_unsigned_to_sink(out, u64::from(VENDOR_ID))?,
        P::MODEL_NAME => encode_app_character_string_to_sink(out, "firmware-link")?,
        P::FIRMWARE_REVISION | P::APPLICATION_SOFTWARE_VERSION => {
            encode_app_character_string_to_sink(out, env!("CARGO_PKG_VERSION"))?
        }
        P::PROTOCOL_VERSION => {
            encode_app_unsigned_to_sink(out, u64::from(bacnet_types::PROTOCOL_VERSION))?
        }
        P::PROTOCOL_REVISION => {
            encode_app_unsigned_to_sink(out, u64::from(bacnet_types::PROTOCOL_REVISION))?
        }
        P::PROTOCOL_SERVICES_SUPPORTED => {
            encode_app_bit_string_to_sink(out, 7, &SERVICES_SUPPORTED)?
        }
        P::PROTOCOL_OBJECT_TYPES_SUPPORTED => {
            encode_app_bit_string_to_sink(out, 7, &OBJECT_TYPES_SUPPORTED)?
        }
        P::MAX_APDU_LENGTH_ACCEPTED => encode_app_unsigned_to_sink(out, u64::from(MAX_APDU))?,
        P::SEGMENTATION_SUPPORTED => enumerated(out, u32::from(Segmentation::NONE.to_raw()))?,
        P::APDU_TIMEOUT => encode_app_unsigned_to_sink(out, 3000)?,
        P::NUMBER_OF_APDU_RETRIES => encode_app_unsigned_to_sink(out, 3)?,
        // An empty list: the device binds to no other device.
        P::DEVICE_ADDRESS_BINDING => 0,
        P::DATABASE_REVISION => encode_app_unsigned_to_sink(out, 1)?,
        P::MAX_MASTER => encode_app_unsigned_to_sink(out, u64::from(MAX_MASTER))?,
        P::MAX_INFO_FRAMES => encode_app_unsigned_to_sink(out, u64::from(MAX_INFO_FRAMES))?,
        _ => return Err(UNKNOWN_PROPERTY),
    };
    Ok(())
}

/// Write an Analog Input property's value.
fn analog_input_property(
    objects: Objects,
    property: PropertyIdentifier,
    out: &mut &mut [u8],
) -> Result<(), Refusal> {
    type P = PropertyIdentifier;
    match property {
        P::OBJECT_IDENTIFIER => encode_app_object_id_to_sink(out, &objects.analog_input)?,
        P::OBJECT_NAME => encode_app_character_string_to_sink(out, ANALOG_INPUT_NAME)?,
        P::OBJECT_TYPE => enumerated(out, ObjectType::ANALOG_INPUT.to_raw())?,
        P::PRESENT_VALUE => encode_app_real_to_sink(out, PRESENT_VALUE)?,
        // IN_ALARM, FAULT, OVERRIDDEN and OUT_OF_SERVICE, all clear: 4 bits.
        P::STATUS_FLAGS => encode_app_bit_string_to_sink(out, 4, &[0x00])?,
        P::EVENT_STATE => enumerated(out, EventState::NORMAL.to_raw())?,
        P::OUT_OF_SERVICE => encode_app_boolean_to_sink(out, false)?,
        P::UNITS => enumerated(out, EngineeringUnits::DEGREES_FAHRENHEIT.to_raw())?,
        _ => return Err(UNKNOWN_PROPERTY),
    };
    Ok(())
}

/// Write an Enumerated.
fn enumerated(out: &mut &mut [u8], value: u32) -> Result<usize, SinkFull> {
    encode_app_enumerated_to_sink(out, value)
}
