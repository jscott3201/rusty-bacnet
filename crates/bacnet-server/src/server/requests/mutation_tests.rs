use super::*;
use crate::mutation::{MutationAuthorizationContext, MutationAuthorizer, MutationTarget};
use bacnet_encoding::{apdu::decode_apdu, npdu::decode_npdu};
use bacnet_objects::{
    binary::BinaryValueObject, file::FileObject, multistate::MultiStateInputObject,
};
use bacnet_services::common::PropertyReference;
use bacnet_services::cov::{SubscribeCOVPropertyRequest, SubscribeCOVRequest};
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_services::file::{AtomicWriteFileRequest, FileWriteAccessMethod};
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_services::object_mgmt::{CreateObjectRequest, DeleteObjectRequest, ObjectSpecifier};
use bacnet_services::wpm::{
    WriteAccessSpecification, WritePropertyAttempt, WritePropertyMultipleRequest,
};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use std::sync::{atomic::AtomicUsize, Mutex as StdMutex};

pub(super) fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

pub(super) fn value(value: PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, &value).unwrap();
    bytes.to_vec()
}

pub(super) fn wpm(specs: Vec<WriteAccessSpecification>) -> Bytes {
    let mut bytes = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: specs,
    }
    .encode(&mut bytes);
    bytes.freeze()
}

pub(super) fn route() -> Option<NpduAddress> {
    Some(NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[9]),
    })
}

pub(super) const SOURCE: &[u8] = &[127, 0, 0, 1, 0xba, 0xc0];

#[derive(Default)]
pub(super) struct TestTransport {
    pub sent: StdMutex<Vec<Bytes>>,
}

impl TransportPort for TestTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        Ok(mpsc::channel(1).1)
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, bytes: &[u8], _: &[u8]) -> Result<(), Error> {
        self.sent
            .lock()
            .unwrap()
            .push(Bytes::copy_from_slice(bytes));
        Ok(())
    }
    async fn send_broadcast(&self, bytes: &[u8]) -> Result<(), Error> {
        self.send_unicast(bytes, &[]).await
    }
    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

pub(super) struct Fixture {
    pub db: Arc<RwLock<ObjectDatabase>>,
    pub table: Arc<RwLock<CovSubscriptionTable>>,
    pub config: ServerConfig,
    pub state: Arc<AtomicU8>,
    pub tracker: Arc<ConfirmedRequestTracker>,
    pub network: Arc<NetworkLayer<TestTransport>>,
}

impl Fixture {
    pub fn new(authorizer: Option<MutationAuthorizer>) -> Self {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(BinaryValueObject::new(1, "one").unwrap()))
            .unwrap();
        db.add(Box::new(BinaryValueObject::new(2, "two").unwrap()))
            .unwrap();
        let mut msi = MultiStateInputObject::new(1, "msi", 3).unwrap();
        msi.set_alarm_values(vec![1]);
        db.add(Box::new(msi)).unwrap();
        let mut file = FileObject::new(1, "file", "binary").unwrap();
        file.set_data(b"sentinel".to_vec());
        db.add(Box::new(file)).unwrap();
        Self {
            db: Arc::new(RwLock::new(db)),
            table: Arc::new(RwLock::new(CovSubscriptionTable::new())),
            config: ServerConfig {
                mutation_authorizer: authorizer,
                ..Default::default()
            },
            state: Arc::new(AtomicU8::new(0)),
            tracker: Arc::new(ConfirmedRequestTracker::default()),
            network: Arc::new(NetworkLayer::new(TestTransport::default())),
        }
    }

    pub async fn dispatch(
        &self,
        service: ConfirmedServiceChoice,
        bytes: Bytes,
        id: u8,
    ) -> Option<Bytes> {
        let (tx, rx) = oneshot::channel();
        BACnetServer::<TestTransport>::handle_confirmed_request(
            &self.db,
            &self.network,
            &self.table,
            &Arc::new(segmented_send::SegmentedSendRegistry::default()),
            &Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
            &Arc::new(Semaphore::new(1)),
            &Arc::new(Mutex::new(ServerTsm::new())),
            &NotificationTransactions::new(),
            &self.tracker,
            &Arc::new(RwLock::new(DeviceBindingTable::new())),
            &self.state,
            &Arc::new(Mutex::new(None)),
            &self.config,
            &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
            SOURCE,
            route(),
            ConfirmedRequestPdu {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: false,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: service,
                service_request: bytes,
            },
            Some(tx),
        )
        .await;
        rx.await.ok()
    }

    pub async fn read(
        &self,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
    ) -> PropertyValue {
        self.db
            .read()
            .await
            .get(&object)
            .unwrap()
            .read_property(property, None)
            .unwrap()
    }

    async fn snapshot(
        &self,
    ) -> (
        Vec<(ObjectIdentifier, PropertyIdentifier, PropertyValue)>,
        Vec<u8>,
        usize,
    ) {
        let db = self.db.read().await;
        let mut oids = db.list_objects();
        oids.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
        let mut values = Vec::new();
        for oid in oids {
            let object = db.get(&oid).unwrap();
            for property in [
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::ALARM_VALUES,
                PropertyIdentifier::FILE_SIZE,
                PropertyIdentifier::ARCHIVE,
            ] {
                if let Ok(value) = object.read_property(property, None) {
                    values.push((oid, property, value));
                }
            }
        }
        let file = db
            .get(&oid(ObjectType::FILE, 1))
            .unwrap()
            .file_storage_internal()
            .unwrap()
            .read_stream(0, 100)
            .unwrap()
            .data;
        (values, file, self.table.read().await.len())
    }
}

