//! B/IP BBMDs and a foreign device behind NAT routers, on loopback (Annex
//! J.7.5, #940).
//!
//! The layout follows the two-site example of Annex J.7.5 (Figure J-7):
//! each site's BBMD sits on a private socket that its router port-forwards
//! from the site's global address, and both BDTs list the two global
//! addresses with all-ones masks. The routers are `shim`'s: every datagram
//! between sites goes through them and comes out with its addresses
//! translated.

mod shim;

use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy, NatTraversal};
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport, ForeignDeviceConfig};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use shim::{Internet, Router};
use tokio::sync::mpsc;
use tokio::time::timeout;

/// A global-broadcast Who-Is.
const WHO_IS: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
/// An I-Am-shaped unicast reply.
const REPLY: &[u8] = &[0x01, 0x00, 0x10, 0x00, 0xC4, 0x02, 0x00, 0x00, 0x07];

fn address_of(mac: &[u8]) -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]),
        u16::from_be_bytes([mac[4], mac[5]]),
    )
}

fn two_hop(address: SocketAddrV4) -> BdtEntry {
    BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: [255; 4],
    }
}

/// A loopback BBMD whose local broadcasts come back to its own socket,
/// where they are dropped as its own echo: nothing else shares its subnet.
fn loopback(port: u16) -> BipTransport {
    BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST)
}

/// A BBMD behind the router at `global`, alone on its subnet, taking
/// foreign devices, with `bdt`.
async fn bbmd(
    global: SocketAddrV4,
    bdt: &[SocketAddrV4],
) -> (BipTransport, mpsc::Receiver<ReceivedNpdu>, SocketAddrV4) {
    let mut transport = loopback(0);
    transport.set_mode(BipMode::Bbmd(BbmdOptions {
        bdt: bdt.iter().copied().map(two_hop).collect(),
        // Every device here registers from 127.0.0.1, and the fast one
        // several times a second.
        foreign_device_policy: Some(ForeignDevicePolicy {
            registration_rate_per_source: 1_000,
            registration_rate_global: 1_000,
            ..ForeignDevicePolicy::default()
        }),
        alone_on_subnet: true,
        nat: Some(NatTraversal::new(global)),
        ..BbmdOptions::default()
    }));
    let rx = transport.start().await.unwrap();
    let private = address_of(transport.local_mac());
    (transport, rx, private)
}

