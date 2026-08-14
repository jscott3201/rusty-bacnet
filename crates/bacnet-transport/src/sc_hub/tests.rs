use super::*;
use crate::sc_frame::{
    decode_sc_bvlc_result, ScBvlcResult, ScOption, BACNET_SC_HUB_SUBPROTOCOL, BROADCAST_VMAC,
};

#[test]
fn bvlc_result_nak_uses_standard_error_values() {
    let nak = build_bvlc_result_nak(
        0x1234,
        ScFunction::ConnectRequest,
        ErrorClass::COMMUNICATION,
        ErrorCode::NODE_DUPLICATE_VMAC,
    );

    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectRequest,
            error_header_marker: 0,
            error_class: 7,
            error_code: 151,
            error_details: String::new(),
        }
    );
}

#[test]
fn connect_request_rejects_hub_vmac_as_duplicate() {
    assert_eq!(
        connect_request_vmac_disposition([0x10; 6], [0x10; 6]),
        ConnectRequestVmacDisposition::Nak(
            ErrorClass::COMMUNICATION,
            ErrorCode::NODE_DUPLICATE_VMAC
        )
    );
    assert_eq!(
        connect_request_vmac_disposition([0x00; 6], [0x10; 6]),
        ConnectRequestVmacDisposition::CloseReserved
    );
    assert_eq!(
        connect_request_vmac_disposition([0x01; 6], [0x10; 6]),
        ConnectRequestVmacDisposition::Accept
    );
}

#[test]
fn hub_registration_accepts_new_vmac_and_uuid() {
    assert_eq!(
        hub_client_registration_decision([0x02; 6], [0x22; 16], [([0x01; 6], [0x11; 16])], 2),
        HubClientRegistrationDecision::Accept
    );
}

#[test]
fn hub_registration_replaces_known_device_uuid() {
    assert_eq!(
        hub_client_registration_decision([0x02; 6], [0x11; 16], [([0x01; 6], [0x11; 16])], 1),
        HubClientRegistrationDecision::Replace {
            old_vmac: [0x01; 6]
        }
    );
}

#[test]
fn hub_registration_replaces_known_device_uuid_with_same_vmac() {
    assert_eq!(
        hub_client_registration_decision([0x01; 6], [0x11; 16], [([0x01; 6], [0x11; 16])], 1),
        HubClientRegistrationDecision::Replace {
            old_vmac: [0x01; 6]
        }
    );
}

#[test]
fn hub_registration_rejects_duplicate_vmac_for_different_uuid() {
    assert_eq!(
        hub_client_registration_decision([0x01; 6], [0x22; 16], [([0x01; 6], [0x11; 16])], 2),
        HubClientRegistrationDecision::NakDuplicateVmac
    );
}

#[test]
fn hub_registration_rejects_new_connection_at_capacity() {
    assert_eq!(
        hub_client_registration_decision([0x03; 6], [0x33; 16], [([0x01; 6], [0x11; 16])], 1),
        HubClientRegistrationDecision::NakMaxClients
    );
}

#[test]
fn relay_limit_decision_accepts_within_target_limits() {
    assert_eq!(
        relay_limit_decision(20, 40, 20, 40),
        RelayLimitDecision::Send
    );
}

#[test]
fn relay_limit_decision_rejects_oversized_npdu_first() {
    assert_eq!(
        relay_limit_decision(21, 41, 20, 40),
        RelayLimitDecision::DropMaxNpdu
    );
}

#[test]
fn relay_limit_decision_rejects_oversized_encoded_bvlc() {
    assert_eq!(
        relay_limit_decision(20, 41, 20, 40),
        RelayLimitDecision::DropMaxBvlc
    );
}

#[test]
fn hub_relay_target_requires_destination_and_no_originating_vmac() {
    let mut msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: Some([0x01; 6]),
        destination_vmac: Some([0x02; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20]),
    };

    assert_eq!(
        hub_relay_target(&msg),
        Err(HubRelayReject::OriginatingVmacPresent)
    );

    msg.originating_vmac = None;
    msg.destination_vmac = None;
    assert_eq!(
        hub_relay_target(&msg),
        Err(HubRelayReject::MissingDestinationVmac)
    );
}

#[test]
fn hub_relay_target_classifies_unicast_and_broadcast_destinations() {
    let mut msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: Some([0x02; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20]),
    };

    assert_eq!(
        hub_relay_target(&msg).unwrap(),
        HubRelayTarget::Unicast([0x02; 6])
    );

    msg.destination_vmac = Some(BROADCAST_VMAC);
    assert_eq!(hub_relay_target(&msg).unwrap(), HubRelayTarget::Broadcast);
}

