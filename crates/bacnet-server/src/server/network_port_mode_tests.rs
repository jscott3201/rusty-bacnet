//! A registered Network Port in FOREIGN and BBMD modes (#939), read through
//! the server's confirmed-request path. These run on the shared test
//! transport; `network_port_bip_mode_tests` repeats the essentials on a real
//! `BipTransport`.

use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::constructed::{decode_bdt_entry_list, decode_fdt_entry_list};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_encoding::primitives::decode_application_value;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{
    ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_transport::bbmd::{BbmdState, BdtEntry, ForeignDevicePolicy};
use bacnet_transport::port::BipPort;
use bacnet_types::bip_port::{BbmdTables, BipPortMode};
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort, PropertyReference,
    ReadAccessSpecification,
};
use bacnet_types::enums::PropertyIdentifier as P;
use std::net::{IpAddr, SocketAddrV4};
use std::time::Instant;

const PORT_IP: [u8; 4] = [127, 0, 0, 1];
const UDP: u16 = 47808;
const BBMD_ROWS: [P; 3] = [
    P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
    P::BBMD_ACCEPT_FD_REGISTRATIONS,
    P::BBMD_FOREIGN_DEVICE_TABLE,
];
const FOREIGN_ROWS: [P; 2] = [P::FD_BBMD_ADDRESS, P::FD_SUBSCRIPTION_LIFETIME];

/// A real BBMD state lent through a view whose clock the test sets by hand,
/// so a registration's time remaining can be read at chosen instants.
pub(super) struct HandClockedBbmd {
    pub(super) state: Arc<std::sync::Mutex<BbmdState>>,
    pub(super) now: std::sync::Mutex<Instant>,
}

impl BbmdTables for HandClockedBbmd {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        self.state.lock().unwrap().bdt_entries()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .accepts_foreign_device_registrations()
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        let now = *self.now.lock().unwrap();
        self.state.lock().unwrap().fdt_entries_at(now)
    }
}

pub(super) fn port_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            1,
            "Port",
            BipPortConfig {
                ip_address: PORT_IP,
                udp_port: UDP,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    db
}

async fn start(mode: BipPortMode) -> BACnetServer<TestTransport> {
    let transport = TestTransport::builder()
        .bip_port(BipPort {
            endpoint: SocketAddrV4::new(PORT_IP.into(), UDP),
            mode,
        })
        .build();
    let config = ServerConfig {
        registered_network_port: Some(port_oid()),
        ..Default::default()
    };
    BACnetServer::start(config, database(), transport)
        .await
        .unwrap()
}

/// Answer one confirmed request from the server's request path.
async fn confirmed<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    service: ConfirmedServiceChoice,
    request: BytesMut,
) -> Apdu {
    let (tx, rx) = oneshot::channel();
    BACnetServer::handle_confirmed_request(
        &server.test_services(),
        &server.confirmed_request_tracker,
        &server.request_tasks.spawner(),
        &[10, 0, 0, 50, 0xBA, 0xC0],
        None,
        ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 9,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: request.freeze(),
        },
        Some(tx),
    )
    .await;
    let reply = rx.await.expect("a reply");
    decode_apdu(decode_npdu(reply).unwrap().payload).unwrap()
}

/// ReadProperty `property` of the port: its value bytes, or the error.
pub(super) async fn rp<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    property: P,
) -> Result<Vec<u8>, (ErrorClass, ErrorCode)> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: port_oid(),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut request);
    match confirmed(server, ConfirmedServiceChoice::READ_PROPERTY, request).await {
        Apdu::ComplexAck(ack) => {
            let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
            assert_eq!(ack.property_identifier, property);
            Ok(ack.property_value)
        }
        Apdu::Error(error) => Err((error.error_class, error.error_code)),
        other => panic!("unexpected ReadProperty reply {other:?}"),
    }
}

/// ReadPropertyMultiple of `properties` on the port, one result each.
async fn rpm<T: TransportPort + 'static>(
    server: &BACnetServer<T>,
    properties: &[P],
) -> Vec<ReadResultElement> {
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: port_oid(),
            list_of_property_references: properties
                .iter()
                .map(|&property| PropertyReference {
                    property_identifier: property,
                    property_array_index: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let reply = confirmed(
        server,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    let Apdu::ComplexAck(ack) = reply else {
        panic!("unexpected RPM reply {reply:?}");
    };
    let mut ack = ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let results = ack.list_of_read_access_results.remove(0).list_of_results;
    assert_eq!(
        results
            .iter()
            .map(|r| r.property_identifier)
            .collect::<Vec<_>>(),
        properties
    );
    results
}

/// The port's Property_List, read through ReadProperty.
pub(super) async fn property_list<T: TransportPort + 'static>(server: &BACnetServer<T>) -> Vec<P> {
    let bytes = rp(server, P::PROPERTY_LIST).await.unwrap();
    let mut listed = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let (value, end) = decode_application_value(&bytes, offset).unwrap();
        let PropertyValue::Enumerated(raw) = value else {
            panic!("Property_List holds enumerations: {value:?}");
        };
        listed.push(P::from_raw(raw));
        offset = end;
    }
    listed
}

pub(super) fn host(ip: [u8; 4], port: u16) -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::from(ip)),
        port,
    }
}