pub(super) fn apdu(bytes: Bytes) -> Apdu {
    let npdu = decode_npdu(bytes).unwrap();
    assert_eq!(npdu.destination, route());
    decode_apdu(npdu.payload).unwrap()
}

pub(super) fn assert_denied(bytes: Bytes, service: ConfirmedServiceChoice, id: u8) {
    let Apdu::Error(error) = apdu(bytes) else {
        panic!("expected Error")
    };
    assert_eq!(error.invoke_id, id);
    assert_eq!(error.service_choice, service);
    assert_eq!(error.error_class, ErrorClass::SERVICES);
    assert_eq!(error.error_code, ErrorCode::SERVICE_REQUEST_DENIED);
}

pub(super) fn cases() -> Vec<(ConfirmedServiceChoice, Bytes, MutationTarget)> {
    let mut cases = Vec::new();
    macro_rules! case {
        ($service:ident, $target:ident, $request:expr) => {{
            let request = $request;
            let mut bytes = BytesMut::new();
            request.encode(&mut bytes);
            cases.push((
                ConfirmedServiceChoice::$service,
                bytes.freeze(),
                MutationTarget::$target(request),
            ));
        }};
    }
    let object = oid(ObjectType::BINARY_VALUE, 1);
    case!(
        WRITE_PROPERTY,
        WriteProperty,
        WritePropertyRequest {
            object_identifier: object,
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            property_value: value(PropertyValue::Enumerated(1)),
            priority: Some(8),
        }
    );
    cases.push((
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        wpm(vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: value(PropertyValue::Enumerated(1)),
                priority: Some(8),
            }],
        }]),
        MutationTarget::WritePropertyMultiple(WritePropertyAttempt {
            reference: BACnetObjectPropertyReference {
                object_identifier: object,
                property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                property_array_index: None,
            },
            value: value(PropertyValue::Enumerated(1)),
            priority: Some(8),
        }),
    ));
    case!(
        CREATE_OBJECT,
        CreateObject,
        CreateObjectRequest {
            object_specifier: ObjectSpecifier::Identifier(oid(ObjectType::BINARY_VALUE, 3)),
            list_of_initial_values: vec![],
        }
    );
    case!(
        DELETE_OBJECT,
        DeleteObject,
        DeleteObjectRequest {
            object_identifier: oid(ObjectType::BINARY_VALUE, 2)
        }
    );
    case!(
        ADD_LIST_ELEMENT,
        AddListElement,
        ListElementRequest {
            object_identifier: oid(ObjectType::MULTI_STATE_INPUT, 1),
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            list_of_elements: value(PropertyValue::Unsigned(2)),
        }
    );
    case!(
        REMOVE_LIST_ELEMENT,
        RemoveListElement,
        ListElementRequest {
            object_identifier: oid(ObjectType::MULTI_STATE_INPUT, 1),
            property_identifier: PropertyIdentifier::ALARM_VALUES,
            property_array_index: None,
            list_of_elements: value(PropertyValue::Unsigned(1)),
        }
    );
    case!(
        ATOMIC_WRITE_FILE,
        AtomicWriteFile,
        AtomicWriteFileRequest {
            file_identifier: oid(ObjectType::FILE, 1),
            access: FileWriteAccessMethod::Stream {
                file_start_position: -1,
                file_data: b"changed".to_vec()
            },
        }
    );
    case!(
        SUBSCRIBE_COV,
        SubscribeCov,
        SubscribeCOVRequest {
            subscriber_process_identifier: 41,
            monitored_object_identifier: object,
            issue_confirmed_notifications: Some(false),
            lifetime: Some(600),
        }
    );
    case!(
        SUBSCRIBE_COV_PROPERTY,
        SubscribeCovProperty,
        SubscribeCOVPropertyRequest {
            subscriber_process_identifier: 41,
            monitored_object_identifier: object,
            issue_confirmed_notifications: Some(false),
            lifetime: Some(600),
            monitored_property_identifier: PropertyIdentifier::PRESENT_VALUE,
            monitored_property_array_index: None,
            cov_increment: None,
        }
    );
    case!(
        SUBSCRIBE_COV_PROPERTY_MULTIPLE,
        SubscribeCovPropertyMultiple,
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 41,
            issue_confirmed_notifications: false,
            lifetime: Some(600),
            max_notification_delay: Some(1),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: object,
                list_of_cov_references: [
                    PropertyIdentifier::PRESENT_VALUE,
                    PropertyIdentifier::STATUS_FLAGS
                ]
                .into_iter()
                .map(|property| COVReference {
                    monitored_property: PropertyReference {
                        property_identifier: property,
                        property_array_index: None
                    },
                    cov_increment: None,
                    timestamped: false,
                })
                .collect(),
            }],
        }
    );
    cases
}

