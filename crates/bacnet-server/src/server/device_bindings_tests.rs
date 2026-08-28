use super::device_bindings::{
    BindingFreshness, DeviceBindingTable, DeviceResolution, ObservationOutcome,
    MAX_DEVICE_BINDINGS, OBSERVED_BINDING_TTL,
};
use super::*;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bytes::Bytes;
use tokio::sync::mpsc;

const LOCAL_PEER: &[u8] = &[0x10, 0x11];
const UPDATED_PEER: &[u8] = &[0x20, 0x21];
const ROUTER: &[u8] = &[0x30, 0x31];
const FINAL_PEER: &[u8] = &[0x40, 0x41, 0x42];
const BROADCAST: &[u8] = &[0xFF, 0xFF];

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

#[derive(Default)]
struct PassiveTransport;

impl TransportPort for PassiveTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        LOCAL_PEER
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        test_broadcast(mac)
    }
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
        apdu: Bytes::new(),
        source_mac: MacAddr::from_slice(source_mac),
        source_network,
        link_layer_group: false,
        is_group: false,
        data_attributes: Vec::new(),
        reply_tx: None,
    }
}

#[tokio::test]
async fn passive_local_and_routed_i_am_share_the_authority_and_dcc_disable_blocks_refresh() {
    let db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let network = Arc::new(NetworkLayer::new(PassiveTransport));
    let config = ServerConfig::default();
    let comm_state = Arc::new(AtomicU8::new(0));
    let bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let local_device = device(100);
    let routed_device = device(101);

    BACnetServer::<PassiveTransport>::handle_unconfirmed_request(
        &db,
        &network,
        &config,
        None,
        &comm_state,
        &bindings,
        i_am_request(local_device),
        &received(LOCAL_PEER, None),
    )
    .await;
    BACnetServer::<PassiveTransport>::handle_unconfirmed_request(
        &db,
        &network,
        &config,
        None,
        &comm_state,
        &bindings,
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

    BACnetServer::<PassiveTransport>::handle_unconfirmed_request(
        &db,
        &network,
        &config,
        None,
        &comm_state,
        &bindings,
        i_am_request(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()),
        &received(LOCAL_PEER, None),
    )
    .await;
    BACnetServer::<PassiveTransport>::handle_unconfirmed_request(
        &db,
        &network,
        &config,
        None,
        &comm_state,
        &bindings,
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: Bytes::from_static(&[0xFF]),
        },
        &received(LOCAL_PEER, None),
    )
    .await;
    assert_eq!(bindings.read().await.len(), 2);

    comm_state.store(1, Ordering::Release);
    BACnetServer::<PassiveTransport>::handle_unconfirmed_request(
        &db,
        &network,
        &config,
        None,
        &comm_state,
        &bindings,
        i_am_request(local_device),
        &received(UPDATED_PEER, None),
    )
    .await;
    BACnetServer::<PassiveTransport>::handle_unconfirmed_request(
        &db,
        &network,
        &config,
        None,
        &comm_state,
        &bindings,
        i_am_request(device(102)),
        &received(LOCAL_PEER, None),
    )
    .await;
    assert_eq!(bindings.read().await.len(), 2, "DCC also blocks insertion");
    assert!(matches!(
        bindings
            .read()
            .await
            .resolve_at(&local_device, Instant::now(), test_broadcast),
        DeviceResolution::ResolvedLocal {
            peer_mac,
            freshness: BindingFreshness::ObservedUntil(_),
        } if peer_mac.as_slice() == LOCAL_PEER
    ));
}

#[derive(Clone)]
struct StartTrackingTransport {
    started: Arc<AtomicBool>,
}

impl TransportPort for StartTrackingTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.started.store(true, Ordering::Release);
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        LOCAL_PEER
    }

    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        test_broadcast(mac)
    }
}

#[tokio::test]
async fn concrete_broadcast_validation_rejects_before_transport_start() {
    let started = Arc::new(AtomicBool::new(false));
    let transport = StartTrackingTransport {
        started: Arc::clone(&started),
    };
    let builder = BACnetServer::<StartTrackingTransport>::generic_builder()
        .transport(transport)
        .device_binding(DeviceBinding::local(device(200), BROADCAST).unwrap())
        .unwrap();

    assert!(builder.build().await.is_err());
    assert!(!started.load(Ordering::Acquire));
}
