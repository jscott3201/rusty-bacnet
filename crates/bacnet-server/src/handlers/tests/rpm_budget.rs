use super::*;
use bacnet_services::rpm::ReadAccessSpecification;
use std::borrow::Cow;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Counting {
    reads: Arc<AtomicUsize>,
    array: bool,
    empty: bool,
}

impl BACnetObject for Counting {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(1)
    }
    fn object_name(&self) -> &str {
        "counting"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        if self.empty {
            Cow::Borrowed(&[])
        } else {
            Cow::Borrowed(&[
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::DESCRIPTION,
            ])
        }
    }
    fn required_properties(&self) -> Cow<'static, [PropertyIdentifier]> {
        if self.empty {
            Cow::Borrowed(&[])
        } else {
            Cow::Borrowed(&[
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::OBJECT_NAME,
            ])
        }
    }
    fn is_array_property(&self, _: PropertyIdentifier) -> bool {
        self.array
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        _: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.array {
            return Ok(PropertyValue::List(vec![PropertyValue::Unsigned(1); 1000]));
        }
        if property == PropertyIdentifier::DESCRIPTION {
            return Err(Error::Protocol { class: 2, code: 32 });
        }
        Ok(PropertyValue::Unsigned(1))
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        unreachable!()
    }
}

fn oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap()
}
fn fixture(array: bool, empty: bool) -> (ObjectDatabase, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(Counting {
        reads: reads.clone(),
        array,
        empty,
    }))
    .unwrap();
    (db, reads)
}
fn reference(id: PropertyIdentifier) -> PropertyReference {
    PropertyReference {
        property_identifier: id,
        property_array_index: None,
    }
}
fn spec(instance: u32, properties: Vec<PropertyReference>) -> ReadAccessSpecification {
    ReadAccessSpecification {
        object_identifier: oid(instance),
        list_of_property_references: properties,
    }
}
fn request(specs: Vec<ReadAccessSpecification>) -> BytesMut {
    let mut bytes = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: specs,
    }
    .encode(&mut bytes);
    bytes
}
fn budget(work: usize, bytes: usize) -> ReadPropertyMultipleBudget {
    ReadPropertyMultipleBudget {
        max_result_elements: work,
        max_service_ack_bytes: bytes,
    }
}

#[test]
fn rpm_work_aggregate_boundaries_preflight_no_reads() {
    for count in [3, 4, 5] {
        let (db, reads) = fixture(false, false);
        let data = request(vec![
            spec(1, vec![reference(PropertyIdentifier::PRESENT_VALUE); 2]),
            spec(
                1,
                vec![reference(PropertyIdentifier::PRESENT_VALUE); count - 2],
            ),
        ]);
        let mut out = BytesMut::from(&b"prefix"[..]);
        let result = handle_rpm_budgeted(&db, &data, &mut out, budget(4, 1024));
        if count > 4 {
            assert!(matches!(result, Err(RpmFailure::Work)));
            assert_eq!(reads.load(Ordering::SeqCst), 0);
            assert_eq!(&out[..], b"prefix");
        } else {
            result.unwrap();
            assert_eq!(reads.load(Ordering::SeqCst), count);
        }
    }
}

#[test]
fn rpm_wildcards_duplicates_unknown_and_index_count_results() {
    let (db, reads) = fixture(false, false);
    for (property, count) in [
        (PropertyIdentifier::ALL, 3),
        (PropertyIdentifier::REQUIRED, 2),
        (PropertyIdentifier::OPTIONAL, 1),
    ] {
        let data = request(vec![
            spec(1, vec![reference(property); 2]),
            spec(99, vec![reference(PropertyIdentifier::ALL)]),
        ]);
        let decoded = ReadPropertyMultipleRequest::decode(&data).unwrap();
        assert!(matches!(
            plan(&db, &decoded, count * 2),
            Err(RpmFailure::Work)
        ));
        assert!(matches!(
            handle_rpm_budgeted(&db, &data, &mut BytesMut::new(), budget(count * 2, 1024)),
            Err(RpmFailure::Work)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        assert_eq!(
            plan(&db, &decoded, count * 2 + 1)
                .unwrap()
                .iter()
                .map(|p| p.properties.len())
                .sum::<usize>(),
            count * 2 + 1
        );
    }
    let data = request(vec![spec(
        1,
        vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: Some(1)
            };
            2
        ],
    )]);
    assert!(matches!(
        handle_rpm_budgeted(&db, &data, &mut BytesMut::new(), budget(1, 1024)),
        Err(RpmFailure::Work)
    ));
    handle_rpm_budgeted(&db, &data, &mut BytesMut::new(), budget(2, 1024)).unwrap();
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn rpm_bytes_exact_wrappers_errors_index_atomic_and_legacy_parity() {
    let (db, _) = fixture(false, false);
    let data = request(vec![
        spec(
            1,
            vec![
                reference(PropertyIdentifier::ALL),
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: Some(0),
                },
            ],
        ),
        spec(99, vec![reference(PropertyIdentifier::ALL)]),
    ]);
    let mut legacy = BytesMut::new();
    handle_read_property_multiple(&db, &data, &mut legacy).unwrap();
    for cap in [legacy.len() - 1, legacy.len(), legacy.len() + 1] {
        let mut out = BytesMut::from(&b"prefix"[..]);
        let result = handle_rpm_budgeted(&db, &data, &mut out, budget(5, cap));
        if cap < legacy.len() {
            assert!(matches!(result, Err(RpmFailure::Bytes)));
            assert_eq!(&out[..], b"prefix");
        } else {
            result.unwrap();
            assert_eq!(&out[6..], &legacy[..]);
        }
    }
}

