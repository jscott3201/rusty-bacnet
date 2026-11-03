//! `CovPolicy::validate` and where the server applies it (#1100).
use super::*;
use bacnet_encoding::npdu::NpduAddress;
use bacnet_objects::database::ObjectDatabase;
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::error::Error;

use crate::server::test_transport::TestTransport;
use crate::server::{BACnetServer, ServerConfig};

fn routed(network: u16, mac: &[u8]) -> CovRecipient {
    CovRecipient::Routed(NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    })
}

fn direct(mac: &[u8]) -> CovRecipient {
    CovRecipient::Direct(MacAddr::from_slice(mac))
}

#[test]
fn cov_policy_validate_accepts_defaults_zero_reservations_and_clamped_relations() {
    // The longest source the network layer delivers (#1199).
    let long_mac = [7; BACnetAddress::MAX_MAC_LEN];
    for policy in [
        CovPolicy::default(),
        CovPolicy::unlimited(),
        CovPolicy {
            reserved_capacity: 0,
            max_indefinite_per_peer: 0,
            allow_indefinite_subscriptions: false,
            ..CovPolicy::default()
        },
        // Relations between caps are clamped by `sanitized`, not refused.
        CovPolicy {
            max_subscriptions_global: 1,
            max_subscriptions_per_peer: 1,
            reserved_capacity: 64,
            max_indefinite_per_peer: 16,
            ..CovPolicy::default()
        },
        CovPolicy {
            max_subscriptions_global: 1,
            max_subscriptions_per_peer: 1,
            max_notifications_per_event: 1,
            max_notification_bytes_per_event: 1,
            max_confirmed_in_flight_per_peer: 1,
            reserved_peers: vec![MacAddr::from_slice(&[1]), MacAddr::from_slice(&long_mac)],
            reserved_recipients: vec![
                direct(&[1]),
                direct(&long_mac),
                routed(1, &[1]),
                routed(65534, &long_mac),
            ],
            ..CovPolicy::default()
        },
    ] {
        assert!(policy.validate().is_ok(), "{policy:?}");
    }
}

#[test]
fn cov_policy_validate_rejects_zero_limits_and_unmatchable_reservations() {
    let defaults = CovPolicy::default;
    for (policy, message) in [
        (
            CovPolicy {
                max_subscriptions_global: 0,
                ..defaults()
            },
            "COV policy max_subscriptions_global must be positive",
        ),
        (
            CovPolicy {
                max_subscriptions_per_peer: 0,
                ..defaults()
            },
            "COV policy max_subscriptions_per_peer must be positive",
        ),
        (
            CovPolicy {
                max_notifications_per_event: 0,
                ..defaults()
            },
            "COV policy max_notifications_per_event must be positive",
        ),
        (
            CovPolicy {
                max_notification_bytes_per_event: 0,
                ..defaults()
            },
            "COV policy max_notification_bytes_per_event must be positive",
        ),
        (
            CovPolicy {
                max_confirmed_in_flight_per_peer: 0,
                ..defaults()
            },
            "COV policy max_confirmed_in_flight_per_peer must be positive",
        ),
        (
            CovPolicy {
                reserved_peers: vec![MacAddr::new()],
                ..defaults()
            },
            "COV policy reserved_peers entries need a MAC of 1..=18 octets",
        ),
        (
            CovPolicy {
                reserved_peers: vec![MacAddr::from_slice(&[7; 256])],
                ..defaults()
            },
            "COV policy reserved_peers entries need a MAC of 1..=18 octets",
        ),
        (
            CovPolicy {
                reserved_recipients: vec![direct(&[])],
                ..defaults()
            },
            "COV policy reserved_recipients entries need a MAC of 1..=18 octets",
        ),
        (
            CovPolicy {
                reserved_recipients: vec![routed(10, &[])],
                ..defaults()
            },
            "COV policy reserved_recipients entries need a MAC of 1..=18 octets",
        ),
        (
            CovPolicy {
                reserved_recipients: vec![routed(10, &[7; 256])],
                ..defaults()
            },
            "COV policy reserved_recipients entries need a MAC of 1..=18 octets",
        ),
        (
            CovPolicy {
                reserved_recipients: vec![routed(0, &[1])],
                ..defaults()
            },
            "COV policy reserved_recipients networks must be 1..=65534",
        ),
        (
            CovPolicy {
                reserved_recipients: vec![routed(0xFFFF, &[1])],
                ..defaults()
            },
            "COV policy reserved_recipients networks must be 1..=65534",
        ),
    ] {
        assert!(
            matches!(policy.validate(), Err(Error::Encoding(m)) if m == message),
            "{policy:?}"
        );
    }
}

/// A reserved entry holds to [`BACnetAddress::MAX_MAC_LEN`], the longest
/// source the network layer delivers, in every form (#1199): 18 octets are
/// accepted and 19 refused, with the error the DCC source restriction uses.
#[test]
fn cov_policy_reserved_macs_hold_to_the_bacnet_address_bound() {
    let entries = |mac: &[u8]| {
        [
            CovPolicy {
                reserved_peers: vec![MacAddr::from_slice(mac)],
                ..CovPolicy::default()
            },
            CovPolicy {
                reserved_recipients: vec![direct(mac)],
                ..CovPolicy::default()
            },
            CovPolicy {
                reserved_recipients: vec![routed(10, mac)],
                ..CovPolicy::default()
            },
        ]
    };
    for policy in entries(&[7; BACnetAddress::MAX_MAC_LEN]) {
        assert!(policy.validate().is_ok(), "{policy:?}");
    }
    for policy in entries(&[7; BACnetAddress::MAX_MAC_LEN + 1]) {
        assert!(
            matches!(policy.validate(), Err(Error::Encoding(m)) if m.ends_with("need a MAC of 1..=18 octets")),
            "{policy:?}"
        );
    }
}

/// Every start path refuses an invalid policy before the transport starts:
/// `TestTransport::never_start` panics if it is started.
#[tokio::test]
async fn invalid_cov_policy_fails_start_before_transport() {
    let policy = CovPolicy {
        max_notifications_per_event: 0,
        ..CovPolicy::default()
    };
    let direct = BACnetServer::start(
        ServerConfig {
            cov_policy: policy.clone(),
            ..Default::default()
        },
        ObjectDatabase::new(),
        TestTransport::never_start(),
    )
    .await;
    let generic = BACnetServer::generic_builder()
        .transport(TestTransport::never_start())
        .cov_policy(policy.clone())
        .build()
        .await;
    let bip = BACnetServer::bip_builder()
        .interface(std::net::Ipv4Addr::LOCALHOST)
        .port(0)
        .cov_policy(policy)
        .build()
        .await;
    for error in [direct.err(), generic.err(), bip.err()] {
        assert!(
            matches!(&error, Some(Error::Encoding(m)) if m.contains("max_notifications_per_event")),
            "{error:?}"
        );
    }
}
