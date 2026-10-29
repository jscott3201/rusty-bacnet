//! A group activated through the Network Port survives a restart over the
//! same persisted-group file (#1954), and a BBMD that serves a group moves
//! to a new one together with its BDT, or not at all (#1963). Loopback
//! multicast with loop on works on Linux and macOS; Windows multicast is
//! untested (multicast.rs).
#![cfg(any(target_os = "linux", target_vendor = "apple"))]

use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bacnet_types::bip_port::{BbmdChanges, BbmdTables};
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostNPort};
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::super::bdt_activation_persist_tests::{TempDir, tables};
use super::BipMulticast;
use crate::bip::BipTransport;
use crate::port::{ReceivedNpdu, TransportPort};

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 93, 1);
const MOVED: Ipv4Addr = Ipv4Addr::new(239, 255, 93, 2);
const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn group(group: Ipv4Addr, port: u16) -> BipMulticast {
    BipMulticast::new(group).unwrap().with_port(port).unwrap()
}

/// A started member on 127.0.0.1 configured with [`GROUP`] at `port`,
/// keeping its group in `path` when given.
async fn member(
    path: Option<&Path>,
    multicast: BipMulticast,
) -> (BipTransport, mpsc::Receiver<ReceivedNpdu>) {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    transport.set_multicast(Some(multicast)).unwrap();
    if let Some(path) = path {
        transport.set_multicast_persist_path(path);
    }
    let rx = transport.start().await.unwrap();
    (transport, rx)
}

/// Wait until `count` saves of `transport`'s group file have finished.
async fn saved(transport: &BipTransport, count: u64) {
    let file = transport.multicast_file().unwrap();
    timeout(Duration::from_secs(10), async {
        while file.finished() < count {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the save finished");
}

/// A broadcast from a member of `multicast`'s group reaches `rx`.
async fn hears_the_group(rx: &mut mpsc::Receiver<ReceivedNpdu>, multicast: BipMulticast) {
    let (mut peer, _peer_rx) = member(None, multicast).await;
    peer.send_broadcast(NPDU).await.unwrap();
    let heard = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("timed out waiting for the group's broadcast")
        .unwrap();
    assert_eq!(&heard.npdu[..], NPDU);
    assert!(heard.link_layer_group);
    peer.stop().await.unwrap();
}

/// Activated, a group is saved; a new transport over the same file, as a
/// new process would build, starts in it ahead of its configured group,
/// reads it back and takes the group's broadcasts. Turning multicast off
/// is kept the same way.
#[tokio::test]
async fn an_activated_group_survives_a_restart() {
    let dir = TempDir::new("multicast-group");
    let path = dir.file("group");
    let port = free_port();
    let (mut first, _rx) = member(Some(&path), group(GROUP, port)).await;
    let view = first.bip_port().unwrap().multicast.unwrap();
    view.activate(MOVED.octets()).unwrap();
    saved(&first, 1).await;
    assert_eq!(first.multicast_persist_failures(), 0);
    first.stop().await.unwrap();
    drop(first);

    let (mut second, mut rx) = member(Some(&path), group(GROUP, port)).await;
    assert_eq!(second.multicast(), Some(group(MOVED, port)));
    let view = second.bip_port().unwrap().multicast.unwrap();
    assert_eq!(view.group(), MOVED.octets());
    assert_eq!(
        second.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new(MOVED, port))
    );
    hears_the_group(&mut rx, group(MOVED, port)).await;

    view.activate([0; 4]).unwrap();
    saved(&second, 1).await;
    second.stop().await.unwrap();
    drop(second);
    let (mut third, _rx) = member(Some(&path), group(GROUP, port)).await;
    assert_eq!(third.multicast(), None);
    third.stop().await.unwrap();
}

/// A save that fails is counted and leaves the activated group in use; a
/// file that can't be read as a group is ignored, with the configured
/// group used.
#[tokio::test]
async fn a_failed_save_is_counted_and_a_bad_file_ignored() {
    let dir = TempDir::new("multicast-group-fail");
    let port = free_port();
    let unwritable = dir.file("missing").join("group");
    let (mut transport, _rx) = member(Some(&unwritable), group(GROUP, port)).await;
    let view = transport.bip_port().unwrap().multicast.unwrap();
    view.activate(MOVED.octets()).unwrap();
    saved(&transport, 1).await;
    assert_eq!(transport.multicast_persist_failures(), 1);
    assert_eq!(transport.multicast(), Some(group(MOVED, port)));
    transport.stop().await.unwrap();

    let junk = dir.file("junk");
    std::fs::write(&junk, b"not a group").unwrap();
    let (mut transport, _rx) = member(Some(&junk), group(GROUP, port)).await;
    assert_eq!(transport.multicast(), Some(group(GROUP, port)));
    transport.stop().await.unwrap();
}

/// A BDT row for 127.0.0.1 at `port` with `mask`.
fn row(port: u16, mask: [u8; 4]) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort::from_socket_addr(
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(),
        ),
        broadcast_mask: Some(mask),
    }
}

