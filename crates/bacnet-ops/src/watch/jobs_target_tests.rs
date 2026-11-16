//! Where a watched point's subscription goes, for every form of a station
//! (#2588): a local one at its MAC, a routed one through the router it
//! names, and a remote one through the router its row names, or no route
//! when it has no row.

use bacnet_client::client::{BACnetClient, CovTarget, Station};
use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::Segmentation;
use bacnet_types::error::Error;

use super::target_of;

#[tokio::test]
async fn every_form_of_a_station_has_its_subscription_target() {
    let client = BACnetClient::generic_builder()
        .transport(TestTransport::new())
        .build()
        .await
        .unwrap();
    assert_eq!(
        target_of(&client, Station::local(&[2])).await.unwrap(),
        CovTarget::local(&[2])
    );
    assert_eq!(
        target_of(&client, Station::routed(&[9], 2, &[5]))
            .await
            .unwrap(),
        CovTarget::routed(&[9], 2, &[5])
    );
    let unknown = target_of(&client, Station::remote(2, &[5])).await;
    assert!(
        matches!(&unknown, Err(Error::UnknownDevice { device }) if device == "2:0x05"),
        "{unknown:?}"
    );
    client
        .add_routed_device(RoutedDeviceConfig {
            instance: 7,
            router_mac: vec![8],
            remote_network: 2,
            remote_mac: vec![5],
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    assert_eq!(
        target_of(&client, Station::remote(2, &[5])).await.unwrap(),
        CovTarget::routed(&[8], 2, &[5]),
        "through the router its row names"
    );
}
