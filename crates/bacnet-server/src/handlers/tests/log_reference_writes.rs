//! Log_DeviceObjectProperty over the wire (#1234): a Trend Log's one
//! BACnetDeviceObjectPropertyReference and a Trend Log Multiple's array of
//! them (Clauses 12.25.8 and 12.30.11), read and written in the Clause 21
//! encoding, and polled from what a client wrote.

use super::*;
use bacnet_encoding::constructed::{decode_log_multiple_record, decode_log_record};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::bitstring::LogStatus;
use bacnet_types::constructed::{
    LogData, LogDatum, LogValue, PropertyReference, ReadAccessSpecification,
};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::RemoteError;
use std::sync::Arc;

const LDOP: PropertyIdentifier = PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY;

/// [0] analog-value 1, [1] present-value.
const AV1_PV: [u8; 7] = [0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55];
/// [0] analog-value 2, [1] present-value.
const AV2_PV: [u8; 7] = [0x0C, 0x00, 0x80, 0x00, 0x02, 0x19, 0x55];
/// [0] analog-value 1, [1] priority-array, [2] slot 16.
const AV1_SLOT16: [u8; 9] = [0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x57, 0x29, 0x10];
/// A [3] member naming Device 856, this database's Device.
const LOCAL_DEVICE: [u8; 5] = [0x3C, 0x02, 0x00, 0x03, 0x58];
/// A [3] member naming Device 9, another device.
const REMOTE_DEVICE: [u8; 5] = [0x3C, 0x02, 0x00, 0x00, 0x09];
/// A [3] member naming analog-input 9, which is no Device.
const NOT_A_DEVICE: [u8; 5] = [0x3C, 0x00, 0x00, 0x00, 0x09];
/// A [3] member naming Device 4194303: an empty Trend Log Multiple element,
/// and an unset Trend Log reference (#1417).
const WILDCARD_DEVICE: [u8; 5] = [0x3C, 0x02, 0x3F, 0xFF, 0xFF];
/// The unset form (#1417): [0] analog-input 4194303, [1] present-value.
const UNSET: [u8; 7] = [0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55];

fn tl1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

fn tlm1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, 1).unwrap()
}

/// Device 856, AV-1 at 21.5 and AV-2 at 7.0, and an unconfigured Trend Log
/// and Trend Log Multiple polled every second, with both clocks bound.
fn database() -> ObjectDatabase {
    let mut db = crate::server::clocked_test_database();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(856, 0)
                .expect("valid Device identity")
                .with_name("D-856"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for (instance, value) in [(1, 21.5), (2, 7.0)] {
        let mut av = AnalogValueObject::new(
            instance,
            format!("AV-{instance}"),
            EngineeringUnits::NO_UNITS,
        )
        .unwrap();
        av.write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            Some(16),
            &crate::command_source::test_origin(),
        )
        .unwrap();
        db.add(Box::new(av)).unwrap();
    }
    let mut tl = TrendLogObject::new(1, "TL-1", 8).unwrap();
    let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    for log in [&mut tl as &mut dyn BACnetObject, &mut tlm] {
        log.write_property(
            PropertyIdentifier::LOG_INTERVAL,
            None,
            PropertyValue::Unsigned(100),
            None,
        )
        .unwrap();
    }
    db.add(Box::new(tl)).unwrap();
    db.add(Box::new(tlm)).unwrap();
    db.set_monotonic_clock_internal(Some(Arc::new(|| std::time::Duration::ZERO)));
    db
}

fn wp(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    index: Option<u32>,
    value: &[u8],
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: LDOP,
        property_array_index: index,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    sourced_wp(db, &request).map(|_| ())
}

fn wpm(db: &mut ObjectDatabase, oid: ObjectIdentifier, value: &[u8]) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: LDOP,
                property_array_index: None,
                value: value.to_vec(),
                priority: None,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    sourced_wpm(db, &request).map(|_| ())
}

/// The ReadProperty-ACK value bytes, after checking ReadPropertyMultiple
/// serves the same ones.
fn rp(db: &ObjectDatabase, oid: ObjectIdentifier, index: Option<u32>) -> Result<Vec<u8>, Error> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: LDOP,
        property_array_index: index,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    let read = handle_read_property(db, &request, &mut ack)
        .map(|_| ReadPropertyACK::decode(&ack).unwrap().property_value);

    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: vec![PropertyReference {
                property_identifier: LDOP,
                property_array_index: index,
            }],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut ack = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut ack).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&ack).unwrap();
    let result = &ack.list_of_read_access_results[0].list_of_results[0];
    assert_eq!(result.property_value, read.as_ref().ok().cloned(), "RPM");
    read
}

