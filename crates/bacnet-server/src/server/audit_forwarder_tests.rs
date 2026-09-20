use super::audit_notification_tests::{
    confirmed_request, notification, oid, request_bytes, MemoryPersistence,
};
use super::*;
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::{
    audit::AuditLogObject,
    device::{DeviceConfig, DeviceObject},
};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::{
    constructed::BACnetDeviceObjectReference,
    enums::{AuditOperation, Reliability},
};
use std::sync::Mutex as StdMutex;

#[path = "audit_forwarder_edge_tests.rs"]
mod edges;

#[path = "audit_forwarder_boundary_tests.rs"]
mod boundaries;
#[path = "audit_forwarder_recovery_tests.rs"]
mod recovery;

#[derive(Clone, Default)]
struct Capture {
    sent: Arc<StdMutex<Vec<Bytes>>>,
    block: Arc<AtomicBool>,
    fail: Arc<AtomicBool>,
}
impl TransportPort for Capture {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(mpsc::channel(1).1)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, data: &[u8], mac: &[u8]) -> Result<(), Error> {
        assert_eq!(mac, &[2]);
        self.sent.lock().unwrap().push(Bytes::copy_from_slice(data));
        if self.block.load(Ordering::Acquire) {
            std::future::pending::<()>().await;
        }
        if self.fail.load(Ordering::Acquire) {
            return Err(Error::Encoding("injected failure".into()));
        }
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        panic!("forwarding must be unicast")
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

struct Fixture {
    server: BACnetServer<Capture>,
    store: Arc<MemoryPersistence>,
    wire: Capture,
}

fn parent() -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 20)),
        object_identifier: oid(ObjectType::AUDIT_LOG, 7),
    }
}

async fn fixture(
    parent: Option<BACnetDeviceObjectReference>,
    binding: Option<DeviceBinding>,
) -> Fixture {
    let store = Arc::new(MemoryPersistence::default());
    fixture_with(10, parent, binding, store).await
}

async fn fixture_with(
    local: u32,
    parent: Option<BACnetDeviceObjectReference>,
    binding: Option<DeviceBinding>,
    store: Arc<MemoryPersistence>,
) -> Fixture {
    fixture_with_capacity(local, parent, binding, store, 16).await
}

async fn fixture_with_capacity(
    local: u32,
    parent: Option<BACnetDeviceObjectReference>,
    binding: Option<DeviceBinding>,
    store: Arc<MemoryPersistence>,
    capacity: u32,
) -> Fixture {
    let mut log = AuditLogObject::new(7, "forwarder", capacity, store.clone()).unwrap();
    log.set_member_of(parent);
    let (server, wire) = start(local, log, binding).await;
    Fixture {
        server,
        store,
        wire,
    }
}

async fn start(
    local: u32,
    log: AuditLogObject,
    binding: Option<DeviceBinding>,
) -> (BACnetServer<Capture>, Capture) {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: local,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(log)).unwrap();
    let wire = Capture::default();
    let server = BACnetServer::start_with_clock_mode_and_bindings(
        ServerConfig {
            audit_notification_sink: Some(oid(ObjectType::AUDIT_LOG, 7)),
            audit_notification_authorizer: Some(Arc::new(|_| true)),
            unconfirmed_audit_notification_authorizer: Some(Arc::new(|_| true)),
            enable_event_enrollment: false,
            ..Default::default()
        },
        db,
        wire.clone(),
        Some(ClockConfig::default()),
        binding.into_iter().collect(),
    )
    .await
    .unwrap();
    (server, wire)
}

async fn ready() -> Fixture {
    fixture(
        Some(parent()),
        Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
    )
    .await
}

fn payload(complete: bool) -> Bytes {
    let mut item = notification(AuditOperation::WRITE);
    if complete {
        item.target_timestamp = item.source_timestamp.clone();
    }
    item.source_comment = Some("source remains source".into());
    item.target_comment = Some("target remains target".into());
    item.current_value = Some(vec![0x21, 42]);
    request_bytes(vec![item])
}

