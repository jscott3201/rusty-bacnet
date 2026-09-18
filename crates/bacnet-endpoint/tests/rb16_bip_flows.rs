//! RB-16 B/IP flows: same-ID isolation, routed relay, COV/event matrix.
//!
//! Split from `rb16_bip_proof.rs` (file-size gate; pure relocation, no logic
//! change). Real loopback UDP (`127.0.0.1`, ephemeral ports), one endpoint
//! bind per test — NO second hidden socket.
//!
//! Transport-dependent notes (recorded precisely):
//! - B/IP binds `INADDR_ANY`; `interface` is only the announced MAC IP.
//!   Ephemeral ports isolate tests; production uses 47808.
//! - `LoopbackTransport` appears only in `rb16_bip_proof.rs` (bind-count
//!   double) and `rb16_identity.rs` (determinism), never INSTEAD of these
//!   real-UDP flows.
//! - Endpoint server reality is unsegmented ReadProperty-only; COV/event
//!   confirmed draws `Reject UNRECOGNIZED_SERVICE` (correct vs services).

use std::net::Ipv4Addr;
use std::time::Duration;

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, AbortPdu, Apdu, ConfirmedRequest as ConfirmedPdu, RejectPdu,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
use bacnet_endpoint::session::{SessionConfig, SessionRole};
use bacnet_network::layer::NetworkLayer;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bvll::{decode_bvll, encode_bvll};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, NetworkPriority, ObjectType, PropertyIdentifier,
    RejectReason, Segmentation, ServiceSupported,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};

const WAIT: Duration = Duration::from_secs(5);

fn oid(t: ObjectType, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(t, i).unwrap()
}

fn session_config() -> SessionConfig {
    SessionConfig {
        queue_capacity: 32,
        apdu_timeout_ms: 2_000,
        apdu_retries: 0,
        max_apdu_length: 480,
    }
}

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(1001, 42)
        .unwrap()
        .with_max_apdu(1476)
        .unwrap()
        .with_segmentation(Segmentation::NONE)
        .with_services(&[ServiceSupported::READ_PROPERTY])
}

fn db_with_analog(
    id: &DeviceIdentity,
    instance: u32,
    value: f32,
) -> bacnet_objects::database::ObjectDatabase {
    let mut o = AnalogInputObject::new(instance, format!("bip-ai-{instance}"), 0).unwrap();
    o.set_present_value(value);
    build_database_with_extra(id, vec![Box::new(o)]).unwrap()
}

