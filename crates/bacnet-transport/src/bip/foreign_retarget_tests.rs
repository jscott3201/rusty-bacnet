//! A running foreign device moved to another BBMD and time-to-live through
//! the registration it lends the Network Port object (#1825).

use super::*;
use bacnet_types::bip_port::ForeignDeviceChanges;
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;
use std::net::SocketAddrV4;

/// Real loopback: generous, since nothing in these tests waits on a timer
/// (the 60 s time-to-live renews at 30 s).
const WAIT: Duration = Duration::from_secs(10);

async fn bbmd() -> UdpSocket {
    UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()
}

fn foreign_to(peer: &UdpSocket, ttl: u16) -> ForeignDeviceConfig {
    ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: peer.local_addr().unwrap().port(),
        ttl,
        renewal_interval: None,
    }
}

fn address_of(peer: &UdpSocket) -> BACnetHostNPort {
    BACnetHostNPort::from_socket_addr(peer.local_addr().unwrap())
}

/// The Register-Foreign-Device `peer` receives next: its time-to-live and
/// where it came from.
async fn registration(peer: &UdpSocket) -> (u16, std::net::SocketAddr) {
    let mut packet = [0; 64];
    let (len, source) = tokio::time::timeout(WAIT, peer.recv_from(&mut packet))
        .await
        .expect("a registration")
        .unwrap();
    assert_eq!(&packet[..4], &[0x81, 5, 0, 6], "{:?}", &packet[..len]);
    (u16::from_be_bytes([packet[4], packet[5]]), source)
}

