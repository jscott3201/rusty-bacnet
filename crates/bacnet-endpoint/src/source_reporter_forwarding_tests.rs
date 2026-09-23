use super::*;
use bacnet_objects::audit::{AuditLogNotificationSink, AuditLogQueryPage, AuditLogStorage};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::event_enrollment::{EventEnrollmentEvalState, EventEnrollmentMonitoredSource};
use bacnet_objects::file::{FileConfiguration, FileObject, FileStorage, FileWriteStart};
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::{MonotonicClock, ReliabilityEvaluation, WritePropertyRollback};
use bacnet_types::bitstring::{AuditOperationFlags, BACnetPriorityFilter};
use bacnet_types::constructed::{
    BACnetAuditLogQueryParameters, BACnetAuditNotification, BACnetObjectSelector,
};
use bacnet_types::enums::{ErrorClass, ErrorCode};

fn read(object: &dyn BACnetObject, property: PropertyIdentifier) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn denied(result: Result<(), Error>) {
    assert!(matches!(result, Err(Error::Protocol { class, code })
        if class == ErrorClass::PROPERTY.to_raw() as u32
            && code == ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32));
}

#[tokio::test]
async fn built_in_configuration_identity_metadata_and_writes_survive_wrapping() {
    let (session, _peer, _) = session(SessionRole::Both);
    let mut db = database();
    let selectors = Some(vec![BACnetObjectSelector::Object(target())]);
    let priorities = BACnetPriorityFilter::from_bits(1 << 7);
    let operations = AuditOperationFlags::from_bits(1 << 1).unwrap();
    let original = db.get_mut(&selected()).unwrap();
    original
        .configure_audit_reporter_with_filters_internal(
            AuditLevel::AUDIT_ALL,
            operations,
            true,
            selectors.clone(),
            priorities,
        )
        .unwrap();
    let metadata = original.property_metadata().into_owned();
    let properties = original.property_list().into_owned();
    let required = original.required_properties().into_owned();
    let values: Vec<_> = properties
        .iter()
        .filter(|&&p| p != PropertyIdentifier::AUDIT_SOURCE_REPORTER)
        .map(|&p| (p, read(original.as_ref(), p)))
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
            assert_eq!(read(object.as_ref(), property), expected);
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
            read(object.as_ref(), PropertyIdentifier::AUDIT_SOURCE_REPORTER),
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
        assert!(object.is_array_property(PropertyIdentifier::MONITORED_OBJECTS));
        assert_eq!(
            object
                .read_property(PropertyIdentifier::MONITORED_OBJECTS, Some(0))
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert!(object
            .read_property(PropertyIdentifier::MONITORED_OBJECTS, Some(2))
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
            read(object.as_ref(), PropertyIdentifier::DESCRIPTION),
            PropertyValue::CharacterString("kept".into())
        );
        let before = object
            .property_list()
            .iter()
            .map(|&p| (p, read(object.as_ref(), p)))
            .collect::<Vec<_>>();
        assert!(object
            .configure_audit_reporter_with_filters_internal(
                AuditLevel::DEFAULT,
                AuditOperationFlags::empty(),
                false,
                None,
                BACnetPriorityFilter::all(),
            )
            .is_err());
        for (property, expected) in before {
            assert_eq!(read(object.as_ref(), property), expected);
        }
        object
            .configure_audit_reporter_with_filters_internal(
                AuditLevel::AUDIT_CONFIG,
                operations,
                false,
                None,
                BACnetPriorityFilter::all(),
            )
            .unwrap();
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::AUDIT_LEVEL),
            PropertyValue::Enumerated(AuditLevel::AUDIT_CONFIG.to_raw())
        );
        assert!(!object
            .property_list()
            .contains(&PropertyIdentifier::MONITORED_OBJECTS));
        object
            .configure_audit_reporter_internal(AuditLevel::NONE, operations, true)
            .unwrap();
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::AUDIT_LEVEL),
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw())
        );
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::AUDIT_SOURCE_REPORTER),
            PropertyValue::Boolean(true)
        );
    }
    session.stop().await.unwrap();
}

const CUSTOM: PropertyIdentifier = PropertyIdentifier::from_raw(5000);

#[derive(Default)]
struct Calls {
    clock_bindings: AtomicUsize,
    monotonic_bindings: AtomicUsize,
    configurations: AtomicUsize,
    clock_reads: AtomicUsize,
}

impl ClockReader for Calls {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.clock_reads.fetch_add(1, Ordering::SeqCst);
        None
    }
}

