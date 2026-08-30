use std::sync::{Arc, Mutex as StdMutex};

use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot};
use bacnet_services::audit::{
    AuditLogQueryAck, AuditLogQueryRequest, BACnetAuditLogQueryParameters,
};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;

#[derive(Default)]
struct MemoryPersistence(StdMutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

#[tokio::test]
async fn audit_log_query_dispatch_returns_a_typed_complex_ack() {
    let audit_log = ObjectIdentifier::new(ObjectType::AUDIT_LOG, 7).unwrap();
    let target = ObjectIdentifier::new(ObjectType::DEVICE, 2).unwrap();
    let mut database = ObjectDatabase::new();
    database
        .add(Box::new(
            AuditLogObject::new(7, "Audit-7", 4, Arc::new(MemoryPersistence::default())).unwrap(),
        ))
        .unwrap();
    let db = Arc::new(RwLock::new(database));

    let request = AuditLogQueryRequest {
        audit_log,
        query_parameters: BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: target,
            target_device_address: None,
            target_object_identifier: None,
            target_property_identifier: None,
            target_array_index: None,
            target_priority: None,
            operations: None,
            successful_actions_only: false,
        },
        start_at_sequence_number: None,
        requested_count: 10,
    };
    let mut service_request = BytesMut::new();
    request.try_encode(&mut service_request).unwrap();

    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let device_bindings = Arc::new(RwLock::new(DeviceBindingTable::new()));
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let config = ServerConfig::default();
    let source_mac = MacAddr::from_slice(&[1]);
    let routed_source = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0a, 0x0b]),
    };
    let confirmed = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 0x42,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::AUDIT_LOG_QUERY,
        service_request: service_request.freeze(),
    };
    let (tx, rx) = oneshot::channel();

    BACnetServer::<BipTransport>::handle_confirmed_request(
        &db,
        &network,
        &cov_table,
        &seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        &notification_transactions,
        &device_bindings,
        &comm_state,
        &dcc_timer,
        &config,
        &source_mac,
        Some(routed_source.clone()),
        confirmed,
        Some(tx),
    )
    .await;

    let npdu = decode_npdu(rx.await.expect("query response")).unwrap();
    assert_eq!(npdu.destination, Some(routed_source));
    let Apdu::ComplexAck(ack) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected AuditLogQuery ComplexAck");
    };
    assert_eq!(ack.invoke_id, 0x42);
    assert_eq!(ack.service_choice, ConfirmedServiceChoice::AUDIT_LOG_QUERY);
    assert_eq!(
        AuditLogQueryAck::decode(&ack.service_ack).unwrap(),
        AuditLogQueryAck {
            audit_log,
            records: Vec::new(),
            no_more_items: true,
        }
    );
}
