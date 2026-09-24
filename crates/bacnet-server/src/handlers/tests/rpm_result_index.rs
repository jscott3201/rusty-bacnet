use crate::handlers::rpm_budget::{handle_rpm_budgeted, handle_rpm_budgeted_observed, RpmFailure};
use bacnet_objects::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyWriteCapability,
};
use std::{
    borrow::Cow,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use super::*;
use crate::server::ReadPropertyMultipleBudget;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;

fn request(oid: ObjectIdentifier, references: &[(PropertyIdentifier, Option<u32>)]) -> BytesMut {
    let mut data = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: references
                .iter()
                .map(
                    |&(property_identifier, property_array_index)| PropertyReference {
                        property_identifier,
                        property_array_index,
                    },
                )
                .collect(),
        }],
    }
    .encode(&mut data)
    .unwrap();
    data
}

#[test]
fn rpm_result_index_scalar_error_omits_index_in_both_builders() {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig::default()).unwrap();
    let oid = device.object_identifier();
    db.add(Box::new(device)).unwrap();
    let data = request(oid, &[(PropertyIdentifier::OBJECT_NAME, Some(7))]);
    let mut indexes = Vec::new();
    for budgeted in [false, true] {
        let mut out = BytesMut::new();
        if budgeted {
            handle_rpm_budgeted(&db, &data, &mut out, ReadPropertyMultipleBudget::default())
                .unwrap();
        } else {
            handle_read_property_multiple(&db, &data, &mut out).unwrap();
        }
        let ack = ReadPropertyMultipleACK::decode(&out).unwrap();
        let result = &ack.list_of_read_access_results[0].list_of_results[0];
        assert_eq!(
            result.error,
            Some((ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY))
        );
        indexes.push(result.property_array_index);
    }
    assert_eq!(indexes, vec![None, None]);
}

const VENDOR_ARRAY: PropertyIdentifier = PropertyIdentifier::from_raw(600);
struct Declared {
    modern: bool,
    reads: Arc<AtomicUsize>,
}
impl BACnetObject for Declared {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap()
    }
    fn object_name(&self) -> &str {
        "declared"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::STATE_TEXT,
            VENDOR_ARRAY,
        ])
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        if !self.modern {
            return Cow::Borrowed(&[]);
        }
        Cow::Owned(
            self.property_list()
                .iter()
                .copied()
                .chain([PropertyIdentifier::PROPERTY_LIST])
                .map(|id| {
                    PropertyMetadata::new(
                        id,
                        PropertyConformance::Optional,
                        None,
                        PropertyWriteCapability::ReadOnly,
                    )
                })
                .collect(),
        )
    }
    fn is_array_property(&self, id: PropertyIdentifier) -> bool {
        // OBJECT_LIST is globally an array, but is deliberately absent here.
        [
            VENDOR_ARRAY,
            PropertyIdentifier::STATE_TEXT,
            PropertyIdentifier::PROPERTY_LIST,
            PropertyIdentifier::OBJECT_LIST,
        ]
        .contains(&id)
    }
    fn read_property(
        &self,
        id: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let error = if id == PropertyIdentifier::STATE_TEXT {
            Some(ErrorCode::READ_ACCESS_DENIED)
        } else if id == PropertyIdentifier::OBJECT_LIST
            || (id == PropertyIdentifier::PROPERTY_LIST && !self.modern)
        {
            Some(ErrorCode::UNKNOWN_PROPERTY)
        } else if index == Some(9) {
            Some(ErrorCode::INVALID_ARRAY_INDEX)
        } else {
            None
        };
        if let Some(code) = error {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: code.to_raw() as u32,
            });
        }
        Ok(PropertyValue::Unsigned(if index == Some(0) {
            2
        } else {
            42
        }))
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