async fn denial_case(index: usize) {
    let (service, bytes, target) = cases().remove(index);
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let captured = observed.clone();
    let fixture = Fixture::new(Some(Arc::new(move |context| {
        captured.lock().unwrap().push(context.clone());
        false
    })));
    let before = fixture.snapshot().await;
    assert_denied(
        fixture.dispatch(service, bytes.clone(), 51).await.unwrap(),
        service,
        51,
    );
    assert_eq!(fixture.snapshot().await, before);
    assert!(fixture.network.transport().sent.lock().unwrap().is_empty());
    assert_eq!(
        *observed.lock().unwrap(),
        vec![MutationAuthorizationContext {
            source_mac: MacAddr::from_slice(SOURCE),
            source_network: route(),
            invoke_id: 51,
            service_choice: service,
            target,
        }]
    );
    // Denial completed the normal tracker path; exact retries do not reauthorize.
    assert!(fixture.dispatch(service, bytes.clone(), 51).await.is_none());
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_denied(
        fixture.dispatch(service, bytes, 52).await.unwrap(),
        service,
        52,
    );
    assert_eq!(observed.lock().unwrap().len(), 2);
}

macro_rules! denial_test {
    ($name:ident, $index:expr) => {
        #[tokio::test]
        async fn $name() {
            denial_case($index).await;
        }
    };
}
denial_test!(deny_write_property, 0);
denial_test!(deny_write_property_multiple, 1);
denial_test!(deny_create_object, 2);
denial_test!(deny_delete_object, 3);
denial_test!(deny_add_list_element, 4);
denial_test!(deny_remove_list_element, 5);
denial_test!(deny_atomic_write_file, 6);
denial_test!(deny_subscribe_cov, 7);
denial_test!(deny_subscribe_cov_property, 8);
denial_test!(deny_subscribe_cov_property_multiple, 9);

