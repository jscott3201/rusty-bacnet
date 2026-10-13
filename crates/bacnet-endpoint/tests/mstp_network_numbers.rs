//! Local Number controls through actual MS/TP frames and the full-server,
//! shared-endpoint and standalone-client owners.
//! LoopbackSerial evidence only; no RS-485 timing or hardware qualification.
#[path = "support/mstp_wire.rs"]
mod mstp_wire;
#[path = "mstp_network_numbers/support.rs"]
mod support;
use bacnet_client::client::BACnetClient;
use bacnet_encoding::{
    apdu::{decode_apdu, Apdu},
    npdu::decode_npdu,
};
use bacnet_transport::{mstp::codec::FrameType, mstp::MstpExecutionMode, port::TransportPort};
use bacnet_types::{
    enums::{ObjectType, PropertyIdentifier},
    primitives::ObjectIdentifier,
};
use support::*;

async fn controls(role: Role, mode: MstpExecutionMode) {
    let (mut owner, mut peer) = fixture(role, mode, Gates::new(false, false, false)).await;
    // An unknown owner and a unicast announcement cannot yield an earlier reply.
    // The first valid reply is a FIFO fence, not a timeout-based silence claim.
    peer.control(NODE, &[1, 0x80, 0x12]).await;
    peer.control(NODE, &[1, 0x80, 0x13, 0, 99, 1]).await;
    peer.control(BROADCAST, &[1, 0x80, 0x12]).await;
    peer.control(BROADCAST, &[1, 0x80, 0x13, 0, 77, 0]).await;
    peer.control(BROADCAST, &[1, 0x80, 0x12]).await;
    peer.number(77).await;
    peer.control(NODE, &[1, 0x80, 0x12]).await;
    peer.number(77).await;
    for (number, flag, expected) in [(78, 0, 78), (79, 1, 79), (80, 0, 79), (81, 1, 81)] {
        peer.control(BROADCAST, &[1, 0x80, 0x13, 0, number, flag])
            .await;
        peer.control(NODE, &[1, 0x80, 0x12]).await;
        peer.number(expected).await;
    }
    // Numeric/flag rejection is existing fail-closed policy, not a new mandate.
    for (destination, bad) in [
        (NODE, vec![1, 0x80, 0x13, 0, 99, 1]),
        (BROADCAST, vec![1, 0x80, 0x13, 0, 0, 1]),
        (BROADCAST, vec![1, 0x80, 0x13, 255, 255, 1]),
        (BROADCAST, vec![1, 0x80, 0x13, 0, 99, 2]),
        (BROADCAST, vec![1, 0x80, 0x13, 0, 99]),
        (BROADCAST, vec![1, 0x88, 0, 4, 1, 9, 0x13, 0, 99, 1]),
        (BROADCAST, vec![1, 0xa0, 255, 255, 0, 255, 0x13, 0, 99, 1]),
    ] {
        peer.control(destination, &bad).await;
        peer.control(NODE, &[1, 0x80, 0x12]).await;
        peer.number(81).await;
    }
    for (i, bad) in [
        vec![1, 0x80, 0x12, 0],
        vec![1, 0x88, 0, 4, 1, 9, 0x12],
        vec![1, 0xa0, 255, 255, 0, 255, 0x12],
    ]
    .iter()
    .enumerate()
    {
        peer.control(NODE, bad).await;
        let next = 82 + i as u8;
        peer.control(BROADCAST, &[1, 0x80, 0x13, 0, next, 1]).await;
        peer.control(NODE, &[1, 0x80, 0x12]).await;
        // An erroneous response to the earlier query would carry the old value.
        peer.number(next.into()).await;
    }
    bounded(owner.stop()).await;
}