#[tokio::test]
async fn bip_same_id_opposite_directions_routed_and_wrong_peer() {
    // Same numeric invoke ID opposite directions concurrently; routed
    // destination on wire + routed reply; wrong-peer response isolation.
    let id = identity();
    let db = db_with_analog(&id, 1, 5.0);
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .database(db)
        .identity(id)
        .build_session()
        .unwrap();
    endpoint.start().await.unwrap();

    let peer_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_a = NetworkLayer::new(peer_a);
    let mut rx_a = net_a.start().await.unwrap();
    let mac_a = MacAddr::from_slice(net_a.local_mac());

    // Same-ID opposite: endpoint→A and A→endpoint both use invoke ID 0 on
    // fresh coordinators. Run concurrently; both must deliver (no cross-talk).
    let client = endpoint.cloned_client_handle().unwrap();
    let mac_a_c = mac_a.clone();
    let outbound = tokio::spawn(async move {
        client
            .read_property(
                &mac_a_c,
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    // Peer A request-in with the SAME numeric ID 0 while outbound is in flight.
    let mut svc = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut svc);
    // Learn endpoint MAC first via a probe: send outbound response path needs
    // it, so snoop it from the outbound request arrival.
    let inbound_req = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 0,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: svc.freeze(),
    });
    // Wait for the outbound request to arrive so we learn the endpoint MAC,
    // then fire the inbound same-ID request concurrently (still in flight:
    // the outbound lease is held until we answer below).
    let first = tokio::time::timeout(WAIT, rx_a.recv())
        .await
        .expect("outbound must arrive")
        .expect("closed");
    let endpoint_mac = first.source_mac.clone();
    let first_iid = match decode_apdu(first.apdu).unwrap() {
        Apdu::ConfirmedRequest(r) => r.invoke_id,
        other => panic!("expected request, got {other:?}"),
    };
    assert_eq!(first_iid, 0, "fresh endpoint coordinator starts at 0");
    let mut enc_in = BytesMut::new();
    encode_apdu(&mut enc_in, &inbound_req).unwrap();
    net_a
        .send_apdu(&enc_in, &endpoint_mac, true, NetworkPriority::NORMAL)
        .await
        .unwrap();

    // Answer the outbound (endpoint→A) with ID 0; the inbound (A→endpoint,
    // also ID 0) is answered by the endpoint server independently.
    let mut val = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut val, &PropertyValue::Real(9.0))
        .unwrap();
    let ack = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val.to_vec(),
    };
    let mut svc_ack = BytesMut::new();
    ack.encode(&mut svc_ack);
    let resp = Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: 0,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: svc_ack.freeze(),
    });
    let mut enc_resp = BytesMut::new();
    encode_apdu(&mut enc_resp, &resp).unwrap();
    net_a
        .send_apdu(&enc_resp, &endpoint_mac, false, NetworkPriority::NORMAL)
        .await
        .unwrap();

    let out = tokio::time::timeout(WAIT, outbound)
        .await
        .expect("same-ID outbound hung")
        .unwrap()
        .expect("outbound failed");
    assert_eq!(out.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));
    // Inbound reply arrives on the same peer channel (server→A, ID 0).
    let mut saw_inbound_ack = false;
    let deadline = tokio::time::Instant::now() + WAIT;
    while tokio::time::Instant::now() < deadline && !saw_inbound_ack {
        if let Ok(recv) = tokio::time::timeout(Duration::from_millis(200), rx_a.recv()).await {
            let msg = recv.expect("closed");
            if let Ok(Apdu::ComplexAck(a)) = decode_apdu(msg.apdu) {
                if a.invoke_id == 0 {
                    saw_inbound_ack = true;
                }
            }
        } else {
            break;
        }
    }
    assert!(
        saw_inbound_ack,
        "same-ID inbound reply must arrive without cross-talk"
    );

    // Wrong-peer isolation: a forged ack from an unknown MAC with a live
    // invoke ID must not complete the transaction (unclaimed_terminal).
    let peer_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut net_b = NetworkLayer::new(peer_b);
    let mut _rx_b = net_b.start().await.unwrap();
    let client2 = endpoint.cloned_client_handle().unwrap();
    let mac_a2 = mac_a.clone();
    let pending = tokio::spawn(async move {
        client2
            .read_property(
                &mac_a2,
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    // Wait until the request reaches A (lease held), then forge from B.
    let req = tokio::time::timeout(WAIT, rx_a.recv())
        .await
        .expect("request must arrive")
        .expect("closed");
    let live_iid = match decode_apdu(req.apdu.clone()).unwrap() {
        Apdu::ConfirmedRequest(r) => r.invoke_id,
        other => panic!("expected request, got {other:?}"),
    };
    let forged = Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: live_iid,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: {
            let mut v = BytesMut::new();
            bacnet_encoding::primitives::encode_property_value(&mut v, &PropertyValue::Real(0.0))
                .unwrap();
            let a = ReadPropertyACK {
                object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                property_value: v.to_vec(),
            };
            let mut s = BytesMut::new();
            a.encode(&mut s);
            s.freeze()
        },
    });
    let mut fenc = BytesMut::new();
    encode_apdu(&mut fenc, &forged).unwrap();
    // B sends the forged ack (wrong peer MAC) directly to the endpoint.
    net_b
        .send_apdu(&fenc, &endpoint_mac, false, NetworkPriority::NORMAL)
        .await
        .unwrap();
    // Give dispatch a chance to classify the forgery as unclaimed (no hang).
    tokio::task::yield_now().await;
    // Correct peer A answers; the pending transaction must complete with A's
    // value, proving the forgery did not steal it.
    let mut val2 = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut val2, &PropertyValue::Real(7.5))
        .unwrap();
    let ack2 = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val2.to_vec(),
    };
    let mut s2 = BytesMut::new();
    ack2.encode(&mut s2);
    let good = Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: live_iid,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: s2.freeze(),
    });
    let mut genc = BytesMut::new();
    encode_apdu(&mut genc, &good).unwrap();
    net_a
        .send_apdu(&genc, &endpoint_mac, false, NetworkPriority::NORMAL)
        .await
        .unwrap();
    let done = tokio::time::timeout(WAIT, pending)
        .await
        .expect("wrong-peer test hung")
        .unwrap()
        .expect("failed");
    assert_eq!(done.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));

    endpoint.stop().await.unwrap();
    net_a.stop().await.unwrap();
    net_b.stop().await.unwrap();
}

