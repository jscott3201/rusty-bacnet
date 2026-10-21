//! A routed confirmed request names one device on one remote network (#1278).
use super::*;
use crate::discovery::{DiscoveredDevice, RoutedDeviceConfig};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice};
use bacnet_types::primitives::ObjectIdentifier;

const ROUTER: [u8; 6] = [2; 6];
const DADR: [u8; 6] = [3; 6];
const ROUTED_DEVICE: u32 = 7;

/// No network, the global broadcast network, and a remote broadcast.
fn not_one_device() -> [(u16, Vec<u8>); 3] {
    [
        (0, DADR.to_vec()),
        (u16::MAX, DADR.to_vec()),
        (DNET, Vec::new()),
    ]
}

async fn client() -> (
    BACnetClient<CaptureTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<SentFrame>,
) {
    let (transport, inbound, outbound) = harness(&[1; 6], 1490);
    let config = ClientConfig {
        apdu_timeout_ms: 20,
        apdu_retries: 0,
        ..ClientConfig::default()
    };
    let client = BACnetClient::start(config, transport).await.unwrap();
    (client, inbound, outbound)
}

fn object() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// A device-table row behind `ROUTER` at `dnet`/`dadr`, as discovery would
/// hold it, so the `_from_device` and `_to_device` methods route to it.
fn routed_row(dnet: u16, dadr: &[u8]) -> DiscoveredDevice {
    DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, ROUTED_DEVICE).unwrap(),
        mac_address: MacAddr::from_slice(&ROUTER),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
        vendor_id: 0,
        last_seen: std::time::Instant::now(),
        source_network: Some(dnet),
        source_address: Some(MacAddr::from_slice(dadr)),
    }
}

/// DNET 0, DNET 65535 and an empty DADR fail every routed confirmed entry
/// point with `Error::Encoding` before a frame, a TSM entry or a routed-path
/// entry exists, while an unconfirmed remote broadcast still goes out.
#[tokio::test]
async fn routed_confirmed_request_to_no_single_device_fails_before_any_state() {
    let (mut client, _inbound, mut outbound) = client().await;
    for (dnet, dadr) in not_one_device() {
        client
            .device_table
            .lock()
            .await
            .upsert(routed_row(dnet, &dadr));
        let pv = PropertyIdentifier::PRESENT_VALUE;
        let results = [
            client
                .confirmed_request_routed(
                    &ROUTER,
                    dnet,
                    &dadr,
                    ConfirmedServiceChoice::READ_PROPERTY,
                    &[0x0c],
                )
                .await
                .map(drop),
            client
                .read_property_routed(&ROUTER, dnet, &dadr, object(), pv, None)
                .await
                .map(drop),
            client
                .read_property_from_device(ROUTED_DEVICE, object(), pv, None)
                .await
                .map(drop),
            client
                .subscribe_cov_to_device(ROUTED_DEVICE, 1, object(), false, Some(60))
                .await,
        ];
        for result in results {
            assert!(
                matches!(result, Err(Error::Encoding(_))),
                "DNET {dnet}, DADR {dadr:?}: {result:?}"
            );
        }
        assert!(
            outbound.try_recv().is_err(),
            "DNET {dnet}: a frame went out"
        );
        assert_eq!(client.tsm.lock().await.pending_count(), 0);
        assert_eq!(client.routed_path_limits.entry_count(), 0);
    }

    client
        .broadcast_network_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &[], DNET)
        .await
        .unwrap();
    let frame = outbound.try_recv().unwrap();
    let destination = decode_npdu(frame.npdu).unwrap().destination.unwrap();
    assert_eq!(destination.network, DNET);
    assert!(destination.mac_address.is_empty());
    client.stop().await.unwrap();
}

/// The path APIs and the request path share one DNET bound: 0 and 65535 are
/// refused without reserving an entry, while 1 and 65534 are accepted, and a
/// request to DNET 65534 goes out and completes.
#[tokio::test]
async fn path_apis_and_routed_requests_share_the_remote_dnet_bound() {
    let (client, inbound, mut outbound) = client().await;
    for dnet in [0, u16::MAX] {
        let refused = [
            client
                .configure_routed_path_max_npdu(&ROUTER, dnet, 1497)
                .await,
            client.clear_routed_path_limit(&ROUTER, dnet).await,
        ];
        for result in refused {
            assert!(
                matches!(&result, Err(Error::Encoding(m)) if m == "routed DNET must be in 1..=65534"),
                "DNET {dnet}: {result:?}"
            );
        }
    }
    assert_eq!(client.routed_path_limits.entry_count(), 0);
    for dnet in [1, u16::MAX - 1] {
        client
            .configure_routed_path_max_npdu(&ROUTER, dnet, 1497)
            .await
            .unwrap();
        client.clear_routed_path_limit(&ROUTER, dnet).await.unwrap();
    }

    let client = Arc::new(client);
    let highest = u16::MAX - 1;
    let task = routed_request(
        Arc::clone(&client),
        ROUTER.to_vec(),
        highest,
        DADR.to_vec(),
        vec![0x0c],
    );
    let frame = outbound.recv().await.unwrap();
    let destination = decode_npdu(frame.npdu.clone())
        .unwrap()
        .destination
        .unwrap();
    assert_eq!(destination.network, highest);
    let request = confirmed_request(frame, &ROUTER);
    inject_simple_ack(&inbound, &ROUTER, highest, &DADR, request.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

/// `add_routed_device` refuses a peer no routed confirmed request could
/// reach, leaving the table unchanged, and accepts DNET 65534 with an
/// 18-octet MAC.
#[tokio::test]
async fn add_routed_device_refuses_a_destination_no_request_could_reach() {
    let (mut client, _inbound, _outbound) = client().await;
    let config = |remote_network, remote_mac: Vec<u8>| RoutedDeviceConfig {
        instance: ROUTED_DEVICE,
        router_mac: ROUTER.to_vec(),
        remote_network,
        remote_mac,
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        max_segments_accepted: None,
    };
    let too_long = vec![3; bacnet_npdu::MAX_MAC_LEN + 1];
    for (dnet, dadr) in [
        (0, DADR.to_vec()),
        (u16::MAX, DADR.to_vec()),
        (DNET, too_long),
    ] {
        let result = client.add_routed_device(config(dnet, dadr)).await;
        assert!(matches!(result, Err(Error::Encoding(_))), "{result:?}");
        assert!(client.get_device(ROUTED_DEVICE).await.is_none());
    }
    let longest = vec![3; bacnet_npdu::MAX_MAC_LEN];
    client
        .add_routed_device(config(u16::MAX - 1, longest))
        .await
        .unwrap();
    assert!(client.get_device(ROUTED_DEVICE).await.is_some());
    client.stop().await.unwrap();
}
