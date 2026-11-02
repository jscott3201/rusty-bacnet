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