#[test]
fn rpm_result_index_presence_vendor_errors_order_budget_and_request_observations() {
    for modern in [false, true] {
        let reads = Arc::new(AtomicUsize::new(0));
        let object = Declared {
            modern,
            reads: reads.clone(),
        };
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let references = [
            (PropertyIdentifier::OBJECT_NAME, Some(1)),
            (VENDOR_ARRAY, Some(0)),
            (PropertyIdentifier::STATE_TEXT, Some(4)),
            (VENDOR_ARRAY, Some(9)),
            (PropertyIdentifier::OBJECT_LIST, Some(0)),
            (PropertyIdentifier::PROPERTY_LIST, Some(0)),
            (VENDOR_ARRAY, None),
            (VENDOR_ARRAY, Some(0)),
        ];
        let data = request(oid, &references);
        let mut plain = BytesMut::new();
        handle_read_property_multiple(&db, &data, &mut plain).unwrap();
        assert_eq!(
            reads.swap(0, Ordering::Relaxed),
            7,
            "one read per nongated occurrence"
        );
        let ack = ReadPropertyMultipleACK::decode(&plain).unwrap();
        let results = &ack.list_of_read_access_results[0].list_of_results;
        assert_eq!(
            results
                .iter()
                .map(|r| r.property_identifier)
                .collect::<Vec<_>>(),
            references.iter().map(|r| r.0).collect::<Vec<_>>()
        );
        assert_eq!(
            results
                .iter()
                .map(|r| r.property_array_index)
                .collect::<Vec<_>>(),
            vec![
                None,
                Some(0),
                Some(4),
                Some(9),
                None,
                modern.then_some(0),
                None,
                Some(0)
            ]
        );
        assert_eq!(
            results
                .iter()
                .map(|r| r.error.map(|e| e.1))
                .collect::<Vec<_>>(),
            vec![
                Some(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
                None,
                Some(ErrorCode::READ_ACCESS_DENIED),
                Some(ErrorCode::INVALID_ARRAY_INDEX),
                Some(ErrorCode::UNKNOWN_PROPERTY),
                (!modern).then_some(ErrorCode::UNKNOWN_PROPERTY),
                None,
                None
            ]
        );
        let budget = ReadPropertyMultipleBudget {
            max_result_elements: references.len(),
            max_service_ack_bytes: plain.len(),
        };
        let mut bounded = BytesMut::new();
        let mut observations = Vec::new();
        handle_rpm_budgeted_observed(
            &db,
            &data,
            &mut bounded,
            budget,
            |object, property, index, error| observations.push((object, property, index, error)),
        )
        .unwrap();
        assert_eq!(bounded, plain);
        assert_eq!(reads.swap(0, Ordering::Relaxed), 7);
        assert_eq!(
            observations.iter().map(|r| (r.1, r.2)).collect::<Vec<_>>(),
            references
        );
        for work in [false, true] {
            let mut prefix = BytesMut::from(&b"prefix"[..]);
            let result = handle_rpm_budgeted(
                &db,
                &data,
                &mut prefix,
                ReadPropertyMultipleBudget {
                    max_result_elements: references.len() - usize::from(work),
                    max_service_ack_bytes: plain.len() - usize::from(!work),
                },
            );
            assert!(matches!(
                (work, result),
                (true, Err(RpmFailure::Work)) | (false, Err(RpmFailure::Bytes))
            ));
            assert_eq!(&prefix[..], b"prefix");
            if work {
                assert_eq!(
                    reads.load(Ordering::Relaxed),
                    0,
                    "work rejected before reads"
                );
            }
            reads.store(0, Ordering::Relaxed);
        }
    }
}

#[test]
fn rpm_result_index_unknown_object_omits_index_without_changing_error() {
    let db = ObjectDatabase::new();
    let data = request(
        ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap(),
        &[
            (PropertyIdentifier::OBJECT_LIST, Some(0)),
            (VENDOR_ARRAY, Some(2)),
        ],
    );
    for budgeted in [false, true] {
        let mut out = BytesMut::new();
        if budgeted {
            handle_rpm_budgeted(&db, &data, &mut out, ReadPropertyMultipleBudget::default())
                .unwrap();
        } else {
            handle_read_property_multiple(&db, &data, &mut out).unwrap();
        }
        let ack = ReadPropertyMultipleACK::decode(&out).unwrap();
        for result in &ack.list_of_read_access_results[0].list_of_results {
            assert_eq!(result.property_array_index, None);
            assert_eq!(
                result.error,
                Some((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
            );
        }
    }
}
