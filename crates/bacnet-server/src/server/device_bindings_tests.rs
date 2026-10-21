use super::device_bindings::{
    BindingFreshness, DeviceBindingTable, DeviceResolution, ObservationOutcome,
    MAX_DEVICE_BINDINGS, OBSERVED_BINDING_TTL,
};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_transport::port::TransportProvenance;
use bytes::Bytes;

const LOCAL_PEER: &[u8] = &[0x10, 0x11];
const UPDATED_PEER: &[u8] = &[0x20, 0x21];
const ROUTER: &[u8] = &[0x30, 0x31];
const FINAL_PEER: &[u8] = &[0x40, 0x41, 0x42];
const BROADCAST: &[u8] = &[0xFF, 0xFF];

#[path = "unconfirmed_warning_tests.rs"]
mod unconfirmed_warnings;

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn no_broadcast(_: &[u8]) -> bool {
    false
}

fn test_broadcast(mac: &[u8]) -> bool {
    mac == BROADCAST
}

#[test]
fn configured_binding_validation_and_duplicate_rejection_are_pre_mutation() {
    let not_device = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap();
    assert!(DeviceBinding::local(not_device, LOCAL_PEER).is_err());
    assert!(DeviceBinding::local(device(1), []).is_err());
    assert!(DeviceBinding::routed(device(1), 0, FINAL_PEER, ROUTER).is_err());
    assert!(DeviceBinding::routed(device(1), 0xFFFF, FINAL_PEER, ROUTER).is_err());
    assert!(DeviceBinding::routed(device(1), 100, [], ROUTER).is_err());
    assert!(DeviceBinding::routed(device(1), 100, FINAL_PEER, []).is_err());

    let mut table = DeviceBindingTable::new();
    let configured = DeviceBinding::local(device(1), LOCAL_PEER).unwrap();
    table
        .insert_configured(configured.clone(), no_broadcast)
        .unwrap();
    let before = table.len();
    assert!(table.insert_configured(configured, no_broadcast).is_err());
    assert_eq!(table.len(), before);

    let broadcast = DeviceBinding::local(device(2), BROADCAST).unwrap();
    assert!(table.insert_configured(broadcast, test_broadcast).is_err());
    assert_eq!(table.len(), before);
}