/// A foreign device behind site `site`'s router, registering with `bbmd`
/// every `renewal`. It binds port 0, so the router learns its address from
/// its first registration ([`Internet::claim_next`]); this returns once it
/// has.
async fn foreign_device(
    internet: &Internet,
    site: usize,
    bbmd: SocketAddrV4,
    renewal: Option<Duration>,
) -> (BipTransport, mpsc::Receiver<ReceivedNpdu>) {
    internet.claim_next(site);
    // Not its own address as the broadcast one: its BBMD's forwards must
    // read as unicast, the only kind a foreign device takes.
    let mut transport =
        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::new(127, 255, 255, 255));
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: *bbmd.ip(),
        bbmd_port: bbmd.port(),
        ttl: 60,
        renewal_interval: renewal,
    });
    let rx = transport.start().await.unwrap();
    timeout(Duration::from_secs(5), async {
        while internet.private(site).is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the device's first registration reaches its router");
    assert_eq!(
        internet.private(site),
        Some(address_of(transport.local_mac()))
    );
    (transport, rx)
}

/// The next NPDU handed up, within `within`.
async fn next(
    rx: &mut mpsc::Receiver<ReceivedNpdu>,
    within: Duration,
    label: &str,
) -> ReceivedNpdu {
    timeout(within, rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{label}: nothing arrived"))
        .expect("channel open")
}

/// Wait until the foreign device's registration is accepted.
async fn registered(transport: &BipTransport) {
    timeout(Duration::from_secs(5), async {
        while !transport
            .bvlc_client_snapshot()
            .foreign_registration
            .registered
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the foreign device registers");
}

/// A broadcast at site A reaches site B and a foreign device registered at
/// B from behind its own router. A Forwarded-NPDU's origin is A's global
/// address, the one B passes on unchanged, so B's answer to it goes back
/// through A's router and arrives from B's global address.
#[tokio::test]
async fn two_sites_and_a_foreign_device_talk_through_their_routers() {
    let internet = Internet::new(&[
        Router::PortForward,
        Router::PortForward,
        Router::Dynamic(Duration::from_secs(60)),
    ])
    .await;
    let (global_a, global_b) = (internet.global(0), internet.global(1));
    let sites = [global_a, global_b];
    let (a, mut a_rx, private_a) = bbmd(global_a, &sites).await;
    let (b, mut b_rx, private_b) = bbmd(global_b, &sites).await;
    internet.place(0, private_a);
    internet.place(1, private_b);
    let (fd, mut fd_rx) =
        foreign_device(&internet, 2, global_b, Some(Duration::from_secs(1))).await;
    registered(&fd).await;

    // B recorded the device under its router's address, not its own.
    let fdt = b.bbmd_state().unwrap().lock().unwrap().fdt();
    let recorded: Vec<_> = fdt
        .iter()
        .map(|entry| SocketAddrV4::new(Ipv4Addr::from(entry.ip), entry.port))
        .collect();
    assert_eq!(recorded, [internet.global(2)]);
    // Each BBMD runs under its global address.
    let (own, bdt) = {
        let state = a.bbmd_state().unwrap().lock().unwrap();
        (state.own_address(), state.bdt())
    };
    assert_eq!(own, global_a);
    assert_eq!(bdt, [two_hop(global_a), two_hop(global_b)]);

    a.send_broadcast(WHO_IS).await.unwrap();
    let at_b = next(&mut b_rx, Duration::from_secs(2), "site B").await;
    assert_eq!(at_b.npdu.as_ref(), WHO_IS);
    assert_eq!(address_of(&at_b.source_mac), global_a, "the global origin");
    let at_fd = next(&mut fd_rx, Duration::from_secs(2), "the foreign device").await;
    assert_eq!(at_fd.npdu.as_ref(), WHO_IS);
    assert_eq!(
        address_of(&at_fd.source_mac),
        global_a,
        "B passed A's forward on with its origin"
    );

    // B answers the origin; the reply reaches A from B's global address.
    b.send_unicast(REPLY, &at_b.source_mac).await.unwrap();
    let reply = next(&mut a_rx, Duration::from_secs(2), "site A").await;
    assert_eq!(reply.npdu.as_ref(), REPLY);
    assert_eq!(address_of(&reply.source_mac), global_b);

    // The foreign device's broadcast: B distributes it to A, with the
    // device's router address as its origin (a public source keeps its own).
    fd.send_broadcast(WHO_IS).await.unwrap();
    let at_a = next(&mut a_rx, Duration::from_secs(2), "site A").await;
    assert_eq!(address_of(&at_a.source_mac), internet.global(2));
}

/// A foreign device behind a router whose mapping lapses after 500 ms keeps
/// receiving the BBMD's broadcasts when it registers every 150 ms; one that
/// registers at the default half time-to-live (30 s) stops receiving them
/// once its mapping lapses (Annex J.7.5). Real time: the shim's mappings
/// and the registration schedule share the host's clock with the sockets.
#[tokio::test]
async fn a_foreign_device_behind_nat_needs_a_short_reregistration_interval() {
    let lapse = Duration::from_millis(500);
    let internet = Internet::new(&[
        Router::PortForward,
        Router::Dynamic(lapse),
        Router::Dynamic(lapse),
    ])
    .await;
    let global_b = internet.global(0);
    let (b, _b_rx, private_b) = bbmd(global_b, &[global_b]).await;
    internet.place(0, private_b);
    let (fast, mut fast_rx) =
        foreign_device(&internet, 1, global_b, Some(Duration::from_millis(150))).await;
    let (slow, mut slow_rx) = foreign_device(&internet, 2, global_b, None).await;
    registered(&fast).await;
    registered(&slow).await;

    // Three times the mapping lapse, and long before the slow device
    // renews.
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    b.send_broadcast(WHO_IS).await.unwrap();
    let at_fast = next(&mut fast_rx, Duration::from_secs(2), "the fast device").await;
    assert_eq!(at_fast.npdu.as_ref(), WHO_IS);
    assert_eq!(address_of(&at_fast.source_mac), global_b);
    assert!(
        timeout(Duration::from_secs(1), slow_rx.recv())
            .await
            .is_err(),
        "the slow device's mapping lapsed"
    );
    assert!(internet.counts().expired >= 1);
    let renewals = fast.bvlc_client_snapshot().foreign_registration;
    // What keeps the mapping open is the device's own traffic: an attempt
    // every 150 ms.
    assert!(renewals.attempts >= 5, "{renewals:?}");
    assert_eq!(renewals.refused, 0, "{renewals:?}");
    drop(slow);
}

/// With NAT set, a BDT that doesn't list the global address can't start.
#[tokio::test]
async fn a_bdt_without_the_global_address_fails_start() {
    let global = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 1), 47809);
    let peer = SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 2), 47809);
    let mut transport = loopback(0);
    transport.set_mode(BipMode::Bbmd(BbmdOptions {
        bdt: vec![two_hop(peer)],
        nat: Some(NatTraversal::new(global)),
        ..BbmdOptions::default()
    }));
    let error = transport.start().await.unwrap_err();
    assert!(error.to_string().contains("global address"), "{error}");
    let mut one_hop = two_hop(peer);
    one_hop.broadcast_mask = [255, 255, 255, 0];
    transport.set_mode(BipMode::Bbmd(BbmdOptions {
        bdt: vec![two_hop(global), one_hop],
        nat: Some(NatTraversal::new(global)),
        ..BbmdOptions::default()
    }));
    let error = transport.start().await.unwrap_err();
    assert!(error.to_string().contains("all ones"), "{error}");
}
