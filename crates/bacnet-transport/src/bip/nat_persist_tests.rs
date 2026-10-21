//! A NAT setting activated through the Network Port is saved with the BDT,
//! and a restart loads the two together, over the configured ones (#940).

use super::bdt_activation_persist_tests::{row, saved, tables, TempDir};
use super::*;
use crate::bbmd::NatTraversal;
use bacnet_types::bip_port::BbmdChanges;
use bacnet_types::constructed::BACnetHostNPort;
use bytes::BytesMut;
use std::fs;
use std::path::Path;

const GLOBAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 1), 47809);
const PEER: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 2), 47809);

fn two_hop(address: SocketAddrV4) -> BdtEntry {
    BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: [255; 4],
    }
}

/// A started BBMD on loopback with `path` as its persisted-BDT file, behind
/// NAT at [`GLOBAL`] when `nat`, with `bdt` configured.
async fn bbmd(path: &Path, nat: bool, bdt: Vec<BdtEntry>) -> BipTransport {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.enable_bbmd(bdt);
    transport.set_bdt_persist_path(path.to_path_buf());
    transport.set_bbmd_nat_traversal(nat.then(|| NatTraversal::new(GLOBAL)));
    let _rx = transport.start().await.unwrap();
    transport
}

/// The started BBMD's NAT setting, own address and BDT.
fn running(transport: &BipTransport) -> (bool, Option<SocketAddrV4>, SocketAddrV4, Vec<BdtEntry>) {
    let state = bbmd::lock(transport.bbmd_state().unwrap());
    (
        state.nat_traversal(),
        state.global_address(),
        state.own_address(),
        state.bdt(),
    )
}

#[tokio::test]
async fn nat_turned_on_over_the_network_survives_a_restart() {
    let dir = TempDir::new("nat-on");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd(&path, false, Vec::new()).await;
    let mut changes = BbmdChanges::default();
    changes.nat_traversal = Some(true);
    changes.global_address = Some(BACnetHostNPort::from_socket_addr(GLOBAL.into()));
    changes.broadcast_distribution_table = Some(vec![
        row(GLOBAL.ip().octets(), GLOBAL.port()),
        row(PEER.ip().octets(), PEER.port()),
    ]);
    tables(&transport).activate(&changes).unwrap();
    saved(&transport, 1).await;
    transport.stop().await.unwrap();

    // The configuration still says no NAT; the activated setting wins.
    let mut restarted = bbmd(&path, false, Vec::new()).await;
    assert_eq!(
        running(&restarted),
        (
            true,
            Some(GLOBAL),
            GLOBAL,
            vec![two_hop(GLOBAL), two_hop(PEER)]
        )
    );
    restarted.stop().await.unwrap();
}

#[tokio::test]
async fn nat_turned_off_over_the_network_survives_a_restart() {
    let dir = TempDir::new("nat-off");
    let path = dir.file("bdt.bin");
    let config = vec![two_hop(GLOBAL), two_hop(PEER)];
    let mut transport = bbmd(&path, true, config.clone()).await;
    assert!(running(&transport).0);
    let mut changes = BbmdChanges::default();
    changes.nat_traversal = Some(false);
    tables(&transport).activate(&changes).unwrap();
    saved(&transport, 1).await;
    let local = transport.bip_port().unwrap().endpoint;
    transport.stop().await.unwrap();

    // The configuration still says NAT; the activated setting wins, and the
    // old global row went with it.
    let mut restarted = bbmd(&path, true, config).await;
    let (nat, global, own, rows) = running(&restarted);
    assert!(!nat);
    assert_eq!(global, Some(GLOBAL), "the address is kept for later");
    assert_eq!(*own.ip(), Ipv4Addr::LOCALHOST);
    assert!(!rows.contains(&two_hop(GLOBAL)), "{rows:?}");
    assert!(rows.contains(&two_hop(PEER)));
    assert_eq!(local.ip(), own.ip());
    restarted.stop().await.unwrap();
}

/// A file from before #940, rows only, loads as it always did, and the
/// configured NAT setting applies.
#[tokio::test]
async fn a_file_without_the_trailer_keeps_the_configured_nat() {
    let dir = TempDir::new("nat-old-file");
    let path = dir.file("bdt.bin");
    let mut rows = BytesMut::new();
    bbmd::encode_bdt_entries(&[two_hop(GLOBAL), two_hop(PEER)], &mut rows);
    fs::write(&path, &rows).unwrap();
    let mut transport = bbmd(&path, true, Vec::new()).await;
    assert_eq!(
        running(&transport),
        (
            true,
            Some(GLOBAL),
            GLOBAL,
            vec![two_hop(GLOBAL), two_hop(PEER)]
        )
    );
    transport.stop().await.unwrap();
}

