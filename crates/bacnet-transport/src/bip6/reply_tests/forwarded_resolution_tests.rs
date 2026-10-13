//! Forwarded-Address-Resolution (U.2.5, U.4.4, U.5): which ones this node
//! answers, where the Address-Resolution-ACK goes, what it learns, and the
//! quota the answers count against.

use super::*;

/// The group a normal-mode node takes a BBMD's multicast copy on.
const GROUP: Ipv6Addr = BACNET_IPV6_MULTICAST_SITE_LOCAL;

/// The asker addresses that can't be answered without a scope ID or that
/// reach no single node.
fn unusable_askers() -> [SocketAddrV6; 7] {
    let port = asker().port();
    [
        SocketAddrV6::new("fe80::40".parse().unwrap(), port, 0, 0),
        SocketAddrV6::new(BACNET_IPV6_MULTICAST_SITE_LOCAL, port, 0, 0),
        SocketAddrV6::new("ff02::1".parse().unwrap(), port, 0, 0),
        SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0),
        SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0),
        SocketAddrV6::new("::ffff:192.0.2.1".parse().unwrap(), port, 0, 0),
        SocketAddrV6::new(*asker().ip(), 0, 0, 0),
    ]
}

/// The trusted path for each mode: unicast from the configured BBMD for a
/// foreign device, a BACnet group from a unicast source otherwise.
fn trusted_path(foreign: bool) -> (SocketAddrV6, Ipv6Addr) {
    if foreign {
        (bbmd(), local_ip())
    } else {
        (sender(), GROUP)
    }
}

/// U.5: a foreign device answers a Forwarded-Address-Resolution from its
/// BBMD that names its VMAC, at the asker's address the message carries,
/// and learns the asker.
#[tokio::test]
async fn a_forwarded_address_resolution_from_the_bbmd_is_answered_at_the_asker() {
    let receiver = receiver(true).await;
    assert_eq!(
        reply(
            &receiver,
            forwarded_resolution(LOCAL_VMAC, asker()),
            bbmd(),
            local_ip()
        )
        .await,
        Some(ack_to(asker()))
    );
    assert_eq!(receiver.neighbours.lookup(ASKER_VMAC), Some(asker()));
}

/// U.4.4, U.5: a BBMD multicasts the request into its own domain, and a
/// node that isn't a foreign device answers the copy that names it, from
/// whichever answerable source and on whichever BACnet group it arrives. A
/// BBMD on the link may well send from its link-local address, which the
/// socket reports with the scope it arrived on.
#[tokio::test]
async fn a_multicast_forwarded_address_resolution_is_answered_in_normal_mode() {
    let link_local_bbmd = SocketAddrV6::new("fe80::5".parse().unwrap(), 0xBAC0, 0, 3);
    for group in [
        BACNET_IPV6_MULTICAST_LINK_LOCAL,
        BACNET_IPV6_MULTICAST_SITE_LOCAL,
        BACNET_IPV6_MULTICAST_ORG_LOCAL,
    ] {
        for source in [bbmd(), sender(), link_local_bbmd] {
            let receiver = receiver(false).await;
            assert_eq!(
                reply(
                    &receiver,
                    forwarded_resolution(LOCAL_VMAC, asker()),
                    source,
                    group
                )
                .await,
                Some(ack_to(asker())),
                "from {source} on {group}"
            );
            assert_eq!(receiver.neighbours.lookup(ASKER_VMAC), Some(asker()));
        }
    }
}

/// A foreign device drops every Forwarded-Address-Resolution but a unicast
/// one from its BBMD; a normal-mode node drops every one but a group copy
/// from a unicast source. None of the dropped ones teaches the VMAC table.
#[tokio::test]
async fn forwarded_address_resolutions_from_an_untrusted_path_are_dropped() {
    let ours = forwarded_resolution(LOCAL_VMAC, asker());
    let foreign = receiver(true).await;
    let other_port = SocketAddrV6::new(*bbmd().ip(), bbmd().port() + 1, 0, 0);
    for source in [sender(), asker(), other_port] {
        assert_eq!(
            reply(&foreign, ours, source, local_ip()).await,
            None,
            "foreign, from {source}"
        );
    }
    assert_eq!(
        reply(&foreign, ours, bbmd(), GROUP).await,
        None,
        "foreign, sent to a group"
    );
    let frame = encoded(&ours).unwrap();
    let mut flagged = datagram(&frame, bbmd(), local_ip());
    flagged.os_group_delivery = Some(true);
    assert_eq!(
        foreign.handle_datagram(&frame, &flagged).await,
        None,
        "foreign, to its own address but flagged by the OS as a group delivery"
    );
    assert_eq!(foreign.neighbours.lookup(ASKER_VMAC), None);

    let normal = receiver(false).await;
    assert_eq!(
        reply(&normal, ours, bbmd(), local_ip()).await,
        None,
        "normal, unicast: no BBMD is configured to vouch for it"
    );
    assert_eq!(
        reply(&normal, ours, sender(), "ff02::1".parse().unwrap()).await,
        None,
        "normal, a group that isn't BACnet's"
    );
    let frame = encoded(&ours).unwrap();
    let mut unicast_delivery = datagram(&frame, sender(), GROUP);
    unicast_delivery.os_group_delivery = Some(false);
    assert_eq!(
        normal.handle_datagram(&frame, &unicast_delivery).await,
        None,
        "normal, the OS says it wasn't a group delivery"
    );
    for source in unanswerable_sources() {
        assert_eq!(
            reply(&normal, ours, source, GROUP).await,
            None,
            "normal, from {source}"
        );
    }
    assert_eq!(normal.neighbours.lookup(ASKER_VMAC), None);
}