/// A refused write: the array index, the value bytes, the error class and
/// code expected, and what the case shows.
type IndexedRefusal = (Option<u32>, Vec<u8>, ErrorClass, ErrorCode, &'static str);

fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode, what: &str) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class: c, code: e }))
            if c == class && e == code),
        "{what}: expected {class:?} / {code:?}, got {result:?}"
    );
}

fn record_count(db: &ObjectDatabase, oid: ObjectIdentifier) -> usize {
    db.get(&oid)
        .unwrap()
        .log_buffer_internal()
        .unwrap()
        .record_count()
}

fn record(db: &ObjectDatabase, oid: ObjectIdentifier, index: usize) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    db.get(&oid)
        .unwrap()
        .log_buffer_internal()
        .unwrap()
        .encode_record(index, &mut bytes);
    bytes.to_vec()
}

fn trend_datum(db: &ObjectDatabase, index: usize) -> LogDatum {
    decode_log_record(&record(db, tl1(), index), 0)
        .unwrap()
        .0
        .log_datum
}

fn multiple_data(db: &ObjectDatabase, index: usize) -> LogData {
    decode_log_multiple_record(&record(db, tlm1(), index), 0)
        .unwrap()
        .0
        .log_data
}

#[test]
fn trend_log_reference_reads_and_writes_in_the_clause_21_form() {
    let mut db = database();
    // No reference yet: the unset form (#1417).
    assert_eq!(rp(&db, tl1(), None).unwrap(), UNSET);

    wp(&mut db, tl1(), None, &AV1_PV).unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), AV1_PV);
    // The change purged the buffer and left its status record (12.25.8).
    assert_eq!(record_count(&db, tl1()), 1);
    assert_eq!(
        trend_datum(&db, 0),
        LogDatum::LogStatus(LogStatus::BUFFER_PURGED)
    );

    // The value already held changes nothing, the buffer included.
    wp(&mut db, tl1(), None, &AV1_PV).unwrap();
    assert_eq!(record_count(&db, tl1()), 1);

    wp(&mut db, tl1(), None, &AV1_SLOT16).unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), AV1_SLOT16);
    wpm(&mut db, tl1(), &AV2_PV).unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), AV2_PV);
    // A reference naming this device's Device is the local one it stands for.
    wp(&mut db, tl1(), None, &[&AV1_PV[..], &LOCAL_DEVICE].concat()).unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), AV1_PV);
    // A NULL is no reference: it succeeds and changes nothing, the buffer
    // included (Clause 15.9.2, #1417).
    wp(&mut db, tl1(), None, &[0x00]).unwrap();
    wpm(&mut db, tl1(), &[0x00]).unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), AV1_PV);
    assert_eq!(record_count(&db, tl1()), 1);
    // The unset form takes the reference away, a change that purges.
    wp(&mut db, tl1(), None, &UNSET).unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), UNSET);
    assert_eq!(record_count(&db, tl1()), 1);
    // So does a reference naming Device 4194303, which names no device.
    wp(&mut db, tl1(), None, &AV1_PV).unwrap();
    wp(
        &mut db,
        tl1(),
        None,
        &[&AV2_PV[..], &WILDCARD_DEVICE].concat(),
    )
    .unwrap();
    assert_eq!(rp(&db, tl1(), None).unwrap(), UNSET);
}

