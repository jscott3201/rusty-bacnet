use super::*;
use bacnet_objects::audit::{AuditLogQueryPage, AuditLogStorage};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::MonotonicClock;
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{BACnetAuditLogQueryParameters, BACnetObjectSelector};
use bacnet_types::enums::{ErrorClass, ErrorCode};

#[test]
fn typed_device_authority_is_forwarded_without_copying() {
    let device = bacnet_objects::device::DeviceObject::new(Default::default()).unwrap();
    let mut object: Box<dyn BACnetObject> = Box::new(device);
    let original = object
        .device_authority_internal()
        .unwrap()
        .object_identifier();
    let owner = bacnet_objects::database::AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        selected(),
    );
    source_reporter::install(&mut object, &owner).unwrap();
    let mut forwarded = object.device_authority_internal().unwrap();
    assert_eq!(forwarded.object_identifier(), original);
    forwarded
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("same authority".into()),
            None,
        )
        .unwrap();
    assert_eq!(
        read(object.as_ref(), PropertyIdentifier::DESCRIPTION),
        PropertyValue::CharacterString("same authority".into())
    );
}

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn denied(result: Result<(), Error>) {
    assert!(matches!(result, Err(Error::Protocol { class, code })
        if class == ErrorClass::PROPERTY
            && code == ErrorCode::WRITE_ACCESS_DENIED));
}

#[tokio::test]
async fn built_in_configuration_identity_metadata_and_writes_survive_wrapping() {
    let (session, _peer, _) = session(SessionRole::Both);
    let mut db = database();
    let selectors: Option<Vec<BACnetObjectSelector>> = None;
    let priorities = BACnetPriorityFilter::from_bits(1 << 7);
    let operations = AuditOperationFlags::from_bits(1 << 1).unwrap();
    let original = db.get_mut(&selected()).unwrap();
    original
        .configure_audit_reporter_internal(
            AuditLevel::AUDIT_ALL,
            operations,
            true,
            selectors.clone(),
            priorities,
            None,
        )
        .unwrap();
    let metadata = original.property_metadata().into_owned();
    let properties = original.property_list().into_owned();
    let required = original.required_properties().into_owned();
    let values: Vec<_> = properties
        .iter()
        .filter(|&&p| p != PropertyIdentifier::AUDIT_SOURCE_REPORTER)
        .map(|&p| (p, read(original, p)))
        .collect();
    let reporter_ptr = std::ptr::from_ref(original.audit_reporter_internal().unwrap());
    let status = original
        .audit_reporter_internal()
        .unwrap()
        .status_internal();
    let mut session = session
        .with_database(db)
        .with_source_audit_reporter(selected());
    session.start().await.unwrap();
    {
        let mut db = session.database.as_ref().unwrap().write().await;
        let object = db.get_mut(&selected()).unwrap();
        assert_eq!(object.object_identifier(), selected());
        assert_eq!(object.object_name(), "Reporter-1");
        assert_eq!(object.property_metadata().as_ref(), metadata);
        assert_eq!(object.property_list().as_ref(), properties);
        assert_eq!(object.required_properties().as_ref(), required);
        for (property, expected) in values {
            assert_eq!(read(object, property), expected);
        }
        assert!(std::ptr::eq(
            reporter_ptr,
            object.audit_reporter_internal().unwrap()
        ));
        assert!(Arc::ptr_eq(
            &status,
            &object.audit_reporter_internal().unwrap().status_internal()
        ));
        // Role projection belongs to the adapter; the borrowed configuration
        // object remains an ordinary Reporter, not a leaked promotion capability.
        assert_eq!(
            read(
                object.audit_reporter_internal().unwrap(),
                PropertyIdentifier::AUDIT_SOURCE_REPORTER
            ),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            read(object, PropertyIdentifier::AUDIT_SOURCE_REPORTER),
            PropertyValue::Boolean(true)
        );
        assert!(!object.is_deleteable());
        assert!(!object.is_createable());
        assert!(!object.is_writable_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER));
        denied(object.write_property(
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            None,
            PropertyValue::Boolean(false),
            None,
        ));
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::MONITORED_OBJECTS));
        assert!(object
            .read_property(PropertyIdentifier::MONITORED_OBJECTS, None)
            .is_err());
        object
            .write_property(
                PropertyIdentifier::DESCRIPTION,
                None,
                PropertyValue::CharacterString("kept".into()),
                None,
            )
            .unwrap();
        assert_eq!(
            read(object, PropertyIdentifier::DESCRIPTION),
            PropertyValue::CharacterString("kept".into())
        );
        let before = object
            .property_list()
            .iter()
            .map(|&p| (p, read(object, p)))
            .collect::<Vec<_>>();
        assert!(object
            .configure_audit_reporter_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                false,
                None,
                BACnetPriorityFilter::all(),
                None,
            )
            .is_err());
        for (property, expected) in before {
            assert_eq!(read(object, property), expected);
        }
        object
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_CONFIG,
                operations,
                false,
                None,
                BACnetPriorityFilter::all(),
                None,
            )
            .unwrap();
        assert_eq!(
            read(object, PropertyIdentifier::AUDIT_LEVEL),
            PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw())
        );
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::MONITORED_OBJECTS));
        object
            .configure_audit_reporter_internal(
                AuditLevel::NONE,
                operations,
                true,
                None,
                BACnetPriorityFilter::empty(),
                None,
            )
            .unwrap();
        assert_eq!(
            read(object, PropertyIdentifier::AUDIT_LEVEL),
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw())
        );
        assert_eq!(
            read(object, PropertyIdentifier::AUDIT_SOURCE_REPORTER),
            PropertyValue::Boolean(true)
        );
    }
    session.stop().await.unwrap();
}