#[test]
fn configured_registration_rejects_entry_beyond_capacity_without_mutation() {
    let mut configured = Vec::new();
    for instance in 0..MAX_DEVICE_BINDINGS as u32 {
        super::device_bindings::register_configured_binding(
            &mut configured,
            DeviceBinding::local(device(instance), LOCAL_PEER).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(configured.len(), MAX_DEVICE_BINDINGS);
    assert!(super::device_bindings::register_configured_binding(
        &mut configured,
        DeviceBinding::local(device(MAX_DEVICE_BINDINGS as u32), LOCAL_PEER).unwrap(),
    )
    .is_err());
    assert_eq!(configured.len(), MAX_DEVICE_BINDINGS);
}

#[test]
fn configured_precedence_and_observed_refresh_expiry_are_deterministic() {
    let now = Instant::now();
    let configured_device = device(10);
    let observed_device = device(11);
    let mut table = DeviceBindingTable::new();
    table
        .insert_configured(
            DeviceBinding::local(configured_device, LOCAL_PEER).unwrap(),
            no_broadcast,
        )
        .unwrap();

    assert_eq!(
        table.observe_i_am_at(configured_device, UPDATED_PEER, None, now, no_broadcast,),
        ObservationOutcome::ConfiguredPreserved
    );
    assert_eq!(
        table.resolve_at(&configured_device, now, no_broadcast),
        DeviceResolution::ResolvedLocal {
            peer_mac: MacAddr::from_slice(LOCAL_PEER),
            freshness: BindingFreshness::Configured,
        }
    );

    assert_eq!(
        table.observe_i_am_at(observed_device, LOCAL_PEER, None, now, no_broadcast),
        ObservationOutcome::Inserted
    );
    let refreshed_at = now + Duration::from_secs(30);
    assert_eq!(
        table.observe_i_am_at(
            observed_device,
            UPDATED_PEER,
            None,
            refreshed_at,
            no_broadcast,
        ),
        ObservationOutcome::Refreshed
    );
    assert_eq!(
        table.resolve_at(
            &observed_device,
            refreshed_at + OBSERVED_BINDING_TTL - Duration::from_nanos(1),
            no_broadcast,
        ),
        DeviceResolution::ResolvedLocal {
            peer_mac: MacAddr::from_slice(UPDATED_PEER),
            freshness: BindingFreshness::ObservedUntil(tokio::time::Instant::from_std(
                refreshed_at + OBSERVED_BINDING_TTL,
            )),
        }
    );
    assert_eq!(
        table.resolve_at(
            &observed_device,
            refreshed_at + OBSERVED_BINDING_TTL,
            no_broadcast,
        ),
        DeviceResolution::Stale
    );
}

#[test]
fn malformed_invalid_identity_and_broadcast_observations_do_not_mutate() {
    let now = Instant::now();
    let mut table = DeviceBindingTable::new();
    let invalid_device = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let invalid_sources = [
        (invalid_device, LOCAL_PEER, None),
        (device(1), &[][..], None),
        (device(2), BROADCAST, None),
    ];
    for (identifier, source, routed) in invalid_sources {
        assert_eq!(
            table.observe_i_am_at(identifier, source, routed, now, test_broadcast),
            ObservationOutcome::RejectedInvalid
        );
    }

    for source_network in [
        NpduAddress {
            network: 0,
            mac_address: MacAddr::from_slice(FINAL_PEER),
        },
        NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::from_slice(FINAL_PEER),
        },
        NpduAddress {
            network: 100,
            mac_address: MacAddr::new(),
        },
    ] {
        assert_eq!(
            table.observe_i_am_at(
                device(3),
                ROUTER,
                Some(&source_network),
                now,
                test_broadcast,
            ),
            ObservationOutcome::RejectedInvalid
        );
    }
    let routed = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(FINAL_PEER),
    };
    assert_eq!(
        table.observe_i_am_at(device(4), BROADCAST, Some(&routed), now, test_broadcast,),
        ObservationOutcome::RejectedInvalid
    );
    assert_eq!(table.len(), 0);

    table
        .insert_configured(
            DeviceBinding::local(device(5), BROADCAST).unwrap(),
            no_broadcast,
        )
        .unwrap();
    assert_eq!(
        table.resolve_at(&device(5), now, test_broadcast),
        DeviceResolution::Invalid
    );
}

#[test]
fn capacity_rejection_stale_reclamation_and_configured_retention_are_bounded() {
    let now = Instant::now();
    let mut full = DeviceBindingTable::new();
    for instance in 0..MAX_DEVICE_BINDINGS as u32 {
        assert_eq!(
            full.observe_i_am_at(device(instance), LOCAL_PEER, None, now, no_broadcast),
            ObservationOutcome::Inserted
        );
    }
    assert_eq!(full.len(), MAX_DEVICE_BINDINGS);
    let rejected = device(MAX_DEVICE_BINDINGS as u32);
    assert_eq!(
        full.observe_i_am_at(rejected, LOCAL_PEER, None, now, no_broadcast),
        ObservationOutcome::RejectedCapacity
    );
    assert_eq!(full.len(), MAX_DEVICE_BINDINGS);
    assert_eq!(
        full.resolve_at(&rejected, now, no_broadcast),
        DeviceResolution::Unknown
    );

    let configured_device = device(0);
    let mut reclaiming = DeviceBindingTable::new();
    reclaiming
        .insert_configured(
            DeviceBinding::local(configured_device, UPDATED_PEER).unwrap(),
            no_broadcast,
        )
        .unwrap();
    for instance in 1..MAX_DEVICE_BINDINGS as u32 {
        assert_eq!(
            reclaiming.observe_i_am_at(device(instance), LOCAL_PEER, None, now, no_broadcast,),
            ObservationOutcome::Inserted
        );
    }
    let after_expiry = now + OBSERVED_BINDING_TTL;
    assert_eq!(
        reclaiming.observe_i_am_at(
            device(MAX_DEVICE_BINDINGS as u32 + 1),
            LOCAL_PEER,
            None,
            after_expiry,
            no_broadcast,
        ),
        ObservationOutcome::Inserted
    );
    assert_eq!(reclaiming.len(), 2, "stale observed rows are reclaimed");
    assert_eq!(
        reclaiming.resolve_at(&configured_device, after_expiry, no_broadcast),
        DeviceResolution::ResolvedLocal {
            peer_mac: MacAddr::from_slice(UPDATED_PEER),
            freshness: BindingFreshness::Configured,
        },
        "configured rows never expire or get evicted"
    );
}