async fn observed(
    transport: &BipTransport,
    predicate: impl Fn(&ForeignRegistrationSnapshot) -> bool,
) -> ForeignRegistrationSnapshot {
    tokio::time::timeout(WAIT, async {
        loop {
            let snapshot = transport.bvlc_client_snapshot().foreign_registration;
            if predicate(&snapshot) {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("registration state did not arrive")
}

fn lent(transport: &BipTransport) -> Arc<dyn ForeignRegistration> {
    match transport.bip_port().unwrap().mode {
        BipPortMode::Foreign { registration } => registration,
        other => panic!("{other:?}"),
    }
}

fn changes(bbmd: Option<BACnetHostNPort>, ttl: Option<u16>) -> ForeignDeviceChanges {
    let mut changes = ForeignDeviceChanges::default();
    changes.bbmd = bbmd;
    changes.subscription_lifetime = ttl;
    changes
}

fn is_out_of_range(error: &Error) -> bool {
    matches!(error, Error::Remote(RemoteError::Error { class, code })
        if *class == ErrorClass::PROPERTY && *code == ErrorCode::VALUE_OUT_OF_RANGE)
}

/// Registered with the first BBMD, the device moves: its next registration
/// goes to the second with the new time-to-live, and the status the TUI and
/// CLI read follows it.
#[tokio::test]
async fn activation_sends_the_next_registration_to_the_new_bbmd_with_the_new_ttl() {
    let (first, second) = (bbmd().await, bbmd().await);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let _rx = transport.start().await.unwrap();
    let (ttl, source) = registration(&first).await;
    assert_eq!(ttl, 60);
    first.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
    observed(&transport, |s| s.registered).await;

    let registration_view = lent(&transport);
    let moved = changes(Some(address_of(&second)), Some(120));
    registration_view.check(&moved).unwrap();
    registration_view.activate(&moved).unwrap();
    assert_eq!(registration_view.bbmd(), address_of(&second));
    assert_eq!(registration_view.subscription_lifetime(), 120);
    assert_eq!(transport.foreign_device_config().unwrap().ttl, 120);

    let (ttl, source) = registration(&second).await;
    assert_eq!(ttl, 120);
    let second_addr = match second.local_addr().unwrap() {
        std::net::SocketAddr::V4(address) => address,
        other => panic!("{other}"),
    };
    // The status moved with it: the newest attempt is to the new BBMD, and
    // the old BBMD's lease no longer counts.
    let moving = observed(&transport, |s| s.last_bbmd == Some(second_addr)).await;
    assert_eq!(moving.last_ttl, Some(120));
    assert_eq!(
        moving.last_outcome,
        Some(ForeignRegistrationOutcome::Pending)
    );
    assert!(!moving.registered);
    assert_eq!(moving.lease_remaining, None);
    second
        .send_to(&[0x81, 0, 0, 6, 0, 0], source)
        .await
        .unwrap();
    let registered = observed(&transport, |s| s.registered).await;
    assert!(registered.lease_remaining.unwrap() > Duration::from_secs(60));
    assert_eq!(registered.accepted, 2);
    // Broadcasts follow: the link the send path reads moved too.
    transport.send_broadcast(&[0x01, 0x00]).await.unwrap();
    // A retried registration on a slow runner may come first; skip it.
    tokio::time::timeout(WAIT, async {
        let mut packet = [0; 64];
        loop {
            let (len, _) = second.recv_from(&mut packet).await.unwrap();
            if packet[1] == 0x09 {
                assert_eq!(&packet[..len], &[0x81, 0x09, 0, 6, 0x01, 0x00]);
                return;
            }
        }
    })
    .await
    .expect("a distributed broadcast");

    // A restart keeps the values it moved to.
    transport.stop().await.unwrap();
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&second).await.0, 120);
    transport.stop().await.unwrap();
}

/// A value the device couldn't register with is refused by the check and
/// by activation, and changes neither what it reports nor where it
/// registers.
#[tokio::test]
async fn a_refused_value_leaves_the_registration_untouched() {
    let first = bbmd().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let _rx = transport.start().await.unwrap();
    registration(&first).await;
    let view = lent(&transport);
    let before = transport.bvlc_client_snapshot().foreign_registration;
    let own = BACnetHostNPort::from_socket_addr(
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, transport.port).into(),
    );
    let host = |host, port| BACnetHostNPort { host, port };
    let refused = [
        changes(Some(own), None),
        changes(
            Some(host(BACnetHostAddress::Name("bbmd.example".into()), 47808)),
            None,
        ),
        changes(Some(host(BACnetHostAddress::None, 47808)), None),
        changes(
            Some(host(BACnetHostAddress::Ip("::1".parse().unwrap()), 47808)),
            None,
        ),
        changes(
            Some(host(
                BACnetHostAddress::Ip(Ipv4Addr::UNSPECIFIED.into()),
                47808,
            )),
            None,
        ),
        changes(
            Some(host(
                BACnetHostAddress::Ip(Ipv4Addr::BROADCAST.into()),
                47808,
            )),
            None,
        ),
        changes(
            Some(host(BACnetHostAddress::Ip(Ipv4Addr::LOCALHOST.into()), 0)),
            None,
        ),
        changes(None, Some(0)),
    ];
    for refused in refused {
        let error = view.check(&refused).unwrap_err();
        assert!(is_out_of_range(&error), "{refused:?}: {error:?}");
        let error = view.activate(&refused).unwrap_err();
        assert!(is_out_of_range(&error), "{refused:?}: {error:?}");
    }
    assert_eq!(view.bbmd(), address_of(&first));
    assert_eq!(view.subscription_lifetime(), 60);
    assert_eq!(
        transport.foreign_device_config(),
        Some(foreign_to(&first, 60))
    );
    let after = transport.bvlc_client_snapshot().foreign_registration;
    assert_eq!(
        (after.attempts, after.last_bbmd, after.last_ttl),
        (before.attempts, before.last_bbmd, before.last_ttl)
    );
    transport.stop().await.unwrap();
}

/// A client can't see the configured renewal interval, so a lifetime it
/// doesn't fit under cuts it to half the lifetime instead of being refused.
#[tokio::test]
async fn a_lifetime_shorter_than_the_configured_renewal_cuts_the_renewal() {
    let first = bbmd().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut config = foreign_to(&first, 60);
    config.renewal_interval = Some(Duration::from_secs(20));
    transport.register_as_foreign_device(config);
    let _rx = transport.start().await.unwrap();
    let view = lent(&transport);
    view.activate(&changes(None, Some(100))).unwrap();
    let kept = transport.foreign_device_config().unwrap();
    assert_eq!(kept.renewal_interval, Some(Duration::from_secs(20)));
    view.activate(&changes(None, Some(20))).unwrap();
    let cut = transport.foreign_device_config().unwrap();
    assert_eq!(
        (cut.ttl, cut.renewal_interval),
        (20, Some(Duration::from_secs(10)))
    );
    // A longer lifetime again restores the configured interval.
    view.activate(&changes(None, Some(100))).unwrap();
    let restored = transport.foreign_device_config().unwrap();
    assert_eq!(
        (restored.ttl, restored.renewal_interval),
        (100, Some(Duration::from_secs(20)))
    );
    transport.stop().await.unwrap();
}

/// Only a running transport takes a change: a stopped one couldn't check a
/// new BBMD against its own address, so the change is refused and nothing
/// moves.
#[tokio::test]
async fn a_change_needs_a_running_transport() {
    let (first, second) = (bbmd().await, bbmd().await);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let moved = changes(Some(address_of(&second)), Some(90));
    let refused = |error: Error| {
        assert!(
            matches!(error, Error::Remote(RemoteError::Error { class, code })
                if class == ErrorClass::PROPERTY && code == ErrorCode::WRITE_ACCESS_DENIED),
            "{error:?}"
        );
    };
    refused(lent(&transport).check(&moved).unwrap_err());
    refused(lent(&transport).activate(&moved).unwrap_err());
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&first).await.0, 60);
    transport.stop().await.unwrap();
    refused(lent(&transport).activate(&moved).unwrap_err());
    assert_eq!(
        transport.foreign_device_config(),
        Some(foreign_to(&first, 60))
    );
}

