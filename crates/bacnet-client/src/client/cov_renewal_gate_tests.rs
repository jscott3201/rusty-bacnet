//! A managed subscription's renewal gate (#1791): renewals wait for a
//! permit and hold it until their answer, the first request goes without
//! one, and a stop while a renewal waits leaves the gate as it was.

use super::*;
use bacnet_encoding::apdu::{SimpleAck, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use tokio::sync::Semaphore;

const CLIENT: [u8; 1] = [0x31];
const PEER: [u8; 1] = [0x32];

/// The invoke ID of the next SubscribeCOV the peer receives.
async fn subscribe(rx: &mut mpsc::Receiver<ReceivedNpdu>) -> u8 {
    let received = rx.recv().await.expect("peer channel closed");
    let npdu = decode_npdu(received.npdu).unwrap();
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected a confirmed request");
    };
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::SUBSCRIBE_COV
    );
    request.invoke_id
}

async fn ack(peer: &LoopbackTransport, invoke_id: u8) {
    let mut apdu = BytesMut::new();
    let ack = Apdu::SimpleAck(SimpleAck {
        invoke_id,
        service_choice: ConfirmedServiceChoice::SUBSCRIBE_COV,
    });
    encode_apdu(&mut apdu, &ack).unwrap();
    let npdu = Npdu {
        payload: apdu.freeze(),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    peer.send_unicast(&buf, &CLIENT).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn renewals_wait_for_and_hold_the_gate() {
    let (client_transport, mut peer) = LoopbackTransport::pair(CLIENT.to_vec(), PEER.to_vec());
    let mut rx = peer.start().await.unwrap();
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(client_transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );
    let gate = Arc::new(Semaphore::new(1));
    // The caller holds the gate for the first request, which goes anyway.
    let held = Arc::clone(&gate).acquire_owned().await.unwrap();
    let start = tokio::spawn({
        let client = Arc::clone(&client);
        let gate = Arc::clone(&gate);
        async move {
            client
                .manage_cov_subscription(
                    CovTarget::local(&PEER),
                    7,
                    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
                    false,
                    20,
                    ManagedCOVSubscriptionOptions::default()
                        .with_renewal_margin(Duration::from_secs(15))
                        .with_renewal_gate(gate),
                )
                .await
        }
    });
    let first = subscribe(&mut rx).await;
    ack(&peer, first).await;
    let managed = start.await.unwrap().unwrap();
    let mut events = managed.events();

    // The renewal falls due after 5 s but waits while the gate is held.
    assert!(timeout(Duration::from_secs(30), rx.recv()).await.is_err());
    drop(held);
    let renewal = subscribe(&mut rx).await;
    assert_eq!(gate.available_permits(), 0, "the renewal holds the gate");
    ack(&peer, renewal).await;
    assert!(matches!(
        events.recv().await.unwrap(),
        ManagedCOVSubscriptionEvent::Renewed { .. }
    ));
    assert_eq!(gate.available_permits(), 1, "and lets go at its answer");

    // A stop while the next renewal waits for the gate ends it at once and
    // sends nothing.
    let held = Arc::clone(&gate).acquire_owned().await.unwrap();
    tokio::time::sleep(Duration::from_secs(10)).await;
    timeout(Duration::from_millis(10), managed.stop())
        .await
        .expect("a stop doesn't wait for the gate");
    drop(held);
    assert_eq!(gate.available_permits(), 1);
    assert!(rx.try_recv().is_err());

    let Ok(mut client) = Arc::try_unwrap(client) else {
        panic!("managed subscription kept a client reference after stop");
    };
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
