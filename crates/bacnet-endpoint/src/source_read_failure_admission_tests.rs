//! Deterministic admission seam: saturated requester leases, without a scheduler
//! race to refill the just-completed READ lease before its audit admission.
use super::*;
use bacnet_encoding::apdu::{decode_apdu, Apdu, SimpleAck};
use bacnet_endpoint_core::coordinator::{
    LeaseMetadata, OutboundTransactionCoordinator, TerminalPolicy,
};
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_network::layer::{NetworkLayer, ReceivedApdu};
use bacnet_objects::audit::AuditReporterObject;
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_types::bitstring::BACnetPriorityFilter;
use bacnet_types::enums::{ConfirmedServiceChoice, Reliability};
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

const WAIT: Duration = Duration::from_secs(1);
fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}
struct Fixture {
    source: Arc<SourceRead>,
    owner: Arc<NotificationTransactions>,
    coordinator: Arc<OutboundTransactionCoordinator>,
    ingress: EndpointIngress<LoopbackTransport>,
    peer: NetworkLayer<LoopbackTransport>,
    records: mpsc::Receiver<ReceivedApdu>,
    status: Arc<AuditReporterStatus>,
}
impl Fixture {
    async fn new(confirmed: bool) -> Self {
        let mac = bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], 30002);
        let (local, remote) = LoopbackTransport::pair(vec![1], mac.to_vec());
        let mut peer = NetworkLayer::new(remote);
        let records = peer.start().await.unwrap();
        let mut ingress = EndpointIngress::new(local, 64);
        let receivers = ingress.start().await.unwrap();
        let mut db = crate::DeviceIdentity::new(123, 42)
            .unwrap()
            .build_database()
            .unwrap();
        let mut reporter = AuditReporterObject::new(1, "Source").unwrap();
        let mut flags = AuditOperationFlags::empty();
        flags.insert(AuditOperation::READ);
        flags.insert(AuditOperation::AUDITING_FAILURE);
        reporter
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_ALL,
                flags,
                confirmed,
                None,
                BACnetPriorityFilter::empty(),
            )
            .unwrap();
        let status = reporter.status_internal();
        db.add(Box::new(reporter)).unwrap();
        let coordinator = Arc::new(OutboundTransactionCoordinator::new());
        let owner = NotificationTransactions::with_coordinator(coordinator.clone());
        let source = SourceRead::new(
            Arc::new(RwLock::new(db)),
            oid(ObjectType::AUDIT_REPORTER, 1),
            StaticSourceAuditRecipient {
                device: oid(ObjectType::DEVICE, 999),
                address: "127.0.0.1:30002".parse().unwrap(),
            },
            Ipv4Addr::BROADCAST,
            receivers.egress,
            &owner,
            1476,
        );
        Self {
            source,
            owner,
            coordinator,
            ingress,
            peer,
            records,
            status,
        }
    }
    fn ticket(&self, confirmed: bool) -> AuditFailureTicket<MacAddr> {
        let route = MacAddr::from_slice(self.peer.local_mac());
        self.source
            .failures
            .observe(AuditFailureContext {
                status: self.status.clone(),
                epoch: self.status.auditing_failure_epoch().unwrap(),
                device: oid(ObjectType::DEVICE, 123),
                confirmed,
                peer: CanonicalPeer::direct(&route),
                route,
                max_apdu: 1476,
            })
            .unwrap()
    }
    fn record(&self, time: BACnetTimeStamp, confirmed: bool, invalid: bool, oversized: bool) {
        let mut notification = BACnetAuditNotification {
            source_timestamp: Some(time),
            target_timestamp: None,
            source_device: BACnetRecipient::Device(oid(ObjectType::DEVICE, 123)),
            source_object: None,
            operation: AuditOperation::READ,
            source_comment: None,
            target_comment: None,
            invoke_id: Some(7),
            source_user_id: None,
            source_user_role: None,
            target_device: BACnetRecipient::Address(BACnetAddress {
                network_number: 0,
                mac_address: MacAddr::from_slice(&[2]),
            }),
            target_object: Some(oid(ObjectType::ANALOG_VALUE, 7)),
            target_property: Some(AuditPropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }),
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        };
        if invalid {
            notification.target_priority = Some(17);
        }
        if oversized {
            notification.source_comment = Some("x".repeat(2000));
        }
        delivery::admit(
            &self.source,
            &self.owner,
            confirmed,
            notification,
            self.status.clone(),
            self.status.begin_delivery(),
            Some(self.ticket(confirmed)),
        );
    }
    async fn summary(
        &mut self,
        confirmed: bool,
        count: u64,
        earliest: BACnetTimeStamp,
    ) -> Option<u8> {
        let envelope = timeout(WAIT, self.records.recv()).await.unwrap().unwrap();
        let (service, invoke) = match decode_apdu(envelope.apdu).unwrap() {
            Apdu::ConfirmedRequest(pdu) if confirmed => (pdu.service_request, Some(pdu.invoke_id)),
            Apdu::UnconfirmedRequest(pdu) if !confirmed => (pdu.service_request, None),
            other => panic!("unexpected {other:?}"),
        };
        let records = AuditNotificationRequest::decode(&service)
            .unwrap()
            .notifications;
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.operation, AuditOperation::AUDITING_FAILURE);
        assert_eq!(record.target_timestamp, Some(earliest));
        assert_eq!(
            record.source_device,
            BACnetRecipient::Device(oid(ObjectType::DEVICE, 123))
        );
        assert_eq!(record.target_device, record.source_device);
        let mut expected = bytes::BytesMut::new();
        bacnet_encoding::primitives::encode_app_unsigned(&mut expected, count);
        assert_eq!(record.current_value.as_deref(), Some(expected.as_ref()));
        assert_eq!(record.source_timestamp, None);
        assert_eq!(record.invoke_id, None);
        assert_eq!(record.result, None);
        invoke
    }
    async fn stop(mut self) {
        self.source.close();
        self.owner.close();
        while self.owner.join_next().await.is_some() {}
        self.ingress.stop().await.unwrap();
        self.peer.stop().await.unwrap();
        assert_eq!(self.coordinator.active_count().unwrap(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn source_failure_global_requester_exhaustion_wakes_and_summary_timeout_never_recurses() {
    let mut fixture = Fixture::new(true).await;
    let mut leases: Vec<_> = (0..256)
        .map(|_| {
            fixture
                .coordinator
                .reserve(LeaseMetadata::requester(
                    CanonicalPeer::direct(&[3]),
                    ConfirmedServiceChoice::READ_PROPERTY,
                    TerminalPolicy::ComplexAck,
                ))
                .unwrap()
        })
        .collect();
    fixture.record(BACnetTimeStamp::SequenceNumber(9), true, false, false);
    fixture.record(BACnetTimeStamp::SequenceNumber(10), true, false, false);
    tokio::task::yield_now().await;
    assert!(fixture.records.try_recv().is_err());
    fixture.coordinator.release(leases.pop().unwrap()).unwrap();
    fixture
        .summary(true, 2, BACnetTimeStamp::SequenceNumber(9))
        .await;
    // Withhold ACK: one three-second attempt, no recursive loss summary.
    tokio::time::advance(Duration::from_secs(3)).await;
    timeout(WAIT, fixture.owner.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(fixture.records.try_recv().is_err());
    assert_eq!(fixture.coordinator.active_count().unwrap(), 255);
    for lease in leases {
        fixture.coordinator.release(lease).unwrap();
    }
    let permits: Vec<_> = (0..64)
        .map(|_| fixture.owner.try_admit_audit().unwrap())
        .collect();
    drop(permits);
    fixture.stop().await;
}

#[tokio::test(start_paused = true)]
async fn source_failure_invalid_oversized_and_closed_records_do_not_enter_count() {
    for confirmed in [false, true] {
        let mut fixture = Fixture::new(confirmed).await;
        let permits: Vec<_> = (0..64)
            .map(|_| fixture.owner.try_admit_audit().unwrap())
            .collect();
        fixture.record(BACnetTimeStamp::SequenceNumber(1), confirmed, true, false);
        fixture.record(BACnetTimeStamp::SequenceNumber(2), confirmed, false, true);
        fixture.record(BACnetTimeStamp::SequenceNumber(3), confirmed, false, false);
        drop(permits);
        let invoke = fixture
            .summary(confirmed, 1, BACnetTimeStamp::SequenceNumber(3))
            .await;
        if let Some(invoke_id) = invoke {
            assert!(fixture.owner.admit_terminal(
                fixture.peer.local_mac(),
                None,
                &Apdu::SimpleAck(SimpleAck {
                    invoke_id,
                    service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
                })
            ));
        }
        timeout(WAIT, fixture.owner.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            fixture
                .source
                .db
                .read()
                .await
                .get(&fixture.source.selected)
                .unwrap()
                .read_property(PropertyIdentifier::RELIABILITY, None)
                .unwrap(),
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
        );
        fixture.owner.close();
        fixture.record(BACnetTimeStamp::SequenceNumber(4), confirmed, false, false);
        assert!(fixture.owner.join_next().await.is_none());
        assert!(fixture.records.try_recv().is_err());
        fixture.stop().await;
    }
}
