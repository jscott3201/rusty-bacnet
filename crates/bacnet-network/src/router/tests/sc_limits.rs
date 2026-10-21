//! A BACnet/SC port that fails over to a hub with another NPDU limit
//! (#1696): the router follows the new hub's limit, so an NPDU too long for
//! it draws reason 4, and one the old hub's limit refused now goes.
//!
//! The router routes between network 1000, on hub A, and network 2000,
//! whose port starts on a primary hub and fails over to a second one when
//! the primary goes away. Waits are generous (#1707): no TLS here, but the
//! failover runs on real time.

use super::data_attributes::sc_hub_accept_with_limits;
use super::*;
use bacnet_npdu::NpduLimits;
use bacnet_transport::sc::{LoopbackWebSocket, ScReconnectConfig, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    decode_sc_message, encode_sc_message, ScFunction, ScMessage, ScOption, Vmac,
};
use tokio::time::{sleep, timeout, Instant};

/// The node on network 1000 that sends, and the one on 2000 it sends to.
const NODE: Vmac = [0x0A; 6];
const PEER: Vmac = [0x22; 6];

/// How long a hub handshake or a send may take to show.
const WAIT: Duration = Duration::from_secs(5);

/// The router and the hubs' ends: hub A, then the primary hub of network
/// 2000's port and the failover hub, not yet connected.
struct Hubs {
    router: BACnetRouter,
    hub_a: LoopbackWebSocket,
    primary: LoopbackWebSocket,
    failover: LoopbackWebSocket,
}

/// Start the router with the primary hub of network 2000 taking `primary`,
/// its Max-BVLC-Length and Max-NPDU-Length.
async fn start(primary: (u16, u16)) -> Hubs {
    let (client_a, hub_a) = LoopbackWebSocket::pair();
    let (client_primary, hub_primary) = LoopbackWebSocket::pair();
    let (client_failover, hub_failover) = LoopbackWebSocket::pair();
    let accept_a = tokio::spawn(async move {
        sc_hub_accept_with_limits(&hub_a, [0xA0; 6], 1476, 1476).await;
        hub_a
    });
    let accept_primary = tokio::spawn(async move {
        sc_hub_accept_with_limits(&hub_primary, [0xB0; 6], primary.0, primary.1).await;
        hub_primary
    });
    let ports = vec![
        RouterPort {
            transport: ScTransport::new(client_a, [0x01; 6]).with_device_uuid([1; 16]),
            network_number: 1000,
        },
        RouterPort {
            transport: ScTransport::new(client_primary, [0x02; 6])
                .with_device_uuid([2; 16])
                .with_reconnect(ScReconnectConfig {
                    initial_delay_ms: 25,
                    max_delay_ms: 25,
                    max_retries: 1,
                })
                .with_failover(client_failover),
            network_number: 2000,
        },
    ];
    let StartedRouter { router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    Hubs {
        router,
        hub_a: accept_a.await.unwrap(),
        primary: accept_primary.await.unwrap(),
        failover: hub_failover,
    }
}

impl Hubs {
    /// Drop the primary hub, let the failover hub take the port with
    /// `limits`, and wait until the router routes by `expected`.
    async fn fail_over(
        self,
        limits: (u16, u16),
        expected: u16,
    ) -> (BACnetRouter, LoopbackWebSocket, LoopbackWebSocket) {
        let Self {
            router,
            hub_a,
            primary,
            failover,
        } = self;
        let accept = tokio::spawn(async move {
            sc_hub_accept_with_limits(&failover, [0xC0; 6], limits.0, limits.1).await;
            failover
        });
        drop(primary);
        let failover = timeout(WAIT, accept)
            .await
            .expect("the port fails over")
            .unwrap();
        let deadline = Instant::now() + WAIT;
        while router.shared.engine.lock().ports()[1].max_npdu != NpduLimits::both(expected) {
            assert!(
                Instant::now() < deadline,
                "the router never routed by {expected}"
            );
            sleep(Duration::from_millis(10)).await;
        }
        (router, hub_a, failover)
    }
}

/// From `NODE` through hub A: an NPDU for `PEER` on network 2000 with an
/// APDU of `apdu` octets. It is 12 octets more on the way in, and 11 once
/// rewritten onto network 2000, which drops the DNET, DADR and hop count
/// and adds `NODE` as SADR.
async fn send_to_peer(hub_a: &LoopbackWebSocket, apdu: usize) {
    send_with_options(hub_a, apdu, Vec::new()).await;
}

async fn send_with_options(hub_a: &LoopbackWebSocket, apdu: usize, data_options: Vec<ScOption>) {
    let mut payload = vec![0x10, 0x08];
    payload.resize(apdu, 0);
    let npdu = Npdu {
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(&PEER),
        }),
        hop_count: 255,
        payload: Bytes::from(payload),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu).unwrap();
    send_encoded_with_options(hub_a, npdu_buf.freeze(), data_options).await;
}