#[test]
fn hub_relay_unicast_adds_originating_vmac_and_strips_destination_vmac() {
    let inbound = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x1234,
        originating_vmac: None,
        destination_vmac: Some([0x02; 6]),
        dest_options: vec![ScOption {
            option_type: 2,
            must_understand: false,
            data: vec![0xAA, 0xBB],
        }],
        data_options: vec![ScOption {
            option_type: 3,
            must_understand: true,
            data: Vec::new(),
        }],
        payload: Bytes::from_static(&[0x01, 0x20, 0xFF]),
    };

    let relay = build_hub_relay_message(&inbound, [0x01; 6], HubRelayTarget::Unicast([0x02; 6]));

    assert_eq!(relay.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(relay.message_id, 0x1234);
    assert_eq!(relay.originating_vmac, Some([0x01; 6]));
    assert_eq!(relay.destination_vmac, None);
    assert_eq!(relay.dest_options, inbound.dest_options);
    assert_eq!(relay.data_options, inbound.data_options);
    assert_eq!(relay.payload, inbound.payload);

    let mut encoded = BytesMut::new();
    encode_sc_message(&mut encoded, &relay);
    let decoded = decode_sc_message(&encoded).unwrap();
    assert_eq!(decoded.originating_vmac, Some([0x01; 6]));
    assert_eq!(decoded.destination_vmac, None);
    assert_eq!(decoded.dest_options.len(), 1);
    assert_eq!(decoded.data_options.len(), 1);
}

#[test]
fn hub_relay_wire_preserves_empty_header_data_marker() {
    let inbound = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x1244,
        originating_vmac: None,
        destination_vmac: Some([0x02; 6]),
        dest_options: vec![ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        }],
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x20, 0x30]),
    };
    let mut wire = BytesMut::new();
    encode_sc_message(&mut wire, &inbound);
    assert_eq!(wire[10], 0x42);
    wire[10] = 0x62;
    let mut wire = wire.to_vec();
    wire.splice(11..11, [0, 0]);
    let decoded = decode_sc_message(&wire).unwrap();

    let relay = encode_hub_relay_frame(
        &wire,
        &decoded,
        [0x01; 6],
        HubRelayTarget::Unicast([0x02; 6]),
    )
    .unwrap();
    assert_eq!(relay[10], 0x62);
    assert_eq!(
        decode_sc_message(&relay).unwrap().originating_vmac,
        Some([0x01; 6])
    );
}

#[test]
fn hub_relay_preserves_large_minimum_size_option_chains() {
    let inbound = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2345,
        originating_vmac: None,
        destination_vmac: Some([0x02; 6]),
        dest_options: minimum_size_options(31),
        data_options: minimum_size_options(31),
        payload: Bytes::from_static(&[0x01, 0x20, 0x99]),
    };

    let relay = build_hub_relay_message(&inbound, [0x01; 6], HubRelayTarget::Unicast([0x02; 6]));
    assert_eq!(relay.originating_vmac, Some([0x01; 6]));
    assert_eq!(relay.destination_vmac, None);
    assert_eq!(relay.dest_options, inbound.dest_options);
    assert_eq!(relay.data_options, inbound.data_options);

    let mut encoded = BytesMut::new();
    encode_sc_message(&mut encoded, &relay);
    assert!(encoded.len() <= 1476);

    let decoded = decode_sc_message(&encoded).unwrap();
    assert_eq!(decoded.originating_vmac, Some([0x01; 6]));
    assert_eq!(decoded.destination_vmac, None);
    assert_eq!(decoded.dest_options, inbound.dest_options);
    assert_eq!(decoded.data_options, inbound.data_options);
    assert_eq!(decoded.payload, inbound.payload);

    let broadcast = build_hub_relay_message(&inbound, [0x01; 6], HubRelayTarget::Broadcast);
    assert_eq!(broadcast.destination_vmac, Some(BROADCAST_VMAC));
    assert_eq!(broadcast.dest_options, inbound.dest_options);
    assert_eq!(broadcast.data_options, inbound.data_options);
}

#[test]
fn hub_relay_broadcast_adds_originating_vmac_and_preserves_broadcast_destination() {
    let inbound = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x5678,
        originating_vmac: None,
        destination_vmac: Some(BROADCAST_VMAC),
        dest_options: Vec::new(),
        data_options: vec![ScOption {
            option_type: 1,
            must_understand: false,
            data: vec![0x01],
        }],
        payload: Bytes::from_static(&[0x01, 0x04, 0x05]),
    };

    let relay = build_hub_relay_message(&inbound, [0x09; 6], HubRelayTarget::Broadcast);

    assert_eq!(relay.originating_vmac, Some([0x09; 6]));
    assert_eq!(relay.destination_vmac, Some(BROADCAST_VMAC));
    assert_eq!(relay.data_options, inbound.data_options);
    assert_eq!(relay.payload, inbound.payload);
}

