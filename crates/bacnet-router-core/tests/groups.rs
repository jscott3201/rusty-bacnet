//! Group DADRs on a delivery port (#1504, #1677; decisions L19, L22, L23
//! and L24): only an Unconfirmed-Request for a group of the port's own goes
//! on, as that port's local broadcast; everything else is dropped and
//! counted, with no reject. The hosted router's rules are in
//! router/dispatch.rs (`group_dadr`) on the #1622 PR 3 branch.

mod fixture;

use bacnet_router_core::message::{MessageType, RejectReason};
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::router::{Discard, Group, Kind};
use bacnet_router_core::PortId;
use fixture::{
    addr, apdu_to, at, bytes, deliver, learn_9_via_r7, reject_data, router, Outcome, Recorder,
    TestRouter, CONFIRMED, NODE_A, R7, UNCONFIRMED,
};

/// The subnet broadcast at port 0's own UDP port.
const OWN_GROUP: &[u8] = &[10, 0, 0, 255, 0xBA, 0xC0];
/// The same broadcast address at another UDP port.
const OTHER_GROUP: &[u8] = &[10, 0, 0, 255, 0xBA, 0xC1];
/// A group on port 1, the MS/TP-like link.
const PORT_1_GROUP: &[u8] = &[0x7F];

fn host() -> Recorder {
    Recorder {
        groups: vec![
            (PortId(0), OWN_GROUP.to_vec(), Group::Broadcast),
            (PortId(0), OTHER_GROUP.to_vec(), Group::Refuse),
            (PortId(1), PORT_1_GROUP.to_vec(), Group::Broadcast),
        ],
        ..Recorder::default()
    }
}

fn dropped(router: &TestRouter, outcome: &Outcome, sent: &[fixture::Sent], drops: u64) {
    assert_eq!(*outcome, Outcome::Discarded(Discard::GroupDadr));
    assert!(sent.is_empty(), "no send and no reject");
    assert_eq!(router.counters().group_dadr_drops, drops);
    assert_eq!(router.counters().rejects, 0);
}

/// L19: an Unconfirmed-Request for a group of the delivery port's own goes
/// out as that port's local broadcast, with no DNET and no DADR, SNET and
/// SADR added, and isn't counted. From another network it isn't delivered
/// locally.
#[test]
fn an_unconfirmed_request_for_an_own_group_becomes_a_local_broadcast() {
    let mut router = router();
    let mut host = host();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        R7,
        &apdu_to(1, OWN_GROUP, UNCONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].port, sent[0].is_broadcast(), sent[0].kind),
        (0, true, Kind::Forward)
    );
    let mut expected = Npdu::apdu(UNCONFIRMED);
    expected.source = Some(addr(2, R7));
    assert_eq!(sent[0].npdu, bytes(&expected));
    assert_eq!(router.counters().group_dadr_drops, 0);
    assert_eq!(
        *host.group_asked.borrow(),
        [(PortId(0), OWN_GROUP.to_vec())]
    );
}

/// L22: from the network it was addressed to, the converted broadcast is
/// also delivered to the router's own application as a group delivery,
/// where a DLEN 0 broadcast would be.
#[test]
fn on_the_arrival_network_it_is_also_delivered_locally() {
    let mut router = router();
    let mut host = host();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(1, OWN_GROUP, UNCONFIRMED),
        at(1),
    );
    assert_eq!(
        outcome,
        Outcome::Local {
            apdu: UNCONFIRMED.to_vec(),
            apdu_offset: 12,
            source: None,
            group: true,
            global: false,
            expecting_reply: false,
            priority: 0,
        }
    );
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].port, sent[0].is_broadcast()), (0, true));
    assert_eq!(sent[0].parsed().destination, None);

    // The same on port 1's group, from port 1's network.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        R7,
        &apdu_to(2, PORT_1_GROUP, UNCONFIRMED),
        at(1),
    );
    assert!(matches!(outcome, Outcome::Local { group: true, .. }));
    assert_eq!(
        (sent.len(), sent[0].port, sent[0].is_broadcast()),
        (1, 1, true)
    );
}

