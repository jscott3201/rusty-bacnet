//! A B/IPv6 foreign device whose configured BBMD is a host name (#2365):
//! a start on `::` looks it up before binding and fails without an
//! address; a start on an explicit interface waits for it, refusing
//! broadcasts and trying the name again 30 s after each failure, and
//! registers once it resolves.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::bip_port::{BipPortMode, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use tokio::net::UdpSocket;
use tokio::sync::Notify;

use super::foreign_retarget_tests::{
    address_of, bbmd, changes, in_real_time, next_within, registration,
};
use super::*;
use crate::bbmd_lookup::table::{Answer, Names};
use crate::port::TransportPort;

const NAME: &str = "bbmd.test";

/// A foreign device of `NAME` at `peer`'s port, on `interface`, its names
/// looked up in `names`; not started.
fn device(interface: Ipv6Addr, peer: &UdpSocket, names: &Arc<Names>) -> Bip6Transport {
    let mut transport = Bip6Transport::new(interface, 0, Some(61));
    transport.set_bbmd_resolver(names.clone());
    let bbmd = format!("{NAME}:{}", peer.local_addr().unwrap().port());
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd: bbmd.parse().unwrap(),
        ttl: 300,
    });
    transport
}

fn status(transport: &Bip6Transport) -> Bip6ForeignStatus {
    transport.foreign_status().expect("a foreign device")
}

fn lent(transport: &Bip6Transport) -> Arc<dyn ForeignRegistration> {
    let BipPortMode::Foreign { registration } = transport.bip6_port().unwrap().mode else {
        panic!("FOREIGN mode");
    };
    registration
}

/// Let the paused clock run `ms` milliseconds, and the lookups with it.
async fn after(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// A resolvable name registers with the BBMD it stands for, the row reads
/// back the name, and the configuration the address in use.
#[tokio::test]
async fn a_configured_name_registers_where_it_resolves() {
    let peer = bbmd().await;
    let names = Arc::new(Names::default());
    names.answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    let mut transport = device(Ipv6Addr::LOCALHOST, &peer, &names);
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&peer).await, 300);
    let status = status(&transport);
    let port = peer.local_addr().unwrap().port();
    assert_eq!(status.bbmd.map(|at| at.port()), Some(port));
    assert!(!status.waiting_for_bbmd);
    assert_eq!((status.lookups.lookups, status.lookups.failures), (1, 0));
    assert_eq!(status.registrations_sent, 1);
    assert_eq!(
        status.bbmd_name,
        Some(format!("{NAME}:{port}").parse().unwrap())
    );
    let config = transport.foreign_device_config().unwrap();
    assert_eq!(config.bbmd.address(), status.bbmd);
    assert_eq!(
        lent(&transport).bbmd().host,
        BACnetHostAddress::Name(NAME.into())
    );
    transport.stop().await.unwrap();
}

/// On `::` the start looks the name up before it binds, since the socket's
/// source is the route's to the BBMD, and binds there.
#[tokio::test]
async fn a_start_on_the_unspecified_address_looks_the_name_up_first() {
    let peer = bbmd().await;
    let names = Arc::new(Names::default());
    names.answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    let mut transport = device(Ipv6Addr::UNSPECIFIED, &peer, &names);
    let _rx = transport.start().await.unwrap();
    assert_eq!(registration(&peer).await, 300);
    assert_eq!(
        *transport.local_address().unwrap().ip(),
        Ipv6Addr::LOCALHOST
    );
    // The start's lookup was the first pass's: one call, and the start
    // registered at the address it found.
    assert_eq!((names.calls(), status(&transport).lookups.lookups), (1, 1));
    assert_eq!(status(&transport).registrations_sent, 1);
    transport.stop().await.unwrap();
}

/// On `::` a name with no usable IPv6 address fails the start, saying why,
/// and counts the lookup.
#[tokio::test]
async fn a_start_on_the_unspecified_address_fails_without_an_address() {
    let peer = bbmd().await;
    let names = Arc::new(Names::default());
    // IPv4 only, as B/IP would take it.
    names.answer(NAME, Answer::addresses([Ipv4Addr::LOCALHOST]));
    let mut transport = device(Ipv6Addr::UNSPECIFIED, &peer, &names);
    let error = transport.start().await.unwrap_err().to_string();
    assert!(error.contains(NAME), "{error}");
    assert!(error.contains("the name has no IPv6 address"), "{error}");
    assert!(error.contains("explicit interface"), "{error}");
    let status = status(&transport);
    assert_eq!((status.lookups.lookups, status.lookups.failures), (1, 1));
    assert!(!status.waiting_for_bbmd);
    assert!(transport.local_address().is_none());
}

