//! Independent raw-wire node admission vectors for Address-Resolution
//! (0x02) and Address-Resolution-ACK (0x03); no hub fallback involved.
//!
//! Well-formed requests earn an ACK (empty here: the fixture transport is
//! unconfigured); well-formed responses stay silently consumed per the
//! response rule. Malformed requests NAK locally while malformed responses
//! stay silent. Hub opaque transit for valid bodies is preserved by
//! existing hub tests; these vectors prove the node gate only.
use super::*;
use tokio::time::timeout;

fn wire(
    raw: u8,
    id: u16,
    source: Option<Vmac>,
    dest: Option<Vmac>,
    flags: u8,
    body: &[u8],
) -> Vec<u8> {
    let mut bytes = vec![
        raw,
        flags | (u8::from(source.is_some()) * 8) | (u8::from(dest.is_some()) * 4),
    ];
    bytes.extend_from_slice(&id.to_be_bytes());
    if let Some(source) = source {
        bytes.extend_from_slice(&source);
    }
    if let Some(dest) = dest {
        bytes.extend_from_slice(&dest);
    }
    bytes.extend_from_slice(body);
    bytes
}

fn nak(raw: u8, id: u16, dest: Option<Vmac>, code: u16, marker: u8) -> Vec<u8> {
    let [hi, lo] = code.to_be_bytes();
    wire(0, id, None, dest, 0, &[raw, 1, marker, 0, 7, hi, lo])
}

fn valid_acks() -> Vec<Vec<u8>> {
    let mut out = vec![
        vec![],
        b"wss://one.example/sc".to_vec(),
        b"wss://one.example/sc wss://two.example:8443/sc".to_vec(),
        b"wss://hub.example.com:47808/.bacnet/sc?profile=primary".to_vec(),
        b"wss://[::1]:47808/sc".to_vec(),
        b"WSS://UPPER.example/SC".to_vec(),
    ];
    let mut long = b"wss://peer.example/".to_vec();
    long.resize(1400, b'x');
    out.push(long);
    out
}

async fn recv(hub: &LoopbackWebSocket) -> Vec<u8> {
    timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap()
}

async fn recv_ack(hub: &LoopbackWebSocket) -> ScMessage {
    decode_sc_message(&recv(hub).await).unwrap()
}

async fn start_with_uris(
    uris: &[&str],
) -> (
    ScTransport<LoopbackWebSocket>,
    mpsc::Receiver<ReceivedNpdu>,
    LoopbackWebSocket,
) {
    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(uris.to_vec());
    let hub_task = tokio::spawn(async move {
        data_attribute_tests::hub_accept(&ws_hub, [0x10; 6]).await;
        ws_hub
    });
    let rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    (transport, rx, ws_hub)
}

