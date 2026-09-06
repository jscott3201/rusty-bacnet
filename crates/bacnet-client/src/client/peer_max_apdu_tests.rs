//! What the client does with a peer that advertises an impossible APDU size.
//!
//! I-Am carries `Max APDU Length Accepted` as an Unsigned octet count rather
//! than the four-bit code of Clause 20.1.2.5, so a peer can put any value on
//! the wire — including values below the 50-octet MinimumMessageSize that
//! Clause 12.11.18 and Clause 5.2.1.2(c) both require.
//!
//! Split from `tests.rs`, which is at the 700-LOC cap.

use std::time::Instant;

use bacnet_encoding::apdu::{self, encode_apdu, Apdu, SimpleAck};
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::enums::{ConfirmedServiceChoice, NetworkPriority, ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;
use tokio::time::{timeout, Duration};

use crate::discovery::DiscoveredDevice;

use super::BACnetClient;

/// Build a client whose device table holds one peer advertising
/// `max_apdu_length`, plus the remote end of the loopback pair.
async fn client_with_peer_advertising(
    max_apdu_length: u32,
) -> (
    BACnetClient<LoopbackTransport>,
    Vec<u8>,
    NetworkLayer<LoopbackTransport>,
    tokio::sync::mpsc::Receiver<ReceivedApdu>,
) {
    let client_mac = vec![0x01];
    let remote_mac = vec![0x02];
    let (client_transport, remote_transport) =
        LoopbackTransport::pair(client_mac, remote_mac.clone());
    let mut remote_network = NetworkLayer::new(remote_transport);
    let remote_rx = remote_network.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();

    client.device_table.lock().await.upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2003).unwrap(),
        mac_address: MacAddr::from_slice(&remote_mac),
        max_apdu_length,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: None,
        source_address: None,
    });

    (client, remote_mac, remote_network, remote_rx)
}

/// The defect: an advertised limit too small to hold even the four-octet
/// unsegmented APDU put a *larger* frame on the wire.
///
/// A four-octet APDU looks too large for a limit of 0..3, so the client fell
/// through to segmentation, where `max_segment_payload` yields a capacity of
/// zero and `split_payload` returned one empty segment anyway — producing a
/// six-octet segmented Confirmed-Request, larger still than the limit that
/// triggered segmentation in the first place. Against `dev` these values put
/// `[0a 05 00 00 01 0c]` on the wire.
///
/// The no-frame assertion comes first deliberately: asserting on the error text
/// first would make the test fail on wording rather than on the oversize frame
/// it exists to catch.
#[tokio::test]
async fn peer_max_apdu_too_small_for_an_unsegmented_apdu_sends_nothing() {
    for advertised in [0u32, 1, 2, 3] {
        let (mut client, remote_mac, mut remote_network, mut remote_rx) =
            client_with_peer_advertising(advertised).await;

        let result = client
            .confirmed_request(&remote_mac, ConfirmedServiceChoice::READ_PROPERTY, &[])
            .await;

        assert!(
            timeout(Duration::from_millis(50), remote_rx.recv())
                .await
                .is_err(),
            "advertised max APDU {advertised} must produce no outbound frame"
        );
        let err = result.expect_err(&format!(
            "advertised max APDU {advertised} is below MinimumMessageSize and must be refused"
        ));
        assert!(
            err.to_string().contains("MinimumMessageSize"),
            "error should name MinimumMessageSize, got: {err}"
        );

        remote_network.stop().await.unwrap();
        client.stop().await.unwrap();
    }
}

/// A deliberate behavior change, not the defect.
///
/// 4 through 49 are large enough to carry the four-octet unsegmented APDU, so
/// they never emitted an oversize frame — `dev` sent a conforming
/// `[02 05 00 0c]`. They are refused now because Clause 12.11.18 and Clause
/// 5.2.1.2(c) both put the floor at 50, so the peer is advertising a value it
/// is not permitted to advertise and its real capacity is unknowable.
#[tokio::test]
async fn peer_max_apdu_below_the_floor_but_above_the_apdu_size_is_also_refused() {
    for advertised in [4u32, 6, 49] {
        let (mut client, remote_mac, mut remote_network, mut remote_rx) =
            client_with_peer_advertising(advertised).await;

        let result = client
            .confirmed_request(&remote_mac, ConfirmedServiceChoice::READ_PROPERTY, &[])
            .await;

        assert!(
            timeout(Duration::from_millis(50), remote_rx.recv())
                .await
                .is_err(),
            "advertised max APDU {advertised} must produce no outbound frame"
        );
        assert!(
            result.is_err(),
            "advertised max APDU {advertised} is below the 50-octet floor"
        );

        remote_network.stop().await.unwrap();
        client.stop().await.unwrap();
    }
}

/// A non-empty payload is refused too.
///
/// This one was never broken: `split_payload` already rejected a non-empty
/// payload at zero capacity, which is exactly why the empty-payload shortcut
/// above it was the hole. The test pins that the new floor check refuses the
/// same case *earlier* — before segmentation is attempted at all — rather than
/// changing whether it is refused.
#[tokio::test]
async fn nonempty_request_to_peer_below_minimum_message_size_sends_nothing() {
    let (mut client, remote_mac, mut remote_network, mut remote_rx) =
        client_with_peer_advertising(3).await;

    let result = client
        .confirmed_request(
            &remote_mac,
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x01, 0x02, 0x03],
        )
        .await;

    assert!(
        result.is_err(),
        "a non-conformant limit refuses any payload"
    );
    assert!(
        timeout(Duration::from_millis(50), remote_rx.recv())
            .await
            .is_err(),
        "no outbound frame"
    );

    remote_network.stop().await.unwrap();
    client.stop().await.unwrap();
}

/// The boundary: exactly MinimumMessageSize still transacts, unsegmented.
///
/// Pairs with the refusal tests deliberately — a guard stuck permanently
/// closed would satisfy those alone.
#[tokio::test]
async fn confirmed_request_at_minimum_message_size_is_sent_unsegmented() {
    let (mut client, remote_mac, mut remote_network, mut remote_rx) =
        client_with_peer_advertising(50).await;

    let responder = tokio::spawn(async move {
        let received = timeout(Duration::from_secs(1), remote_rx.recv())
            .await
            .expect("remote timed out")
            .expect("remote channel closed");
        let Apdu::ConfirmedRequest(req) = apdu::decode_apdu(received.apdu).unwrap() else {
            panic!("expected ConfirmedRequest");
        };
        assert!(
            !req.segmented,
            "an empty request fits MinimumMessageSize unsegmented"
        );

        let ack = Apdu::SimpleAck(SimpleAck {
            invoke_id: req.invoke_id,
            service_choice: req.service_choice,
        });
        let mut buf = BytesMut::new();
        encode_apdu(&mut buf, &ack).unwrap();
        remote_network
            .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
            .await
            .unwrap();
        remote_network.stop().await.unwrap();
    });

    let result = client
        .confirmed_request(&remote_mac, ConfirmedServiceChoice::READ_PROPERTY, &[])
        .await;

    assert!(
        result.expect("50 octets is a conformant limit").is_empty(),
        "SimpleAck carries no service data"
    );
    responder.await.unwrap();
    client.stop().await.unwrap();
}