async fn blocked_producer_progress(role: Role, mode: MstpExecutionMode) {
    let gates = Gates::new(true, false, false);
    let (mut owner, mut peer) = fixture(role, mode, gates.clone()).await;
    peer.control(BROADCAST, &[1, 0x80, 0x13, 0, 77, 1]).await;
    peer.control(NODE, &[1, 0x80, 0x12]).await;
    bounded(gates.queued.acquire()).await.unwrap().forget();
    // Drain the real queued frame; the producer stays held after enqueue, but
    // does not own the serial writer or MAC mutex, so application egress can run.
    peer.number(77).await;
    match &owner {
        Owner::Client(client) => requester_progress(client, &mut peer).await,
        Owner::Server(_) | Owner::Endpoint(_) => responder_progress(&mut peer).await,
    }
    assert_eq!(
        gates.send_dropped.available_permits(),
        0,
        "producer remains held during APDU progress"
    );
    bounded(owner.stop()).await;
    bounded(gates.send_dropped.acquire())
        .await
        .unwrap()
        .forget();
}

/// The owner answers an independent ReadProperty from the peer.
async fn responder_progress(peer: &mut Peer) {
    // Independent ReadProperty: analog-input 1 / present-value, Invoke ID 42.
    // NPDU version1/expecting-reply, confirmed unsegmented APDU, max-APDU480.
    let request = [1, 4, 0, 3, 42, 12, 0x0c, 0, 0, 0, 1, 0x19, 85];
    peer.await_token().await;
    bounded(peer.frame(FrameType::BACNET_DATA_EXPECTING_REPLY, NODE, &request))
        .await
        .unwrap();
    let frame = peer.answer().await;
    assert_eq!(frame.destination, PEER);
    let npdu = decode_npdu(frame.data).unwrap();
    assert!(!npdu.is_network_message);
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::ComplexAck(ack) => {
            assert_eq!(ack.invoke_id, 42);
            assert_eq!(ack.service_choice.to_raw(), 12);
            // Encoded real value42.0 appears inside the returned property value.
            assert!(ack
                .service_ack
                .windows(5)
                .any(|bytes| bytes == [0x44, 0x42, 0x28, 0, 0]));
        }
        other => panic!("expected ReadProperty ACK, got {other:?}"),
    }
}

/// A standalone client has no responder role; its own confirmed request to the
/// peer must complete through the same serial owner instead.
async fn requester_progress(client: &BACnetClient<GatedTransport>, peer: &mut Peer) {
    let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let request = client.read_property(&[PEER], object, PropertyIdentifier::PRESENT_VALUE, None);
    let respond = async {
        let frame = peer.data().await;
        assert_eq!(frame.frame_type, FrameType::BACNET_DATA_EXPECTING_REPLY);
        assert_eq!(frame.destination, PEER);
        let npdu = decode_npdu(frame.data).unwrap();
        assert!(!npdu.is_network_message);
        let invoke_id = match decode_apdu(npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => {
                assert_eq!(request.service_choice.to_raw(), 12);
                // analog-input 1 / present-value, no array index.
                assert_eq!(
                    request.service_request.as_ref(),
                    [0x0c, 0, 0, 0, 1, 0x19, 85]
                );
                request.invoke_id
            }
            other => panic!("expected ReadProperty request, got {other:?}"),
        };
        // NPDU version1/no reply expected; ReadProperty ComplexACK for the same
        // object and property, carrying REAL 42.0 between opening/closing tag 3.
        let ack = [
            1, 0, 0x30, invoke_id, 12, 0x0c, 0, 0, 0, 1, 0x19, 85, 0x3e, 0x44, 0x42, 0x28, 0, 0,
            0x3f,
        ];
        bounded(peer.frame(FrameType::BACNET_DATA_NOT_EXPECTING_REPLY, NODE, &ack))
            .await
            .unwrap();
    };
    let (ack, ()) = bounded(async { tokio::join!(request, respond) }).await;
    let ack = ack.expect("client ReadProperty completes while the Number producer is held");
    assert_eq!(ack.object_identifier, object);
    assert_eq!(ack.property_value, [0x44, 0x42, 0x28, 0, 0]);
}