/// A closed link at `LOCAL_PEER` whose sends succeed and whose literal
/// broadcast MAC is `BROADCAST`.
fn transport() -> TestTransport {
    TestTransport::builder()
        .local_mac(LOCAL_PEER)
        .broadcast_mac(BROADCAST)
        .build()
}

fn i_am_request(identifier: ObjectIdentifier) -> UnconfirmedRequestPdu {
    let i_am = IAmRequest {
        object_identifier: identifier,
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 1,
    };
    let mut service_request = BytesMut::new();
    i_am.encode(&mut service_request);
    UnconfirmedRequestPdu {
        service_choice: UnconfirmedServiceChoice::I_AM,
        service_request: service_request.freeze(),
    }
}

fn received(
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

#[tokio::test]
async fn passive_local_and_routed_i_am_share_the_authority_and_disable_initiation_keeps_observing()
{
    let db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let network = Arc::new(NetworkLayer::new(transport()));
    let config = ServerConfig::default();
    let comm_state = Arc::new(CommState::default());
    let bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let discovery_limiter = Arc::new(DiscoveryLimiter::new(DiscoveryPolicy::default(), None));
    let time_sync_limiter = Arc::new(TimeSyncLimiter::new(TimeSyncPolicy::default()));
    let local_device = device(100);
    let routed_device = device(101);

    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(&db),
            comm_state: Arc::clone(&comm_state),
            device_bindings: Arc::clone(&bindings),
            discovery_limiter: Arc::clone(&discovery_limiter),
            time_sync_limiter: Arc::clone(&time_sync_limiter),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        i_am_request(local_device),
        &received(LOCAL_PEER, None),
    )
    .await;
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(&db),
            comm_state: Arc::clone(&comm_state),
            device_bindings: Arc::clone(&bindings),
            discovery_limiter: Arc::clone(&discovery_limiter),
            time_sync_limiter: Arc::clone(&time_sync_limiter),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        i_am_request(routed_device),
        &received(
            ROUTER,
            Some(NpduAddress {
                network: 200,
                mac_address: MacAddr::from_slice(FINAL_PEER),
            }),
        ),
    )
    .await;

    let now = Instant::now();
    let table = bindings.read().await;
    assert!(matches!(
        table.resolve_at(&local_device, now, test_broadcast),
        DeviceResolution::ResolvedLocal {
            peer_mac,
            freshness: BindingFreshness::ObservedUntil(_),
        } if peer_mac.as_slice() == LOCAL_PEER
    ));
    assert!(matches!(
        table.resolve_at(&routed_device, now, test_broadcast),
        DeviceResolution::ResolvedRouted {
            network: 200,
            final_mac,
            router_mac,
            freshness: BindingFreshness::ObservedUntil(_),
        } if final_mac.as_slice() == FINAL_PEER && router_mac.as_slice() == ROUTER
    ));
    drop(table);

    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(&db),
            comm_state: Arc::clone(&comm_state),
            device_bindings: Arc::clone(&bindings),
            discovery_limiter: Arc::clone(&discovery_limiter),
            time_sync_limiter: Arc::clone(&time_sync_limiter),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        i_am_request(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()),
        &received(LOCAL_PEER, None),
    )
    .await;
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(&db),
            comm_state: Arc::clone(&comm_state),
            device_bindings: Arc::clone(&bindings),
            discovery_limiter: Arc::clone(&discovery_limiter),
            time_sync_limiter: Arc::clone(&time_sync_limiter),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: Bytes::from_static(&[0xFF]),
        },
        &received(LOCAL_PEER, None),
    )
    .await;
    assert_eq!(bindings.read().await.len(), 2);

    // Observing an I-Am initiates nothing, so DISABLE_INITIATION leaves it on.
    comm_state.set_for_test(DccState::DisableInitiation);
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(&db),
            comm_state: Arc::clone(&comm_state),
            device_bindings: Arc::clone(&bindings),
            discovery_limiter: Arc::clone(&discovery_limiter),
            time_sync_limiter: Arc::clone(&time_sync_limiter),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        i_am_request(local_device),
        &received(UPDATED_PEER, None),
    )
    .await;
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &UnconfirmedServices {
            db: Arc::clone(&db),
            comm_state: Arc::clone(&comm_state),
            device_bindings: Arc::clone(&bindings),
            discovery_limiter: Arc::clone(&discovery_limiter),
            time_sync_limiter: Arc::clone(&time_sync_limiter),
            ..UnconfirmedServices::for_test(Arc::clone(&network), config.clone())
        },
        i_am_request(device(102)),
        &received(LOCAL_PEER, None),
    )
    .await;
    assert_eq!(bindings.read().await.len(), 3, "DCC leaves insertion on");
    assert!(matches!(
        bindings
            .read()
            .await
            .resolve_at(&local_device, Instant::now(), test_broadcast),
        DeviceResolution::ResolvedLocal {
            peer_mac,
            freshness: BindingFreshness::ObservedUntil(_),
        } if peer_mac.as_slice() == UPDATED_PEER
    ));
}

