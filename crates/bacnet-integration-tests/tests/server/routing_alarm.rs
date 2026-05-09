use super::*;

// ---------------------------------------------------------------------------
// IAm routing tests (Clause 16.10)
// ---------------------------------------------------------------------------

/// When a WhoIs arrives from a remote network (with SNET/SADR in the NPDU),
/// the server should route the IAm response back to the source rather than
/// just broadcasting locally. The response NPDU should contain DNET/DADR
/// matching the original SNET/SADR, and be sent as unicast to the router MAC.
#[tokio::test]
async fn iam_routed_back_to_remote_whois_requester() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, UnconfirmedRequest};
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
    use bacnet_transport::port::TransportPort;
    use bacnet_types::enums::{NetworkPriority, UnconfirmedServiceChoice};
    use tokio::time::Duration;

    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    // Create a raw transport to act as a "router" forwarding a remote WhoIs.
    let mut raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_rx = raw_transport.start().await.unwrap();
    let _raw_mac = raw_transport.local_mac().to_vec();

    // Build a WhoIs APDU (no range limits -> all devices)
    let who_is_apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    });
    let mut apdu_buf = bytes::BytesMut::new();
    encode_apdu(&mut apdu_buf, &who_is_apdu).expect("valid APDU encoding");

    // Wrap in an NPDU with source routing: SNET=100, SADR=[10,20,30]
    // This simulates a WhoIs that was forwarded by a router from network 100.
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: Some(NpduAddress {
            network: 100,
            mac_address: bacnet_types::MacAddr::from_slice(&[10, 20, 30]),
        }),
        hop_count: 255,
        payload: Bytes::from(apdu_buf.to_vec()),
        ..Npdu::default()
    };

    let mut npdu_buf = bytes::BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    // Send via raw transport unicast to the server
    raw_transport
        .send_unicast(&npdu_buf, &server_mac)
        .await
        .unwrap();

    // Receive the response NPDU from the server.
    // The server should send the IAm as unicast back to our raw transport
    // (since we are the "router" MAC), with DNET/DADR in the NPDU header.
    let received = tokio::time::timeout(Duration::from_secs(3), raw_rx.recv())
        .await
        .expect("Timed out waiting for routed IAm response")
        .expect("Raw transport channel closed");

    // Decode the NPDU to verify routing info
    let response_npdu = decode_npdu(received.npdu.clone()).unwrap();

    // The response should have a destination (DNET/DADR) pointing to the
    // original source: network 100, mac [10, 20, 30].
    let dest = response_npdu
        .destination
        .as_ref()
        .expect("IAm response NPDU should have destination (DNET/DADR) for routed reply");
    assert_eq!(
        dest.network, 100,
        "DNET should match the original source network"
    );
    assert_eq!(
        dest.mac_address,
        MacAddr::from_slice(&[10, 20, 30]),
        "DADR should match the original source MAC"
    );

    // Verify the APDU is an IAm
    let decoded_apdu = apdu::decode_apdu(response_npdu.payload.clone()).unwrap();
    match decoded_apdu {
        Apdu::UnconfirmedRequest(req) => {
            assert_eq!(req.service_choice, UnconfirmedServiceChoice::I_AM);
        }
        other => panic!("Expected UnconfirmedRequest (IAm), got {:?}", other),
    }

    raw_transport.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// When a WhoIs arrives from the local network (no SNET/SADR in the NPDU),
/// the server should broadcast the IAm response locally (no DNET/DADR).
///
/// Note: On localhost, the broadcast may or may not reach our test transport
/// depending on OS behavior. If it does arrive, we verify it has no routing
/// info. This is a best-effort test (same caveat as `who_is_through_server`).
#[tokio::test]
async fn iam_broadcast_for_local_whois() {
    use bacnet_encoding::apdu::{self, encode_apdu, Apdu, UnconfirmedRequest};
    use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
    use bacnet_transport::port::TransportPort;
    use bacnet_types::enums::{NetworkPriority, UnconfirmedServiceChoice};
    use tokio::time::Duration;

    let mut server = make_server().await;
    let server_mac = server.local_mac().to_vec();

    // Create a raw transport
    let mut raw_transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let mut raw_rx = raw_transport.start().await.unwrap();

    // Build a WhoIs APDU with no source routing (local WhoIs)
    let who_is_apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: Bytes::new(),
    });
    let mut apdu_buf = bytes::BytesMut::new();
    encode_apdu(&mut apdu_buf, &who_is_apdu).expect("valid APDU encoding");

    // Wrap in an NPDU without source routing
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: NetworkPriority::NORMAL,
        destination: None,
        source: None,
        payload: Bytes::from(apdu_buf.to_vec()),
        ..Npdu::default()
    };

    let mut npdu_buf = bytes::BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();

    // Send to the server
    raw_transport
        .send_unicast(&npdu_buf, &server_mac)
        .await
        .unwrap();

    // Try to receive the broadcast IAm response.
    // On localhost, broadcast may not reach our transport — this is best-effort.
    match tokio::time::timeout(Duration::from_millis(500), raw_rx.recv()).await {
        Ok(Some(received)) => {
            // Decode the NPDU
            let response_npdu = decode_npdu(received.npdu.clone()).unwrap();

            // For a local broadcast response, there should be NO destination routing
            // (no DNET/DADR — it's a simple local broadcast).
            assert!(
                response_npdu.destination.is_none(),
                "Local IAm broadcast should not have DNET/DADR"
            );

            // Verify the APDU is an IAm
            let decoded_apdu = apdu::decode_apdu(response_npdu.payload.clone()).unwrap();
            match decoded_apdu {
                Apdu::UnconfirmedRequest(req) => {
                    assert_eq!(req.service_choice, UnconfirmedServiceChoice::I_AM);
                }
                other => panic!("Expected UnconfirmedRequest (IAm), got {:?}", other),
            }
        }
        _ => {
            // Broadcast did not reach us on localhost — acceptable.
        }
    }

    raw_transport.stop().await.unwrap();
    server.stop().await.unwrap();
}

// ---------------------------------------------------------------------------
// AcknowledgeAlarm integration test
// ---------------------------------------------------------------------------

/// Send an AcknowledgeAlarm confirmed request to the server and verify
/// we receive a SimpleACK back (service choice 0 = ACKNOWLEDGE_ALARM).
#[tokio::test]
async fn acknowledge_alarm_returns_simple_ack() {
    use bacnet_services::alarm_event::AcknowledgeAlarmRequest;
    use bacnet_types::enums::ConfirmedServiceChoice;
    use bacnet_types::primitives::BACnetTimeStamp;

    let mut server = make_server().await;
    let mut client = make_client().await;
    let server_mac = server.local_mac().to_vec();

    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();

    let request = AcknowledgeAlarmRequest {
        acknowledging_process_identifier: 1,
        event_object_identifier: ai_oid,
        event_state_acknowledged: 3,
        timestamp: BACnetTimeStamp::SequenceNumber(42),
        acknowledgment_source: "operator".into(),
        time_of_acknowledgment: BACnetTimeStamp::SequenceNumber(0),
    };
    let mut buf = bytes::BytesMut::new();
    request.encode(&mut buf).unwrap();

    // Send as a confirmed request -- should get back a SimpleACK (empty payload)
    let result = client
        .confirmed_request(&server_mac, ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, &buf)
        .await;

    // confirmed_request returns the service-ack payload; for SimpleACK that's empty
    let ack_data = result.unwrap();
    assert!(
        ack_data.is_empty(),
        "AcknowledgeAlarm should return SimpleACK (empty payload)"
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