impl Fixture {
    async fn confirmed(&self, invoke: u8, peer: &[u8], data: Bytes) -> Option<Apdu> {
        let s = &self.server;
        let (tx, rx) = oneshot::channel();
        BACnetServer::handle_confirmed_request(
            &s.db,
            &s.network,
            &s.cov_table,
            &s.seg_ack_senders,
            &s.seg_send_permits,
            &s.cov_in_flight,
            &s.server_tsm,
            &s.notification_transactions,
            &s.confirmed_request_tracker,
            &s.device_bindings,
            &s.comm_state,
            &s.dcc_timer,
            &s.config,
            &s.request_tasks.spawner(),
            peer,
            None,
            confirmed_request(invoke, data),
            Some(tx),
        )
        .await;
        rx.await
            .ok()
            .map(|bytes| decode_apdu(decode_npdu(bytes).unwrap().payload).unwrap())
    }

    async fn unconfirmed(&self, data: Bytes) {
        let s = &self.server;
        BACnetServer::handle_unconfirmed_request(
            &s.db,
            &s.network,
            &s.config,
            s._clock.as_ref(),
            &s.comm_state,
            &s.device_bindings,
            &s.discovery_limiter,
            &s.time_sync_limiter,
            &s.notification_transactions,
            UnconfirmedRequestPdu {
                service_choice: UnconfirmedServiceChoice::UNCONFIRMED_AUDIT_NOTIFICATION,
                service_request: data,
            },
            &bacnet_network::layer::ReceivedApdu {
                apdu: Bytes::new(),
                source_mac: MacAddr::from_slice(&[3]),
                ingress_network: None,
                source_network: None,
                link_layer_group: false,
                is_group: false,
                data_attributes: vec![],
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            },
        )
        .await;
    }

    async fn reliability(&self) -> PropertyValue {
        self.server
            .db
            .read()
            .await
            .get(&oid(ObjectType::AUDIT_LOG, 7))
            .unwrap()
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap()
    }

    fn requests(&self) -> Vec<ConfirmedRequestPdu> {
        self.wire
            .sent
            .lock()
            .unwrap()
            .iter()
            .map(|bytes| {
                let Apdu::ConfirmedRequest(req) =
                    decode_apdu(decode_npdu(bytes.clone()).unwrap().payload).unwrap()
                else {
                    panic!("expected confirmed notification")
                };
                assert_eq!(
                    req.service_choice,
                    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION
                );
                assert!(!req.segmented);
                req
            })
            .collect()
    }

