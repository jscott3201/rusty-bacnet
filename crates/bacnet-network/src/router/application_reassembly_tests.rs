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

/// The router at which the stack's SimpleAck for invoke ID 9 leaves on
/// network 2000, after segments came through `paths` in order.
async fn answered_via(paths: &[u8]) -> Option<Vec<u8>> {
    let (mut router, apdus, mut peers) = start(2).await;
    let mut port = router.application_port(apdus);
    let mut frames = port.start().await.unwrap();
    for (sequence, &via) in paths.iter().enumerate() {
        let more = sequence + 1 < paths.len();
        relayed(&peers[1], segment(sequence as u8, more), via).await;
        frames.recv().await.unwrap();
    }
    // The stack's SimpleAck to station 5 on network 3000.
    let ack = [1, 0x20, 0x0B, 0xB8, 1, 5, 255, 0x20, 9, 0x0F];
    let (to, answered) = tokio::join!(
        async {
            let sent = sending(&mut peers[1]).await;
            sent.complete.send(Ok(())).unwrap();
            sent.to
        },
        // The link address is the plan's to choose, not the stack's.
        port.send_unicast(&ack, &[])
    );
    answered.unwrap();
    router.stop().await.unwrap();
    port.stop().await.unwrap();
    to
}

#[tokio::test(start_paused = true)]
async fn a_reassembled_request_is_answered_through_segment_zeros_router() {
    // The paths are distinguishable: an unsegmented request through P2 is
    // answered through P2.
    assert_eq!(answered_via(&[P2]).await, Some(vec![P2]));
    // Segment 0 through P1, the final segment through P2.
    assert_eq!(
        answered_via(&[P1, P2]).await,
        Some(vec![P1]),
        "the terminal answer follows segment 0's path"
    );
}
