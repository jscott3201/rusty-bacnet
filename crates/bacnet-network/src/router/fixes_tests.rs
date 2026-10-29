//! The Clause 6 fixes the hosted router took after it moved onto the
//! engine (#1679), each on the hosted engine through
//! [`envelope_harness`](super::envelope_harness). `bacnet-router-core`'s
//! tests hold each one against the clause.

use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu};
use bacnet_npdu::{NetworkMessageType, NpduLimits};
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::SendRequest;
use super::envelope_harness::Harness;

/// The link sender of the NPDUs below: a B/IP node on port 0.
const SENDER: [u8; 6] = [10, 0, 0, 9, 0xBA, 0xC0];

fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

/// An NPDU of `len` octets carrying an Unconfirmed-Request for `network`
/// and `mac`, with no source fields and a hop count of 255.
fn npdu_of(len: usize, network: u16, mac: &[u8]) -> Npdu {
    let header = 6 + mac.len();
    Npdu {
        destination: Some(address(network, mac)),
        hop_count: 255,
        payload: Bytes::from(vec![0x10; len - header]),
        ..Npdu::default()
    }
}

/// The reason and network of the one reject queued on `port`.
fn reject_on(h: &mut Harness, port: usize) -> (u8, u16, MacAddr) {
    let sent = h.drain(port);
    let [SendRequest::Unicast { npdu, mac, .. }] = &sent[..] else {
        panic!("expected one reject on port {port}, got {sent:?}");
    };
    let npdu = decode_npdu(npdu.clone()).unwrap();
    assert_eq!(npdu.message_type, Some(NetworkMessageType::from_raw(0x03)));
    let [reason, high, low] = npdu.payload[..] else {
        panic!("a reject's data is three octets");
    };
    (reason, u16::from_be_bytes([high, low]), mac.clone())
}

/// The lengths of the NPDUs queued on `port`.
fn lengths_on(h: &mut Harness, port: usize) -> Vec<usize> {
    h.drain(port)
        .into_iter()
        .map(|sent| match sent {
            SendRequest::Unicast { npdu, .. } | SendRequest::Broadcast { npdu, .. } => npdu.len(),
        })
        .collect()
}

// --- R1 (#1665): reason 4 for an NPDU too long for its egress port ---

/// #1665: an NPDU routed onto an MS/TP network whose rewrite is one octet
/// past what the port carries draws reject reason 4, naming that network,
/// back to its sender; it is counted and not sent. With extended frames
/// that is 1,498 octets, and 502 on a port with standard frames only. One
/// octet shorter goes out. The rewrite here gains 4 octets: the B/IP
/// sender's source fields come, the destination fields and hop count go.
#[tokio::test]
async fn r1_an_npdu_too_long_for_an_mstp_port_draws_reason_4() {
    for limit in [1497, 501] {
        let mut h = Harness::with_limits(
            &[1000, 2000],
            &[NpduLimits::STANDARD, NpduLimits::both(limit)],
        );
        let longest = usize::from(limit) - 4;
        h.dispatch(h.ctx(0, &SENDER, npdu_of(longest + 1, 2000, &[5])))
            .await;
        assert_eq!(
            reject_on(&mut h, 0),
            (4, 2000, MacAddr::from_slice(&SENDER))
        );
        h.assert_quiet();
        let counters = h.counters();
        assert_eq!((counters.egress_too_long, counters.rejects), (1, 1));

        h.dispatch(h.ctx(0, &SENDER, npdu_of(longest, 2000, &[5])))
            .await;
        assert_eq!(lengths_on(&mut h, 1), [usize::from(limit)]);
        h.assert_quiet();

        // A remote broadcast goes by the same rule, a DLEN 0 rewrite
        // gaining 5 octets.
        h.dispatch(h.ctx(0, &SENDER, npdu_of(longest, 2000, &[])))
            .await;
        assert_eq!(
            reject_on(&mut h, 0),
            (4, 2000, MacAddr::from_slice(&SENDER))
        );
        h.assert_quiet();
        assert_eq!(h.counters().egress_too_long, 2);
    }
}

/// A transport that reports a limit under the engine's least, 64 octets,
/// still starts: the router takes 64, and refuses a longer rewrite.
#[tokio::test]
async fn r1_a_port_reporting_under_64_octets_carries_64() {
    let mut h = Harness::with_limits(&[1000, 2000], &[NpduLimits::STANDARD, NpduLimits::both(8)]);
    h.dispatch(h.ctx(0, &SENDER, npdu_of(60, 2000, &[5]))).await;
    assert_eq!(lengths_on(&mut h, 1), [64]);
    h.dispatch(h.ctx(0, &SENDER, npdu_of(61, 2000, &[5]))).await;
    assert_eq!(
        reject_on(&mut h, 0),
        (4, 2000, MacAddr::from_slice(&SENDER))
    );
    h.assert_quiet();
}