/// A saved set that fails validation (NAT on, but the rows lack the
/// global address) is refused, and the configured values apply.
#[tokio::test]
async fn a_persisted_set_that_breaks_the_nat_rules_falls_back_to_the_config() {
    let dir = TempDir::new("nat-bad-file");
    let path = dir.file("bdt.bin");
    let mut octets = BytesMut::new();
    bbmd::encode_bdt_entries(&[two_hop(PEER)], &mut octets);
    octets.extend_from_slice(b"NAT1\x01");
    octets.extend_from_slice(&GLOBAL.ip().octets());
    octets.extend_from_slice(&GLOBAL.port().to_be_bytes());
    fs::write(&path, &octets).unwrap();
    let mut transport = bbmd(&path, false, Vec::new()).await;
    let (nat, global, own, rows) = running(&transport);
    assert!(!nat);
    assert_eq!(global, None);
    assert_eq!(rows, [two_hop(own)], "the configured, empty BDT");
    transport.stop().await.unwrap();
}

/// A BDT-only activation on a BBMD whose NAT setting was never activated
/// over the network writes the rows alone, so NAT turned on later in the
/// configuration applies at the next start.
#[tokio::test]
async fn a_bdt_only_activation_leaves_the_nat_setting_to_the_config() {
    let dir = TempDir::new("nat-bdt-only");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd(&path, false, Vec::new()).await;
    let mut changes = BbmdChanges::default();
    changes.broadcast_distribution_table = Some(vec![
        row(GLOBAL.ip().octets(), GLOBAL.port()),
        row(PEER.ip().octets(), PEER.port()),
    ]);
    tables(&transport).activate(&changes).unwrap();
    saved(&transport, 1).await;
    transport.stop().await.unwrap();
    assert_eq!(fs::read(&path).unwrap().len(), 20, "rows only");

    let mut restarted = bbmd(&path, true, Vec::new()).await;
    assert_eq!(
        running(&restarted),
        (
            true,
            Some(GLOBAL),
            GLOBAL,
            vec![two_hop(GLOBAL), two_hop(PEER)]
        )
    );
    restarted.stop().await.unwrap();
}

/// Once a file carries the NAT setting, a later BDT-only activation keeps
/// carrying it.
#[tokio::test]
async fn a_loaded_nat_setting_is_carried_by_later_saves() {
    let dir = TempDir::new("nat-carried");
    let path = dir.file("bdt.bin");
    let mut transport = bbmd(&path, false, Vec::new()).await;
    let mut on = BbmdChanges::default();
    on.nat_traversal = Some(true);
    on.global_address = Some(BACnetHostNPort::from_socket_addr(GLOBAL.into()));
    on.broadcast_distribution_table = Some(vec![row(GLOBAL.ip().octets(), GLOBAL.port())]);
    tables(&transport).activate(&on).unwrap();
    saved(&transport, 1).await;
    transport.stop().await.unwrap();

    let mut restarted = bbmd(&path, false, Vec::new()).await;
    let mut bdt = BbmdChanges::default();
    bdt.broadcast_distribution_table = Some(vec![
        row(GLOBAL.ip().octets(), GLOBAL.port()),
        row(PEER.ip().octets(), PEER.port()),
    ]);
    tables(&restarted).activate(&bdt).unwrap();
    saved(&restarted, 1).await;
    restarted.stop().await.unwrap();
    let (_, nat) = bbmd::decode_file(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        nat,
        Some(bbmd::PersistedNat {
            enabled: true,
            global: Some(GLOBAL)
        })
    );
}

fn file_with(rows: &[BdtEntry], trailer: &[u8]) -> Vec<u8> {
    let mut octets = BytesMut::new();
    bbmd::encode_bdt_entries(rows, &mut octets);
    octets.extend_from_slice(trailer);
    octets.to_vec()
}

fn trailer(tag: &[u8; 4], enabled: u8) -> Vec<u8> {
    let mut out = tag.to_vec();
    out.push(enabled);
    out.extend_from_slice(&GLOBAL.ip().octets());
    out.extend_from_slice(&GLOBAL.port().to_be_bytes());
    out
}

#[test]
fn decode_file_reads_rows_and_the_trailer() {
    let rows = [two_hop(GLOBAL), two_hop(PEER)];
    let (decoded, nat) = bbmd::decode_file(&file_with(&rows, &trailer(b"NAT1", 1))).unwrap();
    assert_eq!(decoded, rows);
    assert_eq!(
        nat,
        Some(bbmd::PersistedNat {
            enabled: true,
            global: Some(GLOBAL)
        })
    );
    let (decoded, nat) = bbmd::decode_file(&file_with(&rows, &[])).unwrap();
    assert_eq!((decoded.as_slice(), nat), (rows.as_slice(), None));
    // An unset global address reads as none.
    let unset = file_with(&rows, b"NAT1\x00\x00\x00\x00\x00\x00\x00");
    assert_eq!(
        bbmd::decode_file(&unset).unwrap().1,
        Some(bbmd::PersistedNat::default())
    );
}

#[test]
fn decode_file_refuses_a_damaged_file() {
    let rows = [two_hop(PEER)];
    // A trailer with another tag.
    assert!(bbmd::decode_file(&file_with(&rows, &trailer(b"NAT2", 1))).is_err());
    // An enabled octet that is neither 0 nor 1.
    assert!(bbmd::decode_file(&file_with(&rows, &trailer(b"NAT1", 2))).is_err());
    // A trailer cut short.
    assert!(bbmd::decode_file(&file_with(&rows, &trailer(b"NAT1", 1)[..7])).is_err());
    // One octet.
    assert!(bbmd::decode_file(&[0x01]).is_err());
}
