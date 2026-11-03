//! Which path the answer to a segmented Confirmed-Request takes through the
//! router's application port (#2341). Segment 0 comes through one immediate
//! router (P1) and the rest, the final segment included, through another
//! (P2), for the same SNET/SADR: the requester may change its next hop
//! mid-request (`RouterResponse::same_origin`). The server answers a
//! reassembled request through segment 0's route, as it does on its own
//! links, so the terminal answer must leave through P1 (#2355).

use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::super::application_replies::CAPACITY;
use super::super::priority_harness::*;

/// The immediate routers on network 2000 that relay station 5's requests.
const P1: u8 = 7;
const P2: u8 = 8;

/// The transaction under test.
const INVOKE: u8 = 9;

/// A WriteProperty APDU from station 5 on network 3000, addressed to this
/// router's address on network 1000. `segment` is `None` for an
/// unsegmented request (SEG clear), else its sequence number and whether
/// more follow.
fn request(invoke_id: u8, segment: Option<(u8, bool)>) -> Vec<u8> {
    let npci = [1, 0x2C, 0x03, 0xE8, 1, 1, 0x0B, 0xB8, 1, 5, 255];
    let apdu = match segment {
        None => vec![0x02, 0x05, invoke_id, 0x0F, 0x09, 0],
        Some((sequence, more_follows)) => {
            let header = 0x0A | if more_follows { 0x04 } else { 0 };
            vec![header, 0x05, invoke_id, sequence, 1, 0x0F, 0x09, sequence]
        }
    };
    [&npci[..], &apdu].concat()
}

/// A router with its application port started, and its peers.
struct Fixture {
    router: super::super::BACnetRouter,
    port: super::RouterApplicationPort,
    frames: tokio::sync::mpsc::Receiver<ReceivedNpdu>,
    peers: Vec<Peer>,
}

impl Fixture {
    async fn new() -> Self {
        let (router, apdus, peers) = start(2).await;
        let mut port = router.application_port(apdus);
        let frames = port.start().await.unwrap();
        Self {
            router,
            port,
            frames,
            peers,
        }
    }

    /// `request(invoke_id, segment)` on network 2000, relayed by the router
    /// at `via`, delivered to the stack.
    async fn receive(&mut self, invoke_id: u8, segment: Option<(u8, bool)>, via: u8) {
        self.peers[1]
            .frames
            .send(ReceivedNpdu::unverified(
                Bytes::from(request(invoke_id, segment)),
                MacAddr::from_slice(&[via]),
                false,
                Vec::new(),
                None,
            ))
            .await
            .unwrap();
        self.frames.recv().await.unwrap();
    }

    /// The router on network 2000 the stack's `apdu` to station 5 on
    /// network 3000 leaves through.
    async fn answer(&mut self, apdu: &[u8]) -> Option<Vec<u8>> {
        let npdu = [&[1, 0x20, 0x0B, 0xB8, 1, 5, 255][..], apdu].concat();
        let (to, answered) = tokio::join!(
            async {
                let sent = sending(&mut self.peers[1]).await;
                sent.complete.send(Ok(())).unwrap();
                sent.to
            },
            // The link address is the plan's to choose, not the stack's.
            self.port.send_unicast(&npdu, &[])
        );
        answered.unwrap();
        to
    }

    /// Fill the reply table to capacity with other transactions, invoke IDs
    /// from `first` on, through P1.
    async fn others(&mut self, first: u8, count: usize) {
        for invoke_id in (first..).take(count) {
            self.receive(invoke_id, None, P1).await;
        }
    }

    async fn stop(mut self) {
        self.router.stop().await.unwrap();
        self.port.stop().await.unwrap();
    }
}

/// The stack's SimpleAck for [`INVOKE`].
const SIMPLE_ACK: [u8; 3] = [0x20, INVOKE, 0x0F];

/// The router the SimpleAck leaves through after `requests`, each a
/// segment (sequence number, more to follow) or `None` for an unsegmented
/// request, and the relaying router.
async fn answered_via(requests: &[(Option<(u8, bool)>, u8)]) -> Option<Vec<u8>> {
    let mut f = Fixture::new().await;
    for &(segment, via) in requests {
        f.receive(INVOKE, segment, via).await;
    }
    let to = f.answer(&SIMPLE_ACK).await;
    f.stop().await;
    to
}

#[tokio::test(start_paused = true)]
async fn a_reassembled_request_is_answered_through_segment_zeros_router() {
    // The paths are distinguishable: an unsegmented request through P2 is
    // answered through P2.
    assert_eq!(answered_via(&[(None, P2)]).await, Some(vec![P2]));
    // An unsegmented request repeated by another path is answered there.
    assert_eq!(
        answered_via(&[(None, P1), (None, P2)]).await,
        Some(vec![P2]),
        "a repeated unsegmented request replaces the plan"
    );
    // Segment 0 through P1, the final segment through P2.
    assert_eq!(
        answered_via(&[(Some((0, true)), P1), (Some((1, false)), P2)]).await,
        Some(vec![P1]),
        "the terminal answer follows segment 0's path"
    );
    // A retransmitted segment 0 through P2 is a retry by another path: the
    // answer goes there, whichever way the rest came.
    assert_eq!(
        answered_via(&[
            (Some((0, true)), P1),
            (Some((0, true)), P2),
            (Some((1, false)), P1),
        ])
        .await,
        Some(vec![P2]),
        "a repeated segment 0 replaces the plan"
    );
}

/// A Segment-ACK the server sends mid-request goes through segment 0's
/// path too, and keeps the plan for the answer.
#[tokio::test(start_paused = true)]
async fn a_segment_ack_mid_request_uses_segment_zeros_router() {
    let mut f = Fixture::new().await;
    f.receive(INVOKE, Some((0, true)), P1).await;
    f.receive(INVOKE, Some((1, true)), P2).await;
    // Segment-ACK as the server: sequence 1, window 1.
    assert_eq!(f.answer(&[0x41, INVOKE, 1, 1]).await, Some(vec![P1]));
    f.receive(INVOKE, Some((2, false)), P2).await;
    assert_eq!(f.answer(&SIMPLE_ACK).await, Some(vec![P1]));
    f.stop().await;
}

/// A transfer still running is not the first evicted from a full table:
/// renewing its plan, by a later segment or by the server's Segment-ACK,
/// moves it behind the requests that arrived since.
#[tokio::test(start_paused = true)]
async fn a_running_transfer_outlives_newer_requests_in_a_full_table() {
    // Renewed by the final segment.
    let mut f = Fixture::new().await;
    f.receive(INVOKE, Some((0, true)), P1).await;
    f.others(10, CAPACITY - 1).await;
    f.receive(INVOKE, Some((1, false)), P2).await;
    f.receive(200, None, P1).await;
    assert_eq!(f.answer(&SIMPLE_ACK).await, Some(vec![P1]));
    f.stop().await;

    // Renewed by the server's Segment-ACK.
    let mut f = Fixture::new().await;
    f.receive(INVOKE, Some((0, true)), P1).await;
    f.others(10, CAPACITY - 1).await;
    assert_eq!(f.answer(&[0x41, INVOKE, 0, 1]).await, Some(vec![P1]));
    f.receive(200, None, P1).await;
    assert_eq!(f.answer(&SIMPLE_ACK).await, Some(vec![P1]));
    f.stop().await;
}