#[test]
fn trend_log_reference_write_refusals_change_nothing() {
    let mut db = database();
    wp(&mut db, tl1(), None, &AV1_PV).unwrap();
    let flat = {
        let mut bytes = BytesMut::new();
        encode_property_value(
            &mut bytes,
            &PropertyValue::List(vec![
                PropertyValue::ObjectIdentifier(
                    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap(),
                ),
                PropertyValue::Unsigned(85),
            ]),
        )
        .unwrap();
        bytes.to_vec()
    };
    let cases: [(Vec<u8>, ErrorClass, ErrorCode, &str); 6] = [
        (
            flat,
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            "the flat form reads used to serve",
        ),
        (
            [&AV2_PV[..], &REMOTE_DEVICE].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            "a reference into another device",
        ),
        (
            [&AV2_PV[..], &NOT_A_DEVICE].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "a [3] that is no Device",
        ),
        (
            AV2_PV[..5].to_vec(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "no [1] property identifier",
        ),
        (
            [AV1_PV, AV2_PV].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "two references",
        ),
        (
            Vec::new(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "no reference",
        ),
    ];
    for (value, class, code, what) in cases {
        assert_refused(wp(&mut db, tl1(), None, &value), class, code, what);
        assert_refused(wpm(&mut db, tl1(), &value), class, code, what);
        assert_eq!(rp(&db, tl1(), None).unwrap(), AV1_PV, "{what}");
        assert_eq!(record_count(&db, tl1()), 1, "{what}");
    }
    // A Trend Log holds a single reference, not an array.
    assert_refused(
        wp(&mut db, tl1(), Some(1), &AV2_PV),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        "indexed write",
    );
    assert_refused(
        rp(&db, tl1(), Some(1)).map(|_| ()),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        "indexed read",
    );
}

#[test]
fn trend_log_multiple_reference_array_reads_and_writes() {
    let mut db = database();
    assert_eq!(rp(&db, tlm1(), None).unwrap(), Vec::<u8>::new());
    assert_eq!(rp(&db, tlm1(), Some(0)).unwrap(), [0x21, 0x00]);

    // A whole write; the second element names this device and is stored
    // local.
    let whole = [&AV1_PV[..], &AV2_PV, &LOCAL_DEVICE].concat();
    wp(&mut db, tlm1(), None, &whole).unwrap();
    assert_eq!(rp(&db, tlm1(), None).unwrap(), [AV1_PV, AV2_PV].concat());
    assert_eq!(rp(&db, tlm1(), Some(0)).unwrap(), [0x21, 0x02]);
    assert_eq!(rp(&db, tlm1(), Some(1)).unwrap(), AV1_PV);
    assert_eq!(rp(&db, tlm1(), Some(2)).unwrap(), AV2_PV);
    assert_refused(
        rp(&db, tlm1(), Some(3)).map(|_| ()),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_ARRAY_INDEX,
        "read past the end",
    );
    assert_eq!(record_count(&db, tlm1()), 1);
    assert_eq!(
        multiple_data(&db, 0),
        LogData::LogStatus(LogStatus::BUFFER_PURGED)
    );

    // One element by index, naming this device: stored local too.
    wp(
        &mut db,
        tlm1(),
        Some(2),
        &[&AV1_SLOT16[..], &LOCAL_DEVICE].concat(),
    )
    .unwrap();
    assert_eq!(rp(&db, tlm1(), Some(2)).unwrap(), AV1_SLOT16);
    assert_eq!(rp(&db, tlm1(), Some(1)).unwrap(), AV1_PV);

    // An element naming Device 4194303 is empty and kept as written.
    let empty = [&AV2_PV[..], &WILDCARD_DEVICE].concat();
    wp(&mut db, tlm1(), Some(1), &empty).unwrap();
    assert_eq!(rp(&db, tlm1(), Some(1)).unwrap(), empty);

    // A whole write may resize the array, to nothing at all.
    wpm(&mut db, tlm1(), &[]).unwrap();
    assert_eq!(rp(&db, tlm1(), Some(0)).unwrap(), [0x21, 0x00]);
    wp(&mut db, tlm1(), None, &AV1_PV).unwrap();
    assert_eq!(rp(&db, tlm1(), None).unwrap(), AV1_PV);
}

#[test]
fn trend_log_multiple_index_0_write_resizes_the_array() {
    // [0] analog-input 4194303, [1] present-value: an empty element.
    const EMPTY: [u8; 7] = [0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55];
    // Total_Record_Count: one more for each purge's status record.
    let total = |db: &ObjectDatabase| {
        db.get(&tlm1())
            .unwrap()
            .read_property(PropertyIdentifier::TOTAL_RECORD_COUNT, None)
            .unwrap()
    };
    let mut db = database();
    wp(&mut db, tlm1(), None, &[AV1_PV, AV2_PV].concat()).unwrap();
    assert_eq!(total(&db), PropertyValue::Unsigned(1));

    // A larger size appends empty elements and purges (Clause 12.1.5.1).
    wp(&mut db, tlm1(), Some(0), &[0x21, 0x04]).unwrap();
    assert_eq!(rp(&db, tlm1(), Some(0)).unwrap(), [0x21, 0x04]);
    assert_eq!(
        rp(&db, tlm1(), None).unwrap(),
        [AV1_PV, AV2_PV, EMPTY, EMPTY].concat()
    );
    assert_eq!(total(&db), PropertyValue::Unsigned(2));

    // The size already held is no change, so the log keeps its records.
    wp(&mut db, tlm1(), Some(0), &[0x21, 0x04]).unwrap();
    assert_eq!(total(&db), PropertyValue::Unsigned(2));

    // A smaller size drops the trailing elements and purges.
    wp(&mut db, tlm1(), Some(0), &[0x21, 0x01]).unwrap();
    assert_eq!(rp(&db, tlm1(), None).unwrap(), AV1_PV);
    assert_eq!(total(&db), PropertyValue::Unsigned(3));
    wp(&mut db, tlm1(), Some(0), &[0x21, 0x00]).unwrap();
    assert_eq!(rp(&db, tlm1(), None).unwrap(), Vec::<u8>::new());
    assert_eq!(total(&db), PropertyValue::Unsigned(4));

    // The empty elements are polled as empty.
    wp(&mut db, tlm1(), Some(0), &[0x21, 0x01]).unwrap();
    db.poll_trend_logs();
    assert_eq!(
        multiple_data(&db, 1),
        LogData::Values(vec![LogValue::Failure {
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::NO_PROPERTY_SPECIFIED,
        }])
    );
}

#[test]
fn trend_log_multiple_reference_write_refusals_change_nothing() {
    let mut db = database();
    wp(&mut db, tlm1(), None, &[AV1_PV, AV2_PV].concat()).unwrap();
    let held = [AV1_PV, AV2_PV].concat();
    let too_many = AV1_PV.repeat(65);
    let cases: [IndexedRefusal; 8] = [
        (
            None,
            [&AV1_PV[..], &REMOTE_DEVICE].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            "a whole write with an element in another device",
        ),
        (
            Some(1),
            [&AV1_PV[..], &REMOTE_DEVICE].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            "an element in another device",
        ),
        (
            Some(2),
            [&AV1_PV[..], &NOT_A_DEVICE].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
            "an element whose [3] is no Device",
        ),
        (
            Some(1),
            [AV1_PV, AV2_PV].concat(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_ENCODING,
            "two references for one element",
        ),
        (
            Some(3),
            AV1_PV.to_vec(),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_ARRAY_INDEX,
            "an element past the end",
        ),
        (
            Some(0),
            vec![0x21, 0x41],
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            "an array size of 65",
        ),
        (
            Some(0),
            vec![0x44, 0x40, 0x00, 0x00, 0x00],
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
            "a REAL array size",
        ),
        (
            None,
            too_many,
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
            "65 references",
        ),
    ];
    for (index, value, class, code, what) in cases {
        assert_refused(wp(&mut db, tlm1(), index, &value), class, code, what);
        assert_eq!(rp(&db, tlm1(), None).unwrap(), held, "{what}");
        assert_eq!(record_count(&db, tlm1()), 1, "{what}");
    }
    // An element has no NULL in its datatype and the array isn't
    // commandable, so a NULL there succeeds and changes nothing (#1396).
    wp(&mut db, tlm1(), Some(1), &[0x00]).unwrap();
    assert_eq!(rp(&db, tlm1(), None).unwrap(), held);
    assert_eq!(record_count(&db, tlm1()), 1);
}

#[test]
fn pollers_sample_the_references_a_client_wrote() {
    let mut db = database();
    wp(&mut db, tl1(), None, &AV1_PV).unwrap();
    wp(&mut db, tlm1(), None, &[AV1_PV, AV2_PV].concat()).unwrap();
    db.poll_trend_logs();
    assert_eq!(record_count(&db, tl1()), 2);
    assert_eq!(trend_datum(&db, 1), LogDatum::RealValue(21.5));
    assert_eq!(record_count(&db, tlm1()), 2);
    assert_eq!(
        multiple_data(&db, 1),
        LogData::Values(vec![LogValue::RealValue(21.5), LogValue::RealValue(7.0)])
    );
}