#[tokio::test]
async fn default_allow_and_allow_all_have_identical_wire_and_state_for_all_ten() {
    assert!(ServerConfig::default().mutation_authorizer.is_none());
    for (service, bytes, _) in cases() {
        let absent = Fixture::new(None);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let allow = Fixture::new(Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            true
        })));
        let before = absent.snapshot().await;
        let baseline = absent.dispatch(service, bytes.clone(), 1).await.unwrap();
        let authorized = allow.dispatch(service, bytes, 1).await.unwrap();
        assert!(
            matches!(
                apdu(baseline.clone()),
                Apdu::SimpleAck(_) | Apdu::ComplexAck(_)
            ),
            "{service:?}"
        );
        assert_eq!(baseline, authorized, "{service:?}");
        assert_eq!(
            absent.snapshot().await,
            allow.snapshot().await,
            "{service:?}"
        );
        assert_ne!(
            absent.snapshot().await,
            before,
            "positive fixture must mutate: {service:?}"
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1, "{service:?}");
    }
}

#[tokio::test]
async fn panics_deny_all_ten_without_mutation() {
    for (service, bytes, _) in cases() {
        let fixture = Fixture::new(Some(Arc::new(|_| panic!("policy panic"))));
        let before = fixture.snapshot().await;
        assert_denied(
            fixture.dispatch(service, bytes, 1).await.unwrap(),
            service,
            1,
        );
        assert_eq!(fixture.snapshot().await, before);
    }
}

#[tokio::test]
async fn dcc_precheck_precedes_decoding_and_authorization_for_all_ten() {
    let calls = Arc::new(AtomicUsize::new(0));
    for (service, bytes, _) in cases() {
        let seen = calls.clone();
        let fixture = Fixture::new(Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            false
        })));
        fixture.state.store(1, Ordering::Release);
        let before = fixture.snapshot().await;
        assert!(fixture.dispatch(service, bytes, 1).await.is_none());
        assert!(fixture.dispatch(service, Bytes::new(), 2).await.is_none());
        assert_eq!(fixture.snapshot().await, before);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn malformed_service_requests_keep_existing_errors_without_authorization() {
    let calls = Arc::new(AtomicUsize::new(0));
    for (service, mut bytes, _) in cases() {
        // Cut inside the first required field, not an optional trailing field.
        bytes.truncate(1);
        let absent = Fixture::new(None);
        let seen = calls.clone();
        let deny = Fixture::new(Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            false
        })));
        let before = deny.snapshot().await;
        let baseline = absent.dispatch(service, bytes.clone(), 1).await.unwrap();
        assert!(matches!(
            apdu(baseline.clone()),
            Apdu::Error(_) | Apdu::Reject(_)
        ));
        assert_eq!(
            baseline,
            deny.dispatch(service, bytes, 1).await.unwrap(),
            "{service:?}"
        );
        assert_eq!(deny.snapshot().await, before);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn malformed_final_field_is_decoded_before_policy_for_non_wpm_services() {
    let calls = Arc::new(AtomicUsize::new(0));
    for (service, mut bytes, _) in cases() {
        if service == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE {
            continue; // Its malformed-suffix prefix behavior is tested separately.
        }
        bytes.truncate(bytes.len() - 1);
        let absent = Fixture::new(None);
        let seen = calls.clone();
        let deny = Fixture::new(Some(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::Relaxed);
            false
        })));
        let baseline = absent.dispatch(service, bytes.clone(), 1).await.unwrap();
        assert!(matches!(
            apdu(baseline.clone()),
            Apdu::Error(_) | Apdu::Reject(_)
        ));
        assert_eq!(
            baseline,
            deny.dispatch(service, bytes, 1).await.unwrap(),
            "{service:?}"
        );
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn builders_install_mutation_policy_and_debug_redacts_callbacks() {
    assert!(BACnetServer::<BipTransport>::generic_builder()
        .mutation_authorizer(|_| false)
        .config
        .mutation_authorizer
        .is_some());
    let config = BACnetServer::bip_builder()
        .mutation_authorizer(|_| true)
        .config;
    assert!(config.mutation_authorizer.is_some());
    assert!(format!("{config:?}").contains("mutation_authorizer: Some(\"<callback>\")"));
    #[cfg(feature = "sc-tls")]
    assert!(BACnetServer::sc_builder()
        .mutation_authorizer(|_| false)
        .config
        .mutation_authorizer
        .is_some());
}