fn changes(group: Ipv4Addr, bdt: Vec<BACnetBDTEntry>) -> BbmdChanges {
    let mut changes = BbmdChanges::default();
    changes.multicast_group = Some(group.octets());
    changes.broadcast_distribution_table = Some(bdt);
    changes
}

/// The rows the BBMD runs with.
fn bdt(transport: &BipTransport) -> Vec<BACnetBDTEntry> {
    tables(transport).broadcast_distribution_table()
}

/// A running group BBMD moves to a new group with a BDT that fits it, in
/// one activation through its tables view, and then takes the new group's
/// broadcasts; a set that doesn't fit, or a listener that can't open,
/// leaves the group and the BDT as they were (#1963). The new group is
/// saved like a member's (#1954).
#[tokio::test]
async fn a_group_bbmd_moves_its_group_with_its_bdt_or_not_at_all() {
    let dir = TempDir::new("multicast-bbmd");
    let (own_port, group_port) = (free_port(), free_port());
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, own_port, Ipv4Addr::LOCALHOST);
    bbmd.set_multicast(Some(group(GROUP, group_port))).unwrap();
    bbmd.enable_bbmd(vec![crate::bbmd::BdtEntry {
        ip: [127, 0, 0, 1],
        port: own_port,
        broadcast_mask: [0xFF; 4],
    }]);
    bbmd.set_multicast_persist_path(dir.file("group"));
    let mut rx = bbmd.start().await.unwrap();
    let view: Arc<dyn BbmdTables> = tables(&bbmd);
    assert!(view.changes_multicast_group());
    let before = bdt(&bbmd);
    let peer = BACnetBDTEntry {
        bbmd_address: BACnetHostNPort::from_socket_addr(
            SocketAddrV4::new(Ipv4Addr::new(10, 9, 8, 7), 47808).into(),
        ),
        broadcast_mask: Some([0xFF; 4]),
    };
    let fitting = changes(MOVED, vec![row(own_port, [0xFF; 4]), peer.clone()]);
    let one_hop = changes(MOVED, vec![row(own_port, [255, 0, 0, 0])]);

    // The own row must be two-hop while it serves a group.
    assert!(view.check(&one_hop).is_err());
    assert!(view.activate(&one_hop).is_err());
    // A listener that can't open: nothing changes, the tables neither.
    view.check(&fitting).unwrap();
    bbmd.multicast
        .fail_next_listener
        .store(true, Ordering::Relaxed);
    assert!(view.activate(&fitting).is_err());
    assert_eq!(bbmd.multicast(), Some(group(GROUP, group_port)));
    assert_eq!(bdt(&bbmd), before);

    view.activate(&fitting).unwrap();
    assert_eq!(bbmd.multicast(), Some(group(MOVED, group_port)));
    assert_eq!(bdt(&bbmd), vec![row(own_port, [0xFF; 4]), peer]);
    assert_eq!(
        bbmd.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new(MOVED, group_port))
    );
    hears_the_group(&mut rx, group(MOVED, group_port)).await;
    saved(&bbmd, 1).await;
    assert_eq!(bbmd.multicast_persist_failures(), 0);
    bbmd.stop().await.unwrap();
}