#[test]
fn rpm_byte_overflow_stops_reads_and_whole_array_is_not_preempted() {
    for array in [false, true] {
        let (db, reads) = fixture(array, false);
        let data = request(vec![spec(
            1,
            vec![reference(PropertyIdentifier::PRESENT_VALUE); 3],
        )]);
        // Header + footer fit (7 bytes), but the first value does not.
        assert!(matches!(
            handle_rpm_budgeted(&db, &data, &mut BytesMut::new(), budget(3, 7)),
            Err(RpmFailure::Bytes)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        reads.store(0, Ordering::SeqCst);
        // A scalar result fits, then the next one fails: earlier reads persist
        // but the third property is never read. A whole array still fails first.
        assert!(matches!(
            handle_rpm_budgeted(&db, &data, &mut BytesMut::new(), budget(3, 13)),
            Err(RpmFailure::Bytes)
        ));
        assert_eq!(reads.load(Ordering::SeqCst), if array { 1 } else { 2 });
    }
}

#[test]
fn rpm_empty_expansions_still_charge_object_wrappers() {
    let (db, reads) = fixture(false, true);
    let data = request(vec![spec(1, vec![reference(PropertyIdentifier::ALL)])]);
    for cap in [6, 7, 8] {
        let mut out = BytesMut::new();
        let result = handle_rpm_budgeted(&db, &data, &mut out, budget(1, cap));
        if cap == 6 {
            assert!(matches!(result, Err(RpmFailure::Bytes)));
        } else {
            result.unwrap();
            // Independently worked context-0 AI:1, open-1, close-1.
            assert_eq!(&out[..], &[0x0c, 0, 0, 0, 1, 0x1e, 0x1f]);
        }
    }
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn rpm_scratch_growth_never_exceeds_cap_even_on_overflow() {
    let mut scratch = Scratch {
        bytes: BytesMut::new(),
        limit: 4,
    };
    scratch.append(&[1, 2], 2).unwrap();
    assert_eq!(scratch.bytes.len(), 2);
    assert!(matches!(
        scratch.append(&[3, 4, 5], 0),
        Err(RpmFailure::Bytes)
    ));
    assert_eq!(scratch.bytes.len(), 2);
    assert!(matches!(
        scratch.append(&[], usize::MAX),
        Err(RpmFailure::Bytes)
    ));
    scratch.append(&[3, 4], 0).unwrap();
    assert_eq!(scratch.bytes.len(), 4);
}

#[test]
fn rpm_decode_failure_keeps_priority_and_caller_prefix() {
    let (db, reads) = fixture(false, false);
    let mut out = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(&db, &[0xff], &mut out, budget(1, 1)),
        Err(RpmFailure::Service(_))
    ));
    assert_eq!(&out[..], b"prefix");
    assert_eq!(reads.load(Ordering::SeqCst), 0);
}

#[test]
fn rpm_success_independent_scalar_golden() {
    let (db, _) = fixture(false, false);
    let data = request(vec![spec(
        1,
        vec![reference(PropertyIdentifier::PRESENT_VALUE)],
    )]);
    let mut out = BytesMut::new();
    handle_rpm_budgeted(&db, &data, &mut out, budget(1, 13)).unwrap();
    // context object AI:1, open results, property 85, open value,
    // application unsigned 1, close value/results (independent vector).
    assert_eq!(
        &out[..],
        &[0x0c, 0, 0, 0, 1, 0x1e, 0x29, 0x55, 0x4e, 0x21, 1, 0x4f, 0x1f]
    );
}

#[test]
fn rpm_migrated_metadata_device_wildcard_and_index_legacy_parity() {
    use bacnet_objects::device::{DeviceConfig, DeviceObject};
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 123,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let wildcard = ObjectIdentifier::new(ObjectType::DEVICE, 4194303).unwrap();
    for property in [
        PropertyIdentifier::ALL,
        PropertyIdentifier::REQUIRED,
        PropertyIdentifier::OPTIONAL,
    ] {
        let data = request(vec![ReadAccessSpecification {
            object_identifier: wildcard,
            list_of_property_references: vec![
                reference(property),
                reference(property),
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_LIST,
                    property_array_index: Some(0),
                },
            ],
        }]);
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &data, &mut legacy).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
        let count = ack.list_of_read_access_results[0].list_of_results.len();
        assert_eq!(
            ack.list_of_read_access_results[0].object_identifier,
            wildcard
        );
        for work in [count - 1, count, count + 1] {
            let mut out = BytesMut::new();
            let result = handle_rpm_budgeted(&db, &data, &mut out, budget(work, legacy.len()));
            if work < count {
                assert!(matches!(result, Err(RpmFailure::Work)));
            } else {
                result.unwrap();
                assert_eq!(out, legacy);
            }
        }
    }
}
