use super::*;
use bacnet_objects::{notification_class::NotificationClass, traits::BACnetObject};
use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::constructed::{BACnetDestination, BACnetRecipient};
use bacnet_types::primitives::Time;
use PropertyIdentifier as P;

#[test]
fn rpm_notification_class_indexed_reads_and_constructed_bytes_are_unchanged() {
    let mut object = NotificationClass::new(7, "NC-7").unwrap();
    object.priority = [12, 34, 56];
    object.ack_required = [true, false, true];
    object.add_destination(BACnetDestination {
        valid_days: 0x7f,
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap()),
        process_identifier: 123,
        issue_confirmed_notifications: true,
        transitions: 0b101,
    });
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();
    // Independently spelled application/choice bytes; no metadata or encoder oracle.
    type ExpectedRead = Result<&'static [u8], ErrorCode>;
    let cases: &[(P, Option<u32>, ExpectedRead)] = &[
        (P::PRIORITY, None, Ok(&[0x21, 12, 0x21, 34, 0x21, 56])),
        (P::PRIORITY, Some(0), Ok(&[0x21, 3])),
        (P::PRIORITY, Some(1), Ok(&[0x21, 12])),
        (P::PRIORITY, Some(2), Ok(&[0x21, 34])),
        (P::PRIORITY, Some(3), Ok(&[0x21, 56])),
        (P::PRIORITY, Some(4), Err(ErrorCode::INVALID_ARRAY_INDEX)),
        (P::ACK_REQUIRED, None, Ok(&[0x82, 5, 0xa0])),
        (
            P::ACK_REQUIRED,
            Some(0),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            P::RECIPIENT_LIST,
            None,
            Ok(&[
                0x82, 1, 0xfe, 0xb4, 0, 0, 0, 0, 0xb4, 23, 59, 59, 99, 0x0c, 2, 0, 0, 42, 0x21,
                123, 0x11, 0x82, 5, 0xa0,
            ]),
        ),
        (
            P::RECIPIENT_LIST,
            Some(0),
            Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            P::PROPERTY_LIST,
            None,
            Ok(&[
                0x91, 28, 0x91, 111, 0x91, 36, 0x91, 81, 0x91, 103, 0x91, 17, 0x91, 86, 0x91, 1,
                0x91, 102,
            ]),
        ),
        (P::PROPERTY_LIST, Some(0), Ok(&[0x21, 9])),
        (P::PROPERTY_LIST, Some(1), Ok(&[0x91, 28])),
        (P::PROPERTY_LIST, Some(9), Ok(&[0x91, 102])),
        (
            P::PROPERTY_LIST,
            Some(10),
            Err(ErrorCode::INVALID_ARRAY_INDEX),
        ),
    ];
    let mut request_bytes = BytesMut::new();
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
    .encode(&mut request_bytes);
    let mut legacy = BytesMut::new();
    handle_read_property_multiple(&db, &request_bytes, &mut legacy).unwrap();
    let ack = ReadPropertyMultipleACK::decode(&legacy).unwrap();
    assert_eq!(ack.list_of_read_access_results.len(), 1);
    let access = &ack.list_of_read_access_results[0];
    assert_eq!(access.object_identifier, oid);
    assert_eq!(access.list_of_results.len(), cases.len());
    for (result, &(p, i, expected)) in access.list_of_results.iter().zip(cases) {
        assert_eq!(result.property_identifier, p);
        assert_eq!(result.property_array_index, i);
        let mut request = BytesMut::new();
        ReadPropertyRequest {
            object_identifier: oid,
            property_identifier: p,
            property_array_index: i,
        }
        .encode(&mut request);
        let mut response = BytesMut::new();
        let rp = handle_read_property(&db, &request, &mut response);
        match expected {
            Ok(bytes) => {
                assert!(result.error.is_none());
                assert_eq!(result.property_value.as_deref(), Some(bytes));
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
    let budget = crate::server::ReadPropertyMultipleBudget {
        max_result_elements: cases.len(),
        max_service_ack_bytes: legacy.len(),
    };
    use crate::handlers::rpm_budget::{handle_rpm_budgeted, RpmFailure};
    let mut bounded = BytesMut::new();
    handle_rpm_budgeted(&db, &request_bytes, &mut bounded, budget).unwrap();
    assert_eq!(bounded, legacy);
    let mut prefix = BytesMut::from(&b"prefix"[..]);
    assert!(matches!(
        handle_rpm_budgeted(
            &db,
            &request_bytes,
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
            &db,
            &request_bytes,
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