fn unknown_property() -> Result<Vec<u8>, (ErrorClass, ErrorCode)> {
    Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
}

#[tokio::test]
async fn normal_mode_has_no_bbmd_or_foreign_rows() {
    let server = start(BipPortMode::Normal).await;
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 0]);
    let listed = property_list(&server).await;
    assert!(listed.contains(&P::BACNET_IP_MODE));
    for property in BBMD_ROWS.into_iter().chain(FOREIGN_ROWS) {
        assert!(!listed.contains(&property), "{property} not listed");
        assert_eq!(
            rp(&server, property).await,
            unknown_property(),
            "{property}"
        );
    }
    let results = rpm(&server, &[P::BBMD_BROADCAST_DISTRIBUTION_TABLE]).await;
    assert_eq!(
        results[0].error,
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
}

#[tokio::test]
async fn foreign_mode_serves_its_bbmd_and_lifetime_through_rp_and_rpm() {
    let server = start(BipPortMode::Foreign {
        bbmd: host([10, 0, 0, 1], 0xBAC1),
        subscription_lifetime: 300,
    })
    .await;
    // BACnetIPMode FOREIGN (1).
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 1]);
    // [0] { ip-address [1] 10.0.0.1 } [1] 47809
    let bbmd = [0x0E, 0x1C, 10, 0, 0, 1, 0x0F, 0x1A, 0xBA, 0xC1];
    assert_eq!(rp(&server, P::FD_BBMD_ADDRESS).await.unwrap(), bbmd);
    // Unsigned 300.
    let lifetime = [0x22, 0x01, 0x2C];
    assert_eq!(
        rp(&server, P::FD_SUBSCRIPTION_LIFETIME).await.unwrap(),
        lifetime
    );
    let listed = property_list(&server).await;
    for property in FOREIGN_ROWS {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in BBMD_ROWS {
        assert!(!listed.contains(&property), "{property} not listed");
        assert_eq!(rp(&server, property).await, unknown_property());
    }
    let results = rpm(
        &server,
        &[
            P::BACNET_IP_MODE,
            P::FD_BBMD_ADDRESS,
            P::FD_SUBSCRIPTION_LIFETIME,
            P::BBMD_ACCEPT_FD_REGISTRATIONS,
        ],
    )
    .await;
    assert_eq!(results[0].property_value.as_deref(), Some(&[0x91, 1][..]));
    assert_eq!(results[1].property_value.as_deref(), Some(&bbmd[..]));
    assert_eq!(results[2].property_value.as_deref(), Some(&lifetime[..]));
    assert_eq!(
        results[3].error,
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
}

/// A BBMD state with a peer row, accepting registrations, lent with a
/// hand-set clock starting at `now`.
fn bbmd_at(now: Instant) -> Arc<HandClockedBbmd> {
    let mut state = BbmdState::new(PORT_IP, UDP);
    state
        .set_bdt(vec![BdtEntry {
            ip: [10, 0, 0, 2],
            port: UDP,
            broadcast_mask: [0xFF; 4],
        }])
        .unwrap();
    state.set_foreign_device_policy(Some(ForeignDevicePolicy::default()));
    Arc::new(HandClockedBbmd {
        state: Arc::new(std::sync::Mutex::new(state)),
        now: std::sync::Mutex::new(now),
    })
}

#[tokio::test]
async fn bbmd_mode_serves_its_tables_through_rp_and_rpm() {
    let now = Instant::now();
    let bbmd = bbmd_at(now);
    assert_eq!(
        bbmd.state
            .lock()
            .unwrap()
            .register_foreign_device_at([10, 0, 0, 9], UDP, 60, now),
        bacnet_types::enums::BvlcResultCode::SUCCESSFUL_COMPLETION
    );
    let server = start(BipPortMode::Bbmd {
        tables: Some(Arc::clone(&bbmd) as Arc<dyn BbmdTables>),
    })
    .await;
    // BACnetIPMode BBMD (2).
    assert_eq!(rp(&server, P::BACNET_IP_MODE).await.unwrap(), [0x91, 2]);
    let peer = BACnetBDTEntry {
        bbmd_address: host([10, 0, 0, 2], UDP),
        broadcast_mask: Some([0xFF; 4]),
    };
    let own = BACnetBDTEntry {
        bbmd_address: host(PORT_IP, UDP),
        broadcast_mask: Some([0xFF; 4]),
    };
    let bdt = rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
        .await
        .unwrap();
    assert_eq!(
        decode_bdt_entry_list(&bdt).unwrap(),
        [peer.clone(), own.clone()]
    );
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        [0x11]
    );
    let registrant = BACnetFDTEntry {
        address: SocketAddrV4::new([10, 0, 0, 9].into(), UDP).into(),
        time_to_live: 60,
        remaining_time_to_live: 90,
    };
    // [0] 10.0.0.9:47808 [1] 60 [2] 90
    let fdt = [0x0D, 0x06, 10, 0, 0, 9, 0xBA, 0xC0, 0x19, 60, 0x29, 90];
    assert_eq!(
        rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap(),
        fdt
    );
    assert_eq!(decode_fdt_entry_list(&fdt).unwrap(), [registrant]);

    let listed = property_list(&server).await;
    for property in BBMD_ROWS {
        assert!(listed.contains(&property), "{property} listed");
    }
    for property in FOREIGN_ROWS {
        assert!(!listed.contains(&property), "{property} not listed");
        assert_eq!(rp(&server, property).await, unknown_property());
    }
    let results = rpm(
        &server,
        &[
            P::BACNET_IP_MODE,
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
            P::BBMD_ACCEPT_FD_REGISTRATIONS,
            P::BBMD_FOREIGN_DEVICE_TABLE,
            P::FD_BBMD_ADDRESS,
        ],
    )
    .await;
    assert_eq!(results[0].property_value.as_deref(), Some(&[0x91, 2][..]));
    assert_eq!(results[1].property_value.as_deref(), Some(&bdt[..]));
    assert_eq!(results[2].property_value.as_deref(), Some(&[0x11][..]));
    assert_eq!(results[3].property_value.as_deref(), Some(&fdt[..]));
    assert_eq!(
        results[4].error,
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
}

/// The object reads the transport's tables on each read: a BDT change, a
/// policy change and a new registration show on the next read, and a
/// registration's time remaining falls as the clock moves.
#[tokio::test]
async fn bbmd_changes_and_the_clock_show_on_the_next_read() {
    let start_time = Instant::now();
    let bbmd = bbmd_at(start_time);
    let server = start(BipPortMode::Bbmd {
        tables: Some(Arc::clone(&bbmd) as Arc<dyn BbmdTables>),
    })
    .await;
    let fdt = |bytes: Vec<u8>| decode_fdt_entry_list(&bytes).unwrap();
    assert!(fdt(rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap()).is_empty());

    {
        let mut state = bbmd.state.lock().unwrap();
        state
            .set_bdt(vec![BdtEntry {
                ip: [10, 0, 0, 3],
                port: 0xBAC1,
                broadcast_mask: [255, 255, 255, 0],
            }])
            .unwrap();
        state.register_foreign_device_at([10, 0, 0, 9], UDP, 300, start_time);
    }
    assert_eq!(
        decode_bdt_entry_list(
            &rp(&server, P::BBMD_BROADCAST_DISTRIBUTION_TABLE)
                .await
                .unwrap()
        )
        .unwrap(),
        [
            BACnetBDTEntry {
                bbmd_address: host([10, 0, 0, 3], 0xBAC1),
                broadcast_mask: Some([255, 255, 255, 0]),
            },
            BACnetBDTEntry {
                bbmd_address: host(PORT_IP, UDP),
                broadcast_mask: Some([0xFF; 4]),
            },
        ]
    );
    let remaining = |entries: Vec<BACnetFDTEntry>| {
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].time_to_live, 300);
        entries[0].remaining_time_to_live
    };
    let read = rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap();
    assert_eq!(remaining(fdt(read)), 330, "TTL plus the grace period");
    for (elapsed, left) in [(45, 285), (329, 1), (330, 0)] {
        *bbmd.now.lock().unwrap() = start_time + Duration::from_secs(elapsed);
        let read = rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap();
        assert_eq!(remaining(fdt(read)), left, "{elapsed} s after registering");
    }
    // Past TTL and grace the entry is gone, though nothing purged it.
    *bbmd.now.lock().unwrap() = start_time + Duration::from_secs(331);
    assert!(fdt(rp(&server, P::BBMD_FOREIGN_DEVICE_TABLE).await.unwrap()).is_empty());

    bbmd.state.lock().unwrap().set_foreign_device_policy(None);
    assert_eq!(
        rp(&server, P::BBMD_ACCEPT_FD_REGISTRATIONS).await.unwrap(),
        [0x10]
    );
}
