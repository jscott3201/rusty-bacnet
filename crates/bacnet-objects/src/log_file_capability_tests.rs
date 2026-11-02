//! Exactly the Trend Log, Trend Log Multiple and Event Log lend `LogBuffer`,
//! and exactly the File lends `FileStorage` and `FileConfiguration`, each
//! through both of its accessors, which lend the same object (#2179). A
//! lender dropped from, or added to, the catalogue fails here until its
//! table changes with it.

use crate::catalogue::representatives;
use crate::traits::BACnetObject;
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use bacnet_types::enums::ObjectType;

/// The built-in types that lend a log buffer.
const LOG_BUFFER_LENDERS: [ObjectType; 3] = [
    ObjectType::TREND_LOG,
    ObjectType::TREND_LOG_MULTIPLE,
    ObjectType::EVENT_LOG,
];

/// The built-in type that lends File storage and configuration.
const FILE_LENDERS: [ObjectType; 1] = [ObjectType::FILE];

/// The address one accessor lends, if any.
type Lent = Option<*const ()>;

/// Check one capability's lenders across the catalogue: `lent` answers what
/// the shared and the mutable accessor lend.
fn lenders_are(
    capability: &str,
    lenders: &[ObjectType],
    lent: impl Fn(&mut Box<dyn BACnetObject>) -> (Lent, Lent),
) {
    let mut seen = BTreeSet::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        let (shared, exclusive) = lent(&mut object);
        assert_eq!(
            shared, exclusive,
            "{object_type:?}: the two {capability} accessors must lend the same object"
        );
        let lends = lenders.contains(&object_type);
        assert_eq!(
            shared.is_some(),
            lends,
            "{object_type:?} {} lend {capability}",
            if lends { "must" } else { "must not" }
        );
        seen.insert(object_type.to_raw());
    }
    for object_type in lenders {
        assert!(
            seen.contains(&object_type.to_raw()),
            "{object_type:?} is not in the catalogue"
        );
    }
}

fn address<T: ?Sized>(lent: &T) -> *const () {
    (lent as *const T).cast()
}

#[test]
fn exactly_the_logs_lend_log_buffer_through_both_accessors() {
    lenders_are("LogBuffer", &LOG_BUFFER_LENDERS, |object| {
        let shared = object.log_buffer().map(address);
        (shared, object.log_buffer_mut().map(|log| address(log)))
    });
}

#[test]
fn every_log_buffer_pages_its_records() {
    for object in representatives() {
        if let Some(log) = object.log_buffer() {
            let object_type = object.object_identifier().object_type();
            let records = log.buffer_records();
            assert!(records.is_some(), "{object_type:?} pages no records");
            assert_eq!(
                records.map(|records| records.record_count()),
                Some(log.record_identities().len()),
                "{object_type:?}: identities must align with the records"
            );
        }
    }
}

#[test]
fn exactly_the_file_lends_file_storage_through_both_accessors() {
    lenders_are("FileStorage", &FILE_LENDERS, |object| {
        let shared = object.file_storage().map(address);
        (shared, object.file_storage_mut().map(|file| address(file)))
    });
}

#[test]
fn exactly_the_file_lends_file_configuration_through_both_accessors() {
    lenders_are("FileConfiguration", &FILE_LENDERS, |object| {
        let shared = object.file_configuration().map(address);
        let exclusive = object.file_configuration_mut().map(|file| address(file));
        (shared, exclusive)
    });
}

/// A custom object of any log type: none of the built-in log state.
struct CustomLog(bacnet_types::primitives::ObjectIdentifier, &'static str);

impl BACnetObject for CustomLog {
    fn object_identifier(&self) -> bacnet_types::primitives::ObjectIdentifier {
        self.0
    }
    fn object_name(&self) -> &str {
        self.1
    }
    fn read_property(
        &self,
        _: bacnet_types::enums::PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<bacnet_types::primitives::PropertyValue, bacnet_types::error::Error> {
        Err(crate::common::unknown_property_error())
    }
    fn write_property(
        &mut self,
        _: bacnet_types::enums::PropertyIdentifier,
        _: Option<u32>,
        _: bacnet_types::primitives::PropertyValue,
        _: Option<u8>,
    ) -> Result<(), bacnet_types::error::Error> {
        Err(crate::common::unknown_property_error())
    }
    fn property_metadata(
        &self,
    ) -> alloc::borrow::Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
}

fn oid(object_type: ObjectType, instance: u32) -> bacnet_types::primitives::ObjectIdentifier {
    bacnet_types::primitives::ObjectIdentifier::new(object_type, instance).unwrap()
}

#[test]
fn stored_as_reflects_the_stored_object_not_its_box() {
    use crate::database::ObjectDatabase;
    use crate::event_log::EventLogObject;
    let mut db = ObjectDatabase::new();
    db.add(Box::new(EventLogObject::new(1, "log", 4).unwrap()))
        .unwrap();
    db.add(Box::new(CustomLog(oid(ObjectType::EVENT_LOG, 2), "custom")))
        .unwrap();
    // Reflecting the Box instead would make this miss.
    assert!(
        db.stored_as::<EventLogObject>(&oid(ObjectType::EVENT_LOG, 1))
            .is_some()
    );
    assert!(
        db.stored_as::<EventLogObject>(&oid(ObjectType::EVENT_LOG, 2))
            .is_none()
    );
    assert!(
        db.stored_as::<CustomLog>(&oid(ObjectType::EVENT_LOG, 1))
            .is_none()
    );
    assert!(
        db.stored_as::<EventLogObject>(&oid(ObjectType::EVENT_LOG, 3))
            .is_none()
    );
}

#[test]
fn a_custom_trend_log_stays_collecting_and_a_custom_event_log_takes_nothing_received() {
    use crate::database::ObjectDatabase;
    let mut db = ObjectDatabase::new();
    let trend = oid(ObjectType::TREND_LOG, 1);
    let event = oid(ObjectType::EVENT_LOG, 2);
    db.add(Box::new(CustomLog(trend, "custom trend"))).unwrap();
    db.add(Box::new(CustomLog(event, "custom event"))).unwrap();
    // Only the built-in Trend Log has an Enable and a window to shut.
    assert!(db.trend_log_collecting(&trend));
    assert!(!db.trend_log_collecting(&oid(ObjectType::TREND_LOG, 9)));
    // Only the built-in Event Log can opt in to received notifications.
    assert!(!db.logs_received(&event));
    assert!(!db.collects_received_event_notifications());
}
