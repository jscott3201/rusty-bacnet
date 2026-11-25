//! A local broadcast from the router's application port (#2601): the
//! application is on each of the router's networks, so its broadcast with
//! no DNET goes to every one of them once, as that link's broadcast, and no
//! further. A unicast with no DNET still names no one station.

use bacnet_npdu::Mac;
use bacnet_router_core::PortId;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::time::{Duration, timeout};

use super::super::driven_port::{Handle, Sent, driven};
use super::super::{BACnetRouter, RouterOptions, RouterPort};
use super::{RouterApplicationPort, SourceSelection};

/// A local Who-Is at Urgent priority, as a stack writes it: no DNET.
const LOCAL_WHO_IS: [u8; 4] = [1, 0x01, 0x10, 0x08];

/// A global Who-Is, sent after the one under test: the next APDU on every
/// port, so nothing else was queued before it.
const GLOBAL_WHO_IS: [u8; 8] = [1, 0x20, 0xFF, 0xFF, 0, 255, 0x10, 0x08];

/// The next APDU the router sends on `port`, past its own network messages.
async fn next_apdu(port: &mut Handle) -> Sent {
    timeout(Duration::from_secs(2), async {
        loop {
            let sent = port.sent.recv().await.expect("the port closed");
            if sent.npdu.get(1).is_some_and(|control| control & 0x80 == 0) {
                return sent;
            }
        }
    })
    .await
    .expect("no APDU sent")
}

/// A router on networks 1000, 2000, ... at MACs 1, 2, ..., one port per
/// limit, the ports from `isolated_from` on isolated, and its application
/// port.
async fn router(
    limits: &[u16],
    isolated_from: usize,
) -> (BACnetRouter, RouterApplicationPort, Vec<Handle>) {
    let (ports, handles): (Vec<_>, Vec<_>) = limits
        .iter()
        .enumerate()
        .map(|(index, &limit)| {
            let (transport, handle) = driven(index as u8 + 1, limit);
            let network = (index as u16 + 1) * 1000;
            let port = RouterPort::new(transport, network).isolated(index >= isolated_from);
            (port, handle)
        })
        .unzip();
    let started = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    let (router, port, _controls) = started.into_application_port();
    (router, port, handles)
}

/// Two of the three ports are isolated, and network 4000 lies beyond the
/// ordinary one. The broadcast goes once on each port, isolated or not, as
/// the link's broadcast with no DNET or hop count, at its priority; none
/// goes toward network 4000's router.
#[tokio::test]
async fn a_local_broadcast_reaches_each_attached_network_once() {
    let (mut router, port, mut handles) = router(&[1497; 3], 1).await;
    assert_eq!(port.attached_networks(), [1000, 2000, 3000]);
    let now = router.now();
    assert!(
        router
            .table()
            .add_learned(4000, PortId(0), Mac::from_array([9]), now)
    );

    port.send_broadcast(&LOCAL_WHO_IS).await.unwrap();
    port.send_broadcast(&GLOBAL_WHO_IS).await.unwrap();
    for handle in &mut handles {
        let local = next_apdu(handle).await;
        assert_eq!((local.npdu.as_slice(), local.to), (&LOCAL_WHO_IS[..], None));
        let global = next_apdu(handle).await;
        assert_eq!(global.npdu, GLOBAL_WHO_IS, "one copy per port, no more");
    }
    assert_eq!(router.counters().originated, 4);
    router.stop().await.unwrap();
}

/// From port 0's address, the copies on the other ports name it as SNET
/// and SADR, as the router's other requests from one address do.
#[tokio::test]
async fn from_one_address_the_other_copies_name_it() {
    let (mut router, port, mut handles) = router(&[1497; 2], 2).await;
    let port = port.with_source(SourceSelection::Port(PortId(0)));
    port.send_broadcast(&LOCAL_WHO_IS).await.unwrap();
    assert_eq!(next_apdu(&mut handles[0]).await.npdu, LOCAL_WHO_IS);
    let named = next_apdu(&mut handles[1]).await;
    // SNET 1000 (0x03E8), SLEN 1, SADR 1: the router on port 0.
    assert_eq!(named.npdu, [1, 0x09, 0x03, 0xE8, 1, 1, 0x10, 0x08]);
    assert_eq!(named.to, None);
    router.stop().await.unwrap();
}

/// A copy too long for its port fails the send with the length refusal,
/// and the copies the other ports carry still go: like a global
/// broadcast, the send is not all or nothing.
#[tokio::test]
async fn a_copy_one_port_cannot_carry_fails_the_send_and_the_rest_go() {
    let (mut router, port, mut handles) = router(&[1497, 480], 2).await;
    let mut long = LOCAL_WHO_IS.to_vec();
    long.resize(600, 0);
    let error = port.send_broadcast(&long).await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::SendTooLong {
                length: 600,
                limit: 480
            }
        ),
        "{error:?}"
    );
    port.send_broadcast(&GLOBAL_WHO_IS).await.unwrap();
    assert_eq!(next_apdu(&mut handles[0]).await.npdu, long);
    assert_eq!(next_apdu(&mut handles[0]).await.npdu, GLOBAL_WHO_IS);
    assert_eq!(
        next_apdu(&mut handles[1]).await.npdu,
        GLOBAL_WHO_IS,
        "the refused copy never left"
    );
    router.stop().await.unwrap();
}

/// The shared test transport in front of the port answers its networks, as
/// the trait asks of a wrapper; on its own it has none.
#[tokio::test]
async fn a_wrapper_forwards_the_attached_networks() {
    use bacnet_transport::test_transport::TestTransport;

    let (mut router, port, _handles) = router(&[1497; 2], 1).await;
    let wrapped = TestTransport::builder().inner(port).build();
    assert_eq!(wrapped.attached_networks(), [1000, 2000]);
    assert!(TestTransport::new().attached_networks().is_empty());
    router.stop().await.unwrap();
}

/// A unicast with no DNET names a MAC on no one network: refused, sending
/// nothing.
#[tokio::test]
async fn a_unicast_with_no_network_names_no_station() {
    let (mut router, port, mut handles) = router(&[1497; 2], 2).await;
    let read = [1, 0x04, 0x00, 0x05, 0x01, 0x0C];
    let error = port.send_unicast(&read, &[5]).await.unwrap_err();
    assert!(
        matches!(&error, Error::InvalidArgument(text)
            if text.to_string().contains("address the device by its network")),
        "{error:?}"
    );
    port.send_broadcast(&GLOBAL_WHO_IS).await.unwrap();
    for handle in &mut handles {
        assert_eq!(next_apdu(handle).await.npdu, GLOBAL_WHO_IS);
    }
    router.stop().await.unwrap();
}
