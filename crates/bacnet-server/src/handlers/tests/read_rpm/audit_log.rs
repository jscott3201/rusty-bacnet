use super::*;
use bacnet_objects::{
    audit::{AuditLogObject, AuditLogPersistence, AuditLogSnapshot},
    traits::BACnetObject,
};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use std::sync::{Arc, Mutex};
use PropertyIdentifier as P;

type ExpectedRead = Result<&'static [u8], ErrorCode>;

#[derive(Default)]
struct MemoryPersistence(Mutex<Option<AuditLogSnapshot>>);

impl AuditLogPersistence for MemoryPersistence {
    fn load(&self, _expected_object: ObjectIdentifier) -> Result<Option<AuditLogSnapshot>, Error> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn commit(&self, snapshot: &AuditLogSnapshot) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(snapshot.clone());
        Ok(())
    }
}

#[test]
fn rpm_audit_log_indexed_reads_and_list_bytes_are_unchanged() {
    let object = AuditLogObject::new(7, "AL-7", 4, Arc::new(MemoryPersistence::default())).unwrap();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    // Independent bytes pin the existing enumerated/Boolean/Unsigned/list
    // projection, not a new wire codec or positional list access.
    let cases: &[(P, Option<u32>, ExpectedRead)] = &[
        (
            P::OBJECT_IDENTIFIER,
            None,
            Ok(&[0xC4, 0x0F, 0x40, 0x00, 0x07]),
        ),
        (
            P::OBJECT_IDENTIFIER,
            Some(0),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            P::OBJECT_NAME,
            None,
            Ok(&[0x75, 0x05, 0x00, 0x41, 0x4C, 0x2D, 0x37]),
        ),
        (
            P::OBJECT_NAME,
            Some(1),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (P::DESCRIPTION, None, Ok(&[0x71, 0x00])),
        (
            P::DESCRIPTION,
            Some(1),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (P::OBJECT_TYPE, None, Ok(&[0x91, 61])),
        (P::LOG_ENABLE, None, Ok(&[0x11])),
        (
            P::LOG_ENABLE,
            Some(0),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (P::BUFFER_SIZE, None, Ok(&[0x21, 4])),
        (
            P::BUFFER_SIZE,
            Some(1),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (P::RECORD_COUNT, None, Ok(&[0x21, 0])),
        (P::TOTAL_RECORD_COUNT, None, Ok(&[0x21, 0])),
        (P::STATUS_FLAGS, None, Ok(&[0x82, 4, 0])),
        (P::EVENT_STATE, None, Ok(&[0x91, 0])),
        (
            P::PROPERTY_LIST,
            None,
            Ok(&[
                0x91, 28, 0x91, 133, 0x91, 126, 0x91, 141, 0x91, 145, 0x91, 111, 0x91, 36,
            ]),
        ),
        (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 7])),
        (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
        (P::PROPERTY_LIST, Some(7), Ok(&[0x91, 36])),
        (
            P::PROPERTY_LIST,
            Some(8),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
        ),
        (
            P::PROPERTY_LIST,
            Some(u32::MAX),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
        ),
        // Unserved Clause 12.64 rows stay unknown, including Reliability.
        (P::LOG_BUFFER, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        (P::RELIABILITY, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
        (P::MEMBER_OF, None, Err(ErrorCode::UNKNOWN_PROPERTY)),
    ];
    assert_indexed_cases(&db, oid, cases);
}

fn assert_indexed_cases(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    cases: &[(P, Option<u32>, ExpectedRead)],
) {
    let mut request = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid,
            list_of_property_references: cases
                .iter()
                .map(|&(p, i, _)| PropertyReference {
                    property_identifier: p,
                    property_array_index: i,
                })
                .collect(),
        }],
    }
    .encode(&mut request);
    let mut legacy = BytesMut::new();
    handle_read_property_multiple(db, &request, &mut legacy).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let access = &ack.list_of_read_access_results[0];
    assert_eq!(access.object_identifier, oid);
    assert_eq!(access.list_of_results.len(), cases.len());
    for (result, &(p, i, expected)) in access.list_of_results.iter().zip(cases) {
        assert_eq!(result.property_identifier, p);
        assert_eq!(result.property_array_index, i);
        let mut rp_request = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: p,
            property_array_index: i,
        }
        .encode(&mut rp_request);
        let mut response = BytesMut::new();
        let rp = handle_read_property(db, &rp_request, &mut response);
        match expected {
            Ok(bytes) => {
                assert!(result.error.is_none(), "{p:?} {i:?}");
                assert_eq!(result.property_value.as_deref(), Some(bytes), "{p:?} {i:?}");
                rp.unwrap();
                let rp_ack = ReadPropertyACK::decode(&response).unwrap();
                assert_eq!(rp_ack.object_identifier, oid);
                assert_eq!(rp_ack.property_identifier, p);
                assert_eq!(rp_ack.property_array_index, i);
                assert_eq!(rp_ack.property_value, bytes);
            }
            Err(expected) => {
                assert!(result.property_value.is_none());
                assert_eq!(result.error, Some((ErrorClass::PROPERTY, expected)));
                assert!(matches!(rp, Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY.to_raw() as u32 && code == expected.to_raw() as u32));
                assert!(response.is_empty());
            }
        }
    }
    use crate::handlers::rpm_budget::{handle_rpm_budgeted, RpmFailure};
    let budget = crate::server::ReadPropertyMultipleBudget {
        max_result_elements: cases.len(),
        max_service_ack_bytes: legacy.len(),
    };
    let mut bounded = BytesMut::new();
    handle_rpm_budgeted(db, &request, &mut bounded, budget).unwrap();
    assert_eq!(bounded, legacy);
    let mut prefix = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(
            db,
            &request,
            &mut prefix,
            crate::server::ReadPropertyMultipleBudget {
                max_result_elements: cases.len() - 1,
                ..budget
            }
        ),
        Err(RpmFailure::Work)
    ));
    assert_eq!(&prefix[..], b"prefix");
    assert!(matches!(
        handle_rpm_budgeted(
            db,
            &request,
            &mut prefix,
            crate::server::ReadPropertyMultipleBudget {
                max_service_ack_bytes: legacy.len() - 1,
                ..budget
            }
        ),
        Err(RpmFailure::Bytes)
    ));
    assert_eq!(&prefix[..], b"prefix");
}
