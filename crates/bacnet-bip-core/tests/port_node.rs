//! The port driver on a node in normal or foreign-device mode
//! ([`NodePort`]): what it sends for the node, the broadcasts it holds for a
//! foreign device's first registration (Annex J.7.4; decision D17), and the
//! storage it refuses.

mod node_support;

use bacnet_bip_codec::{BvlcResultCode, Bvll};
use bacnet_bip_core::Instant;
use bacnet_bip_core::node::{Admission, Cast, Discard, Mode, Node, Permissive, SendRefusal};
use bacnet_bip_core::port::{HeldBroadcast, Ingress, NodePort, PortCounters, PortError};
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// A foreign device's link, holding `hold` broadcasts.
fn holding(hold: u16) -> Mode {
    Mode::Foreign(with(foreign(), |foreign| foreign.hold = hold))
}

fn node(mode: Mode) -> Node {
    Node::new(link(mode), Instant::ZERO).unwrap()
}

#[test]
fn ports_refuse_storage_that_doesnt_fit_the_node() {
    let now = Instant::ZERO;
    let mut none: [HeldBroadcast; 0] = [];
    let mut one = [HeldBroadcast::EMPTY];
    let refused = |mode, held: &mut [HeldBroadcast]| NodePort::new(node(mode), held).err();
    assert_eq!(refused(Mode::Bbmd, &mut none), Some(PortError::Mode));
    assert_eq!(
        refused(holding(2), &mut one),
        Some(PortError::HeldStorage {
            needed: 2,
            available: 1
        })
    );
    assert_eq!(refused(holding(1), &mut one), None);
    assert_eq!(refused(Mode::Normal, &mut none), None);
    // Without a send budget a held broadcast could be as long as the
    // largest BVLL message less its header: far past a slot.
    let unbounded = with(link(holding(1)), |link| link.budget = None);
    let unbounded = Node::<4>::new(unbounded, now).unwrap();
    assert_eq!(
        NodePort::new(unbounded, &mut one).err(),
        Some(PortError::HeldLength {
            needed: 65_525,
            available: 1497
        })
    );

    // Slots of N octets take a link whose broadcasts are no longer: a
    // budget of 510 lets broadcasts of 500 through.
    let budget = |octets| with(link(holding(1)), |link| link.budget = Some(octets));
    assert_eq!(budget(510).max_npdu(Cast::Broadcast), 500);
    let mut short = [HeldBroadcast::<500>::EMPTY];
    let port = NodePort::new(Node::<4>::new(budget(510), now).unwrap(), &mut short);
    assert!(port.is_ok());
    let mut shorter = [HeldBroadcast::<499>::EMPTY];
    let port = NodePort::new(Node::<4>::new(budget(510), now).unwrap(), &mut shorter);
    assert_eq!(
        port.err(),
        Some(PortError::HeldLength {
            needed: 500,
            available: 499
        })
    );
    // The default budget sends B/IP's largest NPDU.
    let mut small = [HeldBroadcast::<501>::EMPTY];
    assert_eq!(
        NodePort::new(node(holding(1)), &mut small).err(),
        Some(PortError::HeldLength {
            needed: 1497,
            available: 501
        })
    );
    // A normal node holds nothing, so any slots do.
    assert!(NodePort::new(node(Mode::Normal), &mut shorter).is_ok());
}

