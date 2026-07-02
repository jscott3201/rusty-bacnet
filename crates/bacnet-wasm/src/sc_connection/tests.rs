use super::*;

#[test]
fn connect_handshake() {
    let vmac = [1, 2, 3, 4, 5, 6];
    let mut conn = ScConnection::new(vmac);
    conn.max_bvlc_length = 1200;
    conn.max_apdu_length = 900;
    assert_eq!(conn.state, ScConnectionState::Disconnected);

    let req = conn.build_connect_request();
    assert_eq!(conn.state, ScConnectionState::Connecting);
    assert_eq!(req.function, ScFunction::ConnectRequest);
    // AB.2.10.1: no VMACs, 26-byte payload
    assert!(req.originating_vmac.is_none());
    assert_eq!(req.payload.len(), 26);
    assert_eq!(u16::from_be_bytes([req.payload[22], req.payload[23]]), 1200);
    assert_eq!(u16::from_be_bytes([req.payload[24], req.payload[25]]), 900);

    // Simulate ConnectAccept with 26-byte payload
    let hub_vmac = [7, 8, 9, 10, 11, 12];
    let mut accept_payload = Vec::with_capacity(26);
    accept_payload.extend_from_slice(&hub_vmac);
    accept_payload.extend_from_slice(&[0u8; 16]); // Device UUID
    accept_payload.extend_from_slice(&1100u16.to_be_bytes());
    accept_payload.extend_from_slice(&480u16.to_be_bytes());
    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(accept_payload),
    };
    assert!(conn.handle_connect_accept(&accept));
    assert_eq!(conn.state, ScConnectionState::Connected);
    assert_eq!(conn.hub_vmac, Some(hub_vmac));
    assert_eq!(conn.hub_max_bvlc_length, 1100);
    assert_eq!(conn.hub_max_apdu_length, 480);
}

#[test]
fn connect_accept_wrong_state() {
    let mut conn = ScConnection::new([1; 6]);
    // Not in Connecting state
    let msg = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: 1,
        originating_vmac: Some([2; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(vec![0; 10]),
    };
    assert!(!conn.handle_connect_accept(&msg));
}

#[test]
fn connect_accept_rejects_wrong_message_id() {
    let mut conn = ScConnection::new([1; 6]);
    let req = conn.build_connect_request();
    let hub_vmac = [2; 6];
    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0u8; 16]);
    payload.extend_from_slice(&1476u16.to_be_bytes());
    payload.extend_from_slice(&1476u16.to_be_bytes());

    let wrong_accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id.wrapping_add(1),
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload.clone()),
    };
    assert!(!conn.handle_connect_accept(&wrong_accept));
    assert_eq!(conn.state, ScConnectionState::Connecting);
    assert_eq!(conn.hub_vmac, None);

    let right_accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    assert!(conn.handle_connect_accept(&right_accept));
    assert_eq!(conn.state, ScConnectionState::Connected);
    assert_eq!(conn.hub_vmac, Some(hub_vmac));
}

#[test]
fn connect_accept_rejects_short_payload() {
    let mut conn = ScConnection::new([1; 6]);
    let req = conn.build_connect_request();
    let short_accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[2; 6]),
    };

    assert!(!conn.handle_connect_accept(&short_accept));
    assert_eq!(conn.state, ScConnectionState::Connecting);
    assert_eq!(conn.hub_vmac, None);
}

