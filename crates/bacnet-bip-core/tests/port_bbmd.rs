//! The port driver on a BBMD ([`BbmdPort`]): the management requests it
//! answers from its tables, so the caller never sees one (Annex J.4.4.1,
//! J.5.2), what it forwards, and the storage it refuses.

mod node_support;

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcResultCode, Bvll, FdtEntries, FdtEntry};
use bacnet_bip_core::bbmd::{
    DeleteRefusal, Registered, RegistrationRefusal, Reservation, Rules, Served, Tables, Target,
};
use bacnet_bip_core::node::{ConfigError, Discard, Link, Mode, Permissive, Policy, SendRefusal};
use bacnet_bip_core::port::{BbmdPort, Ingress, PortError};
use bacnet_bip_core::Instant;
use node_support::*;

/// A foreign device.
const DEVICE: SocketAddrV4 = addr(203, 0, 113, 7, PORT);

/// An unused row.
const NO_ROW: FdtEntry = FdtEntry::new(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0), 0, 0);

const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

type Small = Tables<2, 2>;

/// Tables at [`OWN`] with [`BBMD`] as a two-hop peer.
fn tables() -> Small {
    let mut tables = Small::new(OWN);
    let peer = [BdtEntry::new(BBMD, Ipv4Addr::BROADCAST)];
    tables.bdt.set(BdtEntries::from_entries(&peer)).unwrap();
    tables
}

fn bbmd_link() -> Link {
    link(Mode::Bbmd)
}

/// What `port` makes of `message` from `source`, sent to its own address.
fn receive<'d>(
    port: &mut BbmdPort<'_, 2, 2>,
    payload: &'d [u8],
    source: SocketAddrV4,
    now: Instant,
    policy: &mut (impl Policy + Rules),
    socket: &mut Socket,
) -> Ingress<'d> {
    port.receive(&datagram(payload, source, UNICAST), now, policy, socket)
}

#[test]
fn ports_refuse_storage_that_doesnt_fit() {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    assert_eq!(
        BbmdPort::new(link(Mode::Normal), &mut tables, &mut targets, &mut rows).err(),
        Some(PortError::Mode)
    );
    let tiny = with(bbmd_link(), |link| link.budget = Some(10));
    assert_eq!(
        BbmdPort::new(tiny, &mut tables, &mut targets, &mut rows).err(),
        Some(PortError::Link(ConfigError::Budget))
    );
    assert_eq!(
        BbmdPort::new(bbmd_link(), &mut tables, &mut targets[..4], &mut rows).err(),
        Some(PortError::TargetStorage {
            needed: Small::MAX_TARGETS,
            available: 4
        })
    );
    assert_eq!(
        BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows[..1]).err(),
        Some(PortError::RowStorage {
            needed: 2,
            available: 1
        })
    );
    assert!(BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows).is_ok());
}

/// Each management request is answered over the socket with what Annex J
/// gives it: Register-Foreign-Device with Successful completion or its
/// NAK, 0x0030; each table read with its ACK; Delete-Foreign-Device-Table-
/// Entry with Successful completion, or its NAK, 0x0050, when there is no
/// registration to delete.
#[test]
fn management_requests_are_answered_by_the_port() {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();
    let p = &mut Permissive;

    let register = wire(Bvll::RegisterForeignDevice { ttl: 60 });
    let ingress = receive(&mut port, &register, DEVICE, at(1_000), p, &mut socket);
    let Ingress::Served(Served::Register(outcome)) = ingress else {
        panic!("{ingress:?}");
    };
    assert_eq!(outcome.result, Ok(Registered::New));
    assert_eq!(socket.take(), [(DEVICE, result(SUCCESS))]);

    let zero = wire(Bvll::RegisterForeignDevice { ttl: 0 });
    let ingress = receive(&mut port, &zero, PEER, at(1_000), p, &mut socket);
    assert!(matches!(
        ingress,
        Ingress::Served(Served::Register(outcome))
            if outcome.result == Err(RegistrationRefusal::TimeToLive)
    ));
    let nak = BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK;
    assert_eq!(socket.take(), [(PEER, result(nak))]);

    // Read a second later: 60 + 30 - 1 s left.
    let read_fdt = wire(Bvll::ReadForeignDeviceTable);
    let ingress = receive(&mut port, &read_fdt, PEER, at(2_000), p, &mut socket);
    assert_eq!(ingress, Ingress::Served(Served::ReadFdt(Ok(1))));
    let row = [FdtEntry::new(DEVICE, 60, 89)];
    let ack = Bvll::ReadForeignDeviceTableAck(FdtEntries::from_entries(&row));
    assert_eq!(socket.take(), [(PEER, wire(ack))]);

    let read_bdt = wire(Bvll::ReadBroadcastDistributionTable);
    let ingress = receive(&mut port, &read_bdt, PEER, at(2_000), p, &mut socket);
    assert_eq!(ingress, Ingress::Served(Served::ReadBdt));
    let bdt = [
        BdtEntry::new(BBMD, Ipv4Addr::BROADCAST),
        BdtEntry::new(OWN, Ipv4Addr::BROADCAST),
    ];
    let ack = Bvll::ReadBroadcastDistributionTableAck(BdtEntries::from_entries(&bdt));
    assert_eq!(socket.take(), [(PEER, wire(ack))]);

    let delete = wire(Bvll::DeleteForeignDeviceTableEntry(DEVICE));
    let ingress = receive(&mut port, &delete, PEER, at(3_000), p, &mut socket);
    assert_eq!(ingress, Ingress::Served(Served::DeleteFdtEntry(Ok(()))));
    assert_eq!(socket.take(), [(PEER, result(SUCCESS))]);
    assert!(port.tables().fdt.is_empty());
    let ingress = receive(&mut port, &delete, PEER, at(3_000), p, &mut socket);
    assert_eq!(
        ingress,
        Ingress::Served(Served::DeleteFdtEntry(Err(DeleteRefusal::NotRegistered)))
    );
    let nak = BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK;
    assert_eq!(socket.take(), [(PEER, result(nak))]);
    assert_eq!(port.counters().refused, 0);
}

