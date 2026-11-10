//! PICS writability from the runtime write routes, for the log family and
//! the File object (issue #115).
//!
//! That each row's write capability matches the object's write arm, for
//! every catalogue type and every row, is checked in bacnet-objects
//! (`write_capability_tests`, #2175); the catalogue-wide PICS check is
//! `catalogue_consistency_tests`.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::{PicsConfig, PicsGenerator};
use crate::server::ServerConfig;

#[test]
fn pics_log_family_writability_comes_from_runtime_routes() {
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_objects::event_log::EventLogObject;
    use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};

    let mut db = ObjectDatabase::new();
    db.add(Box::new(EventLogObject::new(1, "EL-1", 3).unwrap()))
        .unwrap();
    db.add(Box::new(TrendLogObject::new(1, "TL-1", 3).unwrap()))
        .unwrap();
    db.add(Box::new(
        TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap(),
    ))
    .unwrap();
    let server_config = ServerConfig::default();
    let pics_config = PicsConfig::default();
    let pics = PicsGenerator::new(&db, &server_config, &pics_config).generate();

    let cases = [
        (
            // Table 12-31 has no Log_Interval or Out_Of_Service (#1064).
            ObjectType::EVENT_LOG,
            &[
                PropertyIdentifier::ENABLE,
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::DESCRIPTION,
                // The window (#1353).
                PropertyIdentifier::START_TIME,
                PropertyIdentifier::STOP_TIME,
            ][..],
        ),
        (
            ObjectType::TREND_LOG,
            &[
                PropertyIdentifier::ENABLE,
                PropertyIdentifier::LOG_INTERVAL,
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
                // Logging_Type, the window, alignment and Trigger (#1353,
                // #1354).
                PropertyIdentifier::LOGGING_TYPE,
                PropertyIdentifier::START_TIME,
                PropertyIdentifier::STOP_TIME,
                PropertyIdentifier::ALIGN_INTERVALS,
                PropertyIdentifier::INTERVAL_OFFSET,
                PropertyIdentifier::TRIGGER,
                // The COV rows (#1480).
                PropertyIdentifier::COV_RESUBSCRIPTION_INTERVAL,
                PropertyIdentifier::CLIENT_COV_INCREMENT,
            ][..],
        ),
        (
            ObjectType::TREND_LOG_MULTIPLE,
            &[
                PropertyIdentifier::ENABLE,
                PropertyIdentifier::LOG_INTERVAL,
                PropertyIdentifier::STOP_WHEN_FULL,
                PropertyIdentifier::RECORD_COUNT,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
                // Logging_Type, the window, alignment and Trigger (#1235).
                PropertyIdentifier::LOGGING_TYPE,
                PropertyIdentifier::START_TIME,
                PropertyIdentifier::STOP_TIME,
                PropertyIdentifier::ALIGN_INTERVALS,
                PropertyIdentifier::INTERVAL_OFFSET,
                PropertyIdentifier::TRIGGER,
            ][..],
        ),
    ];
    for (object_type, expected) in cases {
        let support = pics
            .supported_object_types
            .iter()
            .find(|support| support.object_type == object_type)
            .unwrap();
        let writable = support
            .supported_properties
            .iter()
            .filter(|property| property.access.writable)
            .map(|property| property.property_id)
            .collect::<Vec<_>>();
        // Each log's BUFFER_READY configuration is writable too (#1347),
        // as are the message texts and the inhibit pair (#1329).
        let reporting = [
            PropertyIdentifier::NOTIFICATION_THRESHOLD,
            PropertyIdentifier::NOTIFICATION_CLASS,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
        ];
        let options = [
            PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
            PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
        ];
        assert_eq!(
            writable.len(),
            expected.len() + reporting.len() + options.len(),
            "{object_type:?}"
        );
        for property in expected.iter().chain(&reporting).chain(&options) {
            assert!(writable.contains(property), "{object_type:?} {property:?}");
        }
        // A Trend Log samples a BACnet property, so Table 12-29 footnotes 1
        // and 8 require its window, Log_Interval and Log_DeviceObjectProperty
        // (#1481). Table 12-35 requires the last two outright, and neither it
        // nor Table 12-31 requires a window.
        let optional = |property| {
            support
                .supported_properties
                .iter()
                .find(|row| row.property_id == property)
                .map(|row| row.access.optional)
        };
        let trend = object_type == ObjectType::TREND_LOG;
        for property in [
            PropertyIdentifier::START_TIME,
            PropertyIdentifier::STOP_TIME,
        ] {
            assert_eq!(
                optional(property),
                Some(!trend),
                "{object_type:?} {property:?}"
            );
        }
        if object_type != ObjectType::EVENT_LOG {
            for property in [
                PropertyIdentifier::LOG_INTERVAL,
                PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY,
            ] {
                assert_eq!(
                    optional(property),
                    Some(false),
                    "{object_type:?} {property:?}"
                );
            }
        }
        // The three tables require every BUFFER_READY row of a log that
        // reports intrinsically, as these do, but Event_Message_Texts,
        // which they only permit (#1485).
        let read_only = [
            PropertyIdentifier::RECORDS_SINCE_NOTIFICATION,
            PropertyIdentifier::LAST_NOTIFY_RECORD,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
        ];
        for property in reporting.iter().chain(&read_only) {
            assert_eq!(
                optional(*property),
                Some(false),
                "{object_type:?} {property:?}"
            );
        }
        // The Trend Log's two COV rows (#1480) are writable above; that
        // Table 12-29 only permits them is pinned with the RPM rows in
        // handlers/tests/property_metadata/log.rs.
        let texts = PropertyIdentifier::EVENT_MESSAGE_TEXTS;
        for property in options.iter().chain([&texts]) {
            assert_eq!(optional(*property), Some(true), "{object_type:?}");
        }
    }
}

