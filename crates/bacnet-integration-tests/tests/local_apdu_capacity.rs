//! Raw local declaration is distinct from the ConfirmedRequest header bucket.
use bacnet_encoding::{
    apdu::{Apdu, decode_apdu},
    npdu::decode_npdu,
};
use bacnet_objects::{
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::{loopback::LoopbackTransport, port::TransportPort};
use std::time::Duration;

#[tokio::test]
async fn raw_1474_device_and_server_declaration_emit_iam_1474() {
    let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut incoming = peer.start().await.unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(893, 0)
                .expect("valid Device identity")
                .with_max_apdu(1474)
                .expect("valid Device APDU"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut server = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(893, 0)
                .unwrap()
                .with_max_apdu(1474)
                .expect("valid Device APDU"),
            ..Default::default()
        },
        db,
        transport,
    )
    .await
    .expect("raw Device/I-Am1474 is a valid local declaration");
    server.broadcast_i_am().await.unwrap();
    let received = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    let npdu = decode_npdu(received.npdu).unwrap();
    let Apdu::UnconfirmedRequest(apdu) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected I-Am")
    };
    assert_eq!(apdu.service_choice.to_raw(), 0);
    assert_eq!(
        IAmRequest::decode(&apdu.service_request)
            .unwrap()
            .max_apdu_length,
        1474
    );
    server.stop().await.unwrap();
}

use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SendMode, TestTransport};
use bacnet_types::{enums::ObjectType, primitives::ObjectIdentifier};
use std::{
    future::{Future, poll_fn},
    sync::Arc,
    task::Poll,
};
use tokio::sync::mpsc;