#[test]
fn disconnect_request_and_ack() {
    let vmac = [1; 6];
    let hub_vmac = [2; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;
    conn.hub_vmac = Some(hub_vmac);

    let req = conn.build_disconnect_request().unwrap();
    assert_eq!(conn.state, ScConnectionState::Disconnecting);
    assert_eq!(req.function, ScFunction::DisconnectRequest);
    // AB.2.12.1: no VMACs
    assert!(req.originating_vmac.is_none());
    assert!(req.destination_vmac.is_none());

    // Receive DisconnectAck
    let ack = ScMessage {
        function: ScFunction::DisconnectAck,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    conn.handle_received(&ack);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn disconnect_without_hub_vmac() {
    let mut conn = ScConnection::new([1; 6]);
    assert!(conn.build_disconnect_request().is_err());
}

#[test]
fn encapsulated_npdu_round_trip() {
    let vmac = [1; 6];
    let hub_vmac = [2; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;
    conn.hub_vmac = Some(hub_vmac);

    let npdu = vec![0x01, 0x04, 0x00];
    let msg = conn.build_encapsulated_npdu([3; 6], &npdu);
    assert_eq!(msg.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(msg.destination_vmac, Some([3; 6]));
    assert_eq!(msg.payload.as_ref(), &npdu[..]);
}

#[test]
fn handle_encapsulated_npdu_hub_unicast() {
    let vmac = [1; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 42,
        originating_vmac: Some([2; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x04]),
    };
    let result = conn.handle_received(&msg);
    assert!(result.is_some());
    let received = result.unwrap();
    assert_eq!(received.npdu.as_ref(), &[0x01, 0x04]);
    assert_eq!(received.source_vmac, [2; 6]);
    assert!(received.data_attributes.is_empty());
}

#[test]
fn handle_encapsulated_npdu_rejects_oversized_local_npdu() {
    let vmac = [1; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;
    conn.max_apdu_length = 1;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 42,
        originating_vmac: Some([2; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x04]),
    };
    assert!(conn.handle_received(&msg).is_none());
}

#[test]
fn handle_encapsulated_npdu_drops_non_broadcast_destination_from_hub() {
    let vmac = [1; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 42,
        originating_vmac: Some([2; 6]),
        destination_vmac: Some(vmac),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01]),
    };
    assert!(conn.handle_received(&msg).is_none());
}

#[test]
fn handle_encapsulated_npdu_broadcast() {
    let vmac = [1; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 42,
        originating_vmac: Some([2; 6]),
        destination_vmac: Some([0xFF; 6]), // broadcast
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01]),
    };
    assert!(conn.handle_received(&msg).is_some());
}

#[test]
fn handle_disconnect_request_generates_ack() {
    let vmac = [1; 6];
    let mut conn = ScConnection::new(vmac);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::DisconnectRequest,
        message_id: 99,
        originating_vmac: Some([2; 6]),
        destination_vmac: Some(vmac),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    conn.handle_received(&msg);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    let ack = conn.disconnect_ack_to_send.take().unwrap();
    assert_eq!(ack.function, ScFunction::DisconnectAck);
    assert_eq!(ack.message_id, 99);
    // AB.2.13.1: no VMACs on DisconnectAck
    assert!(ack.originating_vmac.is_none());
    assert!(ack.destination_vmac.is_none());
}

#[test]
fn handle_bvlc_result_ack_keeps_connected() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x0C, 0x00]),
    };
    conn.handle_received(&msg);
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn handle_bvlc_result_nak_disconnects() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01]),
    };
    conn.handle_received(&msg);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn handle_connect_result_duplicate_vmac_installs_replacement() {
    let mut conn = ScConnection::new([1; 6]);
    let req = conn.build_connect_request();
    let replacement = [0x12, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x07, 0x00, 0x97]),
    };
    let result = decode_sc_bvlc_result(&msg).unwrap();

    assert!(conn
        .handle_connect_result(req.message_id, &result, Some(replacement))
        .unwrap());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.local_vmac, replacement);

    let retry = conn.build_connect_request();
    assert_eq!(&retry.payload[0..6], replacement.as_slice());
}

#[test]
fn handle_connect_result_duplicate_vmac_wrong_message_id_does_not_replace_vmac() {
    let original = [0x22, 1, 2, 3, 4, 5];
    let replacement = [0x12, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    let mut conn = ScConnection::new(original);
    let req = conn.build_connect_request();
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: req.message_id.wrapping_add(1),
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x07, 0x00, 0x97]),
    };
    let result = decode_sc_bvlc_result(&msg).unwrap();

    assert!(!conn
        .handle_connect_result(msg.message_id, &result, Some(replacement))
        .unwrap());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.local_vmac, original);

    let retry = conn.build_connect_request();
    assert_eq!(&retry.payload[0..6], original.as_slice());
}

