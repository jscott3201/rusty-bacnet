//! What a B/IPv6 transport reports for Network Port registration (#1579),
//! and the registered port's lease, which the socket keeps while any task
//! that owns it runs.

use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;

use bacnet_types::bip_port::BipPortMode;
use bacnet_types::constructed::BACnetHostNPort;
use bacnet_types::enums::ErrorCode;
use bacnet_types::error::{Error, RemoteError};

use super::*;
use crate::port::TransportPort;

const INTERFACE: Ipv6Addr = Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 5);

#[test]
fn an_unstarted_transport_reports_its_configuration() {
    let mut transport = Bip6Transport::new(INTERFACE, 47809, Some(1234));
    let port = transport.bip6_port().unwrap();
    assert_eq!(port.endpoint, SocketAddrV6::new(INTERFACE, 47809, 0, 0));
    // The VMAC of Device 1234 (Clause H.7.2).
    assert_eq!(port.vmac, Some([0x00, 0x04, 0xD2]));
    assert_eq!(port.multicast_group, BACNET_IPV6_MULTICAST_SITE_LOCAL);
    assert!(matches!(port.mode, BipPortMode::Normal));
    assert!(transport.bip_port().is_none());

    transport.set_broadcast_scope(Bip6BroadcastScope::LinkLocal);
    assert_eq!(
        transport.bip6_port().unwrap().multicast_group,
        BACNET_IPV6_MULTICAST_LINK_LOCAL
    );
    // A random VMAC is chosen at start, so none is known before.
    let random = Bip6Transport::new(INTERFACE, 47809, None);
    assert_eq!(random.bip6_port().unwrap().vmac, None);
}

#[test]
fn a_foreign_device_lends_a_registration_that_takes_changes_while_running() {
    let mut transport = Bip6Transport::new(INTERFACE, 47809, Some(1234));
    let bbmd = Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 0xBB);
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: bbmd,
        bbmd_port: 47810,
        ttl: 300,
    });
    let BipPortMode::Foreign { registration } = transport.bip6_port().unwrap().mode else {
        panic!("FOREIGN mode");
    };
    assert_eq!(
        registration.bbmd(),
        BACnetHostNPort::from_socket_addr(SocketAddrV6::new(bbmd, 47810, 0, 0).into())
    );
    assert_eq!(registration.subscription_lifetime(), 300);
    assert!(registration.can_change());
    // Not running yet: nothing knows the device's own address.
    let error = registration.check(&Default::default()).unwrap_err();
    assert!(
        matches!(
            error,
            Error::Remote(RemoteError::Error {
                code: ErrorCode::WRITE_ACCESS_DENIED,
                ..
            })
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_lease_is_retained_once_before_start_and_released_by_stop() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(77));
    let lease = Arc::new(());
    transport
        .retain_network_port_lease_internal(Arc::clone(&lease))
        .unwrap();
    assert!(
        transport
            .retain_network_port_lease_internal(Arc::new(()))
            .is_err()
    );
    let _rx = transport.start().await.unwrap();
    // The started transport reports its bind and VMAC.
    let port = transport.bip6_port().unwrap();
    let bound = transport.local_address().unwrap();
    assert_eq!(
        port.endpoint,
        SocketAddrV6::new(*bound.ip(), bound.port(), 0, 0)
    );
    assert_ne!(port.endpoint.port(), 0);
    assert_eq!(port.vmac, Some([0x00, 0x00, 77]));
    // The transport and its socket hold the lease while it runs.
    assert_eq!(Arc::strong_count(&lease), 3);
    transport.stop().await.unwrap();
    assert_eq!(Arc::strong_count(&lease), 1);
    // A transport that has started takes no lease.
    assert!(
        transport
            .retain_network_port_lease_internal(Arc::new(()))
            .is_err()
    );
}

#[tokio::test]
async fn a_dropped_transport_releases_the_lease_once_its_tasks_end() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(78));
    let lease = Arc::new(());
    transport
        .retain_network_port_lease_internal(Arc::clone(&lease))
        .unwrap();
    let _rx = transport.start().await.unwrap();
    drop(transport);
    // Drop aborts the tasks; each releases its socket as it ends.
    for _ in 0..100 {
        if Arc::strong_count(&lease) == 1 {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("lease still held: {}", Arc::strong_count(&lease));
}

#[test]
fn a_registration_names_an_ipv6_unicast_bbmd_and_a_lifetime() {
    let config = |bbmd_ip: &str, bbmd_port, ttl| Bip6ForeignDeviceConfig {
        bbmd_ip: bbmd_ip.parse().unwrap(),
        bbmd_port,
        ttl,
    };
    for ip in ["::1", "2001:db8::bb", "fd00::bb"] {
        assert_eq!(config(ip, 47808, 60).check(), Ok(()), "{ip}");
    }
    use Bip6ForeignDeviceRefusal as R;
    for (ip, port, ttl, refusal) in [
        ("::", 47808, 60, R::Unspecified),
        ("ff05::bac0", 47808, 60, R::Multicast),
        ("fe80::1", 47808, 60, R::LinkLocal),
        ("::ffff:192.0.2.1", 47808, 60, R::Ipv4Mapped),
        ("::192.0.2.1", 47808, 60, R::Ipv4Compatible),
        ("::1", 0, 60, R::ZeroPort),
        ("::1", 47808, 0, R::ZeroTtl),
    ] {
        assert_eq!(config(ip, port, ttl).check(), Err(refusal), "{ip}");
    }
}

#[tokio::test]
async fn start_refuses_a_registration_that_cannot_work() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(79));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: "fe80::1".parse().unwrap(),
        bbmd_port: 47808,
        ttl: 60,
    });
    let Err(Error::Transport(error)) = transport.start().await else {
        panic!("a link-local BBMD must be refused");
    };
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("link-local"), "{error}");
}