/// A deliberately asymmetric link: changing remote egress never changes
/// intake. Its receive capacity is fixed at `local`, and its egress limit
/// (206 at first) is set through the transport's handle.
fn capacity_port(inner: LoopbackTransport, local: u16) -> TestTransport {
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .receive_capacity(local)
        .egress_limit(206)
        .build()
}
fn device(raw: u32) -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(893, 0)
            .expect("valid Device identity")
            .with_max_apdu(u16::try_from(raw).expect("Device APDU fits u16"))
            .expect("valid Device APDU"),
        ..Default::default()
    })
    .unwrap()
}
fn database(raw: Option<u32>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    if let Some(raw) = raw {
        db.add(Box::new(device(raw))).unwrap();
    }
    db
}
fn port(local: u16) -> (TestTransport, LoopbackTransport) {
    let (inner, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    (capacity_port(inner, local), peer)
}
async fn iam(incoming: &mut mpsc::Receiver<ReceivedNpdu>) -> IAmRequest {
    let bytes = tokio::time::timeout(Duration::from_secs(2), incoming.recv())
        .await
        .unwrap()
        .unwrap();
    let Apdu::UnconfirmedRequest(req) =
        decode_apdu(decode_npdu(bytes.npdu).unwrap().payload).unwrap()
    else {
        panic!("expected I-Am")
    };
    assert_eq!(req.service_choice.to_raw(), 0);
    IAmRequest::decode(&req.service_request).unwrap()
}

#[tokio::test]
async fn local_capacity_clamps_ceiling_and_never_tracks_remote_egress() {
    for (local, ceiling, effective) in [
        (480, 1476, 480),
        (1476, u32::from(u16::MAX), 1476),
        (1476, 50, 50),
        (1476, 127, 127),
    ] {
        let (transport, mut peer) = port(local);
        let remote = transport.handle();
        let mut incoming = peer.start().await.unwrap();
        let mut server = BACnetServer::start(
            ServerConfig {
                device: bacnet_objects::DeviceIdentity::new(893, 0)
                    .unwrap()
                    .with_max_apdu(u16::try_from(ceiling).expect("Device APDU fits u16"))
                    .expect("valid Device APDU"),
                ..Default::default()
            },
            database(Some(effective)),
            transport,
        )
        .await
        .unwrap();
        for outgoing in [50, 480, 1476] {
            remote.set_egress_limit(outgoing);
            server.broadcast_i_am().await.unwrap();
            assert_eq!(iam(&mut incoming).await.max_apdu_length, effective);
        }
        server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_raw_and_mismatched_device_fail_before_transport_start() {
    assert!(
        bacnet_objects::DeviceIdentity::default()
            .with_max_apdu(49)
            .is_err()
    );
    for (local, ceiling, declared) in [(49, 1476, 1476), (480, 1476, 1476), (1476, 1474, 1476)] {
        let (transport, _peer) = port(local);
        let starts = transport.handle();
        assert!(
            BACnetServer::start(
                ServerConfig {
                    device: bacnet_objects::DeviceIdentity::new(893, 0)
                        .unwrap()
                        .with_max_apdu(u16::try_from(ceiling).expect("Device APDU fits u16"))
                        .expect("valid Device APDU"),
                    ..Default::default()
                },
                database(Some(declared)),
                transport
            )
            .await
            .is_err()
        );
        assert_eq!(starts.starts(), 0);
    }
}

#[tokio::test]
async fn queued_iam_projects_replacement_and_checks_membership_before_accounting() {
    let (transport, mut peer) = port(1476);
    let mut incoming = peer.start().await.unwrap();
    let mut server = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(893, 0)
                .unwrap()
                .with_max_apdu(1474)
                .expect("valid Device APDU"),
            ..Default::default()
        },
        database(Some(1474)),
        transport,
    )
    .await
    .unwrap();
    let db = Arc::clone(server.database());
    let oid = ObjectIdentifier::new(ObjectType::DEVICE, 893).unwrap();
    let mut guard = db.write().await;
    let mut send = Box::pin(server.broadcast_i_am());
    // Poll through synchronous queue admission while the worker cannot read DB.
    assert!(
        poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    guard.remove(&oid).unwrap();
    guard.add(Box::new(device(1476))).unwrap();
    drop(guard);
    send.await.unwrap();
    assert_eq!(iam(&mut incoming).await.max_apdu_length, 1474);
    assert_eq!(server.discovery_counters().i_am_sent, 1);
    assert!(server.discovery_counters().response_bytes_sent > 0);
    assert_eq!(
        server
            .read_local(
                &oid,
                bacnet_types::enums::PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED,
                None
            )
            .await
            .unwrap(),
        bacnet_types::primitives::PropertyValue::Unsigned(1474)
    );
    {
        let mut db = db.write().await;
        db.remove(&oid).unwrap();
        db.add(Box::new(device(1474))).unwrap();
    }
    server.broadcast_i_am().await.unwrap();
    assert_eq!(iam(&mut incoming).await.max_apdu_length, 1474);
    assert_eq!(server.discovery_counters().i_am_sent, 2);
    let before = server.discovery_counters().response_bytes_sent;
    db.write().await.remove(&oid).unwrap();
    assert!(server.broadcast_i_am().await.is_err());
    assert!(incoming.try_recv().is_err());
    assert_eq!(server.discovery_counters().i_am_sent, 2);
    assert_eq!(server.discovery_counters().response_bytes_sent, before);
    server.stop().await.unwrap();
}

#[tokio::test]
async fn no_device_starts_without_iam_and_matching_later_device_recovers() {
    let (transport, mut peer) = port(1476);
    let mut incoming = peer.start().await.unwrap();
    let mut server = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(893, 0).unwrap(),
            ..Default::default()
        },
        database(None),
        transport,
    )
    .await
    .unwrap();
    assert!(server.broadcast_i_am().await.is_err());
    assert!(incoming.try_recv().is_err());
    server
        .database()
        .write()
        .await
        .add(Box::new(device(1476)))
        .unwrap();
    server.broadcast_i_am().await.unwrap();
    assert_eq!(iam(&mut incoming).await.max_apdu_length, 1476);
    server.stop().await.unwrap();
}

#[path = "local_apdu_capacity/sc.rs"]
mod sc;