/// A registration changed between a start's read of it and the node's
/// attachment goes to the node at once.
#[tokio::test]
async fn a_change_racing_a_start_reaches_the_new_node() {
    let (first, second) = (bbmd().await, bbmd().await);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let _rx = transport.start().await.unwrap();
    registration(&first).await;
    let target = Arc::clone(transport.foreign_device.as_ref().unwrap());
    let node = Arc::clone(transport.node.as_ref().unwrap());
    // As if the node had been built from the first BBMD's configuration
    // while an activation moved it to the second.
    target.edit(|config| *config = foreign_to(&second, 90));
    target.attach(&node, &foreign_to(&first, 60)).unwrap();
    assert_eq!(registration(&second).await.0, 90);
    transport.stop().await.unwrap();
}

/// A broadcast whose link was read before a move goes to the BBMD the node
/// decided under, not the one it was encoded for.
#[tokio::test]
async fn a_broadcast_racing_a_move_goes_to_the_new_bbmd() {
    let sink = send_sink::SendSink::default();
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let own = match socket.local_addr().unwrap() {
        std::net::SocketAddr::V4(own) => own,
        other => panic!("{other}"),
    };
    let socket = Arc::new(BipSocket::recording(socket, sink.clone()));
    // No hold, so the broadcast is sent at once after the move too.
    let foreign = |port| {
        let config = ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port: port,
            ttl: 60,
            renewal_interval: None,
        };
        let mut engine = config.engine(config.interval().unwrap());
        engine.hold = 0;
        engine
    };
    let link = test_support::test_link(
        own,
        own,
        bacnet_bip_core::node::Mode::Foreign(foreign(47901)),
    );
    let node =
        Arc::new(node::NodeHandle::new(link, socket, Arc::default(), Arc::default()).unwrap());
    let mover = Arc::clone(&node);
    *node.before_admit.lock().unwrap() = Some(Box::new(move || {
        mover.retarget(foreign(47902)).unwrap();
    }));
    node.send_broadcast(&[0x01, 0x00], None).await.unwrap();
    let sent = sink.take();
    let broadcast = sent
        .iter()
        .find(|sent| sent.bytes.get(1) == Some(&0x09))
        .expect("the broadcast");
    assert_eq!(broadcast.to, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47902));
}

/// The address `transport` is bound to.
fn own_address(transport: &BipTransport) -> ([u8; 4], u16) {
    decode_bip_mac(transport.local_mac()).unwrap()
}