/// A fan-out leaves out a port too short for it, counted, and sends no
/// reject (decision D21 on #1679): the global broadcast still reaches
/// network 3000 and the router's own application.
#[tokio::test]
async fn r1_a_global_broadcast_skips_a_port_too_short_for_it() {
    let mut h = Harness::with_limits(
        &[1000, 2000, 3000],
        &[
            NpduLimits::STANDARD,
            NpduLimits::both(501),
            NpduLimits::STANDARD,
        ],
    );
    h.dispatch(h.ctx(0, &SENDER, npdu_of(600, 0xFFFF, &[])))
        .await;
    // 600 octets plus SNET, SLEN and SADR.
    assert_eq!(lengths_on(&mut h, 2), [609]);
    assert!(h.apdus.try_recv().is_ok(), "delivered locally");
    h.assert_quiet();
    let counters = h.counters();
    assert_eq!((counters.egress_too_long, counters.rejects), (1, 0));
}

// --- R2: reject reasons other than 1 and 2 change no route ---

/// R2 (Clause 6.6.3.5): a reason-4 reject from the router that serves
/// 3000, for a node on network 1000, is passed on to that node and leaves
/// the route to 3000 as it was; before the fix it removed the route, which
/// R1's rejects would make routine.
#[tokio::test]
async fn r2_a_reason_4_reject_is_passed_on_and_keeps_the_route() {
    let mut h = Harness::two_port();
    h.learned(3000, 1, &[9]);
    let before = h.table().lookup(3000);
    let reject = Npdu {
        message_type: Some(NetworkMessageType::from_raw(0x03)),
        destination: Some(address(1000, &SENDER)),
        hop_count: 255,
        payload: Bytes::from_static(&[4, 0x0B, 0xB8]),
        ..Npdu::default()
    };
    h.dispatch(h.ctx(1, &[9], reject)).await;
    let sent = h.drain(0);
    let [SendRequest::Unicast { npdu, mac, .. }] = &sent[..] else {
        panic!("expected the reject passed on to port 0, got {sent:?}");
    };
    assert_eq!(mac.as_slice(), SENDER);
    assert!(npdu.ends_with(&[0x03, 4, 0x0B, 0xB8]));
    h.assert_quiet();
    assert_eq!(h.table().lookup(3000), before);
    let counters = h.table().claim_counters();
    assert_eq!(
        (
            counters.reject_applied,
            counters.reject_dampened,
            counters.reject_kept
        ),
        (0, 0, 1)
    );
}

// --- R4: the unscoped I-Am-Router answer leaves out unreachable networks ---

/// R4 (Clause 6.6.3.2): an unscoped Who-Is-Router from network 1000 is
/// answered with every network reached through another port, ascending,
/// whatever order the table learned them in: 2000 (direct), 2500 (busy,
/// still listed) and 3000. 3001, marked unreachable, is left out, where the
/// router before the fix listed it; 4000, behind the arrival port, never
/// was.
#[tokio::test]
async fn r4_the_unscoped_answer_leaves_out_unreachable_networks() {
    let mut h = Harness::two_port();
    h.learned(3001, 1, &[9]);
    h.learned(2500, 1, &[9]);
    h.learned(3000, 1, &[8]);
    h.learned(4000, 0, &[7]);
    let busy_until = h.now() + bacnet_router_core::Duration::from_secs(30);
    {
        let mut table = h.table();
        table.mark_unreachable(3001);
        table.mark_busy(2500, busy_until);
    }
    let who_is_all = Npdu {
        message_type: Some(NetworkMessageType::from_raw(0x00)),
        ..Npdu::default()
    };
    h.dispatch(h.ctx(0, &SENDER, who_is_all)).await;
    let sent = h.drain(0);
    let [SendRequest::Broadcast { npdu, .. }] = &sent[..] else {
        panic!("expected one I-Am-Router broadcast on port 0, got {sent:?}");
    };
    assert_eq!(
        npdu.as_ref(),
        [0x01, 0x80, 0x01, 0x07, 0xD0, 0x09, 0xC4, 0x0B, 0xB8]
    );
    h.assert_quiet();
}

// --- R5: no search for a network behind the arrival port ---

/// A Who-Is-Router naming `network`, from the link with no source fields.
fn who_is(network: u16) -> Npdu {
    Npdu {
        message_type: Some(NetworkMessageType::from_raw(0x00)),
        payload: Bytes::copy_from_slice(&network.to_be_bytes()),
        ..Npdu::default()
    }
}