#[test]
fn handle_connect_result_generic_nak_does_not_replace_vmac() {
    let original = [0x22, 1, 2, 3, 4, 5];
    let mut conn = ScConnection::new(original);
    let req = conn.build_connect_request();
    let replacement = [0x12, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x06, 0x01, 0x00, 0x00, 0x07, 0x00, 0x96]),
    };
    let result = decode_sc_bvlc_result(&msg).unwrap();

    assert!(!conn
        .handle_connect_result(req.message_id, &result, Some(replacement))
        .unwrap());
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.local_vmac, original);
}

#[test]
fn new_with_device_uuid_sends_supplied_uuid() {
    let uuid = [0xAB; 16];
    let mut conn = ScConnection::new_with_device_uuid([1; 6], uuid);
    let req = conn.build_connect_request();

    assert_eq!(&req.payload[6..22], uuid.as_slice());
}

#[test]
fn handle_malformed_bvlc_result_disconnects() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;

    let msg = ScMessage {
        function: ScFunction::Result,
        message_id: 1,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    conn.handle_received(&msg);
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn heartbeat() {
    let vmac = [1; 6];
    let hub_vmac = [2; 6];
    let mut conn = ScConnection::new(vmac);
    conn.hub_vmac = Some(hub_vmac);

    let hb = conn.build_heartbeat();
    assert_eq!(hb.function, ScFunction::HeartbeatRequest);
    // AB.2.14.1: no VMACs on HeartbeatRequest
    assert!(hb.originating_vmac.is_none());
    assert!(hb.destination_vmac.is_none());
    assert!(hb.payload.is_empty());
}

#[test]
fn heartbeat_ack() {
    let conn = ScConnection::new([1; 6]);
    let ack = conn.build_heartbeat_ack(42);
    assert_eq!(ack.function, ScFunction::HeartbeatAck);
    assert_eq!(ack.message_id, 42);
    // AB.2.15.1: no VMACs on HeartbeatAck
    assert!(ack.originating_vmac.is_none());
    assert!(ack.destination_vmac.is_none());
    assert!(ack.data_options.is_empty());
    assert!(ack.payload.is_empty());
}

#[test]
fn heartbeat_scheduler_sends_after_idle_interval() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(1_000);

    assert_eq!(
        conn.next_heartbeat_action(
            30_999,
            DEFAULT_HEARTBEAT_INTERVAL_MS,
            DEFAULT_HEARTBEAT_TIMEOUT_MS
        ),
        ScHeartbeatAction::None
    );

    let action = conn.next_heartbeat_action(
        31_000,
        DEFAULT_HEARTBEAT_INTERVAL_MS,
        DEFAULT_HEARTBEAT_TIMEOUT_MS,
    );
    let ScHeartbeatAction::Send(heartbeat) = action else {
        panic!("expected heartbeat request after idle interval");
    };
    assert_eq!(heartbeat.function, ScFunction::HeartbeatRequest);
    assert!(heartbeat.originating_vmac.is_none());
    assert!(heartbeat.destination_vmac.is_none());
    assert!(heartbeat.payload.is_empty());
    assert_eq!(
        conn.pending_heartbeat_message_id,
        Some(heartbeat.message_id)
    );

    assert_eq!(
        conn.next_heartbeat_action(
            61_000,
            DEFAULT_HEARTBEAT_INTERVAL_MS,
            DEFAULT_HEARTBEAT_TIMEOUT_MS
        ),
        ScHeartbeatAction::None
    );
}