#[tokio::test]
async fn concrete_broadcast_validation_rejects_before_transport_start() {
    let transport = transport();
    let handle = transport.handle();
    let builder = BACnetServer::<TestTransport>::generic_builder()
        .transport(transport)
        .device_binding(DeviceBinding::local(device(200), BROADCAST).unwrap())
        .unwrap();

    assert!(builder.build().await.is_err());
    assert_eq!(handle.starts(), 0);
}

#[test]
fn who_is_scope_follows_the_last_observation_and_a_fruitless_probe_drops_it() {
    use super::binding_probes::WhoIsScope;
    let now = Instant::now();
    let stale = now + OBSERVED_BINDING_TTL;
    let routed = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(FINAL_PEER),
    };
    let mut table = DeviceBindingTable::new();
    table
        .insert_configured(
            DeviceBinding::local(device(1), LOCAL_PEER).unwrap(),
            no_broadcast,
        )
        .unwrap();
    table.observe_i_am_at(device(2), LOCAL_PEER, None, now, no_broadcast);
    table.observe_i_am_at(device(3), ROUTER, Some(&routed), now, no_broadcast);
    // A device never heard from is asked on every network; one last seen
    // here, or behind a router on network 100, is asked there.
    assert_eq!(table.who_is_scope(&device(4)), WhoIsScope::Global);
    assert_eq!(table.who_is_scope(&device(2)), WhoIsScope::Local);
    assert_eq!(table.who_is_scope(&device(3)), WhoIsScope::Remote(100));

    // A fresh observation and a configured binding are kept.
    for kept in [device(1), device(2), device(3)] {
        table.forget_stale(&kept, stale - Duration::from_millis(1));
    }
    table.forget_stale(&device(1), stale);
    assert_eq!(table.len(), 3);
    // A stale one is dropped, so the device's next Who-Is goes global.
    table.forget_stale(&device(3), stale);
    assert_eq!(table.len(), 2);
    assert_eq!(
        table.resolve_at(&device(3), stale, no_broadcast),
        DeviceResolution::Unknown
    );
    assert_eq!(table.who_is_scope(&device(3)), WhoIsScope::Global);
}

/// Device_Address_Binding at a given instant (#1369): every configured
/// binding, and each observation until it is stale, in instance order, with
/// network 0 for this network and the device's own MAC for a routed one.
#[test]
fn address_binding_list_drops_an_observation_once_it_is_stale() {
    let heard = Instant::now();
    let stale = heard + OBSERVED_BINDING_TTL;
    let routed = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(FINAL_PEER),
    };
    let mut table = DeviceBindingTable::new();
    table
        .insert_configured(
            DeviceBinding::routed(device(5), 200, UPDATED_PEER, ROUTER).unwrap(),
            no_broadcast,
        )
        .unwrap();
    table.observe_i_am_at(device(3), LOCAL_PEER, None, heard, no_broadcast);
    table.observe_i_am_at(device(4), ROUTER, Some(&routed), heard, no_broadcast);
    let binding = |instance, network: u64, mac: &[u8]| {
        let mut encoded = BytesMut::new();
        bacnet_encoding::primitives::encode_app_object_id(&mut encoded, &device(instance));
        bacnet_encoding::primitives::encode_app_unsigned(&mut encoded, network);
        bacnet_encoding::primitives::encode_app_octet_string(&mut encoded, mac);
        PropertyValue::ApplicationData(encoded.to_vec())
    };
    assert_eq!(
        table.address_binding_list(stale - Duration::from_nanos(1)),
        PropertyValue::List(vec![
            binding(3, 0, LOCAL_PEER),
            binding(4, 100, FINAL_PEER),
            binding(5, 200, UPDATED_PEER),
        ])
    );
    assert_eq!(
        table.address_binding_list(stale),
        PropertyValue::List(vec![binding(5, 200, UPDATED_PEER)])
    );
}