/// On a trusted path, a Forwarded-Address-Resolution for another VMAC, or
/// one naming an unusable asker address, is dropped and teaches nothing.
#[tokio::test]
async fn trusted_forwarded_address_resolutions_that_are_not_answered() {
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        let (peer, destination) = trusted_path(foreign);
        assert_eq!(
            reply(
                &receiver,
                forwarded_resolution(OTHER_VMAC, asker()),
                peer,
                destination
            )
            .await,
            None,
            "for another VMAC, foreign={foreign}"
        );
        for unusable in unusable_askers() {
            assert_eq!(
                reply(
                    &receiver,
                    forwarded_resolution(LOCAL_VMAC, unusable),
                    peer,
                    destination
                )
                .await,
                None,
                "asker {unusable}, foreign={foreign}"
            );
        }
        assert_eq!(receiver.neighbours.lookup(ASKER_VMAC), None);
        // The same path with this node's VMAC and a usable asker is
        // answered: each refusal above was for the reason it names.
        assert_eq!(
            reply(
                &receiver,
                forwarded_resolution(LOCAL_VMAC, asker()),
                peer,
                destination
            )
            .await,
            Some(ack_to(asker()))
        );
    }
}

/// The answers count against their own quota, per asker address and in
/// total, in both modes; a new window starts it afresh. A held-back answer
/// still leaves the asker learned.
#[tokio::test(start_paused = true)]
async fn forwarded_address_resolution_answers_stop_at_their_quota() {
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        let (peer, destination) = trusted_path(foreign);
        let ours = forwarded_resolution(LOCAL_VMAC, asker());
        for _ in 0..REPLIES_PER_DESTINATION {
            assert!(reply(&receiver, ours, peer, destination).await.is_some());
        }
        assert_eq!(reply(&receiver, ours, peer, destination).await, None);
        let elsewhere = SocketAddrV6::new("2001:db8:0:5::41".parse().unwrap(), 0xBAC0, 0, 0);
        assert!(
            reply(
                &receiver,
                forwarded_resolution(LOCAL_VMAC, elsewhere),
                peer,
                destination
            )
            .await
            .is_some(),
            "another asker has its own"
        );

        tokio::time::advance(REPLY_WINDOW).await;
        let mut admitted = 0;
        for host in 0..=u16::try_from(REPLIES_PER_WINDOW / REPLIES_PER_DESTINATION).unwrap() {
            let asker = SocketAddrV6::new(
                Ipv6Addr::new(0x2001, 0xdb8, 0, 6, 0, 0, 0, host),
                0xBAC0,
                0,
                0,
            );
            for _ in 0..REPLIES_PER_DESTINATION {
                let message = forwarded_resolution(LOCAL_VMAC, asker);
                admitted += u32::from(reply(&receiver, message, peer, destination).await.is_some());
            }
        }
        assert_eq!(admitted, REPLIES_PER_WINDOW, "foreign={foreign}");

        let late = SocketAddrV6::new("2001:db8:0:5::42".parse().unwrap(), 0xBAC0, 0, 0);
        let late_vmac = Bip6Vmac::new([0x00, 0x06, 0x42]);
        let held_back = Bvll6::ForwardedAddressResolution {
            original_source: late_vmac,
            target: LOCAL_VMAC,
            original_address: late.into(),
        };
        assert_eq!(reply(&receiver, held_back, peer, destination).await, None);
        assert_eq!(receiver.neighbours.lookup(late_vmac), Some(late));
    }
}