// Deliberately implements the legacy three-argument Reporter hook only. No new
// trait implementation is required for existing downstream Reporter types.
struct ExtendedReporter {
    reporter: AuditReporterObject,
    calls: Arc<Calls>,
    clock: Option<Arc<dyn ClockReader>>,
    monotonic: Option<Arc<MonotonicClock>>,
    value: u64,
    file: FileObject,
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
    ) -> Result<(), Error> {
        self.reporter
            .configure_audit_reporter_internal(level, operations, confirmed)?;
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
    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
        self.calls.clock_bindings.fetch_add(1, Ordering::SeqCst);
    }
    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.monotonic = clock;
        self.calls.monotonic_bindings.fetch_add(1, Ordering::SeqCst);
    }
    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.monotonic.as_ref().map(|clock| clock())
    }
    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        let _ = self.clock.as_ref().unwrap().read_clock();
        self.value += elapsed.as_secs();
        true
    }
    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.value = now.as_secs();
        true
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn supports_cov_property(&self, p: PropertyIdentifier) -> bool {
        p == CUSTOM
    }
    fn cov_increment(&self) -> Option<f32> {
        Some(1.25)
    }
    fn cov_snapshot_internal(&self) -> Option<Box<dyn BACnetObject>> {
        Some(Box::new(
            AuditReporterObject::new(17, "Custom snapshot").unwrap(),
        ))
    }
    fn binary_lighting_blink_count_internal(&self) -> u64 {
        123
    }
    fn capture_write_property_rollback(
        &mut self,
        p: PropertyIdentifier,
        value: &PropertyValue,
    ) -> Option<WritePropertyRollback> {
        assert_eq!(p, CUSTOM);
        assert_eq!(value, &PropertyValue::Unsigned(42));
        Some(WritePropertyRollback::new(self.value))
    }
    fn restore_write_property_rollback(
        &mut self,
        rollback: WritePropertyRollback,
    ) -> Result<(), Error> {
        self.value = rollback.downcast::<u64>()?;
        Ok(())
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
    fn evaluate_reliability_internal(&mut self) -> Result<ReliabilityEvaluation, Error> {
        Ok(ReliabilityEvaluation::Changed {
            old_reliability: 1,
            new_reliability: 2,
        })
    }
    fn reliability_evaluation_inhibited_internal(&self) -> bool {
        true
    }
    fn file_configuration_internal(&self) -> Option<&dyn FileConfiguration> {
        Some(&self.file)
    }
    fn file_configuration_internal_mut(&mut self) -> Option<&mut dyn FileConfiguration> {
        Some(&mut self.file)
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        Some(&self.file)
    }
    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        Some(&mut self.file)
    }
    fn audit_log_storage_internal(&self) -> Option<&dyn AuditLogStorage> {
        Some(self)
    }
    fn audit_log_notification_sink_internal(
        &mut self,
    ) -> Option<&mut dyn AuditLogNotificationSink> {
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
}

impl AuditLogNotificationSink for ExtendedReporter {
    fn notification_logging_enabled(&self) -> bool {
        true
    }
    fn store_notifications(&mut self, _: &[BACnetAuditNotification], _: u32) -> Result<(), Error> {
        Err(Error::Encoding("custom sink failure".into()))
    }
}