#[test]
fn hub_relay_recipients_selects_only_matching_unicast_destination() {
    let connected = [[0x01; 6], [0x02; 6], [0x03; 6]];

    assert_eq!(
        hub_relay_recipient_vmacs(HubRelayTarget::Unicast([0x02; 6]), [0x01; 6], connected,),
        vec![[0x02; 6]]
    );
    assert!(
        hub_relay_recipient_vmacs(HubRelayTarget::Unicast([0x04; 6]), [0x01; 6], connected,)
            .is_empty()
    );
}

#[test]
fn hub_relay_recipients_broadcasts_to_all_except_origin() {
    let connected = [[0x01; 6], [0x02; 6], [0x03; 6]];

    let mut recipients = hub_relay_recipient_vmacs(HubRelayTarget::Broadcast, [0x01; 6], connected);
    recipients.sort();

    assert_eq!(recipients, vec![[0x02; 6], [0x03; 6]]);
}

#[test]
fn known_unhandled_function_is_not_classified_as_unknown() {
    let nak = build_bvlc_result_nak(
        0x1234,
        ScFunction::ConnectAccept,
        ErrorClass::COMMUNICATION,
        unexpected_bvlc_function_error_code(ScFunction::ConnectAccept),
    );

    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::ConnectAccept,
            error_header_marker: 0,
            error_class: 7,
            error_code: 150,
            error_details: String::new(),
        }
    );
    assert_eq!(
        unexpected_bvlc_function_error_code(ScFunction::Unknown(0x42)),
        ErrorCode::BVLC_FUNCTION_UNKNOWN
    );
}

#[test]
fn direct_connection_function_naks_as_unexpected_data() {
    let nak = build_bvlc_result_nak(
        0x2233,
        ScFunction::AddressResolution,
        ErrorClass::COMMUNICATION,
        unexpected_bvlc_function_error_code(ScFunction::AddressResolution),
    );

    assert_eq!(
        decode_sc_bvlc_result(&nak).unwrap(),
        ScBvlcResult::Nak {
            result_for: ScFunction::AddressResolution,
            error_header_marker: 0,
            error_class: 7,
            error_code: 150,
            error_details: String::new(),
        }
    );
}

#[test]
fn websocket_subprotocol_offer_accepts_hub_protocol_in_list() {
    let request = tokio_tungstenite::tungstenite::handshake::server::Request::builder()
        .header(
            "Sec-WebSocket-Protocol",
            format!("chat, {BACNET_SC_HUB_SUBPROTOCOL}, other"),
        )
        .body(())
        .unwrap();

    assert!(offers_websocket_subprotocol(
        &request,
        BACNET_SC_HUB_SUBPROTOCOL
    ));
}

#[test]
fn websocket_subprotocol_offer_rejects_direct_connection_protocol() {
    let request = tokio_tungstenite::tungstenite::handshake::server::Request::builder()
        .header("Sec-WebSocket-Protocol", "dc.bsc.bacnet.org")
        .body(())
        .unwrap();

    assert!(!offers_websocket_subprotocol(
        &request,
        BACNET_SC_HUB_SUBPROTOCOL
    ));
}

#[test]
fn websocket_subprotocol_offer_rejects_missing_hub_protocol() {
    let request = tokio_tungstenite::tungstenite::handshake::server::Request::builder()
        .header("Sec-WebSocket-Protocol", "dc.bsc.bacnet.org")
        .body(())
        .unwrap();

    assert!(!offers_websocket_subprotocol(
        &request,
        BACNET_SC_HUB_SUBPROTOCOL
    ));

    let request_without_header =
        tokio_tungstenite::tungstenite::handshake::server::Request::builder()
            .body(())
            .unwrap();
    assert!(!offers_websocket_subprotocol(
        &request_without_header,
        BACNET_SC_HUB_SUBPROTOCOL
    ));
}

#[test]
fn websocket_subprotocol_error_response_is_bad_request() {
    let response = websocket_subprotocol_error_response();

    assert_eq!(
        response.status(),
        tokio_tungstenite::tungstenite::http::StatusCode::BAD_REQUEST
    );
    assert!(response
        .body()
        .as_ref()
        .unwrap()
        .contains(BACNET_SC_HUB_SUBPROTOCOL));
}

fn minimum_size_options(count: usize) -> Vec<ScOption> {
    (0..count)
        .map(|i| ScOption {
            option_type: (i % 31 + 1) as u8,
            must_understand: i % 2 == 0,
            data: Vec::new(),
        })
        .collect()
}
