//! The high-level client's view of routed path limits (#2667): decoded
//! paths, set and clear, and the path a routed device handle is reached by.

use bacnet_client::client::RoutedPathLimitSource;
use bacnet_client::discovery::RoutedDeviceConfig;
use bacnet_types::enums::Segmentation;

use super::*;
use crate::device::DeviceAddress;
use crate::device::test_bus::{Bus, client, mac};

/// A path set by hand lists with its router in the link's own form, its
/// source and both counters; clearing it lists nothing. A path not listed
/// reads as the default.
#[tokio::test]
async fn a_limit_set_and_cleared_shows_decoded() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let path = client.routed_path_at("10.0.0.254:47808", 5).await.unwrap();
    assert_eq!(path.router_mac.as_slice(), mac(254));
    assert_eq!(path.to_string(), "10.0.0.254:47808/5");
    assert_eq!(
        client.routed_paths(),
        RoutedPaths {
            paths: Vec::new(),
            too_long_reject_drops: 0,
            below_minimum_reject_drops: 0,
        }
    );
    let default = client.routed_paths().status(&path);
    assert_eq!(
        (default.max_npdu, default.source),
        (DEFAULT_ROUTED_PATH_MAX_NPDU, RoutedPathLimitSource::Default)
    );

    client.set_routed_path_limit(&path, 128).unwrap();
    let shown = client.routed_paths();
    let status = RoutedPathStatus {
        path: path.clone(),
        max_npdu: 128,
        source: RoutedPathLimitSource::Configured,
        configured_max_npdu: Some(128),
        learned_max_npdu: None,
        below_minimum_reject_seen: false,
    };
    assert_eq!(shown.paths, std::slice::from_ref(&status));
    assert_eq!(shown.status(&path), status);
    assert_eq!(
        serde_json::to_value(&shown).unwrap(),
        serde_json::json!({
            "paths": [{
                "router": "10.0.0.254:47808",
                "network": 5,
                "max_npdu": 128,
                "source": "configured",
                "configured_max_npdu": 128,
                "learned_max_npdu": null,
                "below_minimum_reject_seen": false,
            }],
            "too_long_reject_drops": 0,
            "below_minimum_reject_drops": 0,
        })
    );

    client.clear_routed_path_limit(&path).unwrap();
    assert!(client.routed_paths().paths.is_empty());
}

/// Refusals: a limit that carries no request and a network outside 1 to
/// 65534 are parse errors; a device instance is not a router.
#[tokio::test]
async fn refused_paths_and_limits_are_parse_errors() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    let path = client.routed_path(&mac(254), 5);
    assert!(matches!(
        client.set_routed_path_limit(&path, 10),
        Err(OpsError::Parse { what: "routed path limit", ref input, .. })
            if input == "10.0.0.254:47808/5"
    ));
    for network in [0, 65535] {
        assert!(matches!(
            client.routed_path_at("10.0.0.254:47808", network).await,
            Err(OpsError::Parse {
                what: "network",
                ..
            })
        ));
    }
    assert!(matches!(
        client.routed_path_at("1234", 5).await,
        Err(OpsError::Parse {
            what: "address",
            ..
        })
    ));
    assert!(client.routed_paths().paths.is_empty());
}

/// A handle to a device behind a router names the path its requests take,
/// through the router the discovery table holds; a local device has none,
/// and a remote one with no router known fails as unknown.
#[tokio::test(start_paused = true)]
async fn a_routed_handle_names_its_path() {
    let bus = Bus::new();
    let (client, _sent) = client(&bus).await;
    client
        .low_level()
        .add_routed_device(RoutedDeviceConfig {
            instance: 77,
            router_mac: mac(254).to_vec(),
            remote_network: 5,
            remote_mac: vec![9],
            max_apdu_length: 480,
            segmentation_supported: Segmentation::NO_SEGMENTATION,
            max_segments_accepted: None,
        })
        .await
        .unwrap();
    let routed = client
        .device(DeviceAddress::new(5, &[9]).unwrap())
        .await
        .unwrap();
    assert_eq!(
        routed.routed_path().await.unwrap(),
        Some(client.routed_path(&mac(254), 5))
    );

    let local = client
        .device(DeviceAddress::local(&mac(7)).unwrap())
        .await
        .unwrap();
    assert_eq!(local.routed_path().await.unwrap(), None);

    // The table loses the row: no router is known for network 5 now.
    client.low_level().clear_devices().await;
    assert!(matches!(
        routed.routed_path().await,
        Err(OpsError::UnknownDevice { .. })
    ));
}