async fn stopped_producer(role: Role, mode: MstpExecutionMode, bare_drop: bool) {
    let gates = Gates::new(true, true, !bare_drop);
    let (mut owner, mut peer) = fixture(role, mode, gates.clone()).await;
    peer.control(BROADCAST, &[1, 0x80, 0x13, 0, 77, 0]).await;
    peer.control(NODE, &[1, 0x80, 0x12]).await;
    bounded(gates.queued.acquire()).await.unwrap().forget();
    // Actual queue admission happened. The optional serial gate also prevents
    // autonomous lost-token recovery from completing a number frame.
    if bare_drop {
        drop(owner);
    } else {
        {
            let stop = owner.stop();
            tokio::pin!(stop);
            bounded(async {
                tokio::select! {
                    _ = &mut stop => panic!("stop must wait for the transport gate"),
                    permit = gates.stop_started.acquire() => { permit.unwrap().forget(); }
                }
            })
            .await;
            // Drop this stop waiter while cleanup is known to be pending.
        }
        bounded(gates.send_dropped.acquire())
            .await
            .unwrap()
            .forget();
        gates.stop_release.add_permits(1);
        bounded(owner.stop()).await;
        drop(owner);
    }
    if bare_drop {
        bounded(gates.send_dropped.acquire())
            .await
            .unwrap()
            .forget();
    }
    peer.assert_released(&gates).await;
}

#[tokio::test]
async fn mstp_number_transport_group_and_any_delegation() {
    for mode in [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread] {
        let (mut transport, peer) = transport(mode, Gates::new(false, false, false)).await;
        assert!(transport.supports_local_nonrouter_number_controls());
        let mut received = transport.start().await.unwrap();
        for (destination, group) in [(NODE, false), (BROADCAST, true)] {
            let bytes = [1, 0x80, 0x13, 0, 77, 0];
            peer.control(destination, &bytes).await;
            let packet = bounded(received.recv()).await.unwrap();
            assert_eq!(packet.link_layer_group, group);
            assert_eq!(packet.source_mac.as_slice(), [PEER]);
            assert_eq!(packet.npdu.as_ref(), bytes);
        }
        bounded(transport.stop()).await.unwrap();
    }
}
macro_rules! mode_tests {
    ($name:ident, $role:expr, $mode:expr) => {
        #[tokio::test]
        async fn $name() {
            controls($role, $mode).await;
        }
    };
}
mode_tests!(
    mstp_number_server_tokio_wire,
    Role::Server,
    MstpExecutionMode::Tokio
);
mode_tests!(
    mstp_number_server_dedicated_wire,
    Role::Server,
    MstpExecutionMode::DedicatedThread
);
mode_tests!(
    mstp_number_endpoint_tokio_wire,
    Role::Endpoint,
    MstpExecutionMode::Tokio
);
mode_tests!(
    mstp_number_endpoint_dedicated_wire,
    Role::Endpoint,
    MstpExecutionMode::DedicatedThread
);
mode_tests!(
    mstp_number_client_tokio_wire,
    Role::Client,
    MstpExecutionMode::Tokio
);
mode_tests!(
    mstp_number_client_dedicated_wire,
    Role::Client,
    MstpExecutionMode::DedicatedThread
);
const MODES: [MstpExecutionMode; 2] =
    [MstpExecutionMode::Tokio, MstpExecutionMode::DedicatedThread];
#[tokio::test]
async fn mstp_number_held_producer_preserves_application_progress() {
    for role in [Role::Server, Role::Endpoint] {
        for mode in MODES {
            blocked_producer_progress(role, mode).await;
        }
    }
}
#[tokio::test]
async fn mstp_number_client_held_producer_preserves_request_progress() {
    for mode in MODES {
        blocked_producer_progress(Role::Client, mode).await;
    }
}
#[tokio::test]
async fn mstp_number_queued_producer_canceled_stop_and_drop() {
    for role in [Role::Server, Role::Endpoint] {
        for mode in MODES {
            for bare_drop in [false, true] {
                stopped_producer(role, mode, bare_drop).await;
            }
        }
    }
}
#[tokio::test]
async fn mstp_number_client_queued_producer_canceled_stop_and_drop() {
    for mode in MODES {
        for bare_drop in [false, true] {
            stopped_producer(Role::Client, mode, bare_drop).await;
        }
    }
}
