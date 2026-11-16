//! One device on two seats (#2617): a segmented answer no lease holds is
//! the client's to abort, and the node's DeviceCommunicationControl state,
//! which the seated server sets and every seated client follows.
use std::sync::Mutex;

use super::*;
use crate::coordinator::{
    CanonicalPeer, LeaseMetadata, OutboundTransactionCoordinator, TerminalPolicy,
};
use crate::seat::{InitiationGate, SeatRoles};
use bacnet_types::enums::ConfirmedServiceChoice;

/// A segmented Complex-ACK's first segment, a Segment-ACK and an Abort the
/// answering side sent, and an unsegmented Complex-ACK, none of which any
/// lease holds.
const SEGMENTED_ANSWER: &[u8] = &[0x3c, 201, 0x00, 0x04, 0x0c, 0xc4];
const SERVER_SEGMENT_ACK: &[u8] = &[0x41, 202, 0x00, 0x01];
const SERVER_ABORT: &[u8] = &[0x71, 203, 0x00];
const UNSEGMENTED_ANSWER: &[u8] = &[0x30, 204, 0x0c, 0xc4];

async fn started(capacity: usize) -> (EndpointIngress<TestTransport>, IngressReceivers, Link) {
    let (transport, link) = test_transport();
    let mut endpoint = EndpointIngress::new(transport, capacity);
    let ingress = endpoint.start().await.unwrap();
    (endpoint, ingress, link)
}

/// Clause 5.4.4.1: in its idle state the client aborts a segmented answer
/// and a server's Segment-ACK it has no transaction for, so each goes to the
/// client seat whether or not a server is seated. What it would only drop,
/// a server's Abort or an unsegmented answer, stays with the endpoint, which
/// counts it.
#[tokio::test]
async fn a_segmented_answer_no_lease_holds_is_the_clients() {
    for with_server in [false, true] {
        let (endpoint, mut ingress, link) = started(8).await;
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let mut client = endpoint
            .client_seat(Arc::clone(&coordinator), SeatRoles::new(true), 8)
            .unwrap();
        let mut server = with_server.then(|| endpoint.server_seat(coordinator).unwrap());
        for apdu in [SEGMENTED_ANSWER, SERVER_SEGMENT_ACK] {
            inject(&link, apdu).await;
            assert_eq!(receive(&mut client.apdus).await.apdu.as_ref(), apdu);
        }
        for apdu in [SERVER_ABORT, UNSEGMENTED_ANSWER] {
            inject(&link, apdu).await;
            assert_eq!(
                receive(&mut ingress.terminal_or_segment)
                    .await
                    .apdu
                    .as_ref(),
                apdu
            );
        }
        assert!(client.apdus.try_recv().is_err());
        if let Some(server) = server.as_mut() {
            assert!(server.apdus.try_recv().is_err(), "the server takes none");
        }
        assert!(ingress.terminal_or_segment.try_recv().is_err());
    }
}

/// A flood of stray segmented answers takes at most half of the client's
/// seat, so the answer to the client's own request still gets in, and what
/// the flood could not place is counted (#2635).
#[tokio::test]
async fn stray_answers_leave_room_for_the_clients_own() {
    let (endpoint, _ingress, link) = started(8).await;
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let mut client = endpoint
        .client_seat(Arc::clone(&coordinator), SeatRoles::new(true), 8)
        .unwrap();
    let lease = coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::direct(&[0x11]),
            ConfirmedServiceChoice::READ_PROPERTY,
            TerminalPolicy::EitherAck,
        ))
        .unwrap();
    for invoke_id in 100..108 {
        inject(&link, &[0x3c, invoke_id, 0x00, 0x04, 0x0c, 0xc4]).await;
    }
    let answer = [0x30, lease.invoke_id(), 0x0c, 0xc4];
    inject(&link, &answer).await;

    // Four strays fill the half a copy may take; the other four are
    // dropped, and the client's own answer finds the room they left.
    let mut taken = Vec::new();
    for _ in 0..5 {
        taken.push(receive(&mut client.apdus).await.apdu);
    }
    assert!(client.apdus.try_recv().is_err());
    assert_eq!(taken[4].as_ref(), answer);
    let counters = endpoint.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (5, 4));
}

/// Every state a gate was told, in order.
#[derive(Default)]
struct Told(Mutex<Vec<bool>>);

impl InitiationGate for Told {
    fn restrict(&self, restricted: bool) {
        self.0.lock().unwrap().push(restricted);
    }
}

impl Told {
    fn seen(&self) -> Vec<bool> {
        self.0.lock().unwrap().clone()
    }
}

fn follow(dcc: &crate::seat::NodeDcc) -> Arc<Told> {
    let told = Arc::new(Told::default());
    let gate: std::sync::Weak<dyn InitiationGate> = Arc::downgrade(&told) as _;
    dcc.follow(gate);
    told
}

#[tokio::test]
async fn the_seated_server_sets_the_node_state_every_client_follows() {
    let (endpoint, _ingress, _link) = started(8).await;
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let client = endpoint
        .client_seat(Arc::clone(&coordinator), SeatRoles::new(true), 8)
        .unwrap();
    let server = endpoint.server_seat(Arc::clone(&coordinator)).unwrap();
    let early = follow(&client.dcc);

    server.dcc.set(true);
    assert!(client.dcc.is_restricted());
    // One that follows once it is in force starts restricted.
    let late = follow(&client.dcc);
    // Only a change is passed on.
    server.dcc.set(true);
    server.dcc.set(false);
    assert_eq!(early.seen(), [true, false]);
    assert_eq!(late.seen(), [true, false]);

    // A gate that is gone is let go.
    drop(late);
    server.dcc.set(true);
    assert_eq!(early.seen(), [true, false, true]);
}

/// The server's stop or drop enables the node again, as a restart would,
/// and a server seated since can't be overruled by one that went before.
#[tokio::test]
async fn the_node_state_ends_with_its_server() {
    let (endpoint, _ingress, _link) = started(8).await;
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let node = endpoint.node_dcc().unwrap();
    let told = follow(&node);

    let first = endpoint.server_seat(Arc::clone(&coordinator)).unwrap();
    first.dcc.set(true);
    first.dcc.release();
    assert!(!node.is_restricted());
    assert_eq!(told.seen(), [true, false]);

    // A dropped seat frees its place; its hold, kept elsewhere, is stale.
    let crate::seat::ServerSeat {
        dcc: stale, apdus, ..
    } = first;
    drop(apdus);
    let second = endpoint.server_seat(Arc::clone(&coordinator)).unwrap();
    second.dcc.set(true);
    stale.set(false);
    drop(stale);
    assert!(node.is_restricted(), "only the newest server sets it");
    drop(second);
    assert!(!node.is_restricted());
    assert_eq!(told.seen(), [true, false, true, false]);
}