const CUSTOM: PropertyIdentifier = PropertyIdentifier::from_raw(5000);
const CUSTOM_LIST: PropertyIdentifier = PropertyIdentifier::from_raw(5001);

#[derive(Default)]
struct Calls {
    clock_bindings: AtomicUsize,
    monotonic_bindings: AtomicUsize,
    configurations: AtomicUsize,
}

impl ClockReader for Calls {
    fn read_clock(&self) -> Option<ClockFrame> {
        None
    }
}

// A custom Reporter implements the same complete configuration contract as the
// built-in object; wrapping must preserve its override and all typed settings.
// trait_tests checks that each method reaches the wrapped object.
struct ExtendedReporter {
    reporter: AuditReporterObject,
    calls: Arc<Calls>,
    value: u64,
    eval: EventEnrollmentEvalState,
    source: Option<EventEnrollmentMonitoredSource>,
}

impl BACnetObject for ExtendedReporter {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.reporter.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.reporter.object_name()
    }
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        Some(&self.reporter)
    }
    fn configure_audit_reporter_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
        selectors: Option<Vec<BACnetObjectSelector>>,
        priorities: BACnetPriorityFilter,
        maximum_send_delay: Option<bacnet_objects::audit::AuditSendDelay>,
    ) -> Result<(), Error> {
        self.reporter.configure_audit_reporter_internal(
            level,
            operations,
            confirmed,
            selectors,
            priorities,
            maximum_send_delay,
        )?;
        self.calls.configurations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn read_property(
        &self,
        p: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if p == CUSTOM {
            Ok(PropertyValue::Unsigned(self.value))
        } else {
            self.reporter.read_property(p, index)
        }
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        if p == CUSTOM {
            assert_eq!(index, Some(3));
            assert_eq!(priority, Some(7));
            let PropertyValue::Unsigned(value) = value else {
                panic!("wrong value")
            };
            self.value = value;
            Ok(())
        } else {
            self.reporter.write_property(p, index, value, priority)
        }
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.reporter.property_metadata()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let mut properties = self.reporter.property_list().into_owned();
        properties.push(CUSTOM);
        Cow::Owned(properties)
    }
    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[CUSTOM])
    }
    fn is_writable_property(&self, p: PropertyIdentifier) -> bool {
        p == CUSTOM || self.reporter.is_writable_property(p)
    }
    fn is_array_property(&self, p: PropertyIdentifier) -> bool {
        p == CUSTOM || self.reporter.is_array_property(p)
    }
    fn is_list_property(&self, p: PropertyIdentifier) -> bool {
        p == CUSTOM_LIST || self.reporter.is_list_property(p)
    }
    fn bind_clock_internal(&mut self, _: Option<Arc<dyn ClockReader>>) {
        self.calls.clock_bindings.fetch_add(1, Ordering::SeqCst);
    }
    fn bind_monotonic_clock_internal(&mut self, _: Option<Arc<MonotonicClock>>) {
        self.calls.monotonic_bindings.fetch_add(1, Ordering::SeqCst);
    }
    fn enrollment_eval_state_internal(&self) -> Option<EventEnrollmentEvalState> {
        Some(self.eval.clone())
    }
    fn set_enrollment_eval_state_internal(
        &mut self,
        state: EventEnrollmentEvalState,
    ) -> Result<(), Error> {
        self.eval = state;
        Ok(())
    }
    fn enrollment_eval_source_internal(&self) -> Option<Option<EventEnrollmentMonitoredSource>> {
        Some(self.source)
    }
    fn set_enrollment_eval_source_internal(
        &mut self,
        source: Option<EventEnrollmentMonitoredSource>,
    ) -> Result<(), Error> {
        self.source = source;
        Ok(())
    }
    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        Some(self)
    }
}