/// With the release on, a move asks the BBMD the device left to delete its
/// entry: a real BBMD's FDT no longer lists the device, the success is
/// counted, and the device registers with the new BBMD as it would without
/// the release (#1852). With its access list empty, the default, the same
/// BBMD would refuse the deletion: one reason the release is off by default.
#[tokio::test]
async fn a_move_releases_the_old_bbmds_entry_when_asked() {
    let mut old = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    old.enable_bbmd(vec![]);
    old.enable_foreign_device_registration(ForeignDevicePolicy::default());
    // This stack's BBMD lets only the senders its access list names delete
    // a registration.
    old.set_bbmd_management_acl(vec![Ipv4Addr::LOCALHOST.octets()]);
    let _old_rx = old.start().await.unwrap();
    let (_, old_port) = own_address(&old);
    let second = bbmd().await;
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: old_port,
        ttl: 60,
        renewal_interval: None,
    });
    transport.set_release_previous_bbmd(true);
    let _rx = transport.start().await.unwrap();
    observed(&transport, |s| s.registered).await;
    let (ip, port) = own_address(&transport);
    let listed = || {
        old.bbmd_state()
            .unwrap()
            .lock()
            .unwrap()
            .is_registered_foreign_device(ip, port)
    };
    assert!(listed());

    lent(&transport)
        .activate(&changes(Some(address_of(&second)), None))
        .unwrap();
    registration(&second).await;
    let released = observed(&transport, |s| s.previous_released == 1).await;
    assert_eq!(released.previous_releases, 1);
    assert_eq!(
        (
            released.previous_release_refused,
            released.previous_release_unanswered
        ),
        (0, 0)
    );
    assert!(!listed(), "the old BBMD still lists the device");
    transport.stop().await.unwrap();
    old.stop().await.unwrap();
}

/// A NAK, or silence, from the BBMD the device left is counted and fails
/// nothing: the device still registers with the new BBMD. A lifetime-only
/// change moves nowhere, so it asks nothing, and with the release off a
/// move asks nothing either.
#[tokio::test]
async fn a_refused_or_unanswered_release_is_counted_and_the_move_stands() {
    for nak in [true, false] {
        let (first, second) = (bbmd().await, bbmd().await);
        let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        transport.register_as_foreign_device(foreign_to(&first, 60));
        transport.set_release_previous_bbmd(true);
        let _rx = transport.start().await.unwrap();
        let (_, source) = registration(&first).await;
        first.send_to(&[0x81, 0, 0, 6, 0, 0], source).await.unwrap();
        observed(&transport, |s| s.registered).await;
        lent(&transport).activate(&changes(None, Some(90))).unwrap();
        lent(&transport)
            .activate(&changes(Some(address_of(&second)), None))
            .unwrap();

        // The old BBMD is asked to delete the device's own entry; a
        // registration the lifetime change sent may come first.
        let (ip, port) = own_address(&transport);
        let mut packet = [0; 64];
        let from = tokio::time::timeout(WAIT, async {
            loop {
                let (len, from) = first.recv_from(&mut packet).await.unwrap();
                if packet[1] == 0x08 {
                    assert_eq!(len, 10);
                    return from;
                }
            }
        })
        .await
        .expect("a Delete-Foreign-Device-Table-Entry");
        assert_eq!(&packet[..4], &[0x81, 0x08, 0, 10]);
        assert_eq!(packet[4..8], ip);
        assert_eq!(packet[8..10], port.to_be_bytes());
        let counted = if nak {
            first
                .send_to(&[0x81, 0, 0, 6, 0, 0x50], from)
                .await
                .unwrap();
            observed(&transport, |s| s.previous_release_refused == 1).await
        } else {
            tokio::time::pause();
            let counted = observed(&transport, |s| s.previous_release_unanswered == 1).await;
            tokio::time::resume();
            counted
        };
        assert_eq!(
            (counted.previous_releases, counted.previous_released),
            (1, 0)
        );
        assert_eq!(registration(&second).await.0, 90, "nak: {nak}");
        transport.stop().await.unwrap();
    }

    let (first, second) = (bbmd().await, bbmd().await);
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    transport.register_as_foreign_device(foreign_to(&first, 60));
    let _rx = transport.start().await.unwrap();
    lent(&transport)
        .activate(&changes(Some(address_of(&second)), None))
        .unwrap();
    registration(&second).await;
    let snapshot = transport.bvlc_client_snapshot();
    assert_eq!(snapshot.foreign_registration.previous_releases, 0);
    assert_eq!(snapshot.delete_fdt_entry.sent, 0);
    transport.stop().await.unwrap();
}
