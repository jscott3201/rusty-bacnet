//! A BBMD that serves a B/IP-M group (Annex J.8.3, #941): it lends its
//! group to the Network Port through a view that can't change it alone
//! (its tables view changes it, multicast/persist_tests.rs), it refuses a BDT whose own row
//! isn't two-hop, and on one host it joins a group's members, a plain peer
//! BBMD and a foreign device into one network.
// The loopback tests run on Linux and macOS only; elsewhere their helpers
// go unused. Windows multicast is untested (multicast.rs).
#![cfg_attr(
    not(any(target_os = "linux", target_vendor = "apple")),
    allow(dead_code, unused_imports)
)]

use super::*;
use bacnet_types::enums::ErrorCode;
use bacnet_types::error::RemoteError;
use tokio::time::timeout;

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 94, 7);
const TWO_HOP: [u8; 4] = [255, 255, 255, 255];

fn group(port: u16) -> BipMulticast {
    BipMulticast::new(GROUP).unwrap().with_port(port).unwrap()
}

/// A BDT row for 127.0.0.1 at `port` with `mask`.
fn row(port: u16, mask: [u8; 4]) -> BdtEntry {
    BdtEntry {
        ip: [127, 0, 0, 1],
        port,
        broadcast_mask: mask,
    }
}

#[test]
fn a_bbmd_with_an_interface_lends_its_group() {
    // At the group's port, as Windows requires.
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 47809, Ipv4Addr::LOCALHOST);
    bbmd.enable_bbmd(Vec::new());
    let view = bbmd.bip_port().unwrap().multicast.expect("lent");
    assert_eq!(view.group(), [0; 4], "off");
    // Read-only before start too, so the PICS and writes agree.
    assert!(!view.can_change());
    bbmd.set_multicast(Some(group(47809))).unwrap();
    assert_eq!(view.group(), GROUP.octets());
    assert!(matches!(
        bbmd.bip_port().unwrap().mode,
        BipPortMode::Bbmd { .. }
    ));
    // A BBMD on the wildcard address has no interface to join a group on.
    let mut wildcard = BipTransport::new(Ipv4Addr::UNSPECIFIED, 47809, Ipv4Addr::BROADCAST);
    wildcard.enable_bbmd(Vec::new());
    assert!(wildcard.bip_port().unwrap().multicast.is_none());
    assert!(wildcard.set_multicast(Some(group(47809))).is_err());
}

/// A UDP port no socket holds now.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
fn free_port() -> u16 {
    std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn a_bbmd_for_a_group_needs_a_two_hop_own_row_and_keeps_its_group() {
    let (own_port, group_port) = (free_port(), free_port());
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, own_port, Ipv4Addr::LOCALHOST);
    bbmd.set_multicast(Some(group(group_port))).unwrap();
    bbmd.enable_bbmd(vec![row(own_port, [255, 0, 0, 0])]);
    let error = bbmd.start().await.unwrap_err();
    assert!(error.to_string().contains("all-ones"), "{error}");
    // The configuration is kept for another try, which takes a two-hop row.
    bbmd.enable_bbmd(vec![row(own_port, TWO_HOP)]);
    let _rx = bbmd.start().await.unwrap();
    let state = Arc::clone(bbmd.bbmd_state().unwrap());
    assert!(bbmd::lock(&state).serves_group());
    // A one-hop own row is refused from now on.
    let refused = bbmd::lock(&state).set_bdt(vec![row(own_port, [255, 0, 0, 0])]);
    assert!(refused.is_err());

    // The Network Port reads the group, and can't change it through this
    // view: it changes only with the BBMD's tables (#1963).
    let view = bbmd.bip_port().unwrap().multicast.expect("lent");
    assert_eq!(view.group(), GROUP.octets());
    assert!(!view.can_change());
    for group in [[239, 255, 94, 8], [0; 4]] {
        let Err(Error::Remote(RemoteError::Error { code, .. })) = view.check(group) else {
            panic!("refused");
        };
        assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED);
        assert!(view.activate(group).is_err());
    }
    assert_eq!(bbmd.multicast().map(|m| m.group()), Some(GROUP));

    // Restarted without a group, it is a plain BBMD again.
    bbmd.stop().await.unwrap();
    bbmd.set_multicast(None).unwrap();
    let _rx = bbmd.start().await.unwrap();
    assert!(!bbmd::lock(&state).serves_group());
    bbmd.stop().await.unwrap();
}

