//! Hold genuine outbound admission before network dispatch, then replace it.
use super::*;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::primitives::PropertyValue;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn outbound_admitted_write_keeps_identity_after_replacement_without_reply_retarget() {
    let ca = TestCa::new();
    let (mut peer, mut port) = Peer::start(&ca).await;
    let mut original = port.rx.take().unwrap();
    let (forward, rx) = mpsc::channel(4);
    port.rx = Some(rx);
    let (observed, mut observations) = mpsc::channel(2);
    let (release_a, wait_a) = tokio::sync::oneshot::channel();
    let (release_b, wait_b) = tokio::sync::oneshot::channel();
    // No synthetic provenance or route: only the scheduling boundary between
    // real transport admission and the unchanged NetworkLayer is controlled.
    let bridge = tokio::spawn(async move {
        let a = original.recv().await.unwrap();
        observed
            .send(a.provenance.direct_sc_identity().unwrap())
            .await
            .unwrap();
        let b = original.recv().await.unwrap();
        observed
            .send(b.provenance.direct_sc_identity().unwrap())
            .await
            .unwrap();
        wait_a.await.unwrap();
        forward.send(a).await.unwrap();
        wait_b.await.unwrap();
        forward.send(b).await.unwrap();
        while let Some(next) = original.recv().await {
            if forward.send(next).await.is_err() {
                break;
            }
        }
    });
    let authorized = Arc::new(Mutex::new(Vec::new()));
    let seen = authorized.clone();
    let config = bacnet_server::server::ServerConfig {
        policy: Some(Arc::new(bacnet_server::FnPolicy::new().mutation(
            move |context| {
                seen.lock()
                    .unwrap()
                    .push(context.direct_sc_identity().unwrap());
                true
            },
        ))),
        ..Default::default()
    };
    let mut server = bacnet_server::server::BACnetServer::start_clockless(
        config,
        database("held-outbound"),
        port,
    )
    .await
    .unwrap();
    let mut data = BytesMut::new();
    WritePropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: vec![0x72, 0, b'A'],
        priority: None,
    }
    .encode(&mut data)
    .unwrap();
    peer.send(&confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        90,
        data.freeze(),
    ))
    .await;
    let a = bounded(observations.recv()).await.unwrap();
    let b = bacnet_transport::sc_tls::TlsWebSocket::connect_direct(
        &format!(
            "wss://localhost:{}/.bacnet/sc",
            peer.local.local_addr().port()
        ),
        ca.tls("replacement"),
    )
    .await
    .unwrap();
    let mut connection = bacnet_transport::sc::ScConnection::new(REMOTE, [2; 16]);
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &connection.build_connect_request());
    b.send(&bytes).await.unwrap();
    assert_eq!(
        decode_sc_message(&b.recv().await.unwrap())
            .unwrap()
            .function,
        ScFunction::ConnectAccept
    );
    bytes.clear();
    encode_sc_message(
        &mut bytes,
        &connection
            .build_direct_encapsulated_npdu(&wire(&read_name(91)), &[])
            .unwrap(),
    );
    b.send(&bytes).await.unwrap();
    let identity_b = bounded(observations.recv()).await.unwrap();
    assert_ne!(a.leaf_sha256(), identity_b.leaf_sha256());
    assert_ne!(a.incarnation(), identity_b.incarnation());
    release_a.send(()).unwrap();
    bounded(async {
        loop {
            if server
                .read_local(&csv_oid(), PropertyIdentifier::DESCRIPTION, None)
                .await
                .unwrap()
                == PropertyValue::CharacterString("A".into())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(*authorized.lock().unwrap(), vec![a]);
    release_b.send(()).unwrap();
    let reply = bounded(async {
        tokio::select! {
            direct=b.recv()=>direct.unwrap(),
            hub=peer.hub.recv()=>panic!("retired request escaped through Hub: {hub:?}"),
        }
    })
    .await;
    let reply = decode_apdu(
        decode_npdu(decode_sc_message(&reply).unwrap().payload)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert!(
        matches!(reply,Apdu::ComplexAck(ack) if ack.invoke_id==91),
        "B's first application reply must belong to B"
    );
    assert!(futures_util::poll!(Box::pin(peer.hub.recv()).as_mut()).is_pending());
    server.stop().await.unwrap();
    bridge.abort();
    let _ = bridge.await;
    peer.stop().await;
}