    fn ack(&self, invoke: u8, peer: &[u8], service: ConfirmedServiceChoice) -> bool {
        self.server.notification_transactions.admit_terminal(
            peer,
            None,
            &Apdu::SimpleAck(SimpleAck {
                invoke_id: invoke,
                service_choice: service,
            }),
        )
    }
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_commits_before_one_send_and_preserves_notification_bytes() {
    let mut f = ready().await;
    let data = payload(true);
    assert!(matches!(
        f.confirmed(201, &[3], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    let snapshot = f.store.snapshot.lock().unwrap().clone().unwrap();
    assert_eq!(snapshot.records.len(), 1);
    assert_eq!(snapshot.completed_receipts.len(), 1);
    settle().await;
    let requests = f.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].service_request, data);
    assert_ne!(requests[0].invoke_id, 201);
    assert!(
        f.server.db.try_write().is_ok(),
        "network wait must not retain database lock"
    );
    assert!(!f.ack(requests[0].invoke_id, &[99], requests[0].service_choice));
    assert!(!f.ack(
        requests[0].invoke_id,
        &[2],
        ConfirmedServiceChoice::READ_PROPERTY
    ));
    assert!(f.ack(requests[0].invoke_id, &[2], requests[0].service_choice));
    settle().await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(
        *f.store.snapshot.lock().unwrap(),
        Some(snapshot),
        "forwarding never deletes records"
    );
    assert!(
        f.confirmed(201, &[3], data.clone()).await.is_none(),
        "duplicate receives no second ACK"
    );
    // A new requester is not a receipt duplicate; the complete match is a
    // receipt-only commit, so it is ACKed but not forwarded again.
    assert!(matches!(
        f.confirmed(201, &[4], data.clone()).await,
        Some(Apdu::SimpleAck(_))
    ));
    f.unconfirmed(data).await;
    settle().await;
    assert_eq!(f.requests().len(), 1);
    assert_eq!(
        f.store
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .completed_receipts
            .len(),
        2
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_unconfirmed_merge_failure_and_no_receipt() {
    let mut f = ready().await;
    let data = payload(false);
    f.unconfirmed(data.clone()).await;
    settle().await;
    assert_eq!(f.requests()[0].service_request, data);
    assert!(f
        .store
        .snapshot
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .completed_receipts
        .is_empty());
    let mut target = notification(AuditOperation::WRITE);
    target.target_timestamp = target.source_timestamp.take();
    f.unconfirmed(request_bytes(vec![target])).await;
    settle().await;
    assert_eq!(f.requests().len(), 2, "complementary merge changes content");
    assert_eq!(
        f.store
            .snapshot
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .records
            .len(),
        1
    );
    let before = f.store.snapshot.lock().unwrap().clone();
    f.store.fail.store(true, Ordering::Release);
    let bad = request_bytes(vec![notification(AuditOperation::GENERAL)]);
    assert!(matches!(
        f.confirmed(202, &[3], bad.clone()).await,
        Some(Apdu::Error(_))
    ));
    f.unconfirmed(bad).await;
    settle().await;
    assert_eq!(f.requests().len(), 2);
    assert_eq!(*f.store.snapshot.lock().unwrap(), before);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_invalid_parent_is_visible_at_startup_without_send() {
    let mut invalid = vec![parent(); 4];
    invalid[0].device_identifier = None;
    invalid[1].device_identifier = Some(oid(ObjectType::DEVICE, 10));
    invalid[2].object_identifier = oid(ObjectType::ANALOG_INPUT, 7);
    invalid[3].device_identifier = Some(oid(ObjectType::ANALOG_INPUT, 20));
    for parent in invalid {
        let mut f = fixture(
            Some(parent),
            Some(DeviceBinding::local(oid(ObjectType::DEVICE, 20), [2]).unwrap()),
        )
        .await;
        assert_eq!(
            f.reliability().await,
            PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
        );
        assert!(f.requests().is_empty());
        assert!(matches!(
            f.confirmed(201, &[3], payload(false)).await,
            Some(Apdu::SimpleAck(_))
        ));
        settle().await;
        assert!(f.requests().is_empty());
        f.server.stop().await.unwrap();
    }
    let mut f = fixture(Some(parent()), None).await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn audit_forwarding_saturation_deadline_shutdown_are_bounded_without_retry() {
    let mut f = ready().await;
    f.wire.block.store(true, Ordering::Release);
    for _ in 0..65 {
        f.unconfirmed(payload(false)).await;
    }
    settle().await;
    assert_eq!(f.requests().len(), 64);
    assert_eq!(f.server.notification_transactions.active_count(), 64);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 0)
    );
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
    assert!(f.server.db.try_write().is_ok());
    tokio::time::advance(Duration::from_secs(3)).await;
    settle().await;
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert_eq!(
        f.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(
        f.requests().len(),
        64,
        "never retries or queues the 65th batch"
    );
    f.wire.block.store(false, Ordering::Release);
    f.unconfirmed(payload(false)).await;
    settle().await;
    let last = f.requests().pop().unwrap();
    assert!(f.ack(last.invoke_id, &[2], last.service_choice));
    settle().await;
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    f.wire.block.store(true, Ordering::Release);
    f.unconfirmed(payload(false)).await;
    settle().await;
    f.server.stop().await.unwrap();
    assert_eq!(f.server.notification_transactions.active_count(), 0);
    assert!(f.server.notification_transactions.workers_empty());
    assert_eq!(
        f.reliability().await,
        PropertyValue::Enumerated(Reliability::COMMUNICATION_FAILURE.to_raw())
    );
}