#[test]
fn pics_file_resize_writability_matches_single_instance_metadata() {
    use bacnet_objects::database::ObjectDatabase;
    use bacnet_objects::file::FileObject;
    use bacnet_types::enums::FileAccessMethod;

    fn property_access(
        file: FileObject,
        property: PropertyIdentifier,
    ) -> Option<super::PropertyAccess> {
        let mut db = ObjectDatabase::new();
        db.add(Box::new(file)).unwrap();
        PicsGenerator::new(&db, &ServerConfig::default(), &PicsConfig::default())
            .generate()
            .supported_object_types
            .into_iter()
            .find(|support| support.object_type == ObjectType::FILE)
            .unwrap()
            .supported_properties
            .into_iter()
            .find(|support| support.property_id == property)
            .map(|support| support.access)
    }

    let stream = FileObject::new(1, "STREAM", "raw").unwrap();
    assert!(
        property_access(stream, PropertyIdentifier::FILE_SIZE)
            .unwrap()
            .writable
    );
    let stream = FileObject::new(2, "STREAM", "raw").unwrap();
    assert!(property_access(stream, PropertyIdentifier::RECORD_COUNT).is_none());

    let mut record = FileObject::new(3, "RECORD", "raw").unwrap();
    record.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    assert!(
        !property_access(record, PropertyIdentifier::FILE_SIZE)
            .unwrap()
            .writable
    );
    let mut record = FileObject::new(4, "RECORD", "raw").unwrap();
    record.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    assert!(
        property_access(record, PropertyIdentifier::RECORD_COUNT)
            .unwrap()
            .writable
    );

    let mut read_only_stream = FileObject::new(5, "RO-STREAM", "raw").unwrap();
    read_only_stream.set_read_only(true);
    assert!(
        !property_access(read_only_stream, PropertyIdentifier::FILE_SIZE)
            .unwrap()
            .writable
    );
    let mut read_only_record = FileObject::new(6, "RO-RECORD", "raw").unwrap();
    read_only_record.set_file_access_method(FileAccessMethod::RECORD_ACCESS.to_raw());
    read_only_record.set_read_only(true);
    assert!(
        !property_access(read_only_record, PropertyIdentifier::RECORD_COUNT)
            .unwrap()
            .writable
    );
}