#[tokio::test]
async fn legacy_custom_capabilities_clocks_indexes_and_private_state_are_retained() {
    let (session, _peer, _) = session(SessionRole::ClientOnly);
    let calls = Arc::new(Calls::default());
    let monitored = (target(), PropertyIdentifier::DESCRIPTION, None);
    let mut db = database();
    db.add(Box::new(ExtendedReporter {
        reporter: AuditReporterObject::new(1, "Custom Reporter").unwrap(),
        calls: calls.clone(),
        clock: None,
        monotonic: None,
        value: 7,
        file: FileObject::new(5, "Private file capability", "test").unwrap(),
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
        assert!(object.is_writable_property(CUSTOM));
        assert_eq!(read(object.as_ref(), CUSTOM), PropertyValue::Unsigned(7));
        let rollback = object
            .capture_write_property_rollback(CUSTOM, &PropertyValue::Unsigned(42))
            .unwrap();
        object
            .write_property(CUSTOM, Some(3), PropertyValue::Unsigned(42), Some(7))
            .unwrap();
        assert_eq!(read(object.as_ref(), CUSTOM), PropertyValue::Unsigned(42));
        object.restore_write_property_rollback(rollback).unwrap();
        assert_eq!(read(object.as_ref(), CUSTOM), PropertyValue::Unsigned(7));
        assert!(object
            .restore_write_property_rollback(WritePropertyRollback::new(false))
            .is_err());
        assert_eq!(read(object.as_ref(), CUSTOM), PropertyValue::Unsigned(7));
        let operations = AuditOperationFlags::from_bits(1 << 1).unwrap();
        object
            .configure_audit_reporter_internal(AuditLevel::AUDIT_ALL, operations, true)
            .unwrap();
        object
            .configure_audit_reporter_with_filters_internal(
                AuditLevel::NONE,
                operations,
                false,
                None,
                BACnetPriorityFilter::all(),
            )
            .unwrap();
        assert_eq!(calls.configurations.load(Ordering::SeqCst), 2);
        assert!(object
            .configure_audit_reporter_with_filters_internal(
                AuditLevel::AUDIT_ALL,
                operations,
                true,
                Some(vec![]),
                BACnetPriorityFilter::all()
            )
            .is_err());
        assert_eq!(calls.configurations.load(Ordering::SeqCst), 2);
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::AUDIT_LEVEL),
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw())
        );
        assert!(object.supports_cov());
        assert!(object.supports_cov_property(CUSTOM));
        assert!(!object.supports_cov_property(PropertyIdentifier::DESCRIPTION));
        assert_eq!(object.cov_increment(), Some(1.25));
        assert_eq!(
            object.cov_snapshot_internal().unwrap().object_name(),
            "Custom snapshot"
        );
        assert_eq!(object.binary_lighting_blink_count_internal(), 123);
        assert_eq!(
            object.next_monotonic_deadline_internal(),
            Some(Duration::from_secs(99))
        );
        assert!(object.advance_time_internal(Duration::from_secs(5)));
        assert_eq!(calls.clock_reads.load(Ordering::SeqCst), 1);
        assert_eq!(read(object.as_ref(), CUSTOM), PropertyValue::Unsigned(12));
        assert!(object.advance_monotonic_time_internal(Duration::from_secs(33)));
        assert_eq!(read(object.as_ref(), CUSTOM), PropertyValue::Unsigned(33));
        object.bind_clock_internal(Some(calls.clone()));
        object.bind_monotonic_clock_internal(Some(Arc::new(|| Duration::from_secs(101))));
        assert_eq!(calls.clock_bindings.load(Ordering::SeqCst), clock_binds + 1);
        assert_eq!(
            calls.monotonic_bindings.load(Ordering::SeqCst),
            monotonic_binds + 1
        );
        assert_eq!(
            object.next_monotonic_deadline_internal(),
            Some(Duration::from_secs(101))
        );
        assert_eq!(
            object.enrollment_eval_source_internal(),
            Some(Some(monitored))
        );
        object.set_enrollment_eval_source_internal(None).unwrap();
        assert_eq!(object.enrollment_eval_source_internal(), Some(None));
        let eval = object.enrollment_eval_state_internal().unwrap();
        object.set_enrollment_eval_state_internal(eval).unwrap();
        assert_eq!(
            object.evaluate_reliability_internal().unwrap(),
            ReliabilityEvaluation::Changed {
                old_reliability: 1,
                new_reliability: 2
            }
        );
        assert!(object.reliability_evaluation_inhibited_internal());
        object
            .file_configuration_internal_mut()
            .unwrap()
            .set_stream_data(vec![1, 2])
            .unwrap();
        assert_eq!(
            object
                .file_configuration_internal()
                .unwrap()
                .stream_data()
                .unwrap(),
            &[1, 2]
        );
        object
            .file_storage_internal_mut()
            .unwrap()
            .write_stream(FileWriteStart::Append, &[3])
            .unwrap();
        assert_eq!(
            object
                .file_storage_internal()
                .unwrap()
                .read_stream(0, 3)
                .unwrap()
                .data,
            vec![1, 2, 3]
        );
        let sink = object.audit_log_notification_sink_internal().unwrap();
        assert!(sink.notification_logging_enabled());
        assert!(
            matches!(sink.store_notifications(&[], 77), Err(Error::Encoding(message)) if message == "custom sink failure")
        );
        assert_eq!(
            read(object.as_ref(), PropertyIdentifier::AUDIT_SOURCE_REPORTER),
            PropertyValue::Boolean(true)
        );
        assert!(!object.is_deleteable());
    }
    session.stop().await.unwrap();
}