/// A BBMD on 127.0.0.1 at `own_port` with a two-hop own row, in `multicast`
/// when given; not started.
fn group_bbmd(own_port: u16, multicast: Option<BipMulticast>) -> BipTransport {
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, own_port, Ipv4Addr::LOCALHOST);
    bbmd.set_multicast(multicast).unwrap();
    bbmd.enable_bbmd(vec![crate::bbmd::BdtEntry {
        ip: [127, 0, 0, 1],
        port: own_port,
        broadcast_mask: [0xFF; 4],
    }]);
    bbmd
}

fn serves_group(bbmd: &BipTransport) -> bool {
    crate::bbmd::lock(bbmd.bbmd_state().unwrap()).serves_group()
}

/// A restart puts the BBMD's own-row rule where the group it starts with
/// says, even when a run-time activation moved the group since the last
/// start (#1963): left at run time and set again before the restart, the
/// BBMD serves it again; joined at run time and cleared before the
/// restart, it serves none.
#[tokio::test]
async fn a_restart_follows_the_group_whatever_moved_it_at_run_time() {
    let (own_port, group_port) = (free_port(), free_port());
    let multicast = group(GROUP, group_port);

    // Started in the group, left at run time, set again: serves it.
    let mut bbmd = group_bbmd(own_port, Some(multicast));
    let _rx = bbmd.start().await.unwrap();
    let mut leave = BbmdChanges::default();
    leave.multicast_group = Some([0; 4]);
    tables(&bbmd).activate(&leave).unwrap();
    assert!(!serves_group(&bbmd));
    bbmd.stop().await.unwrap();
    bbmd.set_multicast(Some(multicast)).unwrap();
    let _rx = bbmd.start().await.unwrap();
    assert!(serves_group(&bbmd));
    bbmd.stop().await.unwrap();

    // Started without one, joined at run time, cleared: serves none.
    let mut bbmd = group_bbmd(own_port, None);
    let _rx = bbmd.start().await.unwrap();
    assert!(!serves_group(&bbmd));
    tables(&bbmd)
        .activate(&changes(GROUP, vec![row(own_port, [0xFF; 4])]))
        .unwrap();
    assert!(serves_group(&bbmd));
    bbmd.stop().await.unwrap();
    bbmd.set_multicast(None).unwrap();
    let _rx = bbmd.start().await.unwrap();
    assert!(!serves_group(&bbmd));
    bbmd.stop().await.unwrap();
}

/// When the link refuses to move after the tables went in, the tables are
/// taken back out: the BDT and the group rule as they were, the old group
/// in use and still heard.
#[tokio::test]
async fn a_link_that_wont_move_undoes_the_tables() {
    let (own_port, group_port) = (free_port(), free_port());
    let mut bbmd = group_bbmd(own_port, Some(group(GROUP, group_port)));
    let mut rx = bbmd.start().await.unwrap();
    let before = bdt(&bbmd);
    // Leaving the group lifts the rule, so a one-hop own row goes in first.
    let leave = changes(Ipv4Addr::UNSPECIFIED, vec![row(own_port, [255, 0, 0, 0])]);
    tables(&bbmd).check(&leave).unwrap();
    bbmd.multicast
        .fail_next_subnet
        .store(true, Ordering::Relaxed);
    assert!(tables(&bbmd).activate(&leave).is_err());
    assert_eq!(bdt(&bbmd), before);
    assert!(serves_group(&bbmd));
    assert_eq!(bbmd.multicast(), Some(group(GROUP, group_port)));
    assert_eq!(
        bbmd.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new(GROUP, group_port))
    );
    hears_the_group(&mut rx, group(GROUP, group_port)).await;
    bbmd.stop().await.unwrap();
}
