//! Data attributes on one request's frames (#2723): every frame of a
//! request made through the view carries them (its first, each retry and
//! each segment, to every station form), and nothing else the client sends
//! does. What was sent is read from the shared test transport's send log.
use bacnet_encoding::apdu::{Apdu, SegmentAck, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, encode_npdu};
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendLog, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice, UnconfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use bytes::BytesMut;
use tokio::sync::mpsc;

use super::{BACnetClient, ClientConfig, Station};

const PEER: &[u8] = &[2];
const ROUTER: &[u8] = &[9];
const WRITE: ConfirmedServiceChoice = ConfirmedServiceChoice::WRITE_PROPERTY;

fn attribute(tag: u8) -> DataAttribute {
    DataAttribute {
        option_type: 31,
        must_understand: false,
        data: vec![0x01, 0x02, tag],
    }
}

/// `apdu` from the peer, as the link hands it up.
fn from_peer(apdu: Apdu) -> ReceivedNpdu {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: payload.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(PEER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

/// A client whose largest APDU is 50 octets, so a longer request goes
/// segmented, with `retries` per request. The peer acknowledges every
/// segment it gets, and the whole request once the last one is in; it
/// answers nothing else.
async fn client(retries: u8) -> (BACnetClient<TestTransport>, SendLog) {
    let (inbound_tx, inbound_rx) = mpsc::channel(16);
    let transport = TestTransport::builder()
        .local_mac(&[1])
        .inbound(inbound_rx)
        .on_send(move |frame| {
            let inbound = inbound_tx.clone();
            async move {
                if let Apdu::ConfirmedRequest(request) = frame.apdu()
                    && request.segmented
                {
                    let ack = SegmentAck {
                        negative_ack: false,
                        sent_by_server: true,
                        invoke_id: request.invoke_id,
                        sequence_number: request.sequence_number.unwrap(),
                        actual_window_size: 1,
                    };
                    inbound
                        .send(from_peer(Apdu::SegmentAck(ack)))
                        .await
                        .unwrap();
                    if !request.more_follows {
                        let done = SimpleAck {
                            invoke_id: request.invoke_id,
                            service_choice: request.service_choice,
                        };
                        inbound
                            .send(from_peer(Apdu::SimpleAck(done)))
                            .await
                            .unwrap();
                    }
                }
                Ok(())
            }
        })
        .build();
    let sent = transport.sent();
    let config = ClientConfig {
        apdu_timeout_ms: 20,
        apdu_retries: retries,
        max_apdu_length: 50,
        proposed_window_size: 1,
        ..ClientConfig::default()
    };
    (BACnetClient::start(config, transport).await.unwrap(), sent)
}

fn timed_out<T: std::fmt::Debug>(result: &Result<T, Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Abort { reason }))
        if *reason == AbortReason::TSM_TIMEOUT)
}

/// Every segment of a long request carries the attributes, and so
/// does the first send and the retry of an unanswered one, to a station on
/// this link, through a router and by local broadcast.
#[tokio::test(start_paused = true)]
async fn every_frame_of_a_request_made_with_attributes_carries_them() {
    let (mut client, sent) = client(1).await;
    let secure = client.with_data_attributes(vec![attribute(1)]);
    assert_eq!(secure.attributes(), [attribute(1)]);

    secure
        .confirmed_request(PEER, WRITE, &[0x0C; 100])
        .await
        .unwrap();
    let segments = sent.take();
    assert!(segments.len() >= 3, "{} segments", segments.len());
    for frame in &segments {
        let Apdu::ConfirmedRequest(request) = frame.apdu() else {
            panic!("a segment");
        };
        assert!(request.segmented);
        assert_eq!(frame.attributes, [attribute(1)]);
    }

    for to in [
        Station::local(PEER),
        Station::routed(ROUTER, 5, &[7]),
        Station::routed_via_local_broadcast(5, &[7]),
    ] {
        let result = secure.confirmed_request(&to, WRITE, &[0x0C]).await;
        assert!(timed_out(&result), "{to:?}: {result:?}");
        let frames = sent.take();
        assert_eq!(frames.len(), 2, "{to:?}: the request and its retry");
        for frame in &frames {
            assert_eq!(frame.attributes, [attribute(1)], "{to:?}");
        }
    }
    client.stop().await.unwrap();
}

/// A request made without the view carries none, even while another made
/// with it is open; each carries its own, through a typed call too; and
/// unconfirmed requests carry none.
#[tokio::test(start_paused = true)]
async fn only_the_views_requests_carry_its_attributes() {
    let (mut client, sent) = client(0).await;
    let one = client.with_data_attributes(vec![attribute(1)]);
    let two = client.with_data_attributes(vec![attribute(2), attribute(3)]);
    let object = bacnet_types::primitives::ObjectIdentifier::new(
        bacnet_types::enums::ObjectType::ANALOG_INPUT,
        1,
    )
    .unwrap();
    let property = bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE;
    let (first, second, plain) = tokio::join!(
        one.confirmed_request(&[3], WRITE, &[0x0C]),
        two.read_property(&[4], object, property, None),
        client.confirmed_request(&[5], WRITE, &[0x0C]),
    );
    assert!(timed_out(&first) && timed_out(&second) && timed_out(&plain));
    let by_mac = |mac: u8| {
        sent.frames()
            .into_iter()
            .find(|frame| frame.mac.as_slice() == [mac])
            .unwrap()
            .attributes
    };
    assert_eq!(by_mac(3), [attribute(1)]);
    assert_eq!(by_mac(4), [attribute(2), attribute(3)]);
    assert!(by_mac(5).is_empty());

    sent.clear();
    client
        .unconfirmed_request(&[6], UnconfirmedServiceChoice::WHO_IS, &[])
        .await
        .unwrap();
    assert!(sent.frame(0).attributes.is_empty());
    client.stop().await.unwrap();
}