/// A host that lets only its own station delete registrations, through the
/// policy it hands the port.
#[derive(Debug, Default)]
struct OnlyPeerDeletes;

impl Policy for OnlyPeerDeletes {}

impl Rules for OnlyPeerDeletes {
    fn admit_delete(&mut self, sender: SocketAddrV4, _entry: SocketAddrV4) -> bool {
        sender == PEER
    }
}

#[test]
fn the_hosts_rules_reach_the_tables() {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();
    let mut rules = OnlyPeerDeletes;
    let register = wire(Bvll::RegisterForeignDevice { ttl: 60 });
    receive(&mut port, &register, DEVICE, at(0), &mut rules, &mut socket);
    let delete = wire(Bvll::DeleteForeignDeviceTableEntry(DEVICE));
    let ingress = receive(&mut port, &delete, DEVICE, at(0), &mut rules, &mut socket);
    assert_eq!(
        ingress,
        Ingress::Served(Served::DeleteFdtEntry(Err(DeleteRefusal::NotAdmitted)))
    );
    assert!(port.tables().fdt.is_registered_at(DEVICE, at(0)));
    let ingress = receive(&mut port, &delete, PEER, at(0), &mut rules, &mut socket);
    assert_eq!(ingress, Ingress::Served(Served::DeleteFdtEntry(Ok(()))));
}

/// A management request from an address a reply can't reach (here UDP
/// port 0) is dropped: nothing is registered or sent.
#[test]
fn nothing_is_served_to_an_unanswerable_source() {
    let mut tables = tables();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();
    let register = wire(Bvll::RegisterForeignDevice { ttl: 60 });
    let nowhere = addr(203, 0, 113, 9, 0);
    let ingress = receive(
        &mut port,
        &register,
        nowhere,
        at(0),
        &mut Permissive,
        &mut socket,
    );
    assert_eq!(ingress, Ingress::Discarded(Discard::UnanswerableSource));
    assert_eq!(socket.calls, 0);
    assert!(port.tables().fdt.is_empty());
}

/// Each datagram of a BBMD port's own that the socket refuses is counted
/// once: a forwarded copy, a NAK, and a management answer. Taking the
/// counts starts them again from zero.
#[test]
fn refused_datagrams_of_a_bbmd_port_are_counted() {
    let mut tables = tables();
    let now = at(1_000);
    tables
        .fdt
        .register(
            DEVICE,
            60,
            now,
            &Default::default(),
            Reservation::Ordinary,
            || true,
        )
        .result
        .unwrap();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket {
        refuse: Some(SendRefusal::SocketFull),
        ..Socket::default()
    };
    let forwarded = Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    };
    for (message, from, to, refused) in [
        // A subnet broadcast, to the peer and the device.
        (Bvll::OriginalBroadcastNpdu(NPDU), PEER, TO_BROADCAST, 2),
        // The peer's forward, to the subnet and the device.
        (forwarded, BBMD, UNICAST, 4),
        // A broadcast from a device that isn't registered: its NAK.
        (Bvll::DistributeBroadcastToNetwork(NPDU), PEER, UNICAST, 5),
        // A table read: its ACK.
        (Bvll::ReadBroadcastDistributionTable, PEER, UNICAST, 6),
    ] {
        let payload = wire(message);
        let ingress = port.receive(
            &datagram(&payload, from, to),
            now,
            &mut Permissive,
            &mut socket,
        );
        assert!(!matches!(ingress, Ingress::Discarded(_)), "{ingress:?}");
        assert_eq!(port.counters().refused, refused, "{message:?}");
    }
    assert_eq!(port.take_counters().refused, 6);
    assert_eq!(port.counters().refused, 0);
}

/// The BBMD's own broadcast goes on the subnet, then to its peer and its
/// foreign device as Forwarded-NPDUs (Annex J.4.5). When the socket refuses
/// the subnet's copy, the broadcast is refused and no other copy goes out:
/// the caller's send is the caller's to count, not the port's.
#[test]
fn a_broadcast_the_socket_refuses_forwards_nothing() {
    let mut tables = tables();
    tables
        .fdt
        .register(
            DEVICE,
            60,
            at(0),
            &Default::default(),
            Reservation::Ordinary,
            || true,
        )
        .result
        .unwrap();
    let mut targets = [Target::UNUSED; Small::MAX_TARGETS];
    let mut rows = [NO_ROW; 2];
    let mut port = BbmdPort::new(bbmd_link(), &mut tables, &mut targets, &mut rows).unwrap();
    let mut socket = Socket::default();
    assert_eq!(port.broadcast(NPDU, at(1), &mut socket), Ok(()));
    let forwarded = wire(Bvll::ForwardedNpdu {
        origin: OWN,
        npdu: NPDU,
    });
    assert_eq!(
        socket.take(),
        [
            (BROADCAST, wire(Bvll::OriginalBroadcastNpdu(NPDU))),
            (BBMD, forwarded.clone()),
            (DEVICE, forwarded),
        ]
    );

    socket.refuse = Some(SendRefusal::SocketFull);
    let calls = socket.calls;
    assert_eq!(
        port.broadcast(NPDU, at(2), &mut socket),
        Err(SendRefusal::SocketFull)
    );
    assert_eq!(socket.calls, calls + 1, "the subnet's copy alone was tried");
    assert_eq!(port.counters().refused, 0);
}
