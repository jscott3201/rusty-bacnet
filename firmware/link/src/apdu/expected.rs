//! What the apdu scenario reports in a good run. `tests/apdu.rs` compares
//! the native run's summary with it, and the emulation build of the `apdu`
//! image the emulated CPU's, so the two can't drift apart.
//!
//! A function rather than a `const`, as the other scenarios have: the ACK's
//! value is a `Vec`, so the summary is built on the heap.

use alloc::vec;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::{DeviceInstanceRange, IAmRequest, WhoIsRequest};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, RejectReason, Segmentation};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;

use super::{Decoded, Summary};

/// DNET 0xFFFF with an empty DADR: every network.
fn every_network() -> NpduAddress {
    NpduAddress {
        network: 0xFFFF,
        mac_address: MacAddr::new(),
    }
}

/// The device: MS/TP station 7 on network 5, behind the router.
fn device_address() -> NpduAddress {
    NpduAddress {
        network: 5,
        mac_address: MacAddr::from_slice(&[7]),
    }
}

fn object(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).expect("a valid object identifier")
}

/// The summary [`run`](super::run) returns in a good run.
///
/// # Panics
///
/// Never: the object identifiers and the Who-Is range it builds are valid.
pub fn expected() -> Summary {
    Summary {
        // One NPDU per message, octet for octet the hand-written datagram,
        // and each decoding back, layer by layer, to what was encoded.
        encoded: 4,
        encoded_octets: 14 + 25 + 18 + 23,
        golden_matches: 4,
        round_trips: 4,
        // The datagrams decode to the values their comments give.
        who_is: Some(Decoded {
            destination: Some(every_network()),
            source: None,
            hop_count: 255,
            expecting_reply: false,
            invoke_id: None,
            parameters: WhoIsRequest {
                range: Some(DeviceInstanceRange::new(1000, 2000).expect("a valid range")),
            },
        }),
        i_am: Some(Decoded {
            destination: Some(every_network()),
            source: Some(device_address()),
            hop_count: 254,
            expecting_reply: false,
            invoke_id: None,
            parameters: IAmRequest {
                object_identifier: object(ObjectType::DEVICE, 1234),
                max_apdu_length: 480,
                segmentation_supported: Segmentation::NONE,
                vendor_id: 261,
            },
        }),
        read_property: Some(Decoded {
            destination: Some(device_address()),
            source: None,
            hop_count: 255,
            expecting_reply: true,
            invoke_id: Some(42),
            parameters: ReadPropertyRequest {
                object_identifier: object(ObjectType::ANALOG_INPUT, 1),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            },
        }),
        read_property_ack: Some(Decoded {
            destination: None,
            source: Some(device_address()),
            hop_count: 255,
            expecting_reply: false,
            invoke_id: Some(42),
            parameters: ReadPropertyACK {
                object_identifier: object(ObjectType::ANALOG_INPUT, 1),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                // Application tag 4 (REAL), length 4, then 72.5.
                property_value: vec![0x44, 0x42, 0x91, 0x00, 0x00],
            },
        }),
        present_value: Some(PropertyValue::Real(72.5)),
        // Protocol version 2, DNET 0 and a 100-octet APDU size.
        refusals: 3,
        rejections: [
            // The Confirmed-Request has no service choice, and the
            // ReadProperty request no property identifier.
            Some(RejectReason::MISSING_REQUIRED_PARAMETER),
            Some(RejectReason::MISSING_REQUIRED_PARAMETER),
            // The Unsigned after the ReadProperty request's arguments.
            Some(RejectReason::TOO_MANY_ARGUMENTS),
            // The Who-Is's low limit needs its high limit (#1447).
            Some(RejectReason::MISSING_REQUIRED_PARAMETER),
            // A vendor identifier is an Unsigned16.
            Some(RejectReason::PARAMETER_OUT_OF_RANGE),
        ],
        errors: 0,
    }
}