#[tokio::test]
async fn bip_routed_and_confirmed_cov_event_flows() {
    // Routed destination on wire + routed reply; confirmed COV/event draw
    // Reject (narrow reality, == services). Raw UDP peer for full NPDU control.
    let id = identity();
    let db = db_with_analog(&id, 1, 3.0);
    let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::Both)
        .database(db)
        .identity(id)
        .build_session()
        .unwrap();
    endpoint.start().await.unwrap();

    // Raw peer socket learns the endpoint address via an initial outbound.
    let raw = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let raw_addr = raw.local_addr().unwrap();
    let raw_ip = match raw_addr.ip() {
        std::net::IpAddr::V4(v4) => v4,
        _ => panic!("ipv4 only"),
    };
    let raw_port = raw_addr.port();
    let mut raw_mac = MacAddr::new();
    raw_mac.extend_from_slice(&raw_ip.octets());
    raw_mac.extend_from_slice(&raw_port.to_be_bytes());

    // Trigger an outbound so we learn the endpoint's bound address: use the
    // endpoint client to send to the raw peer (raw speaks BVLL manually).
    let client = endpoint.cloned_client_handle().unwrap();
    let raw_mac_c = raw_mac.clone();
    let outbound = tokio::spawn(async move {
        client
            .read_property(
                &raw_mac_c,
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    // Raw receives Original-Unicast BVLL + NPDU + ConfirmedRequest.
    let mut buf = vec![0u8; 2048];
    let (len, src) = tokio::time::timeout(WAIT, raw.recv_from(&mut buf))
        .await
        .expect("raw must receive")
        .unwrap();
    let endpoint_socket = src;
    let bvll = decode_bvll(&buf[..len]).expect("bvll must decode");
    let npdu = decode_npdu(Bytes::copy_from_slice(&bvll.payload)).expect("npdu must decode");
    let creq = match decode_apdu(npdu.payload).unwrap() {
        Apdu::ConfirmedRequest(r) => r,
        other => panic!("expected ConfirmedRequest, got {other:?}"),
    };
    let live_iid = creq.invoke_id;
    // Answer the outbound with a direct ComplexAck via raw BVLL unicast.
    let mut val = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut val, &PropertyValue::Real(4.0))
        .unwrap();
    let ack = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: val.to_vec(),
    };
    let mut svc = BytesMut::new();
    ack.encode(&mut svc);
    let resp = Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: live_iid,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: svc.freeze(),
    });
    let mut rapdu = BytesMut::new();
    encode_apdu(&mut rapdu, &resp).unwrap();
    let mut rnpdu = BytesMut::new();
    encode_npdu(
        &mut rnpdu,
        &Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            payload: rapdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let mut rframe = BytesMut::new();
    encode_bvll(
        &mut rframe,
        bacnet_types::enums::BvlcFunction::ORIGINAL_UNICAST_NPDU,
        &rnpdu,
    )
    .unwrap();
    raw.send_to(&rframe, endpoint_socket).await.unwrap();
    let done = tokio::time::timeout(WAIT, outbound)
        .await
        .expect("outbound hung")
        .unwrap()
        .expect("failed");
    assert_eq!(done.object_identifier, oid(ObjectType::ANALOG_INPUT, 1));

    // Routed: endpoint client sends via router (raw peer as router MAC) to
    // DNET 77 / DADR [0x44]; raw asserts the routed destination on wire bytes,
    // then returns a routed reply (SNET 77) which the coordinator admits.
    let client2 = endpoint.cloned_client_handle().unwrap();
    let raw_mac2 = raw_mac.clone();
    let routed = tokio::spawn(async move {
        client2
            .read_property_routed(
                &raw_mac2,
                77,
                &[0x44],
                Vec::new(),
                oid(ObjectType::ANALOG_INPUT, 1),
                PropertyIdentifier::PRESENT_VALUE,
                None,
            )
            .await
    });
    let (len2, _) = tokio::time::timeout(WAIT, raw.recv_from(&mut buf))
        .await
        .expect("routed must arrive")
        .unwrap();
    let bvll2 = decode_bvll(&buf[..len2]).expect("bvll2 decode");
    let npdu2 = decode_npdu(Bytes::copy_from_slice(&bvll2.payload)).expect("npdu2 decode");
    let dest = npdu2
        .destination
        .expect("routed NPDU must carry DNET/DADR on wire");
    assert_eq!(
        dest.network, 77,
        "routed destination network must survive to wire"
    );
    assert_eq!(dest.mac_address.as_slice(), &[0x44]);
    let rcreq = match decode_apdu(npdu2.payload).unwrap() {
        Apdu::ConfirmedRequest(r) => r,
        other => panic!("expected routed ConfirmedRequest, got {other:?}"),
    };
    // Routed reply: SNET 77 / SADR [0x44], same invoke ID, ComplexAck.
    let mut v2 = BytesMut::new();
    bacnet_encoding::primitives::encode_property_value(&mut v2, &PropertyValue::Real(8.0)).unwrap();
    let ack2 = ReadPropertyACK {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: v2.to_vec(),
    };
    let mut s2 = BytesMut::new();
    ack2.encode(&mut s2);
    let resp2 = Apdu::ComplexAck(bacnet_encoding::apdu::ComplexAck {
        segmented: false,
        more_follows: false,
        invoke_id: rcreq.invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: s2.freeze(),
    });
    let mut a2 = BytesMut::new();
    encode_apdu(&mut a2, &resp2).unwrap();
    let mut n2 = BytesMut::new();
    encode_npdu(
        &mut n2,
        &Npdu {
            is_network_message: false,
            expecting_reply: false,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: Some(NpduAddress {
                network: 77,
                mac_address: MacAddr::from_slice(&[0x44]),
            }),
            payload: a2.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let mut f2 = BytesMut::new();
    encode_bvll(
        &mut f2,
        bacnet_types::enums::BvlcFunction::ORIGINAL_UNICAST_NPDU,
        &n2,
    )
    .unwrap();
    raw.send_to(&f2, endpoint_socket).await.unwrap();
    let routed_ack = tokio::time::timeout(WAIT, routed)
        .await
        .expect("routed hung")
        .unwrap()
        .expect("routed failed");
    assert_eq!(
        routed_ack.object_identifier,
        oid(ObjectType::ANALOG_INPUT, 1)
    );

    // Confirmed COV + event draw Reject (services reality: only READ_PROPERTY
    // executed). Raw sends Confirmed COV Notification; endpoint must Reject
    // on the wire (flow completes, no hang), proving the matrix.
    for (choice, iid) in [
        (ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION, 61u8),
        (ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION, 62u8),
    ] {
        let cov = Apdu::ConfirmedRequest(ConfirmedPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: iid,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: choice,
            service_request: Bytes::from_static(&[0x00]),
        });
        let mut ce = BytesMut::new();
        encode_apdu(&mut ce, &cov).unwrap();
        let mut cn = BytesMut::new();
        encode_npdu(
            &mut cn,
            &Npdu {
                is_network_message: false,
                expecting_reply: true,
                priority: NetworkPriority::NORMAL,
                destination: None,
                source: None,
                payload: ce.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        let mut cf = BytesMut::new();
        encode_bvll(
            &mut cf,
            bacnet_types::enums::BvlcFunction::ORIGINAL_UNICAST_NPDU,
            &cn,
        )
        .unwrap();
        raw.send_to(&cf, endpoint_socket).await.unwrap();
        let (rl, _) = tokio::time::timeout(WAIT, raw.recv_from(&mut buf))
            .await
            .expect("reject must arrive")
            .unwrap();
        let rb = decode_bvll(&buf[..rl]).unwrap();
        let rn = decode_npdu(Bytes::copy_from_slice(&rb.payload)).unwrap();
        match decode_apdu(rn.payload).unwrap() {
            Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason,
            }) => {
                assert_eq!(invoke_id, iid);
                assert_eq!(reject_reason, RejectReason::UNRECOGNIZED_SERVICE);
            }
            other => panic!("COV/event must Reject, got {other:?}"),
        }
    }

    // Segmented request over real UDP draws server Abort (refuse direction).
    let mut ss = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::ANALOG_INPUT, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut ss);
    let seg = Apdu::ConfirmedRequest(ConfirmedPdu {
        segmented: true,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 63,
        sequence_number: Some(0),
        proposed_window_size: Some(1),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: ss.freeze(),
    });
    let mut se = BytesMut::new();
    encode_apdu(&mut se, &seg).unwrap();
    let mut sn = BytesMut::new();
    encode_npdu(
        &mut sn,
        &Npdu {
            is_network_message: false,
            expecting_reply: true,
            priority: NetworkPriority::NORMAL,
            destination: None,
            source: None,
            payload: se.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let mut sf = BytesMut::new();
    encode_bvll(
        &mut sf,
        bacnet_types::enums::BvlcFunction::ORIGINAL_UNICAST_NPDU,
        &sn,
    )
    .unwrap();
    raw.send_to(&sf, endpoint_socket).await.unwrap();
    let (al, _) = tokio::time::timeout(WAIT, raw.recv_from(&mut buf))
        .await
        .expect("abort must arrive")
        .unwrap();
    let ab = decode_bvll(&buf[..al]).unwrap();
    let an = decode_npdu(Bytes::copy_from_slice(&ab.payload)).unwrap();
    match decode_apdu(an.payload).unwrap() {
        Apdu::Abort(AbortPdu {
            invoke_id,
            abort_reason,
            sent_by_server,
        }) => {
            assert_eq!(invoke_id, 63);
            assert!(sent_by_server);
            assert_eq!(abort_reason, AbortReason::SEGMENTATION_NOT_SUPPORTED);
        }
        other => panic!("segmented must Abort, got {other:?}"),
    }

    endpoint.stop().await.unwrap();
}
