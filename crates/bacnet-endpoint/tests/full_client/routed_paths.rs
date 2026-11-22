//! A seated client's routed path after a request the device answered on its
//! retry (#2651), at the session's default timing (6000 ms, 3 retries): the
//! path is free at once, and inside what was the 24 s hold a reason 4 for a
//! smaller request is dropped and counted, as on a standalone client. A reject
//! for a frame within the 228-octet floor is dropped too (#2665).
use super::*;
use std::sync::Arc;

use bacnet_client::client::Station;
use bacnet_encoding::apdu::SimpleAck;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_types::enums::ConfirmedServiceChoice;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// A router with no node on the link: what goes to it is read from the
/// node's send log, and its answers are put on the link by the test.
const ROUTER: u8 = 9;
const DNET: u16 = 100;
const DADR: [u8; 1] = [3];

/// The invoke IDs of the Confirmed-Requests the node sent the router.
fn requests_to_router(lab: &Lab) -> Vec<u8> {
    apdus(&lab.node_sent)
        .into_iter()
        .filter(|(mac, _)| *mac == Wire::mac(ROUTER))
        .filter_map(|(_, apdu)| match apdu {
            Apdu::ConfirmedRequest(request) => Some(request.invoke_id),
            _ => None,
        })
        .collect()
}

/// An NPDU from the router: `message_type` with `payload`, or an APDU
/// relayed from [`DADR`] on [`DNET`].
fn from_router(lab: &Lab, message_type: Option<NetworkMessageType>, payload: Bytes) {
    let source = message_type.is_none().then(|| NpduAddress {
        network: DNET,
        mac_address: MacAddr::from_slice(&DADR),
    });
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            message_type,
            source,
            payload,
            ..Npdu::default()
        },
    )
    .unwrap();
    lab.wire.deliver(
        &Wire::mac(NODE),
        wire::frame(&Wire::mac(ROUTER), npdu.freeze(), false),
    );
}

fn ack(lab: &Lab, invoke_id: u8) {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .unwrap();
    from_router(lab, None, apdu.freeze());
}

fn reject(lab: &Lab) {
    from_router(
        lab,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK),
        Bytes::from(vec![
            RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
            (DNET >> 8) as u8,
            DNET as u8,
        ]),
    );
}

fn send(
    client: &Arc<BACnetClient<TestTransport>>,
    octets: usize,
) -> JoinHandle<Result<Bytes, bacnet_types::error::Error>> {
    let client = Arc::clone(client);
    tokio::spawn(async move {
        client
            .confirmed_request(
                &Station::routed(&Wire::mac(ROUTER), DNET, &DADR),
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &vec![0x41; octets],
            )
            .await
    })
}

/// The `n`th request to the router, once it has gone out.
async fn nth_request(lab: &Lab, n: usize, within: Duration) -> u8 {
    eventually(within, "the request to the router", async || {
        requests_to_router(lab).len() > n
    })
    .await;
    requests_to_router(lab)[n]
}

#[tokio::test(start_paused = true)]
async fn a_seated_client_frees_an_answered_path_at_once_and_gates_a_stale_reject() {
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.client().unwrap());
    client
        .configure_routed_path_max_npdu(&Wire::mac(ROUTER), DNET, 1497)
        .unwrap();

    // 300 octets, answered on the retry one timeout after the first frame.
    let answered = send(&client, 300);
    let first = nth_request(&lab, 0, WAIT).await;
    let sent = Instant::now();
    let retry = nth_request(&lab, 1, Duration::from_secs(7)).await;
    assert!(sent.elapsed() >= Duration::from_secs(6));
    assert_eq!(retry, first);
    ack(&lab, retry);
    assert!(answered.await.unwrap().unwrap().is_empty());

    // The next request goes out at once, not 24 s later.
    let smaller = send(&client, 220);
    let next = nth_request(&lab, 2, Duration::from_millis(10)).await;
    assert_ne!(next, retry);

    // A reason 4 now may be for the 300-octet frames: it is dropped.
    reject(&lab);
    eventually(WAIT, "the reject counted", async || {
        client.too_long_reject_drops() == 1
    })
    .await;
    assert!(!smaller.is_finished());
    ack(&lab, next);
    assert!(smaller.await.unwrap().unwrap().is_empty());

    drop(client);
    stop(lab).await;
}

/// A reason 4 for other traffic to the network lands on a seated client's
/// one-frame request (#2665): it is dropped and counted, the request ends
/// on its own answer, and the path learns nothing, so a larger request
/// goes out whole.
#[tokio::test(start_paused = true)]
async fn a_seated_client_drops_a_reject_for_a_frame_below_the_floor() {
    let lab = lab(SessionRole::Both).await;
    let client = Arc::new(lab.session.client().unwrap());
    client
        .configure_routed_path_max_npdu(&Wire::mac(ROUTER), DNET, 1497)
        .unwrap();

    let tiny = send(&client, 1);
    let first = nth_request(&lab, 0, WAIT).await;
    reject(&lab);
    eventually(WAIT, "the reject counted", async || {
        client.below_minimum_reject_drops() == 1
    })
    .await;
    assert_eq!(client.too_long_reject_drops(), 0);
    assert!(!tiny.is_finished());
    ack(&lab, first);
    assert!(tiny.await.unwrap().unwrap().is_empty());

    let large = send(&client, 300);
    let next = nth_request(&lab, 1, Duration::from_millis(10)).await;
    ack(&lab, next);
    assert!(large.await.unwrap().unwrap().is_empty());
    assert_eq!(requests_to_router(&lab).len(), 2, "sent whole, no segments");

    drop(client);
    stop(lab).await;
}
