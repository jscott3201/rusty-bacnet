//! Group DADRs on a delivery port (#1504, #1677; decisions L19, L22, L23
//! and L24): only an Unconfirmed-Request for a group of the port's own goes
//! on, as that port's local broadcast; everything else is dropped and
//! counted, with no reject.

mod fixture;

use bacnet_router_core::npdu::NetworkPriority;
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_router_core::router::{Discard, Group, GroupClassifier, Kind, Options, PortConfig};
use fixture::{
    CONFIRMED, NODE_A, Outcome, R7, Recorder, UNCONFIRMED, addr, apdu_to, at, bytes, deliver,
    learn_9_via_r7, reject_data,
};
use std::{cell::RefCell, rc::Rc};

/// The subnet broadcast at port 0's own UDP port.
const OWN_GROUP: &[u8] = &[10, 0, 0, 255, 0xBA, 0xC0];
/// The same broadcast address at another UDP port.
const OTHER_GROUP: &[u8] = &[10, 0, 0, 255, 0xBA, 0xC1];
/// A group on port 1, the MS/TP-like link.
const PORT_1_GROUP: &[u8] = &[0x7F];

type Calls = Rc<RefCell<Vec<Vec<u8>>>>;

#[derive(Clone)]
struct CustomGroups {
    matches: &'static [(&'static [u8], Group)],
    asked: Option<Calls>,
}

impl GroupClassifier for CustomGroups {
    const NONE: Self = Self {
        matches: &[],
        asked: None,
    };

    fn classify(&self, mac: &[u8]) -> Group {
        if let Some(asked) = &self.asked {
            asked.borrow_mut().push(mac.to_vec());
        }
        self.matches
            .iter()
            .find(|(address, _)| *address == mac)
            .map_or(Group::No, |(_, group)| *group)
    }
}

type TestRouter = fixture::TestRouter<CustomGroups>;

fn router() -> TestRouter {
    let port = |network, mac, matches| {
        PortConfig::new(
            network,
            bacnet_router_core::npdu::Mac::from_slice(mac).unwrap(),
            bacnet_router_core::npdu::NpduLimits::both(1497),
            CustomGroups {
                matches,
                asked: Some(Rc::default()),
            },
        )
    };
    fixture::router_with(
        &[
            port(
                1,
                fixture::OWN_0,
                &[(OWN_GROUP, Group::Broadcast), (OTHER_GROUP, Group::Refuse)],
            ),
            port(2, fixture::OWN_1, &[(PORT_1_GROUP, Group::Broadcast)]),
        ],
        Options::HOSTED,
    )
}

fn host() -> Recorder {
    Recorder::default()
}

fn dropped(router: &TestRouter, outcome: &Outcome, sent: &[fixture::Sent], drops: u32) {
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
        *router.ports()[0].groups.asked.as_ref().unwrap().borrow(),
        [OWN_GROUP.to_vec()]
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
            priority: NetworkPriority::Normal,
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
/// asking the classifier. A group of one port means nothing on another.
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
    assert!(
        router
            .ports()
            .iter()
            .all(|port| port.groups.asked.as_ref().unwrap().borrow().is_empty())
    );

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
    assert_eq!(
        router.ports()[1]
            .groups
            .asked
            .as_ref()
            .unwrap()
            .borrow()
            .len(),
        1
    );
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
        let mut proprietary = Npdu::message(NetworkMessageType::from_raw(0x80), UNCONFIRMED);
        proprietary.vendor_id = Some(260);
        proprietary.destination = Some(addr(1, group));
        let mut i_am = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
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
        let data = reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 9);
        let mut reject = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
        reject.destination = Some(addr(1, group));
        let (outcome, sent) = deliver(&mut router, &mut host, 1, R7, &bytes(&reject), at(1));
        dropped(&router, &outcome, &sent, drops);
    }
    assert_eq!(router.table().claim_counters().reject_applied, 1);
    assert_eq!(router.counters().unroutable_rejects, 0);
}

#[test]
fn reusing_storage_releases_custom_rules_of_removed_ports() {
    let mut storage = fixture::Store::<CustomGroups>::new();
    let calls = Rc::default();
    let weak = Rc::downgrade(&calls);
    let port = |network, groups| {
        PortConfig::new(
            network,
            bacnet_router_core::npdu::Mac::from_array([1]),
            bacnet_router_core::npdu::NpduLimits::both(1497),
            groups,
        )
    };
    {
        let ports = [
            port(1, CustomGroups::NONE),
            port(
                2,
                CustomGroups {
                    matches: &[],
                    asked: Some(calls),
                },
            ),
        ];
        let _router =
            bacnet_router_core::Router::new(&mut storage, &ports, Options::HOSTED, at(0)).unwrap();
    }
    assert!(
        weak.upgrade().is_some(),
        "storage retains its installed rule"
    );
    let _router = bacnet_router_core::Router::new(
        &mut storage,
        &[port(1, CustomGroups::NONE)],
        Options::HOSTED,
        at(0),
    )
    .unwrap();
    assert!(
        weak.upgrade().is_none(),
        "a removed port no longer retains caller state"
    );
}

/// Public table edits can name an absent port, inside or beyond storage.
/// Classification must leave the established forwarding guard in charge.
fn assert_unconfigured_port(npdu: &[u8]) {
    for port in [2, 255] {
        let mut router = router();
        assert!(
            router
                .table_mut()
                .add_direct(9, bacnet_router_core::PortId(port))
        );
        let (outcome, sent) = deliver(&mut router, &mut host(), 0, NODE_A, npdu, at(1));
        assert_eq!(outcome, Outcome::Discarded(Discard::UnknownPort));
        assert!(sent.is_empty());
        assert_eq!(router.counters().unknown_port, 1);
        assert_eq!(router.counters().group_dadr_drops, 0);
        assert_eq!(router.counters().rejects, 0);
        assert!(
            router.ports().iter().all(|port| port
                .groups
                .asked
                .as_ref()
                .unwrap()
                .borrow()
                .is_empty())
        );
    }
}

#[test]
fn unconfigured_direct_ports_for_apdus_are_counted_without_classification() {
    assert_unconfigured_port(&apdu_to(9, OWN_GROUP, UNCONFIRMED));
}

#[test]
fn unconfigured_direct_ports_for_network_messages_are_counted_without_classification() {
    let mut npdu = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0, 44]);
    npdu.destination = Some(addr(9, OWN_GROUP));
    assert_unconfigured_port(&bytes(&npdu));
}

#[test]
fn unconfigured_direct_ports_for_forwarded_rejects_are_counted_without_classification() {
    let data = reject_data(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 77);
    let mut npdu = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
    npdu.destination = Some(addr(9, OWN_GROUP));
    assert_unconfigured_port(&bytes(&npdu));
}