/// One node of the loopback network, and what it hears.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
struct Node {
    name: &'static str,
    transport: BipTransport,
    rx: mpsc::Receiver<ReceivedNpdu>,
}

#[cfg(any(target_os = "linux", target_vendor = "apple"))]
impl Node {
    async fn start(name: &'static str, mut transport: BipTransport) -> Self {
        let rx = transport.start().await.unwrap();
        Self {
            name,
            transport,
            rx,
        }
    }

    fn mac(&self) -> Vec<u8> {
        self.transport.local_mac().to_vec()
    }

    /// The next NPDU heard must be `npdu`, from `source`, by broadcast.
    async fn hears(&mut self, npdu: &[u8], source: &[u8]) {
        let heard = timeout(Duration::from_secs(2), self.rx.recv())
            .await
            .unwrap_or_else(|_| {
                let drops = self.transport.receive_counters();
                panic!("{} heard nothing; drops: {drops:?}", self.name)
            })
            .expect("receive channel open");
        assert_eq!(&heard.npdu[..], npdu, "{}", self.name);
        assert_eq!(&heard.source_mac[..], source, "{}", self.name);
        assert!(heard.link_layer_group, "{}", self.name);
    }

    /// Nothing more arrives for a while.
    async fn hears_nothing(&mut self) {
        let heard = timeout(Duration::from_millis(300), self.rx.recv()).await;
        assert!(heard.is_err(), "{}: unexpected NPDU {heard:?}", self.name);
    }
}

/// A Who-Is, an I-Am and a third broadcast, told apart.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const FROM_MEMBER: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const FROM_PEER: &[u8] = &[0x01, 0x00, 0x10, 0x08, 0x09, 0x01];
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
const FROM_FOREIGN: &[u8] = &[0x01, 0x00, 0x10, 0x08, 0x09, 0x02];