/// On an explicit interface a name that fails still starts the device,
/// which waits: no registration goes, broadcasts are refused and counted,
/// and the name is tried again 30 s after each failure until it resolves,
/// when the device registers there at once.
#[tokio::test(start_paused = true)]
async fn a_failing_name_waits_and_registers_once_it_resolves() {
    let peer = bbmd().await;
    let names = Arc::new(Names::default());
    let mut transport = device(Ipv6Addr::LOCALHOST, &peer, &names);
    transport.set_release_previous_bbmd(true);
    let _rx = transport.start().await.unwrap();
    after(1).await;
    let waiting = status(&transport);
    assert!(waiting.waiting_for_bbmd);
    assert_eq!(waiting.bbmd, None);
    assert_eq!((waiting.lookups.lookups, waiting.lookups.failures), (1, 1));
    let refused = transport.send_broadcast(&[0x01, 0x00]).await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(status(&transport).broadcasts_refused, 1);
    assert_eq!(status(&transport).registrations_sent, 0);

    after(29_000).await;
    assert_eq!(names.calls(), 1);
    after(1_100).await;
    assert_eq!(names.calls(), 2);
    let mut probe = [0; 64];
    assert!(
        peer.try_recv_from(&mut probe).is_err(),
        "nothing registered"
    );

    names.answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    after(30_000).await;
    assert_eq!(names.calls(), 3);
    // The clock is held while the datagram arrives: a jump to the renewal
    // would send a second registration before the count below.
    let long = Duration::from_secs(3600);
    let first = next_within(&peer, long, |message| match message {
        Bvll6::RegisterForeignDevice { ttl, .. } => Some(ttl),
        _ => None,
    });
    assert_eq!(in_real_time(first).await, 300);
    let resolved = status(&transport);
    assert!(!resolved.waiting_for_bbmd);
    assert_eq!(
        (resolved.lookups.lookups, resolved.lookups.failures),
        (3, 2)
    );
    // The device had no BBMD to release.
    assert_eq!(resolved.releases.releases, 0);
    assert_eq!(resolved.registrations_sent, 1);
    transport.send_broadcast(&[0x01, 0x00]).await.unwrap();
    transport.stop().await.unwrap();
}

/// The retry follows the newest failure: a name written inside the
/// window fails and moves it 30 s past that failure, as does a restart's
/// first lookup; a stop cancels it.
#[tokio::test(start_paused = true)]
async fn the_retry_follows_the_newest_failure_and_a_stop_cancels_it() {
    let peer = bbmd().await;
    let names = Arc::new(Names::default());
    let mut transport = device(Ipv6Addr::LOCALHOST, &peer, &names);
    let _rx = transport.start().await.unwrap();
    after(20_000).await;
    assert_eq!(names.calls(), 1);
    // A name written at 20 s fails at once: the retry moves to 50 s.
    let other = BACnetHostNPort {
        host: BACnetHostAddress::Name("other.test".into()),
        port: 47808,
    };
    lent(&transport)
        .activate(&changes(Some(other), None))
        .unwrap();
    after(1).await;
    assert_eq!(names.calls(), 2);
    // The device waits for the name written now, and a lifetime may be
    // written while it waits.
    let config = transport.foreign_device_config().unwrap();
    assert_eq!(config.bbmd, "other.test:47808".parse().unwrap());
    assert!(lent(&transport).check(&changes(None, Some(120))).is_ok());
    after(11_000).await;
    assert_eq!(names.calls(), 2, "no retry at 30 s");
    after(19_100).await;
    assert_eq!(names.calls(), 3, "the retry at 50 s");

    // A restart at 60 s looks the name up at once; its failure sets the
    // retry at 90 s.
    after(9_900).await;
    transport.stop().await.unwrap();
    let _rx = transport.start().await.unwrap();
    after(1).await;
    assert_eq!(names.calls(), 4);
    after(25_000).await;
    assert_eq!(names.calls(), 4, "no retry from before the restart");
    after(5_100).await;
    assert_eq!(names.calls(), 5, "the retry at 90 s");

    // A stop cancels the retry.
    after(10_000).await;
    transport.stop().await.unwrap();
    after(60_000).await;
    assert_eq!(names.calls(), 5);
}

/// An address written while the start's lookup runs wins: the device
/// registers there, and the lookup's late answer moves nothing.
#[tokio::test]
async fn an_address_written_during_the_start_lookup_wins() {
    let peer = bbmd().await;
    let written = bbmd().await;
    let names = Arc::new(Names::default());
    let gate = Arc::new(Notify::new());
    names.answer(NAME, Answer::gated(&gate, [Ipv6Addr::LOCALHOST]));
    let mut transport = device(Ipv6Addr::LOCALHOST, &peer, &names);
    let _rx = transport.start().await.unwrap();
    lent(&transport)
        .activate(&changes(Some(address_of(&written)), None))
        .unwrap();
    assert_eq!(registration(&written).await, 300);
    gate.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let status = status(&transport);
    assert_eq!(
        status.bbmd.map(|at| at.port()),
        Some(written.local_addr().unwrap().port())
    );
    assert!(!status.waiting_for_bbmd);
    let mut probe = [0; 64];
    assert!(
        peer.try_recv_from(&mut probe).is_err(),
        "nothing went to the name's BBMD"
    );
    transport.stop().await.unwrap();
}

