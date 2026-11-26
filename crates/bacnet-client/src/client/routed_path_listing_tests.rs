//! The routed-path listing and the configured limit's lower bound (#2667):
//! [`BACnetClient::routed_path_limits`] names each path's limit and where it
//! comes from, and `configure_routed_path_max_npdu` refuses a limit that
//! could carry no request at all.

use std::sync::Arc;

use bacnet_types::MacAddr;
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;
use tokio::time::{Duration, sleep};

use super::routed_path_answered_tests::{LARGE, Path, too_long};
use super::routed_path_limit_tests::{DNET, confirmed_request, harness};
use super::{BACnetClient, ClientConfig, RoutedPathLimit, RoutedPathLimitSource, Station};

pub(super) const ROUTER: [u8; 1] = [2];

pub(super) fn listed(
    configured: Option<u16>,
    learned: Option<u16>,
    below_minimum_reject_seen: bool,
) -> RoutedPathLimit {
    RoutedPathLimit {
        router_mac: MacAddr::from_slice(&ROUTER),
        dnet: DNET,
        configured_max_npdu: configured,
        learned_max_npdu: learned,
        below_minimum_reject_seen,
    }
}

/// The boundary: on a link with `local_mac`, a limit one octet under 50
/// octets of APDU after the smallest forwarded NPCI (10 octets and the
/// local MAC) is refused before anything is reserved, and the bound itself
/// is taken and carries a request to a one-octet station. Below the bound
/// is refused at every size; a limit under 228 above it is taken.
#[tokio::test]
async fn a_limit_that_can_carry_no_request_is_refused_at_configure_time() {
    for (local_mac, smallest) in [(vec![1; 6], 66u16), (vec![1], 61)] {
        let (transport, _inbound, mut outbound) = harness(&local_mac, 1490);
        let mut client = BACnetClient::generic_builder()
            .transport(transport)
            .build()
            .await
            .unwrap();
        for refused in [0, 1, smallest - 1] {
            let result = client.configure_routed_path_max_npdu(&ROUTER, DNET, refused);
            assert!(
                matches!(&result, Err(Error::InvalidArgument(message))
                    if message.to_string().contains(&format!("give at least {smallest}"))),
                "{refused}: {result:?}"
            );
        }
        assert!(client.routed_path_limits().is_empty(), "nothing reserved");

        client
            .configure_routed_path_max_npdu(&ROUTER, DNET, smallest)
            .unwrap();
        assert_eq!(
            client.routed_path_limits(),
            [listed(Some(smallest), None, false)]
        );
        let request = {
            let client = &client;
            async move {
                client
                    .confirmed_request(
                        &Station::routed(&ROUTER, DNET, &[3]),
                        ConfirmedServiceChoice::READ_PROPERTY,
                        &[0x0c],
                    )
                    .await
            }
        };
        // The request goes out (and then times out unanswered); a limit
        // under the bound would have refused it before the send.
        let sent = tokio::select! {
            sent = outbound.recv() => sent.unwrap(),
            result = request => panic!("refused before the send: {result:?}"),
        };
        confirmed_request(sent, &ROUTER);
        client.stop().await.unwrap();
    }
}

/// What the listing holds: a configured limit, a learned one below it
/// after a reason 4 the client believed, and a configure that clears the
/// learned one. Clearing the limit leaves nothing listed.
#[tokio::test(start_paused = true)]
async fn the_listing_names_configured_and_learned_limits() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(1497), None, false)]
    );
    let [only] = path.client.routed_path_limits().try_into().unwrap();
    assert_eq!(
        (only.max_npdu(), only.source()),
        (1497, RoutedPathLimitSource::Configured)
    );

    // A 315-octet forwarded NPDU refused as too long: 314 is learned.
    let task = path.request(LARGE, 0x41);
    path.next_frame().await;
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    let [only] = path.client.routed_path_limits().try_into().unwrap();
    assert_eq!(only, listed(Some(1497), Some(314), false));
    assert_eq!(
        (only.max_npdu(), only.source()),
        (314, RoutedPathLimitSource::Learned)
    );

    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 480)
        .unwrap();
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(480), None, false)]
    );
    path.client.clear_routed_path_limit(&ROUTER, DNET).unwrap();
    assert!(path.client.routed_path_limits().is_empty());
    path.stop().await;
}

/// A reason 4 dropped below the floor lists the path with the default
/// limit and the flag, which a configured limit then resets.
#[tokio::test(start_paused = true)]
async fn a_below_minimum_reject_lists_its_path_until_the_limit_is_set() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    assert!(path.client.routed_path_limits().is_empty());
    let task = path.request(1, 0x41);
    let frame = path.next_frame().await;
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    let [only] = path.client.routed_path_limits().try_into().unwrap();
    assert_eq!(only, listed(None, None, true));
    assert_eq!(
        (only.max_npdu(), only.source()),
        (228, RoutedPathLimitSource::Default)
    );
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());

    path.client
        .configure_routed_path_max_npdu(&ROUTER, DNET, 128)
        .unwrap();
    assert_eq!(
        path.client.routed_path_limits(),
        [listed(Some(128), None, false)]
    );
    path.stop().await;
}

/// Several paths come in router, then network, order.
#[tokio::test]
async fn the_listing_is_ordered_by_router_then_network() {
    let (transport, _inbound, _outbound) = harness(&[1], 1490);
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(transport)
            .build()
            .await
            .unwrap(),
    );
    for (router, dnet) in [([9u8], 2u16), ([2], 7), ([2], 3)] {
        client
            .configure_routed_path_max_npdu(&router, dnet, 300)
            .unwrap();
    }
    let order: Vec<(Vec<u8>, u16)> = client
        .routed_path_limits()
        .into_iter()
        .map(|path| (path.router_mac.to_vec(), path.dnet))
        .collect();
    assert_eq!(order, [(vec![2], 3), (vec![2], 7), (vec![9], 2)]);
    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

/// Clearing a path the table doesn't hold reserves nothing, so it succeeds
/// on a full table instead of failing for capacity; a bad router MAC is
/// still refused.
#[tokio::test]
async fn clearing_an_unknown_path_reserves_nothing() {
    use super::routed_path_limits::MAX_ROUTED_PATH_ENTRIES;
    let (transport, _inbound, _outbound) = harness(&[1], 1490);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    for path in 1..=MAX_ROUTED_PATH_ENTRIES {
        let path = u16::try_from(path).unwrap();
        client
            .configure_routed_path_max_npdu(&path.to_be_bytes(), path, 1497)
            .unwrap();
    }
    client.clear_routed_path_limit(&[0xff, 0xfe], 300).unwrap();
    assert_eq!(
        client.routed_path_limits.entry_count(),
        MAX_ROUTED_PATH_ENTRIES
    );
    assert!(matches!(
        client.clear_routed_path_limit(&[], 300),
        Err(Error::InvalidArgument(_))
    ));
    client.stop().await.unwrap();
}