/// The number of the network this device is attached to, and another one.
const THIS_NETWORK: u16 = 7;
const REMOTE_NETWORK: u16 = 5;

/// The source of a request `ROUTER` relays from `FINAL_PEER` on `network`.
fn relayed(network: u16) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(FINAL_PEER),
    }
}

fn configured(bindings: Vec<DeviceBinding>) -> DeviceBindingTable {
    DeviceBindingTable::from_configured(bindings, test_broadcast).unwrap()
}

/// A binding routed through this network's own number is the local binding
/// it is once that number is known (#1404). A local binding names a direct
/// request from its MAC and, with the number known, one a router here relays
/// back with this network and that MAC as SNET and SADR. With the number
/// unknown, or for a binding routed to another network, a routed binding
/// names only a request relayed from its own network. The router's own MAC
/// names nobody.
#[test]
fn source_binding_takes_a_binding_routed_through_this_network_as_local() {
    use bacnet_objects::command_source::CommandDeviceBinding::{Unique, Unknown};
    let routed = |network, mac| {
        configured(vec![
            DeviceBinding::routed(device(1), network, mac, ROUTER).unwrap()
        ])
    };
    let here = routed(THIS_NETWORK, FINAL_PEER);
    let elsewhere = routed(REMOTE_NETWORK, FINAL_PEER);
    let plain = configured(vec![DeviceBinding::local(device(1), FINAL_PEER).unwrap()]);
    // Bound through this network at its link broadcast MAC: no single node.
    let broadcast = routed(THIS_NETWORK, BROADCAST);
    let local = Some(THIS_NETWORK);
    let (from_here, from_there) = (relayed(THIS_NETWORK), relayed(REMOTE_NETWORK));
    let one = Unique(device(1));
    for (table, local_network, immediate, source, expected) in [
        (&here, local, FINAL_PEER, None, one),
        (&here, local, ROUTER, Some(&from_here), one),
        (&here, local, ROUTER, None, Unknown),
        (&here, local, ROUTER, Some(&from_there), Unknown),
        (&here, None, FINAL_PEER, None, Unknown),
        (&here, None, ROUTER, Some(&from_here), one),
        (&plain, local, ROUTER, Some(&from_here), one),
        (&plain, None, ROUTER, Some(&from_here), Unknown),
        (&elsewhere, local, FINAL_PEER, None, Unknown),
        (&elsewhere, local, ROUTER, Some(&from_here), Unknown),
        (&elsewhere, local, ROUTER, Some(&from_there), one),
        (&broadcast, local, BROADCAST, None, Unknown),
    ] {
        assert_eq!(
            table.source_binding(immediate, source, local_network, test_broadcast),
            expected,
            "{local_network:?} {immediate:?} {source:?}"
        );
    }
}

/// A local binding and one routed through this network at the same MAC both
/// name a request from there once the number is known, direct or relayed
/// back with this network as its SNET, so it stays ambiguous. While the
/// number is unknown the direct form names only the local binding and the
/// relayed form only the routed one.
#[test]
fn source_binding_keeps_two_bindings_for_one_source_ambiguous() {
    use bacnet_objects::command_source::CommandDeviceBinding::{Ambiguous, Unique};
    let table = configured(vec![
        DeviceBinding::local(device(1), FINAL_PEER).unwrap(),
        DeviceBinding::routed(device(2), THIS_NETWORK, FINAL_PEER, ROUTER).unwrap(),
    ]);
    let from_here = relayed(THIS_NETWORK);
    for (immediate, source, local_network, expected) in [
        (FINAL_PEER, None, Some(THIS_NETWORK), Ambiguous),
        (ROUTER, Some(&from_here), Some(THIS_NETWORK), Ambiguous),
        (FINAL_PEER, None, None, Unique(device(1))),
        (ROUTER, Some(&from_here), None, Unique(device(2))),
    ] {
        assert_eq!(
            table.source_binding(immediate, source, local_network, test_broadcast),
            expected,
            "{local_network:?} {immediate:?} {source:?}"
        );
    }
}
