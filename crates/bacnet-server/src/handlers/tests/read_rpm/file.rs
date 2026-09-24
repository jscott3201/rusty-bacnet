use super::*;
use bacnet_objects::{file::FileObject, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleACK};
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

#[test]
fn rpm_file_indexed_property_list_and_scalar_gates_preserve_bytes() {
    for method in [1, 0] {
        let mut object = FileObject::new(1, "FILE-1", "raw").unwrap();
        object.set_file_access_method(method);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let references = [
            (P::PROPERTY_LIST, Some(0)),
            (P::PROPERTY_LIST, Some(1)),
            (P::PROPERTY_LIST, Some(10)),
            (P::PROPERTY_LIST, Some(11)),
            (P::PROPERTY_LIST, Some(12)),
            (P::FILE_SIZE, Some(0)),
            (P::RECORD_COUNT, Some(1)),
            (P::RECORD_COUNT, None),
        ];
        let request = ReadPropertyMultipleRequest {
            list_of_read_access_specs: vec![ReadAccessSpecification {
                object_identifier: oid,
                list_of_property_references: references
                    .iter()
                    .map(|&(p, index)| PropertyReference {
                        property_identifier: p,
                        property_array_index: index,
                    })
                    .collect(),
            }],
        };
        let mut bytes = BytesMut::new();
        request.encode(&mut bytes).unwrap();
        let mut legacy = BytesMut::new();
        handle_read_property_multiple(&db, &bytes, &mut legacy).unwrap();
        let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
        let results = &ack.list_of_read_access_results[0].list_of_results;
        assert_eq!(results.len(), references.len());
        for (result, (p, index)) in results.iter().zip(references) {
            assert_eq!(result.property_identifier, p);
            assert_eq!(
                result.property_array_index,
                if p == P::PROPERTY_LIST { index } else { None }
            );
            let expected = match (p, index) {
                (P::PROPERTY_LIST, Some(0)) => {
                    Ok(PropertyValue::Unsigned(if method == 0 { 11 } else { 10 }))
                }
                (P::PROPERTY_LIST, Some(1)) => {
                    Ok(PropertyValue::Enumerated(P::DESCRIPTION.to_raw()))
                }
                (P::PROPERTY_LIST, Some(10)) => {
                    Ok(PropertyValue::Enumerated(P::RELIABILITY.to_raw()))
                }
                (P::PROPERTY_LIST, Some(11)) if method == 0 => {
                    Ok(PropertyValue::Enumerated(P::RECORD_COUNT.to_raw()))
                }
                (P::PROPERTY_LIST, _) => Err(ErrorCode::INVALID_ARRAY_INDEX),
                (_, Some(_)) => Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
                (P::RECORD_COUNT, None) if method == 0 => Ok(PropertyValue::Unsigned(0)),
                _ => Err(ErrorCode::UNKNOWN_PROPERTY),
            };
            match expected {
                Ok(expected) => {
                    assert!(result.error.is_none());
                    let value_bytes = result.property_value.as_ref().unwrap();
                    let (value, end) =
                        bacnet_encoding::primitives::decode_application_value(value_bytes, 0)
                            .unwrap();
                    assert_eq!(value, expected);
                    assert_eq!(end, value_bytes.len());
                }
                Err(code) => {
                    assert_eq!(result.error, Some((ErrorClass::PROPERTY, code)));
                    assert!(result.property_value.is_none());
                }
            }
        }
        let budget = crate::server::ReadPropertyMultipleBudget {
            max_result_elements: references.len(),
            max_service_ack_bytes: legacy.len(),
        };
        let mut bounded = BytesMut::new();
        crate::handlers::rpm_budget::handle_rpm_budgeted(&db, &bytes, &mut bounded, budget)
            .unwrap();
        assert_eq!(bounded, legacy);
        for too_small in [
            crate::server::ReadPropertyMultipleBudget {
                max_result_elements: references.len() - 1,
                ..budget
            },
            crate::server::ReadPropertyMultipleBudget {
                max_service_ack_bytes: legacy.len() - 1,
                ..budget
            },
        ] {
            let mut prefix = BytesMut::from(&b"prefix"[..]);
            let failure = crate::handlers::rpm_budget::handle_rpm_budgeted(
                &db,
                &bytes,
                &mut prefix,
                too_small,
            );
            if too_small.max_result_elements < references.len() {
                assert!(matches!(
                    failure,
                    Err(crate::handlers::rpm_budget::RpmFailure::Work)
                ));
            } else {
                assert!(matches!(
                    failure,
                    Err(crate::handlers::rpm_budget::RpmFailure::Bytes)
                ));
            }
            assert_eq!(&prefix[..], b"prefix");
        }
    }
}
