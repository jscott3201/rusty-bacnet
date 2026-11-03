//! Which path the answer to a segmented Confirmed-Request takes through the
//! router's application port (#2341). Segment 0 comes through one immediate
//! router (P1) and the rest, the final segment included, through another
//! (P2), for the same SNET/SADR: the requester may change its next hop
//! mid-request (`RouterResponse::same_origin`). The server answers a
//! reassembled request through segment 0's route, as it does on its own
//! links, so the terminal answer must leave through P1.

use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::super::priority_harness::*;

/// The immediate routers on network 2000 that relay station 5's segments.
const P1: u8 = 7;
const P2: u8 = 8;

/// A segment of WriteProperty invoke ID 9 from station 5 on network 3000,
/// addressed to this router's address on network 1000.
fn segment(sequence: u8, more_follows: bool) -> Vec<u8> {
    let header = 0x0A | if more_follows { 0x04 } else { 0 };
    vec![
        1, 0x2C, 0x03, 0xE8, 1, 1, 0x0B, 0xB8, 1, 5, 255, header, 0x05, 9, sequence, 1, 0x0F, 0x09,
        sequence,
    ]
}

/// `npdu` on `peer`'s link, relayed by the router at `via`.
async fn relayed(peer: &Peer, npdu: Vec<u8>, via: u8) {
    peer.frames
        .send(ReceivedNpdu::unverified(
            Bytes::from(npdu),
            MacAddr::from_slice(&[via]),
            false,
            Vec::new(),
            None,
        ))
        .await
        .unwrap();
}

/// A router with its application port started, and network 2000's peer.
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

    /// Segment `sequence` through the router at `via`, delivered to the stack.
    async fn segment(&mut self, sequence: u8, more_follows: bool, via: u8) {
        relayed(&self.peers[1], segment(sequence, more_follows), via).await;
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

    async fn stop(mut self) {
        self.router.stop().await.unwrap();
        self.port.stop().await.unwrap();
    }
}

/// The stack's SimpleAck for invoke ID 9.
const SIMPLE_ACK: [u8; 3] = [0x20, 9, 0x0F];

/// The router the SimpleAck leaves through after `segments`, each a
/// (sequence number, relaying router); the last has no more to follow.
async fn answered_via(segments: &[(u8, u8)]) -> Option<Vec<u8>> {
    let mut f = Fixture::new().await;
    for (index, &(sequence, via)) in segments.iter().enumerate() {
        f.segment(sequence, index + 1 < segments.len(), via).await;
    }
    let to = f.answer(&SIMPLE_ACK).await;
    f.stop().await;
    to
}

#[tokio::test(start_paused = true)]
async fn a_reassembled_request_is_answered_through_segment_zeros_router() {
    // The paths are distinguishable: an unsegmented request through P2 is
    // answered through P2.
    assert_eq!(answered_via(&[(0, P2)]).await, Some(vec![P2]));
    // Segment 0 through P1, the final segment through P2.
    assert_eq!(
        answered_via(&[(0, P1), (1, P2)]).await,
        Some(vec![P1]),
        "the terminal answer follows segment 0's path"
    );
    // A retransmitted segment 0 through P2 is a retry by another path: the
    // answer goes there, whichever way the rest came.
    assert_eq!(
        answered_via(&[(0, P1), (0, P2), (1, P1)]).await,
        Some(vec![P2]),
        "a repeated segment 0 replaces the plan"
    );
}

/// A Segment-ACK the server sends mid-request goes through segment 0's
/// path too, and keeps the plan for the answer.
#[tokio::test(start_paused = true)]
async fn a_segment_ack_mid_request_uses_segment_zeros_router() {
    let mut f = Fixture::new().await;
    f.segment(0, true, P1).await;
    f.segment(1, true, P2).await;
    // Segment-ACK as the server: invoke ID 9, sequence 1, window 1.
    assert_eq!(f.answer(&[0x41, 9, 1, 1]).await, Some(vec![P1]));
    f.segment(2, false, P2).await;
    assert_eq!(f.answer(&SIMPLE_ACK).await, Some(vec![P1]));
    f.stop().await;
}