/// J.8.3 on one host: a BBMD serving a group, two members, a plain peer
/// BBMD reached two-hop, and a foreign device registered with the group's
/// BBMD. Each broadcast reaches every other node once.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn a_group_bbmd_joins_members_a_plain_peer_and_a_foreign_device() {
    let (group_port, bbmd_port, peer_port) = (free_port(), free_port(), free_port());
    let multicast = group(group_port);

    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, bbmd_port, Ipv4Addr::LOCALHOST);
    bbmd.set_multicast(Some(multicast)).unwrap();
    bbmd.enable_bbmd(vec![row(bbmd_port, TWO_HOP), row(peer_port, TWO_HOP)]);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let mut bbmd = Node::start("group BBMD", bbmd).await;

    let mut peer = BipTransport::new(Ipv4Addr::LOCALHOST, peer_port, Ipv4Addr::LOCALHOST);
    peer.enable_bbmd(vec![row(peer_port, TWO_HOP), row(bbmd_port, TWO_HOP)]);
    let mut peer = Node::start("plain peer", peer).await;

    let mut members = Vec::new();
    for name in ["member A", "member B"] {
        let mut member = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
        member.set_multicast(Some(multicast)).unwrap();
        members.push(Node::start(name, member).await);
    }
    let [mut a, mut b] = <[Node; 2]>::try_from(members).ok().unwrap();

    // A broadcast address other than its own: a loopback test's own address
    // as its broadcast one would make every unicast a broadcast too, and a
    // foreign device takes its BBMD's forwards only by unicast.
    let loopback_broadcast = Ipv4Addr::new(127, 255, 255, 255);
    let mut foreign = BipTransport::new(Ipv4Addr::LOCALHOST, 0, loopback_broadcast);
    foreign.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port,
        ttl: 60,
        renewal_interval: None,
    });
    let mut foreign = Node::start("foreign device", foreign).await;
    let state = Arc::clone(bbmd.transport.bbmd_state().unwrap());
    timeout(Duration::from_secs(2), async {
        while bbmd::lock(&state).fdt().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the foreign device registers");

    // A member's broadcast: the other member hears it through the group,
    // the BBMD forwards it to its peer and its foreign device.
    a.transport.send_broadcast(FROM_MEMBER).await.unwrap();
    let source = a.mac();
    for node in [&mut b, &mut bbmd, &mut peer, &mut foreign] {
        node.hears(FROM_MEMBER, &source).await;
    }

    // The peer's broadcast reaches the BBMD two-hop, which passes it to the
    // group and its foreign device.
    peer.transport.send_broadcast(FROM_PEER).await.unwrap();
    let source = peer.mac();
    for node in [&mut a, &mut b, &mut bbmd, &mut foreign] {
        node.hears(FROM_PEER, &source).await;
    }

    // The foreign device's broadcast goes to the group and the peer.
    foreign
        .transport
        .send_broadcast(FROM_FOREIGN)
        .await
        .unwrap();
    let source = foreign.mac();
    for node in [&mut a, &mut b, &mut bbmd, &mut peer] {
        node.hears(FROM_FOREIGN, &source).await;
    }

    // No node heard anything twice, or its own broadcast.
    for node in [&mut a, &mut b, &mut bbmd, &mut peer, &mut foreign] {
        node.hears_nothing().await;
    }
    assert!(bbmd.transport.receive_counters().own_echo >= 1);
    for node in [a, b, bbmd, peer, foreign] {
        let mut transport = node.transport;
        transport.stop().await.unwrap();
    }
}

/// A BDT peer at the BBMD's own port on its own IP subnet is outside J.8.1
/// for a group BBMD, and start warns about it; a plain BBMD, or one whose
/// mask isn't known, says nothing.
#[test]
fn start_warns_about_a_peer_on_the_group_bbmds_subnet() {
    let mut state = BbmdState::new([192, 0, 2, 10], 47808);
    let peer = |ip: [u8; 4], port| BdtEntry {
        ip,
        port,
        broadcast_mask: TWO_HOP,
    };
    state
        .set_bdt(vec![
            peer([192, 0, 2, 20], 47808),
            peer([192, 0, 2, 30], 47809),
            peer([198, 51, 100, 1], 47808),
        ])
        .unwrap();
    let mask = Some(Ipv4Addr::new(255, 255, 255, 0));
    assert_eq!(warn_about_peers_on_the_group_subnet(&state, mask), 0);
    state.set_serves_group(true).unwrap();
    assert_eq!(warn_about_peers_on_the_group_subnet(&state, mask), 1);
    assert_eq!(warn_about_peers_on_the_group_subnet(&state, None), 0);
}

/// A persisted BDT whose own row is one-hop is invalid for a group BBMD:
/// start falls back to the configured BDT, as for any invalid file.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn a_persisted_one_hop_own_row_falls_back_to_the_configured_bdt() {
    let (own_port, group_port) = (free_port(), free_port());
    let path = std::env::temp_dir().join(format!(
        "rusty-bacnet-group-bbmd-{}-{own_port}.bdt",
        std::process::id()
    ));
    let mut wire = bytes::BytesMut::new();
    bbmd::encode_bdt_entries(&[row(own_port, [255, 0, 0, 0])], &mut wire);
    std::fs::write(&path, &wire).unwrap();

    let peer = row(47900, TWO_HOP);
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, own_port, Ipv4Addr::LOCALHOST);
    bbmd.set_multicast(Some(group(group_port))).unwrap();
    bbmd.enable_bbmd(vec![row(own_port, TWO_HOP), peer.clone()]);
    bbmd.set_bdt_persist_path(path.clone());
    let started = bbmd.start().await;
    let _ = std::fs::remove_file(&path);
    let _rx = started.expect("the configured BDT is used");
    let bdt = bbmd::lock(bbmd.bbmd_state().unwrap()).bdt();
    assert_eq!(bdt, [row(own_port, TWO_HOP), peer]);
    bbmd.stop().await.unwrap();
}

/// A one-hop own row set while the group was off is refused when a later
/// start adds the group.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
#[tokio::test]
async fn a_start_that_adds_a_group_refuses_a_one_hop_own_row() {
    let (own_port, group_port) = (free_port(), free_port());
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, own_port, Ipv4Addr::LOCALHOST);
    bbmd.enable_bbmd(Vec::new());
    let _rx = bbmd.start().await.unwrap();
    let state = Arc::clone(bbmd.bbmd_state().unwrap());
    bbmd::lock(&state)
        .set_bdt(vec![row(own_port, [255, 0, 0, 0])])
        .unwrap();
    bbmd.stop().await.unwrap();

    bbmd.set_multicast(Some(group(group_port))).unwrap();
    let error = bbmd.start().await.unwrap_err();
    assert!(error.to_string().contains("all-ones"), "{error}");
    assert!(!bbmd::lock(&state).serves_group());
}