/// R5 (Clause 6.6.3.2): a Who-Is-Router from network 1000 for 3000, which
/// the table reaches through port 0 itself, or for 1000, port 0's own
/// network, draws no answer and goes to no other port: the router that
/// serves it is on network 1000 already. Before the fix it was searched for
/// on network 2000. One for an unknown network still is.
#[tokio::test]
async fn r5_a_network_behind_the_arrival_port_is_neither_answered_nor_searched() {
    let mut h = Harness::two_port();
    h.learned(3000, 0, &[9]);
    for network in [3000, 1000] {
        h.dispatch(h.ctx(0, &SENDER, who_is(network))).await;
        h.assert_quiet();
    }
    h.dispatch(h.ctx(0, &SENDER, who_is(4000))).await;
    let relayed = lengths_on(&mut h, 1);
    assert_eq!(relayed.len(), 1, "an unknown network is searched for");
    h.assert_quiet();
}

// --- R6: the hop count comes down before the router decides ---

/// R6 (Clauses 6.2.3, 6.3.2): an NPDU for 3000, behind the router at [9] on
/// port 1, that arrives with a hop count of 1 would leave with 0, so it goes
/// nowhere, counted, with no reject; before the fix it went on with 0. A
/// global broadcast with 1 left is delivered here and passed on nowhere.
/// Onto network 2000, attached, an NPDU with 1 left still goes, since it
/// leaves with no hop count at all.
#[tokio::test]
async fn r6_a_relay_never_leaves_with_a_hop_count_of_0() {
    let mut h = Harness::two_port();
    h.learned(3000, 1, &[9]);
    let mut to_3000 = npdu_of(20, 3000, &[5]);
    to_3000.hop_count = 1;
    h.dispatch(h.ctx(0, &SENDER, to_3000)).await;
    h.assert_quiet();
    let counters = h.counters();
    assert_eq!((counters.hop_count_drops, counters.rejects), (1, 0));

    let mut global = npdu_of(20, 0xFFFF, &[]);
    global.hop_count = 1;
    h.dispatch(h.ctx(0, &SENDER, global)).await;
    h.assert_quiet();
    assert!(h.apdus.try_recv().is_ok(), "delivered locally");
    assert_eq!(h.counters().hop_count_drops, 2);

    let mut to_2000 = npdu_of(20, 2000, &[5]);
    to_2000.hop_count = 1;
    h.dispatch(h.ctx(0, &SENDER, to_2000)).await;
    assert_eq!(lengths_on(&mut h, 1), [24]);
    h.assert_quiet();
}

// --- R7: the router's own address on any of its networks is local ---

/// R7 (Clause 6.1, Figure 6-12): a confirmed request from network 1000 for
/// the router's own MAC on network 2000 ([2]) reaches the router's own
/// application, with network 1000 as where it came in, and nothing goes on
/// either wire. Before the fix it went out on network 2000, addressed to
/// the router itself.
#[tokio::test]
async fn r7_an_apdu_for_the_routers_address_on_another_port_is_local() {
    let mut h = Harness::two_port();
    let request = Npdu {
        destination: Some(address(2000, &[super::envelope_harness::PORT_MACS[1]])),
        hop_count: 255,
        expecting_reply: true,
        payload: Bytes::from_static(&[0x00, 0x05, 0x01, 0x0C]),
        ..Npdu::default()
    };
    h.dispatch(h.ctx(0, &SENDER, request)).await;
    h.assert_quiet();
    let delivered = h.apdus.try_recv().expect("delivered locally");
    assert_eq!(delivered.apdu.as_ref(), [0x00, 0x05, 0x01, 0x0C]);
    assert_eq!(delivered.ingress_network, Some(1000));
    assert_eq!(delivered.source_mac.as_slice(), SENDER);
    assert!(delivered.source_network.is_none());
    assert!(!delivered.is_group && !delivered.global_broadcast);
}

/// R7 matches network and MAC together: a request for network 3000, behind
/// the router at [9] on port 1, whose DADR happens to equal one of the
/// router's own MACs ([1] or [2]) names another node there, so it is routed
/// on to [9] with its DADR, and nothing is delivered here.
#[tokio::test]
async fn r7_the_routers_mac_on_a_remote_network_is_routed_on() {
    use super::envelope_harness::PORT_MACS;
    let mut h = Harness::two_port();
    h.learned(3000, 1, &[9]);
    for mac in PORT_MACS {
        h.dispatch(h.ctx(0, &SENDER, npdu_of(20, 3000, &[mac])))
            .await;
        let sent = h.drain(1);
        let [SendRequest::Unicast { npdu, mac: to, .. }] = &sent[..] else {
            panic!("expected the request routed on port 1, got {sent:?}");
        };
        assert_eq!(to.as_slice(), [9]);
        let npdu = decode_npdu(npdu.clone()).unwrap();
        assert_eq!(npdu.destination, Some(address(3000, &[mac])));
        h.assert_quiet();
        assert!(h.apdus.try_recv().is_err(), "not delivered here");
    }
}