/// A name written while the start's lookup runs wins too: the configured
/// name's late answer is dropped uncounted, the written name is looked up
/// once it ends, and the device registers where that one resolves.
#[tokio::test]
async fn a_name_written_during_the_start_lookup_wins() {
    let peer = bbmd().await;
    let written = bbmd().await;
    let names = Arc::new(Names::default());
    let gate = Arc::new(Notify::new());
    names.answer(NAME, Answer::gated(&gate, [Ipv6Addr::LOCALHOST]));
    names.answer("other.test", Answer::addresses([Ipv6Addr::LOCALHOST]));
    let mut transport = device(Ipv6Addr::LOCALHOST, &peer, &names);
    let _rx = transport.start().await.unwrap();
    let other = BACnetHostNPort {
        host: BACnetHostAddress::Name("other.test".into()),
        port: written.local_addr().unwrap().port(),
    };
    lent(&transport)
        .activate(&changes(Some(other), None))
        .unwrap();
    assert!(status(&transport).waiting_for_bbmd);
    gate.notify_one();
    assert_eq!(registration(&written).await, 300);
    let status = status(&transport);
    assert_eq!(
        status.bbmd.map(|at| at.port()),
        Some(written.local_addr().unwrap().port())
    );
    assert_eq!((names.calls(), status.lookups.lookups), (2, 1));
    let mut probe = [0; 64];
    assert!(
        peer.try_recv_from(&mut probe).is_err(),
        "nothing went to the configured name's BBMD"
    );
    transport.stop().await.unwrap();
}

/// Only the retry still armed acts: one a newer failure replaced while it
/// woke leaves the newer one in place and looks nothing up.
#[tokio::test(start_paused = true)]
async fn only_the_armed_retry_acts() {
    let names = Arc::new(Names::default());
    let config = Bip6ForeignDeviceConfig {
        bbmd: format!("{NAME}:47808").parse().unwrap(),
        ttl: 60,
    };
    let target = foreign_target::ForeignTarget::new(config, names.clone());
    target.attach("[::1]:47809".parse().unwrap());
    target.attempting();
    after(1).await;
    assert_eq!(names.calls(), 1);
    let armed = target.retry_due().expect("a failure arms the retry");
    target.retry(armed - Duration::from_secs(5));
    after(1).await;
    assert_eq!(names.calls(), 1, "a replaced retry looks nothing up");
    assert_eq!(target.retry_due(), Some(armed));
    target.retry(armed);
    after(1).await;
    assert_eq!(names.calls(), 2);
    target.detach();
}

/// A start on `::` that fails after its lookup, here at the bind, keeps
/// the BBMD configured by name: the next start looks it up again, and
/// registers where it resolves.
#[tokio::test]
async fn a_failed_start_on_the_unspecified_address_keeps_the_name() {
    let peer = bbmd().await;
    let names = Arc::new(Names::default());
    names.answer(NAME, Answer::addresses([Ipv6Addr::LOCALHOST]));
    // The port the device binds, held exclusively elsewhere.
    let held = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .unwrap();
    crate::port_ownership::claim_exclusive(&held).unwrap();
    held.bind(&std::net::SocketAddr::from((Ipv6Addr::LOCALHOST, 0)).into())
        .unwrap();
    let port = held.local_addr().unwrap().as_socket().unwrap().port();
    let mut transport = Bip6Transport::new(Ipv6Addr::UNSPECIFIED, port, Some(61));
    transport.set_bbmd_resolver(names.clone());
    let bbmd = format!("{NAME}:{}", peer.local_addr().unwrap().port());
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd: bbmd.parse().unwrap(),
        ttl: 300,
    });
    transport.start().await.unwrap_err();
    assert_eq!(names.calls(), 1);
    let config = transport.foreign_device_config().unwrap();
    assert_eq!(config.bbmd, bbmd.parse().unwrap(), "still the name");
    drop(held);
    let _rx = transport.start().await.unwrap();
    assert_eq!(names.calls(), 2);
    assert_eq!(registration(&peer).await, 300);
    assert!(
        transport
            .foreign_device_config()
            .unwrap()
            .bbmd
            .address()
            .is_some()
    );
    transport.stop().await.unwrap();
}

/// A configured name is checked for its form and port, and its address
/// once looked up.
#[test]
fn a_configured_name_is_checked_for_its_form() {
    let config = |host: &str, port, ttl| Bip6ForeignDeviceConfig {
        bbmd: Bip6ForeignBbmd::Name {
            host: host.into(),
            port,
        },
        ttl,
    };
    assert_eq!(config(NAME, 47808, 60).check(), Ok(()));
    assert_eq!(
        config("bbmd test", 47808, 60).check(),
        Err(Bip6ForeignDeviceRefusal::HostName)
    );
    assert_eq!(
        config(NAME, 0, 60).check(),
        Err(Bip6ForeignDeviceRefusal::ZeroPort)
    );
    assert_eq!(
        config(NAME, 47808, 0).check(),
        Err(Bip6ForeignDeviceRefusal::ZeroTtl)
    );
}
