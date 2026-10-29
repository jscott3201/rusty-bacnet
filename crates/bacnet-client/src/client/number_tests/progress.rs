use super::*;
use bacnet_encoding::{
    apdu::{Apdu, decode_apdu},
    npdu::decode_npdu,
};
use bacnet_types::{enums::ConfirmedServiceChoice, error::Error};
use std::sync::Arc;

#[tokio::test]
async fn client_number_held_egress_preserves_routed_reject_and_apdu_dispatch() {
    let (client, inbound, mut outbound, gates) = harness(true).await;
    let client = Arc::new(client);
    inject(&inbound, &number(77, 1), true).await;
    inject(&inbound, QUERY, true).await;
    bounded(gates.entered.acquire()).await.unwrap().forget();
    client
        .configure_routed_path_max_npdu(&[2], 100, 1497)
        .await
        .unwrap();
    let routed = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .confirmed_request_routed(
                    &[2],
                    100,
                    &[3],
                    ConfirmedServiceChoice::WRITE_PROPERTY,
                    &[0x44; 300],
                )
                .await
        })
    };
    let sent = bounded(outbound.recv()).await.unwrap();
    assert_eq!(sent.destination.as_slice(), &[2]);
    let npdu = decode_npdu(sent.npdu).unwrap();
    let Apdu::ConfirmedRequest(req) = decode_apdu(npdu.payload).unwrap() else {
        panic!("routed request")
    };
    assert!(!req.segmented);
    inject(&inbound, &[1, 0x80, 3, 4, 0, 100], false).await;
    assert!(matches!(
        bounded(routed).await.unwrap(),
        Err(Error::RoutedPathTooLong { dnet: 100 })
    ));
    // The learned limit now segments the same payload. This observes the real
    // Reject correlation result rather than calling the path-limit handler.
    let retry = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .confirmed_request_routed(
                    &[2],
                    100,
                    &[3],
                    ConfirmedServiceChoice::WRITE_PROPERTY,
                    &[0x44; 300],
                )
                .await
        })
    };
    let sent = bounded(outbound.recv()).await.unwrap();
    let npdu = decode_npdu(sent.npdu).unwrap();
    let Apdu::ConfirmedRequest(req) = decode_apdu(npdu.payload).unwrap() else {
        panic!("bounded routed request")
    };
    assert!(req.segmented);
    retry.abort();
    let _ = retry.await;
    let direct = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .confirmed_request(&[2], ConfirmedServiceChoice::WRITE_PROPERTY, &[0])
                .await
        })
    };
    let sent = bounded(outbound.recv()).await.unwrap();
    let npdu = decode_npdu(sent.npdu).unwrap();
    let Apdu::ConfirmedRequest(req) = decode_apdu(npdu.payload).unwrap() else {
        panic!("direct request")
    };
    inject(&inbound, &[1, 0, 0x20, req.invoke_id, 15], false).await;
    assert!(bounded(direct).await.unwrap().unwrap().is_empty());
    assert_eq!(
        gates.dropped.available_permits(),
        0,
        "Number send remains held during both results"
    );
    gates.release.add_permits(1);
    reply(&mut outbound, 77).await;
    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}
