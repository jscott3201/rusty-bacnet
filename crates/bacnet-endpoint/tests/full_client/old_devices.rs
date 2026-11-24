//! Peers that predate the 2020 standard, or just misbehave: answers nobody
//! asked for, octets that are no APDU, and a device with neither
//! ReadPropertyMultiple nor segmentation. On a seat they are counted and
//! dropped, or fail a call as they fail a standalone client's; nothing
//! stalls.
use super::*;
use bacnet_encoding::apdu::{AbortPdu, ComplexAck, ErrorPdu, RejectPdu, SegmentAck, SimpleAck};
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, PropertyIdentifier, RejectReason,
};

#[tokio::test(start_paused = true)]
async fn answers_nobody_asked_for_and_octets_that_are_no_apdu_are_counted_and_dropped() {
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.client().unwrap();
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let service_choice = ConfirmedServiceChoice::READ_PROPERTY;
    let stray = [
        Apdu::SimpleAck(SimpleAck {
            invoke_id: 200,
            service_choice,
        }),
        Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows: true,
            invoke_id: 201,
            sequence_number: Some(0),
            proposed_window_size: Some(4),
            service_choice,
            service_ack: Bytes::from_static(&[0x0c]),
        }),
        Apdu::Error(ErrorPdu {
            invoke_id: 202,
            service_choice,
            error_class: ErrorClass::DEVICE,
            error_code: ErrorCode::OTHER,
            error_data: Bytes::new(),
        }),
        Apdu::Reject(RejectPdu {
            invoke_id: 203,
            reject_reason: RejectReason::OTHER,
        }),
        Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id: 204,
            abort_reason: AbortReason::OTHER,
        }),
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: 205,
            sequence_number: 0,
            actual_window_size: 1,
        }),
    ];
    let sent = lab.node_sent.len();
    for apdu in &stray {
        lab.wire
            .deliver(&node, wire::frame(&peer, npdu_of(apdu, false), false));
    }
    // A truncated Complex-ACK, and a PDU type nothing defines.
    for octets in [&[0x30][..], &[0x90, 0x01]] {
        let npdu = [&[0x01, 0x00][..], octets].concat();
        lab.wire
            .deliver(&node, wire::frame(&peer, Bytes::from(npdu), false));
    }
    eventually(WAIT, "each stray answer counted", async || {
        let counters = lab.session.policy_counters();
        (counters.unclaimed_terminal, counters.ingress_policy) == (4, 2)
    })
    .await;

    // A stray segment of a Complex-ACK and a stray server Segment-ACK reach
    // the client, which aborts each as an idle client does (Clause 5.4.4.1,
    // #2617). The other strays, a server's Abort among them, which an idle
    // client would only drop, stay with the endpoint, which counts them, and
    // the node says nothing back to them.
    eventually(WAIT, "the client's two Aborts", async || {
        lab.node_sent.len() == sent + 2
    })
    .await;
    assert_eq!(lab.session.client_seat_counters().apdus, 2);
    let aborts: Vec<_> = lab.node_sent.frames()[sent..]
        .iter()
        .map(|frame| {
            assert_eq!(frame.mac, peer);
            let npdu = decode_npdu(frame.npdu.clone()).unwrap();
            match decode_apdu(npdu.payload).unwrap() {
                Apdu::Abort(abort) => (abort.sent_by_server, abort.invoke_id, abort.abort_reason),
                other => panic!("{other:?}"),
            }
        })
        .collect();
    assert_eq!(
        aborts,
        [201, 205].map(|id| (false, id, AbortReason::INVALID_APDU_IN_THIS_STATE))
    );
    // And nothing is stuck behind them.
    client
        .read_property(&peer, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();

    drop(client);
    stop(lab).await;
}

/// A device at node 4 that rejects ReadPropertyMultiple as a service it
/// doesn't know and aborts every other confirmed request as needing
/// segmentation it lacks.
async fn old_device(wire: &Wire) -> tokio::task::JoinHandle<()> {
    let mut transport = wire.port(4).build();
    let mut inbound = transport.start().await.unwrap();
    tokio::spawn(async move {
        while let Some(received) = inbound.recv().await {
            let Ok(npdu) = decode_npdu(received.npdu) else {
                continue;
            };
            let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) else {
                continue;
            };
            let invoke_id = request.invoke_id;
            let answer = if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
            {
                Apdu::Reject(RejectPdu {
                    invoke_id,
                    reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
                })
            } else {
                Apdu::Abort(AbortPdu {
                    sent_by_server: true,
                    invoke_id,
                    abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
                })
            };
            let _ = transport
                .send_unicast(&npdu_of(&answer, false), &received.source_mac)
                .await;
        }
    })
}

/// What `client` gets from the old device: a ReadPropertyMultiple, then a
/// read of its Object_List.
async fn ask_the_old_device(client: &BACnetClient<TestTransport>) -> [String; 2] {
    let old = Wire::mac(4);
    let device = oid(ObjectType::DEVICE, 4004);
    let multiple = client
        .read_property_multiple(
            &old,
            vec![ReadAccessSpecification {
                object_identifier: device,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                }],
            }],
        )
        .await;
    let list = client
        .read_property(&old, device, PropertyIdentifier::OBJECT_LIST, None)
        .await;
    [
        format!("{:?}", multiple.unwrap_err()),
        format!("{:?}", list.unwrap_err()),
    ]
}

#[tokio::test(start_paused = true)]
async fn a_device_with_no_rpm_and_no_segmentation_fails_a_seated_client_as_a_standalone_one() {
    let lab = lab(SessionRole::Both).await;
    let device = old_device(&lab.wire).await;
    let standalone = other_client(&lab.wire).await;
    let expected = ask_the_old_device(&standalone).await;
    assert!(expected[0].contains("Reject"), "{expected:?}");
    assert!(expected[1].contains("Abort"), "{expected:?}");

    let client = lab.session.client().unwrap();
    let asked = tokio::time::Instant::now();
    assert_eq!(ask_the_old_device(&client).await, expected);
    // Each refusal ended its request at once, not at a timeout.
    assert_eq!(asked.elapsed(), Duration::ZERO);
    assert_eq!(lab.session.active_leases(), 0);
    assert_eq!(lab.session.policy_counters().unclaimed_terminal, 0);

    device.abort();
    drop((client, standalone));
    stop(lab).await;
}