#[test]
fn heartbeat_ack_clears_only_matching_outstanding_request() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(0);

    let ScHeartbeatAction::Send(heartbeat) = conn.next_heartbeat_action(30_000, 30_000, 60_000)
    else {
        panic!("expected heartbeat request");
    };

    let wrong_ack = ScMessage {
        function: ScFunction::HeartbeatAck,
        message_id: heartbeat.message_id.wrapping_add(1),
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    assert!(!conn.handle_heartbeat_ack(&wrong_ack, 31_000));
    assert_eq!(
        conn.pending_heartbeat_message_id,
        Some(heartbeat.message_id)
    );

    let ack = ScMessage {
        function: ScFunction::HeartbeatAck,
        message_id: heartbeat.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    assert!(conn.handle_heartbeat_ack(&ack, 31_000));
    assert_eq!(conn.pending_heartbeat_message_id, None);

    assert_eq!(
        conn.next_heartbeat_action(60_999, 30_000, 60_000),
        ScHeartbeatAction::None
    );
    assert!(matches!(
        conn.next_heartbeat_action(61_000, 30_000, 60_000),
        ScHeartbeatAction::Send(_)
    ));
}

#[test]
fn heartbeat_ack_rejects_vmac_fields_until_timeout() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(0);

    let ScHeartbeatAction::Send(heartbeat) = conn.next_heartbeat_action(100, 100, 300) else {
        panic!("expected heartbeat request");
    };

    let bad_ack = ScMessage {
        function: ScFunction::HeartbeatAck,
        message_id: heartbeat.message_id,
        originating_vmac: Some([2; 6]),
        destination_vmac: Some([1; 6]),
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    assert!(!conn.handle_heartbeat_ack(&bad_ack, 150));
    assert_eq!(
        conn.pending_heartbeat_message_id,
        Some(heartbeat.message_id)
    );

    assert_eq!(
        conn.next_heartbeat_action(300, 100, 300),
        ScHeartbeatAction::None
    );
    assert_eq!(
        conn.next_heartbeat_action(301, 100, 300),
        ScHeartbeatAction::Disconnect
    );
    assert_eq!(conn.state, ScConnectionState::Disconnected);
    assert_eq!(conn.pending_heartbeat_message_id, None);
}

#[test]
fn heartbeat_ack_rejects_destination_options_until_timeout() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(0);

    let ScHeartbeatAction::Send(heartbeat) = conn.next_heartbeat_action(100, 100, 300) else {
        panic!("expected heartbeat request");
    };

    let bad_ack = ScMessage {
        function: ScFunction::HeartbeatAck,
        message_id: heartbeat.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: vec![crate::sc_frame::ScOption {
            option_type: 2,
            must_understand: true,
            data: Vec::new(),
        }],
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    assert!(!conn.handle_heartbeat_ack(&bad_ack, 150));
    assert_eq!(
        conn.pending_heartbeat_message_id,
        Some(heartbeat.message_id)
    );

    assert_eq!(
        conn.next_heartbeat_action(301, 100, 300),
        ScHeartbeatAction::Disconnect
    );
}

#[test]
fn heartbeat_scheduler_fails_closed_on_non_monotonic_time() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(1_000);

    assert_eq!(
        conn.next_heartbeat_action(999, 100, 300),
        ScHeartbeatAction::Disconnect
    );
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn heartbeat_scheduler_times_out_on_large_forward_elapsed_time() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(1_000);

    assert_eq!(
        conn.next_heartbeat_action(1_301, 100, 300),
        ScHeartbeatAction::Disconnect
    );
    assert_eq!(conn.state, ScConnectionState::Disconnected);
}

#[test]
fn inbound_bvlc_activity_defers_browser_heartbeat_and_timeout() {
    let mut conn = ScConnection::new([1; 6]);
    conn.state = ScConnectionState::Connected;
    conn.start_heartbeat_tracking(0);

    conn.record_heartbeat_activity(250);
    assert_eq!(
        conn.next_heartbeat_action(299, 100, 300),
        ScHeartbeatAction::None
    );
    assert_eq!(
        conn.next_heartbeat_action(349, 100, 300),
        ScHeartbeatAction::None
    );
    assert!(matches!(
        conn.next_heartbeat_action(350, 100, 300),
        ScHeartbeatAction::Send(_)
    ));
    assert_eq!(conn.state, ScConnectionState::Connected);
}

#[test]
fn message_id_wraps() {
    let mut conn = ScConnection::new([1; 6]);
    conn.next_message_id = u16::MAX;
    assert_eq!(conn.next_id(), u16::MAX);
    assert_eq!(conn.next_id(), 0);
    assert_eq!(conn.next_id(), 1);
}