/// #1504: any other APDU for a group of the port's own is dropped and
/// counted, and so is anything for a group outside it (L24).
#[test]
fn other_apdus_and_foreign_groups_are_dropped() {
    let mut router = router();
    let mut host = host();
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        1,
        R7,
        &apdu_to(1, OWN_GROUP, CONFIRMED),
        at(1),
    );
    dropped(&router, &outcome, &sent, 1);
    for apdu in [UNCONFIRMED, CONFIRMED] {
        let (outcome, sent) = deliver(
            &mut router,
            &mut host,
            1,
            R7,
            &apdu_to(1, OTHER_GROUP, apdu),
            at(1),
        );
        assert_eq!(outcome, Outcome::Discarded(Discard::GroupDadr));
        assert!(sent.is_empty());
    }
    // From the arrival network too: nothing delivered, nothing sent back.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(1, OTHER_GROUP, UNCONFIRMED),
        at(1),
    );
    dropped(&router, &outcome, &sent, 4);
}

/// Only a directly attached port's group counts: a DADR beyond another
/// router is the next router's business, and goes on unchanged, without
/// asking the host. A group of one port means nothing on another.
#[test]
fn only_a_directly_attached_ports_groups_count() {
    let mut router = router();
    let mut host = host();
    learn_9_via_r7(&mut router, at(0));
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(9, PORT_1_GROUP, CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(R7)));
    assert_eq!(sent[0].parsed().destination, Some(addr(9, PORT_1_GROUP)));
    assert!(host.group_asked.borrow().is_empty());

    // Port 0's group address, sent to port 1: an ordinary unicast there.
    let (outcome, sent) = deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, OWN_GROUP, CONFIRMED),
        at(1),
    );
    assert_eq!(outcome, Outcome::Handled);
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(OWN_GROUP)));
    // And a DLEN 0 broadcast is never asked about.
    deliver(
        &mut router,
        &mut host,
        0,
        NODE_A,
        &apdu_to(2, &[], UNCONFIRMED),
        at(1),
    );
    assert_eq!(host.group_asked.borrow().len(), 1);
    assert_eq!(router.counters().group_dadr_drops, 0);
}

/// L23: a network message routed to a group DADR is dropped and counted,
/// never turned into a broadcast, whatever its data looks like.
#[test]
fn network_messages_for_a_group_are_dropped() {
    let mut router = router();
    let mut host = host();
    let mut drops = 0;
    for group in [OWN_GROUP, OTHER_GROUP] {
        let mut proprietary = Npdu::message(MessageType::from_raw(0x80), UNCONFIRMED);
        proprietary.vendor_id = Some(260);
        proprietary.destination = Some(addr(1, group));
        let mut i_am = Npdu::message(MessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
        i_am.destination = Some(addr(1, group));
        for frame in [bytes(&proprietary), bytes(&i_am)] {
            let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &frame, at(1));
            drops += 1;
            dropped(&router, &outcome, &sent, drops);
        }
    }
    assert!(router.table().lookup(44).is_none());
}

/// L24: a received reject passing on toward a group DADR is dropped and
/// counted like any network message, after the table has taken it.
#[test]
fn a_reject_passing_on_to_a_group_is_dropped() {
    let mut router = router();
    let mut host = host();
    learn_9_via_r7(&mut router, at(0));
    for (group, drops) in [(OWN_GROUP, 1), (OTHER_GROUP, 2)] {
        let data = reject_data(RejectReason::NOT_DIRECTLY_CONNECTED, 9);
        let mut reject = Npdu::message(MessageType::REJECT_MESSAGE_TO_NETWORK, &data);
        reject.destination = Some(addr(1, group));
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &bytes(&reject), at(1));
        dropped(&router, &outcome, &sent, drops);
    }
    assert_eq!(router.table().claim_counters().reject_applied, 1);
    assert_eq!(router.counters().unroutable_rejects, 0);
}
