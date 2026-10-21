//! Frames for the decode and capture tests, built with the stack's own
//! encoders.

use bacnet_encoding::apdu::{
    encode_apdu, Apdu, ComplexAck, ConfirmedRequest, ErrorPdu, UnconfirmedRequest,
};
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_npdu::NetworkPriority;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::bip6::{encode_bvll6, Bip6Vmac, Bvll6};
use bacnet_transport::bvll::{encode_bvll, BvlcResultCode, Bvll};
use bacnet_transport::mstp::codec::{Frame, FrameType};
use bacnet_transport::sc_frame::{encode_sc_message, ScFunction, ScMessage, Vmac};
use bacnet_types::enums::{
    ConfirmedServiceChoice, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier, Segmentation,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

/// `apdu` in an NPDU without routing.
pub(crate) fn npdu(apdu: &Apdu, expecting_reply: bool) -> Vec<u8> {
    routed_npdu(apdu, expecting_reply, None)
}

/// `apdu` in an NPDU a router relayed from `source`, SNET and SADR.
pub(crate) fn routed_npdu(
    apdu: &Apdu,
    expecting_reply: bool,
    source: Option<(u16, &[u8])>,
) -> Vec<u8> {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let npdu = Npdu {
        expecting_reply,
        priority: NetworkPriority::Normal,
        destination: None,
        source: source.map(|(network, mac)| bacnet_encoding::npdu::NpduAddress {
            network,
            mac_address: bacnet_types::MacAddr::from_slice(mac),
        }),
        hop_count: 255,
        message_type: None,
        vendor_id: None,
        payload: payload.freeze(),
    };
    let mut out = BytesMut::new();
    encode_npdu(&mut out, &npdu).unwrap();
    out.to_vec()
}

/// A ReadProperty request for analog-input 1's present-value.
pub(crate) fn rp_request(invoke_id: u8) -> Apdu {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut request);
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: request.freeze(),
    })
}

/// Its answer: the value 72.5.
pub(crate) fn rp_ack(invoke_id: u8) -> Apdu {
    rp_ack_with(invoke_id, vec![0x44, 0x42, 0x91, 0x00, 0x00])
}

/// An unconfirmed request whose parameters are `octets`, as given.
pub(crate) fn unconfirmed(octets: &[u8]) -> Apdu {
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::UNCONFIRMED_PRIVATE_TRANSFER,
        service_request: Bytes::copy_from_slice(octets),
    })
}

/// A ReadProperty answer carrying `value`, application-tagged as given.
pub(crate) fn rp_ack_with(invoke_id: u8, value: Vec<u8>) -> Apdu {
    let mut ack = BytesMut::new();
    ReadPropertyACK {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value,
    }
    .encode(&mut ack);
    Apdu::ComplexAck(ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: ack.freeze(),
    })
}

/// An Error answering `service`: property, unknown-property.
pub(crate) fn error(invoke_id: u8, service: ConfirmedServiceChoice) -> Apdu {
    Apdu::Error(ErrorPdu {
        invoke_id,
        service_choice: service,
        error_class: ErrorClass::PROPERTY,
        error_code: ErrorCode::UNKNOWN_PROPERTY,
        error_data: Bytes::new(),
    })
}

/// An I-Am from device `instance`.
pub(crate) fn i_am(instance: u32) -> Apdu {
    let mut request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 260,
    }
    .encode(&mut request);
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: request.freeze(),
    })
}

/// A B/IP Original-Unicast-NPDU, or Original-Broadcast-NPDU, of `npdu`.
pub(crate) fn bip(npdu: &[u8], broadcast: bool) -> Vec<u8> {
    let message = if broadcast {
        Bvll::OriginalBroadcastNpdu(npdu)
    } else {
        Bvll::OriginalUnicastNpdu(npdu)
    };
    let mut out = BytesMut::new();
    encode_bvll(&mut out, &message).unwrap();
    out.to_vec()
}

/// A B/IP BVLC-Result NAK for a refused registration.
pub(crate) fn bip_nak() -> Vec<u8> {
    let mut out = BytesMut::new();
    let code = BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
    encode_bvll(&mut out, &Bvll::BvlcResult(code)).unwrap();
    out.to_vec()
}

/// A B/IPv6 Original-Broadcast-NPDU from VMAC 0x000102.
pub(crate) fn bip6_broadcast(npdu: &[u8]) -> Vec<u8> {
    let message = Bvll6::OriginalBroadcastNpdu {
        source: Bip6Vmac::new([0, 1, 2]),
        npdu,
    };
    let mut out = BytesMut::new();
    encode_bvll6(&mut out, &message).unwrap();
    out.to_vec()
}

/// A BVLC-SC Encapsulated-NPDU to `destination`.
pub(crate) fn sc_npdu(npdu: &[u8], destination: Option<Vmac>) -> Vec<u8> {
    let message = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x1234,
        originating_vmac: Some([2, 0, 0, 0, 0, 1]),
        destination_vmac: destination,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::copy_from_slice(npdu),
    };
    let mut out = BytesMut::new();
    encode_sc_message(&mut out, &message);
    out.to_vec()
}

/// An MS/TP frame, preamble to CRC.
pub(crate) fn mstp(frame_type: FrameType, destination: u8, source: u8, data: &[u8]) -> Vec<u8> {
    let mut wire = vec![0; 16 + data.len() * 2];
    let length = Frame::new(frame_type, destination, source, data)
        .encode_into(&mut wire)
        .unwrap();
    wire.truncate(length);
    wire
}
