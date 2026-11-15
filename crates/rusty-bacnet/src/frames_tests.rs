use super::*;
use std::future::Future;
use std::task::{Context, Waker};

#[tokio::test(start_paused = true)]
async fn owner_close_releases_receiver_and_wakes_pending_read() {
    let tap = bacnet_transport::tap::FrameTap::new();
    let owner = FrameSubscriptions::default();
    let iterator = owner.subscribe(tap.subscribe(), "local".into(), Clock::fixed());
    let (pending, _hook) = iterator.subscription.stream.next();
    tokio::pin!(pending);
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(tap.counters().subscribers, 1);
    owner.close();
    assert_eq!(tap.counters().subscribers, 0);
    assert!(pending.await.is_none());
    let fresh = owner.subscribe(tap.subscribe(), "new generation".into(), Clock::fixed());
    assert!(iterator.subscription.stream.next().0.await.is_none());
    assert_eq!(tap.counters().subscribers, 1);
    drop(fresh);
    assert_eq!(tap.counters().subscribers, 0);
}

#[tokio::test(start_paused = true)]
async fn cancelling_native_wait_keeps_subscription_until_iterator_drop() {
    let tap = bacnet_transport::tap::FrameTap::new();
    let owner = FrameSubscriptions::default();
    let iterator = owner.subscribe(tap.subscribe(), "local".into(), Clock::fixed());
    {
        let (pending, _hook) = iterator.subscription.stream.next();
        tokio::pin!(pending);
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(tap.counters().subscribers, 1);
    drop(iterator);
    assert_eq!(tap.counters().subscribers, 0);
}

/// `Frame.link` is the `transport=` token, and a link no Python owner runs
/// reads as the documented fallback.
#[test]
fn every_link_reads_as_its_transport_token() {
    let tokens = [
        (DataLink::Bip, "bip"),
        (DataLink::Bip6, "ipv6"),
        (DataLink::Sc, "sc"),
        (DataLink::Mstp, "mstp"),
        (DataLink::Ethernet, "other"),
        (DataLink::Loopback, "other"),
    ];
    for (link, token) in tokens {
        assert_eq!(link_token(link), token, "{link}");
    }
}

/// Each link's `local_address` and `peer` share one form: socket addresses
/// on B/IP and B/IPv6, decimal stations on MS/TP, colon hex for MACs. SC's
/// peer is the hub connection, named as `ScEvent.hub` names it.
#[test]
fn each_link_writes_local_and_peer_addresses_alike() {
    use bacnet_transport::sc::ScActiveHub;
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

    let local = |link, mac: &[u8], socket| local_address(link, mac, socket).map(|a| a.to_string());
    let v4 = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 7), 47808));
    assert_eq!(
        local(DataLink::Bip, &[], Some(v4)).unwrap(),
        "192.0.2.7:47808"
    );
    assert_eq!(peer_text(TapPeer::Udp(v4)), "192.0.2.7:47808");
    assert_eq!(
        local(DataLink::Bip, &[192, 0, 2, 7, 0xBA, 0xC0], None),
        None
    );

    let v6 = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 47808, 0, 0));
    let scoped = SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 47809, 0, 3));
    // The socket address, never the VMAC in the MAC.
    assert_eq!(
        local(DataLink::Bip6, &[0, 4, 0xD2], Some(v6)).unwrap(),
        "[::1]:47808"
    );
    assert_eq!(peer_text(TapPeer::Udp(v6)), "[::1]:47808");
    assert_eq!(
        local(DataLink::Bip6, &[], Some(scoped)).unwrap(),
        "[fe80::1%3]:47809"
    );
    assert_eq!(peer_text(TapPeer::Udp(scoped)), "[fe80::1%3]:47809");
    assert_eq!(local(DataLink::Bip6, &[0, 4, 0xD2], None), None);

    assert_eq!(local(DataLink::Mstp, &[3], None).unwrap(), "3");
    assert_eq!(peer_text(TapPeer::Mstp(3)), "3");
    assert_eq!(peer_text(TapPeer::Mstp(255)), "255");
    assert_eq!(local(DataLink::Mstp, &[], None), None);
    assert_eq!(local(DataLink::Mstp, &[0, 3], None), None);

    let vmac = [0x02, 0, 0, 0, 0, 0x2A];
    assert_eq!(
        local(DataLink::Sc, &vmac, None).unwrap(),
        "02:00:00:00:00:2a"
    );
    assert_eq!(peer_text(TapPeer::ScHub(ScActiveHub::Primary)), "primary");
    assert_eq!(peer_text(TapPeer::ScHub(ScActiveHub::Failover)), "failover");
    assert_eq!(local(DataLink::Sc, &[], None), None);

    let mac = [0x00, 0x1B, 0x63, 0x84, 0x45, 0xE6];
    assert_eq!(
        local(DataLink::Ethernet, &mac, None).unwrap(),
        "00:1b:63:84:45:e6"
    );
    assert_eq!(peer_text(TapPeer::Ethernet(mac)), "00:1b:63:84:45:e6");
}

/// The repr names the frame without its octets.
#[test]
fn a_frame_repr_is_short() {
    let tap = bacnet_transport::tap::FrameTap::new();
    let mut receiver = tap.subscribe();
    let octets = [0x81; 600];
    tap_record(&tap, TapDirection::Inbound, &octets);
    let frame = PyFrame {
        frame: receiver.try_recv().unwrap(),
        local_address: "127.0.0.1:47808".into(),
        missed_before: 0,
        monotonic: 0.0,
    };
    assert_eq!(
        frame.__repr__(),
        "Frame(direction='inbound', link='bip', peer='192.0.2.9:47808', length=600)"
    );
}

/// What a B/IP transport would hand its tap, from 192.0.2.9:47808.
fn tap_record(tap: &bacnet_transport::tap::FrameTap, direction: TapDirection, octets: &[u8]) {
    tap.record_for_test(
        direction,
        DataLink::Bip,
        TapPeer::Udp(SocketAddr::from(([192, 0, 2, 9], 47808))),
        octets,
    );
}
