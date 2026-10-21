//! A Device binding never takes a group address of the link, its broadcast
//! MAC or any other such as a multicast address (#1493). A configured one
//! stops the server before it starts, and an I-Am from one, directly or
//! relayed by a router there, binds nothing: a forged I-Am can't make the
//! server send its confirmed requests to a group.

use super::*;
use crate::server::test_transport::{TestTransport, BIP_LOCAL_MAC};
use bacnet_transport::port::TransportProvenance;

/// A multicast address: a group destination, not the link's broadcast.
const GROUP: [u8; 6] = [224, 0, 0, 1, 0xBA, 0xC0];
const STATION: [u8; 6] = [127, 0, 0, 1, 0xBA, 0xC1];

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn group_link() -> TestTransport {
    TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .group_mac(&GROUP)
        .build()
}

#[tokio::test(start_paused = true)]
async fn a_configured_binding_at_a_group_address_stops_startup() {
    for binding in [
        DeviceBinding::local(device(9), GROUP),
        DeviceBinding::routed(device(9), 700, [0x33], GROUP),
    ] {
        let started = BACnetServer::generic_builder()
            .transport(group_link())
            .device_binding(binding.unwrap())
            .unwrap()
            .build()
            .await;
        let Err(error) = started else {
            panic!("a server started with a binding at a group address");
        };
        let error = error.to_string();
        assert!(
            error.contains("Device 9 is bound at e0:00:00:01:ba:c0"),
            "{error}"
        );
        assert!(error.contains("broadcast or group address"), "{error}");
    }

    let mut server = BACnetServer::generic_builder()
        .transport(group_link())
        .device_binding(DeviceBinding::local(device(9), STATION).unwrap())
        .unwrap()
        .build()
        .await
        .expect("a binding at a station starts");
    server.stop().await.unwrap();
}

fn i_am(identifier: ObjectIdentifier) -> UnconfirmedRequestPdu {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: identifier,
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 1,
    }
    .encode(&mut service_request);
    UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    }
}

/// An I-Am as the network layer hands it up: from `source_mac`, relayed from
/// `source_network` when routed.
fn from(
    source_mac: &[u8],
    source_network: Option<NpduAddress>,
) -> bacnet_network::layer::ReceivedApdu {
    bacnet_network::layer::ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(source_mac),
        ingress_network: None,
        source_network,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

#[tokio::test(start_paused = true)]
async fn an_i_am_from_a_group_address_binds_nothing() {
    let network = Arc::new(NetworkLayer::new(group_link()));
    let services = UnconfirmedServices::for_test(network, ServerConfig::default());
    let routed = || {
        Some(NpduAddress {
            network: 700,
            mac_address: MacAddr::from_slice(&[0x33]),
        })
    };
    // Device 10 claims the group as its own address, Device 11 a router at
    // the group, and Device 12 a station.
    for (instance, source_mac, source_network) in [
        (10, GROUP, None),
        (11, GROUP, routed()),
        (12, STATION, None),
    ] {
        BACnetServer::<TestTransport>::handle_unconfirmed_request(
            &services,
            i_am(device(instance)),
            &from(&source_mac, source_network),
        )
        .await;
    }

    let table = services.device_bindings.read().await;
    let resolve = |instance| table.resolve_at(&device(instance), Instant::now(), |_| false);
    assert_eq!(resolve(10), DeviceResolution::Unknown);
    assert_eq!(resolve(11), DeviceResolution::Unknown);
    assert!(
        matches!(resolve(12), DeviceResolution::ResolvedLocal { peer_mac, .. } if peer_mac[..] == STATION),
        "a station's I-Am binds it"
    );
}