/// Requests only a BBMD serves draw their NAK from the port, and a
/// datagram the node drops goes nowhere.
#[test]
fn naks_and_drops_stay_at_the_port() {
    let mut port: NodePort<'_> = NodePort::new(node(Mode::Normal), &mut []).unwrap();
    let mut socket = Socket::default();
    let payload = wire(Bvll::ReadBroadcastDistributionTable);
    let ingress = port.receive(
        &datagram(&payload, PEER, UNICAST),
        at(0),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(ingress, Ingress::Handled);
    let nak = BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK;
    assert_eq!(socket.take(), [(PEER, result(nak))]);
    let ingress = port.receive(
        &datagram(&[0x81, 0x0A], PEER, UNICAST),
        at(0),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(ingress, Ingress::Discarded(Discard::TooShort));
    assert_eq!(socket.calls, 1);
}

/// A peer's NPDU reaches the caller from its source; a unicast goes to one
/// node, and never to a group address (#1677).
#[test]
fn npdus_arrive_and_leave_by_annex_j() {
    let mut port: NodePort<'_> = NodePort::new(node(Mode::Normal), &mut []).unwrap();
    let mut socket = Socket::default();
    let payload = wire(Bvll::OriginalUnicastNpdu(NPDU));
    let ingress = port.receive(
        &datagram(&payload, PEER, UNICAST),
        at(0),
        &mut Permissive,
        &mut socket,
    );
    let Ingress::Npdu(delivery) = ingress else {
        panic!("{ingress:?}");
    };
    assert_eq!(
        (delivery.npdu, delivery.source, delivery.group),
        (NPDU, PEER, false)
    );

    assert_eq!(port.unicast(PEER, NPDU, at(0), &mut socket), Ok(()));
    assert_eq!(
        port.unicast(BROADCAST, NPDU, at(0), &mut socket),
        Err(SendRefusal::GroupDestination)
    );
    assert_eq!(
        port.broadcast(NPDU, at(0), &mut socket),
        Ok(Admission::Send)
    );
    assert_eq!(
        socket.take(),
        [
            (PEER, wire(Bvll::OriginalUnicastNpdu(NPDU))),
            (BROADCAST, wire(Bvll::OriginalBroadcastNpdu(NPDU))),
        ]
    );
}

/// A foreign device's port registers at start and holds what it
/// broadcasts until the BBMD answers, refusing one past its hold; the
/// success releases them in order, each as a
/// Distribute-Broadcast-To-Network to the BBMD, and the next broadcast goes
/// at once.
#[test]
fn a_foreign_port_holds_its_broadcasts_until_registered() {
    let mut held = [<HeldBroadcast>::EMPTY; 2];
    let mut port: NodePort<'_> = NodePort::new(node(holding(2)), &mut held).unwrap();
    let mut socket = Socket::default();
    assert_eq!(port.next_deadline(), None, "nothing until start");
    port.start(at(0), &mut socket);
    let ttl = wire(Bvll::RegisterForeignDevice { ttl: 60 });
    assert_eq!(socket.take(), [(BBMD, ttl)]);
    assert_eq!(port.next_deadline(), Some(at(3_000)));

    let first = [0x01, 0x00, 0x10, 0x08];
    let second = [0x01, 0x00, 0x10, 0x09];
    assert_eq!(
        port.broadcast(&first, at(1), &mut socket),
        Ok(Admission::Hold)
    );
    assert_eq!(
        port.broadcast(&second, at(2), &mut socket),
        Ok(Admission::Hold)
    );
    assert_eq!(
        port.broadcast(NPDU, at(3), &mut socket),
        Err(SendRefusal::HoldFull)
    );
    assert!(socket.take().is_empty(), "nothing goes out while held");

    let success = result(SUCCESS);
    let ingress = port.receive(
        &datagram(&success, BBMD, UNICAST),
        at(10),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(ingress, Ingress::Handled);
    let dbtn = |npdu| (BBMD, wire(Bvll::DistributeBroadcastToNetwork(npdu)));
    assert_eq!(socket.take(), [dbtn(&first), dbtn(&second)]);
    assert_eq!(
        port.broadcast(NPDU, at(11), &mut socket),
        Ok(Admission::Send)
    );
    assert_eq!(socket.take(), [dbtn(NPDU)]);
}

/// The BBMD refuses the first registration: the held broadcasts are
/// dropped, never sent (D17).
#[test]
fn a_refused_first_registration_drops_the_held_broadcasts() {
    let mut held = [<HeldBroadcast>::EMPTY; 2];
    let mut port: NodePort<'_> = NodePort::new(node(holding(2)), &mut held).unwrap();
    let mut socket = Socket::default();
    port.start(at(0), &mut socket);
    assert_eq!(
        port.broadcast(NPDU, at(1), &mut socket),
        Ok(Admission::Hold)
    );
    socket.take();
    let nak = result(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK);
    port.receive(
        &datagram(&nak, BBMD, UNICAST),
        at(10),
        &mut Permissive,
        &mut socket,
    );
    assert!(socket.take().is_empty());
    let counters = port.node_mut().take_counters();
    assert_eq!(
        (counters.broadcasts_discarded, counters.broadcasts_released),
        (1, 0)
    );
}

/// Each datagram of the port's own that the socket refuses is counted
/// once, and a refused registration is the node's to count. Taking the
/// counts starts them again from zero; looking doesn't.
#[test]
fn refused_datagrams_are_counted_and_drained() {
    let mut held = [<HeldBroadcast>::EMPTY; 1];
    let mut port: NodePort<'_> = NodePort::new(node(holding(1)), &mut held).unwrap();
    let mut socket = Socket {
        refuse: Some(SendRefusal::SocketFull),
        ..Socket::default()
    };
    port.start(at(0), &mut socket);
    assert_eq!(port.counters(), PortCounters::default());
    let counters = port.node_mut().take_counters();
    assert_eq!(counters.register_foreign_device.local_errors, 1);

    let read = wire(Bvll::ReadForeignDeviceTable);
    port.receive(
        &datagram(&read, PEER, UNICAST),
        at(1),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(port.counters().refused, 1, "the NAK");
    assert_eq!(port.counters().refused, 1, "a look takes nothing");
    assert_eq!(port.take_counters().refused, 1);
    assert_eq!(port.counters(), PortCounters::default(), "drained");
    assert_eq!(
        port.broadcast(NPDU, at(2), &mut socket),
        Err(SendRefusal::SocketFull),
        "the caller's own send is the caller's to count"
    );
    assert_eq!(port.counters().refused, 0);
}