#[test]
fn advertised_uris_accept_valid_forms() {
    let (client_a, _) = LoopbackWebSocket::pair();
    let _ = ScTransport::new(client_a, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(vec![
            "wss://one.example/sc".to_string(),
            "wss://two.example:8443/sc".to_string(),
        ]);
    let (client_b, _) = LoopbackWebSocket::pair();
    let _ = ScTransport::new(client_b, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(["wss://one.example/sc", "WSS://UPPER.example/SC"]);
    let (client_c, _) = LoopbackWebSocket::pair();
    let long = format!("wss://peer.example/{}", "x".repeat(1380));
    let _ = ScTransport::new(client_c, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris([long.as_str()]);
}

#[test]
#[should_panic(expected = "advertised URI")]
fn advertised_uris_reject_bad_scheme_at_set() {
    let (client, _) = LoopbackWebSocket::pair();
    let _ = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(["ws://one.example/sc"]);
}

#[test]
#[should_panic(expected = "advertised URI")]
fn advertised_uris_reject_missing_host_at_set() {
    let (client, _) = LoopbackWebSocket::pair();
    let _ = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(["wss://one.example/sc", "wss:///sc"]);
}

#[test]
#[should_panic(expected = "advertised URI")]
fn advertised_uris_reject_over_budget_list_at_set() {
    let (client, _) = LoopbackWebSocket::pair();
    let uris: Vec<String> = (0..8)
        .map(|i| format!("wss://peer{i}.example/{}", "x".repeat(1380)))
        .collect();
    let _ = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(uris);
}

#[test]
fn ack_builder_shape_copies_request_id_dest_mirror() {
    let conn = ScConnection::new([1; 6], [1; 16]);
    let before = conn.clone();
    let payload = b"wss://one.example/sc wss://two.example:8443/sc";
    let first = conn.build_address_resolution_ack(0x2234, None, payload);
    assert_eq!(first.function, ScFunction::AddressResolutionAck);
    assert_eq!(first.message_id, 0x2234);
    assert_eq!(first.originating_vmac, None);
    assert_eq!(first.destination_vmac, None);
    assert!(first.dest_options.is_empty());
    assert!(first.data_options.is_empty());
    assert_eq!(first.payload.as_ref(), payload);
    let second = conn.build_address_resolution_ack(0x2235, Some([0x22; 6]), &[]);
    assert_eq!(second.message_id, 0x2235);
    assert_eq!(second.destination_vmac, Some([0x22; 6]));
    assert!(second.payload.is_empty(), "unconfigured answers stay empty");
    // Builder is pure: no state-machine or counter effect.
    assert_eq!(conn.state, before.state);
    assert_eq!(conn.local_vmac, before.local_vmac);
    assert_eq!(conn.next_message_id, before.next_message_id);
    assert_eq!(conn.disconnect_ack_to_send, before.disconnect_ack_to_send);
}

#[test]
fn answer_predicate_accepts_only_answerable_requests() {
    fn request(
        function: ScFunction,
        origin: Option<Vmac>,
        dest: Option<Vmac>,
        payload: &[u8],
    ) -> ScMessage {
        ScMessage {
            function,
            message_id: 0x2233,
            originating_vmac: origin,
            destination_vmac: dest,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::copy_from_slice(payload),
        }
    }
    assert_eq!(
        address_resolution::answer_destination(&request(
            ScFunction::AddressResolution,
            None,
            None,
            &[]
        )),
        Some(None)
    );
    assert_eq!(
        address_resolution::answer_destination(&request(
            ScFunction::AddressResolution,
            Some([0x22; 6]),
            None,
            &[]
        )),
        Some(Some([0x22; 6]))
    );
    // Responses are never answered, even when well-formed.
    assert_eq!(
        address_resolution::answer_destination(&request(
            ScFunction::AddressResolutionAck,
            Some([0x22; 6]),
            None,
            b"wss://one.example/sc"
        )),
        None
    );
    // Malformed shapes keep the existing NAK path (no answer destination).
    assert_eq!(
        address_resolution::answer_destination(&request(
            ScFunction::AddressResolution,
            Some([0x22; 6]),
            None,
            &[0x00]
        )),
        None
    );
    // Explicit destinations are not for the local node.
    assert_eq!(
        address_resolution::answer_destination(&request(
            ScFunction::AddressResolution,
            Some([0x22; 6]),
            Some([0x44; 6]),
            &[]
        )),
        None
    );
    // Reserved origins stay silent.
    for origin in [Some([0; 6]), Some(BROADCAST_VMAC)] {
        assert_eq!(
            address_resolution::answer_destination(&request(
                ScFunction::AddressResolution,
                origin,
                None,
                &[]
            )),
            None
        );
    }
}

#[tokio::test]
async fn answer_configured_uris_echoed_with_copied_ids() {
    let (mut transport, mut rx, hub) =
        start_with_uris(&["wss://one.example/sc", "wss://two.example:8443/sc"]).await;
    hub.send(&wire(2, 0x2234, None, None, 0, &[]))
        .await
        .unwrap();
    let first = recv_ack(&hub).await;
    assert_eq!(first.function, ScFunction::AddressResolutionAck);
    assert_eq!(first.message_id, 0x2234);
    assert_eq!(first.originating_vmac, None);
    assert_eq!(first.destination_vmac, None);
    assert!(first.dest_options.is_empty());
    assert!(first.data_options.is_empty());
    assert_eq!(
        first.payload.as_ref(),
        b"wss://one.example/sc wss://two.example:8443/sc"
    );
    hub.send(&wire(2, 0x2235, Some([0x22; 6]), None, 0, &[]))
        .await
        .unwrap();
    let second = recv_ack(&hub).await;
    assert_eq!(second.function, ScFunction::AddressResolutionAck);
    assert_eq!(
        second.destination_vmac,
        Some([0x22; 6]),
        "answer must be routable back to the requesting node"
    );
    assert_eq!(second.message_id, 0x2235);
    assert_eq!(
        second.payload.as_ref(),
        b"wss://one.example/sc wss://two.example:8443/sc"
    );
    barrier(&hub).await;
    assert!(rx.try_recv().is_err());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn no_answer_once_disconnected() {
    let (mut transport, mut rx, hub) = start_with_uris(&["wss://one.example/sc"]).await;
    // A valid request is answered while connected.
    hub.send(&wire(2, 0x2234, None, None, 0, &[]))
        .await
        .unwrap();
    let reply = recv_ack(&hub).await;
    assert_eq!(reply.function, ScFunction::AddressResolutionAck);
    // Peer-initiated disconnect retires the connection; the transport
    // acknowledges the disconnect itself.
    hub.send(&wire(8, 0x2236, None, None, 0, &[]))
        .await
        .unwrap();
    let ack = recv_ack(&hub).await;
    assert_eq!(ack.function, ScFunction::DisconnectAck);
    assert_eq!(
        transport.connection().unwrap().lock().await.state,
        ScConnectionState::Disconnected
    );
    // The same valid request is now silent: no answers when not connected.
    hub.send(&wire(2, 0x2237, None, None, 0, &[]))
        .await
        .unwrap();
    barrier(&hub).await;
    assert!(rx.try_recv().is_err());
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn answer_keeps_heartbeat_and_npdu_interop() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_advertised_uris(["wss://one.example/sc"])
        .with_test_heartbeat_timing_ms(80, 700);
    let (rx, ()) = tokio::join!(
        transport.start(),
        super::data_attribute_tests::hub_accept(&hub, [0x10; 6])
    );
    let mut rx = rx.unwrap();
    let probe = recv(&hub).await;
    assert_eq!(probe[0], 0x0A);
    // A request earns exactly one answer; accepted traffic keeps the
    // heartbeat budget alive like NPDUs.
    hub.send(&wire(2, 0x2234, Some([0x22; 6]), None, 0, &[]))
        .await
        .unwrap();
    let reply = recv_ack(&hub).await;
    assert_eq!(reply.function, ScFunction::AddressResolutionAck);
    assert_eq!(reply.destination_vmac, Some([0x22; 6]));
    assert_eq!(reply.payload.as_ref(), b"wss://one.example/sc");
    let next = timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    assert!(rx.try_recv().is_err());
    // Positive controls only AFTER the resolution window.
    hub.send(&[0x0B, 0, next[2], next[3]]).await.unwrap();
    hub.send(&wire(1, 0, Some([0x22; 6]), None, 0, &[1, 0, 0x30]))
        .await
        .unwrap();
    let npdu = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    transport.stop().await.unwrap();
    assert_eq!(npdu.npdu.as_ref(), &[1, 0, 0x30]);
}

async fn barrier(hub: &LoopbackWebSocket) {
    // An invalid known control is an ordered, non-activity barrier, not a valid
    // sentinel that could conceal unintended activity from resolution traffic.
    hub.send(&[0x0A, 0, 0x44, 0x55, 0x42]).await.unwrap();
    assert_eq!(recv(hub).await, [0, 0, 0x44, 0x55, 0x0A, 1, 0, 0, 7, 0, 7]);
}

#[tokio::test]
async fn address_resolution_valid_shapes_answered_or_silent_without_npdu() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    // Empty request is the only valid request shape. The fixture transport
    // is unconfigured, so each answer carries a valid empty URI list with
    // the request ID copied and the destination mirrored to the origin.
    for source in [None, Some([0x22; 6])] {
        hub.send(&wire(2, 0x2233, source, None, 0, &[]))
            .await
            .unwrap();
        let reply = recv_ack(&hub).await;
        assert_eq!(reply.function, ScFunction::AddressResolutionAck);
        assert_eq!(reply.message_id, 0x2233);
        assert_eq!(reply.destination_vmac, source);
        assert!(reply.payload.is_empty());
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    // Empty and well-formed URI lists are valid responses.
    for payload in valid_acks() {
        for source in [None, Some([0x22; 6])] {
            hub.send(&wire(3, 0x2233, source, None, 0, &payload))
                .await
                .unwrap();
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    // Non-MU destination options stay silent on responses; a valid request
    // carrying them is still answerable.
    for (function, payload) in [(2, vec![]), (3, b"wss://one.example/sc".to_vec())] {
        let mut optioned = vec![0x1F];
        optioned.extend_from_slice(&payload);
        hub.send(&wire(function, 0x2235, None, None, 2, &optioned))
            .await
            .unwrap();
        if function == 2 {
            let reply = recv_ack(&hub).await;
            assert_eq!(reply.function, ScFunction::AddressResolutionAck);
            assert_eq!(reply.message_id, 0x2235);
            assert!(reply.payload.is_empty());
        }
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn address_resolution_request_negative_matrix_nak_vs_silence() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    // (body, expected NAK code) for locally-addressed requests.
    let faults: Vec<(Vec<u8>, u16)> = vec![
        (vec![0x00], 7),
        (vec![0x01, 0x02], 7),
        (b"wss://one.example/sc".to_vec(), 7),
        (vec![0x2B; 64], 7),
    ];
    for (body, code) in faults {
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(2, 0x2233, source, None, 0, &body);
            hub.send(&bytes).await.unwrap();
            assert_eq!(
                recv(&hub).await,
                nak(2, 0x2233, source, code, 0),
                "request source {source:02x?} body {body:02x?}"
            );
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    // Data Options are out of range even around an empty request.
    for source in [None, Some([0x22; 6])] {
        hub.send(&wire(2, 0x2233, source, None, 1, &[0x01]))
            .await
            .unwrap();
        assert_eq!(recv(&hub).await, nak(2, 0x2233, source, 80, 0));
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    // Must-Understand destination option faults carry the wire marker.
    let mut mu_body = vec![0x42];
    for source in [None, Some([0x22; 6])] {
        hub.send(&wire(2, 0x2234, source, None, 2, &mu_body))
            .await
            .unwrap();
        assert_eq!(recv(&hub).await, nak(2, 0x2234, source, 146, 0x42));
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    mu_body.clear();
    // Addressed, broadcast, and reserved-origin envelopes stay silent for
    // both valid and forbidden request shapes without spending a budget.
    let silent_bodies: Vec<Vec<u8>> = vec![vec![], vec![0x00], b"wss://a.example/".to_vec()];
    for body in silent_bodies {
        for (source, dest) in [
            (None, Some([0x44; 6])),
            (Some([0x22; 6]), Some([0x44; 6])),
            (None, Some(BROADCAST_VMAC)),
            (Some([0x22; 6]), Some(BROADCAST_VMAC)),
            (Some([0; 6]), None),
            (Some(BROADCAST_VMAC), None),
            (Some([0; 6]), Some([0x44; 6])),
        ] {
            hub.send(&wire(2, 0x2235, source, dest, 0, &body))
                .await
                .unwrap();
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn address_resolution_ack_malformed_is_always_silent() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    // Every malformed response is discarded without a Result, per the
    // response rule, and without activity. Valid empty/nonempty lists are
    // covered by the silence test above.
    let mut malformed: Vec<Vec<u8>> = vec![
        vec![0xFF, 0x00],
        b" wss://one.example/sc".to_vec(),
        b"wss://one.example/sc ".to_vec(),
        b"wss://one.example/sc  wss://two.example/sc".to_vec(),
        b" ".to_vec(),
        b"ws://one.example/sc".to_vec(),
        b"wss://".to_vec(),
        b"wss:///sc".to_vec(),
        b"not-a-uri".to_vec(),
        b"wss://one.example/sc http://two.example/sc".to_vec(),
        "wss://one.example/sc\u{00e9}".as_bytes().to_vec(),
    ];
    malformed.push({
        let mut body = vec![0x01];
        body.extend_from_slice(b"wss://one.example/sc");
        body
    });
    malformed.push({
        let mut body = vec![0x42];
        body.extend_from_slice(b"wss://one.example/sc");
        body
    });
    for body in malformed {
        let flags = if body.first() == Some(&0x01) {
            1
        } else if body.first() == Some(&0x42) {
            2
        } else {
            0
        };
        for source in [None, Some([0x22; 6])] {
            hub.send(&wire(3, 0x2233, source, None, flags, &body))
                .await
                .unwrap();
            // No NAK may precede the barrier; the barrier proves silence.
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    // Addressed and reserved envelopes stay silent for valid lists too.
    for payload in [vec![], b"wss://one.example/sc".to_vec()] {
        for (source, dest) in [
            (None, Some([0x44; 6])),
            (Some([0x22; 6]), Some([0x44; 6])),
            (None, Some(BROADCAST_VMAC)),
            (Some([0; 6]), None),
            (Some(BROADCAST_VMAC), None),
        ] {
            hub.send(&wire(3, 0x2235, source, dest, 0, &payload))
                .await
                .unwrap();
            barrier(&hub).await;
            assert!(rx.try_recv().is_err());
        }
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn address_resolution_silence_and_codes_never_consult_expired_budget() {
    use super::rejection::{reject, RejectionBudget, RejectionExpired};
    struct NoIo;
    impl WebSocketPort for NoIo {
        async fn send(&self, _: &[u8]) -> Result<(), Error> {
            panic!("unexpected send")
        }
        async fn recv(&self) -> Result<Vec<u8>, Error> {
            panic!("unexpected receive")
        }
    }
    let expired = RejectionBudget::new(Instant::now() - Duration::from_secs(1), 1);
    // Forbidden request shapes must consult the budget (expiry surfaces).
    for body in [vec![0x00], vec![0x01, 0x02], b"wss://a.example/".to_vec()] {
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(2, 0, source, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Err(RejectionExpired),
                "request source {source:02x?}"
            );
        }
    }
    // Request MU faults also consult the budget when recoverable.
    let bytes = wire(2, 0, None, None, 2, &[0x42]);
    let msg = decode_sc_message(&bytes).unwrap();
    assert_eq!(
        reject(&msg, &bytes, &NoIo, expired).await,
        Err(RejectionExpired)
    );
    // Malformed responses never consult the budget: silence is free.
    for body in [
        vec![0xFF],
        b" wss://a.example/".to_vec(),
        b"ws://a.example/".to_vec(),
    ] {
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(3, 0, source, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Ok(true),
                "ack body {body:02x?}"
            );
        }
    }
    // Silent envelopes and valid shapes never consult the budget.
    for (function, body) in [(2, vec![]), (3, vec![]), (3, b"wss://a.example/".to_vec())] {
        for (source, dest) in [
            (None, Some([0x44; 6])),
            (None, Some(BROADCAST_VMAC)),
            (Some([0; 6]), None),
            (Some(BROADCAST_VMAC), None),
        ] {
            let bytes = wire(function, 0, source, dest, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Ok(true),
                "silent function {function}"
            );
        }
        for source in [None, Some([0x22; 6])] {
            let bytes = wire(function, 0, source, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(
                reject(&msg, &bytes, &NoIo, expired).await,
                Ok(false),
                "valid function {function}"
            );
        }
    }
}

#[test]
fn address_resolution_direct_connection_is_pure_in_all_states_and_codec_is_permissive() {
    for state in [
        ScConnectionState::Disconnected,
        ScConnectionState::Connecting,
        ScConnectionState::Connected,
        ScConnectionState::Disconnecting,
    ] {
        let mut conn = ScConnection::new([1; 6], [1; 16]);
        conn.build_connect_request();
        conn.state = state;
        conn.hub_vmac = Some([0x10; 6]);
        conn.hub_device_uuid = Some([0x33; 16]);
        conn.disconnect_ack_to_send = Some(conn.build_heartbeat_ack(0x4455));
        let before = conn.clone();
        for function in [2, 3] {
            for dest in [None, Some(BROADCAST_VMAC), Some([1; 6])] {
                for body in [&[][..], &[0x00][..], b"wss://a.example/".as_slice()] {
                    let bytes = wire(function, u16::MAX, Some([0x22; 6]), dest, 0, body);
                    let msg = decode_sc_message(&bytes).unwrap();
                    let mut encoded = BytesMut::new();
                    encode_sc_message(&mut encoded, &msg);
                    assert_eq!(encoded.as_ref(), bytes);
                    assert!(conn.handle_received(&msg).is_none());
                    assert_eq!(conn.state, before.state);
                    assert_eq!(conn.local_vmac, before.local_vmac);
                    assert_eq!(conn.device_uuid, before.device_uuid);
                    assert_eq!(conn.hub_vmac, before.hub_vmac);
                    assert_eq!(conn.hub_device_uuid, before.hub_device_uuid);
                    assert_eq!(conn.max_bvlc_length, before.max_bvlc_length);
                    assert_eq!(conn.max_apdu_length, before.max_apdu_length);
                    assert_eq!(conn.hub_max_bvlc_length, before.hub_max_bvlc_length);
                    assert_eq!(conn.hub_max_apdu_length, before.hub_max_apdu_length);
                    assert_eq!(conn.next_message_id, before.next_message_id);
                    assert_eq!(
                        conn.pending_connect_message_id,
                        before.pending_connect_message_id
                    );
                    assert_eq!(conn.connect_retry_allowed, before.connect_retry_allowed);
                    assert_eq!(conn.disconnect_ack_to_send, before.disconnect_ack_to_send);
                }
            }
        }
    }
}

#[tokio::test]
async fn address_resolution_malformed_generic_wire_is_still_silent() {
    let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
    for bytes in [
        vec![0x02],
        vec![0x02, 0x80, 0, 1],
        vec![0x02, 8, 0, 1],
        vec![0x02, 4, 0, 1],
        vec![0x02, 2, 0, 1, 0],
        vec![0x03, 1, 0, 1, 0x7E, 0, 1],
    ] {
        assert!(decode_sc_message(&bytes).is_err());
        hub.send(&bytes).await.unwrap();
        barrier(&hub).await;
        assert!(rx.try_recv().is_err());
    }
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn address_resolution_result_for_keeps_existing_parse_and_fatal_policy() {
    for raw in [2, 3] {
        for (body, fatal) in [
            (vec![raw, 0], false),
            (vec![raw, 1, 0, 0, 7, 0, 150], true),
            (vec![raw, 1], true),
        ] {
            let (mut transport, mut rx, hub) = super::data_attribute_tests::start_transport().await;
            let bytes = wire(0, 0x2233, None, None, 0, &body);
            let msg = decode_sc_message(&bytes).unwrap();
            assert_eq!(msg.function, ScFunction::Result);
            hub.send(&bytes).await.unwrap();
            let mut states = transport.connection_state_changes();
            let finished = if fatal {
                timeout(Duration::from_secs(1), async {
                    while *states.borrow_and_update() != ScConnectionState::Disconnected {
                        states.changed().await.unwrap();
                    }
                })
                .await
            } else {
                barrier(&hub).await;
                assert_eq!(*states.borrow(), ScConnectionState::Connected);
                Ok(())
            };
            let extra = timeout(Duration::from_millis(20), hub.recv()).await;
            transport.stop().await.unwrap();
            finished.unwrap();
            assert!(extra.is_err(), "response on Result");
            assert!(rx.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn address_resolution_valid_traffic_keeps_heartbeat_and_npdu_interop() {
    let (client, hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(80, 700);
    let (rx, ()) = tokio::join!(
        transport.start(),
        super::data_attribute_tests::hub_accept(&hub, [0x10; 6])
    );
    let mut rx = rx.unwrap();
    let probe = recv(&hub).await;
    assert_eq!(probe[0], 0x0A);
    // Valid requests earn one empty ACK each (fixture is unconfigured);
    // valid responses stay silent.
    for _ in 0..4 {
        hub.send(&wire(2, 0, None, None, 0, &[])).await.unwrap();
        assert!(recv_ack(&hub).await.payload.is_empty());
        hub.send(&wire(
            3,
            1,
            Some([0x22; 6]),
            None,
            0,
            b"wss://one.example/sc",
        ))
        .await
        .unwrap();
    }
    // No rejection reply may precede the next liveness probe; accepted
    // resolution traffic keeps the heartbeat budget alive like NPDUs.
    let next = timeout(Duration::from_secs(1), hub.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next[0], 0x0A);
    assert_ne!(&next[2..4], &probe[2..4]);
    assert!(rx.try_recv().is_err());
    // Positive controls only AFTER the resolution window.
    hub.send(&[0x0B, 0, next[2], next[3]]).await.unwrap();
    hub.send(&wire(1, 0, Some([0x22; 6]), None, 0, &[1, 0, 0x30]))
        .await
        .unwrap();
    let npdu = timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    transport.stop().await.unwrap();
    assert_eq!(npdu.npdu.as_ref(), &[1, 0, 0x30]);
}