async fn send_encoded_with_options(
    hub: &LoopbackWebSocket,
    npdu: Bytes,
    data_options: Vec<ScOption>,
) {
    let message = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x4455,
        originating_vmac: Some(NODE),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options,
        payload: npdu,
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &message);
    hub.send(&buf).await.unwrap();
}

/// The next NPDU `hub` relays to `to`, skipping everything else.
async fn next_npdu_to(hub: &LoopbackWebSocket, to: Vmac) -> Bytes {
    let wanted = async {
        loop {
            let message = decode_sc_message(&hub.recv().await.unwrap()).unwrap();
            if message.function == ScFunction::EncapsulatedNpdu
                && message.destination_vmac == Some(to)
            {
                return message.payload;
            }
        }
    };
    timeout(WAIT, wanted).await.expect("the NPDU comes")
}

/// Whether `npdu` is a reject with reason 4 naming network 2000.
fn is_reason_4(npdu: Bytes) -> bool {
    let npdu = decode_npdu(npdu).unwrap();
    npdu.message_type == Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
        && npdu.payload.as_ref() == [4, 0x07, 0xD0]
}

/// From a hub that carries 1,497 octets to one that carries 480: a
/// 601-octet relay draws reason 4 and the count.
#[tokio::test]
async fn a_hub_that_carries_less_draws_reason_4() {
    let hubs = start((1600, 1497)).await;
    assert_eq!(
        hubs.router.shared.engine.lock().ports()[1].max_npdu,
        NpduLimits::both(1497)
    );
    let (mut router, hub_a, _failover) = hubs.fail_over((1200, 480), 480).await;
    send_to_peer(&hub_a, 590).await;
    assert!(is_reason_4(next_npdu_to(&hub_a, NODE).await));
    assert_eq!(router.counters().egress_too_long, 1);
    router.stop().await;
}

/// From a hub that carries 480 octets to one that carries 1,497: the
/// 601-octet relay the first refused with reason 4 now goes.
#[tokio::test]
async fn a_hub_that_carries_more_takes_what_the_old_one_refused() {
    let hubs = start((1200, 480)).await;
    send_to_peer(&hubs.hub_a, 590).await;
    assert!(is_reason_4(next_npdu_to(&hubs.hub_a, NODE).await));
    let (mut router, hub_a, failover) = hubs.fail_over((1600, 1497), 1497).await;
    send_to_peer(&hub_a, 590).await;
    assert_eq!(next_npdu_to(&failover, PEER).await.len(), 601);
    assert_eq!(router.counters().egress_too_long, 1);
    router.stop().await;
}

/// The NPDU fits, but forwarded attributes push its encoded SC frame over
/// the hub's bound. A following fitting send is an ordered progress barrier.
#[tokio::test]
async fn forwarded_attributes_beyond_max_bvlc_return_reason_4() {
    let mut hubs = start((100, 1497)).await;
    send_with_options(
        &hubs.hub_a,
        2,
        vec![ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![0xAB; 100],
        }],
    )
    .await;
    send_to_peer(&hubs.hub_a, 2).await;
    assert_eq!(next_npdu_to(&hubs.primary, PEER).await.len(), 13);
    let counts = hubs.router.counters();
    assert_eq!(
        (counts.egress_too_long, counts.refused, counts.rejects),
        (1, 1, 1)
    );
    assert!(is_reason_4(next_npdu_to(&hubs.hub_a, NODE).await));
    hubs.router.stop().await;
}

#[tokio::test]
async fn forwarded_reject_attributes_beyond_max_bvlc_do_not_make_feedback() {
    let mut hubs = start((100, 1497)).await;
    // Directed Reject-Message-To-Network for PEER, naming network 77.
    let mut reject = vec![1, 0xa0, 7, 0xd0, 6];
    reject.extend_from_slice(&PEER);
    reject.extend_from_slice(&[255, 3, 4, 0, 77]);
    send_encoded_with_options(
        &hubs.hub_a,
        Bytes::from(reject),
        vec![ScOption {
            option_type: 31,
            must_understand: false,
            data: vec![0xab; 100],
        }],
    )
    .await;
    send_to_peer(&hubs.hub_a, 2).await;
    assert_eq!(next_npdu_to(&hubs.primary, PEER).await.len(), 13);
    let counts = hubs.router.counters();
    assert_eq!(
        (counts.egress_too_long, counts.refused, counts.rejects),
        (1, 1, 0)
    );
    hubs.router.stop().await;
}
