//! The client seat's tiers (#2635): copies of a burst of I-Ams fill at most
//! half of it, so a client that falls behind while they arrive, here one
//! that never reads, still gets its COV notification and the answer to its
//! own request; the burst's overflow is dropped and counted.
use super::*;
use bacnet_endpoint_core::coordinator::{CanonicalPeer, LeaseMetadata, TerminalPolicy};
use bacnet_transport::bvll::encode_bip_mac;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::Bytes;
use std::time::Duration;

use super::super::eventually::eventually;

use crate::session::server_seat::tests::wire::{self, Wire};

const NODE: u8 = 1;
const PEER: u8 = 2;
const BURST: usize = 300;

/// Device `instance`'s I-Am, from a station of its own.
fn i_am(instance: u32) -> bacnet_transport::port::ReceivedNpdu {
    let mut npdu = vec![0x01, 0x00, 0x10, 0x00, 0xC4];
    npdu.extend_from_slice(&((8u32 << 22) | instance).to_be_bytes());
    npdu.extend_from_slice(&[0x22, 0x01, 0xE0, 0x91, 0x03, 0x21, 0x2A]);
    let station = encode_bip_mac([10, 0, (instance >> 8) as u8, instance as u8], 47_808);
    wire::frame(&station, Bytes::from(npdu), true)
}

#[tokio::test(start_paused = true)]
async fn a_seat_nobody_reads_keeps_room_for_the_answer_to_its_clients_request() {
    let wire = Wire::new();
    let (node, peer) = (Wire::mac(NODE), Wire::mac(PEER));
    let transport = wire.port_with_queue(NODE, 1_024).build();
    let mut session = EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(crate::DeviceIdentity::new(123, 42).unwrap());
    session.start().await.unwrap();
    let mut seat = session.client_seat().unwrap();
    let lease = seat
        .coordinator
        .reserve(LeaseMetadata::requester(
            CanonicalPeer::direct(&peer),
            ConfirmedServiceChoice::WRITE_PROPERTY,
            TerminalPolicy::SimpleAck,
        ))
        .unwrap();
    let answer = Bytes::from(vec![0x01, 0x00, 0x20, lease.invoke_id(), 0x0f]);

    wire.hold(&node);
    for instance in 0..BURST as u32 {
        wire.deliver(&node, i_am(instance));
    }
    // A ConfirmedCOVNotification, which beside a server role is the
    // client's alone.
    let notification = Bytes::from_static(&[0x01, 0x04, 0x00, 0x05, 0x22, 0x01]);
    wire.deliver(&node, wire::frame(&peer, notification, false));
    wire.deliver(&node, wire::frame(&peer, answer, false));
    wire.release(&node);
    eventually(
        Duration::from_secs(5),
        "the whole burst classified",
        async || {
            let counters = session.client_seat_counters();
            counters.apdus + counters.full_drops == BURST as u64 + 2
        },
    )
    .await;

    // Half of the seat's 256 slots took I-Ams; the notification and the
    // answer took slots of the tiers kept for them.
    let counters = session.client_seat_counters();
    assert_eq!((counters.apdus, counters.full_drops), (130, 172));
    let queued: Vec<ReceivedApdu> = std::iter::from_fn(|| seat.apdus.try_recv().ok()).collect();
    assert_eq!(queued.len(), 130);
    assert!(
        queued[..128]
            .iter()
            .all(|received| received.apdu[0] == 0x10)
    );
    assert_eq!(queued[128].apdu.as_ref(), [0x00, 0x05, 0x22, 0x01]);
    assert_eq!(queued[129].apdu.as_ref(), [0x20, lease.invoke_id(), 0x0f]);
    assert!(session.is_running());

    drop(seat);
    session.stop().await.unwrap();
}