impl AuditLogStorage for ExtendedReporter {
    fn query(
        &self,
        _: &BACnetAuditLogQueryParameters,
        _: Option<u64>,
        _: u16,
    ) -> AuditLogQueryPage {
        AuditLogQueryPage {
            records: vec![],
            no_more_items: false,
        }
    }

    fn retained_records(
        &self,
    ) -> &std::collections::VecDeque<bacnet_types::constructed::BACnetAuditLogRecordResult> {
        static EMPTY: std::collections::VecDeque<
            bacnet_types::constructed::BACnetAuditLogRecordResult,
        > = std::collections::VecDeque::new();
        &EMPTY
    }
}

#[tokio::test]
async fn custom_capabilities_clocks_indexes_and_private_state_are_retained() {
    let (session, _peer, _) = session(SessionRole::ClientOnly);
    let calls = Arc::new(Calls::default());
    let monitored = (target(), PropertyIdentifier::DESCRIPTION, None);
    let mut db = database();
    db.add(Box::new(ExtendedReporter {
        reporter: AuditReporterObject::new(1, "Custom Reporter").unwrap(),
        calls: calls.clone(),
        value: 7,
        eval: EventEnrollmentEvalState::default(),
        source: Some(monitored),
    }))
    .unwrap();
    db.set_clock_reader(Some(calls.clone()));
    db.set_monotonic_clock_internal(Some(Arc::new(|| Duration::from_secs(99))));
    db.set_enrollment_eval_state_invalidated(selected(), true);
    db.set_enrollment_eval_source(selected(), Some(monitored));
    let mut objects = db.list_objects();
    objects.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
    let mut reporters = db.find_by_type(ObjectType::AUDIT_REPORTER);
    reporters.sort_by_key(|oid| oid.instance_number());
    let clock_binds = calls.clock_bindings.load(Ordering::SeqCst);
    let monotonic_binds = calls.monotonic_bindings.load(Ordering::SeqCst);
    let capability = std::ptr::from_ref(
        db.get(&selected())
            .unwrap()
            .audit_reporter_internal()
            .unwrap(),
    );
    let storage = std::ptr::from_ref(
        db.get(&selected())
            .unwrap()
            .audit_log_storage_internal()
            .unwrap(),
    )
    .cast::<()>();
    let mut session = session
        .with_database(db)
        .with_source_audit_reporter(selected());
    session.start().await.unwrap();
    {
        let mut db = session.database.as_ref().unwrap().write().await;
        let mut after = db.list_objects();
        after.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
        assert_eq!(after, objects);
        let mut after = db.find_by_type(ObjectType::AUDIT_REPORTER);
        after.sort_by_key(|oid| oid.instance_number());
        assert_eq!(after, reporters);
        assert_eq!(
            db.find_by_name("Custom Reporter")
                .unwrap()
                .object_identifier(),
            selected()
        );
        assert!(db.find_by_name("Reporter-1").is_none());
        assert!(db
            .check_name_available(&target(), "Custom Reporter")
            .is_err());
        assert!(db.enrollment_eval_state_invalidated(&selected()));
        assert_eq!(db.enrollment_eval_source(&selected()), Some(monitored));
        assert_eq!(calls.clock_bindings.load(Ordering::SeqCst), clock_binds);
        assert_eq!(
            calls.monotonic_bindings.load(Ordering::SeqCst),
            monotonic_binds
        );
        let object = db.get_mut(&selected()).unwrap();
        assert!(std::ptr::eq(
            capability,
            object.audit_reporter_internal().unwrap()
        ));
        assert_eq!(
            storage,
            std::ptr::from_ref(object.audit_log_storage_internal().unwrap()).cast::<()>()
        );
        assert_eq!(object.object_name(), "Custom Reporter");
        assert_eq!(object.object_identifier(), selected());
        assert!(object.property_list().contains(&CUSTOM));
        assert_eq!(object.required_properties().as_ref(), &[CUSTOM]);
        assert!(object.is_array_property(CUSTOM));
        assert!(object.is_list_property(CUSTOM_LIST));
        assert!(!object.is_list_property(CUSTOM));
        assert!(object.is_writable_property(CUSTOM));
        assert_eq!(read(object, CUSTOM), PropertyValue::Unsigned(7));
        object
            .write_property(CUSTOM, Some(3), PropertyValue::Unsigned(42), Some(7))
            .unwrap();
        assert_eq!(read(object, CUSTOM), PropertyValue::Unsigned(42));
        object
            .write_property(CUSTOM, Some(3), PropertyValue::Unsigned(7), Some(7))
            .unwrap();
        assert_eq!(read(object, CUSTOM), PropertyValue::Unsigned(7));
        let operations = AuditOperationFlags::from_bits((1 << 1) | (1 << 63)).unwrap();
        let priorities = BACnetPriorityFilter::from_bits(1 << 7);
        object
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_ALL,
                operations,
                true,
                None,
                priorities,
                None,
            )
            .unwrap();
        assert_eq!(calls.configurations.load(Ordering::SeqCst), 1);
        assert_eq!(
            read(object, PropertyIdentifier::AUDIT_LEVEL),
            PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw())
        );
        assert_eq!(
            read(object, PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS),
            PropertyValue::Boolean(true)
        );
        let reporter = object.audit_reporter_internal().unwrap();
        assert!(reporter.monitors_object_internal(target()));
        assert!(reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(8)));
        assert!(!reporter.reports_write_internal(PropertyIdentifier::PRESENT_VALUE, Some(16)));
        let before = object
            .property_list()
            .iter()
            .map(|&p| (p, read(object, p)))
            .collect::<Vec<_>>();
        assert!(object
            .configure_audit_reporter_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                false,
                None,
                BACnetPriorityFilter::all(),
                None,
            )
            .is_err());
        assert_eq!(calls.configurations.load(Ordering::SeqCst), 1);
        for (property, expected) in before {
            assert_eq!(read(object, property), expected);
        }
        object
            .configure_audit_reporter_internal(
                AuditLevel::NONE,
                operations,
                false,
                None,
                BACnetPriorityFilter::all(),
                None,
            )
            .unwrap();
        assert_eq!(calls.configurations.load(Ordering::SeqCst), 2);
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::MONITORED_OBJECTS));
        assert_eq!(
            read(object, PropertyIdentifier::AUDIT_SOURCE_REPORTER),
            PropertyValue::Boolean(true)
        );
        assert!(!object.is_deleteable());
    }
    session.stop().await.unwrap();
}

#[path = "source_reporter_intrinsic_tests.rs"]
mod intrinsic;

#[path = "source_reporter_life_safety_tests.rs"]
mod life_safety;

#[path = "source_reporter_policy_tests.rs"]
mod object_policy;

#[path = "source_reporter_schedule_tests.rs"]
mod schedule;
