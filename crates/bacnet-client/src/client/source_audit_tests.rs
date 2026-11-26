//! What an audited request names and how its answer is recorded (#2617).
//! The diversion itself, and the records on the wire, are tested with an
//! endpoint session in `bacnet-endpoint`.

use super::source_audit_targets::{attempted, audit_result, decode};
use super::*;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{
    ReadAccessResult, ReadPropertyMultipleACK, ReadPropertyMultipleRequest, ReadResultElement,
};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    AbortReason, AuditOperation, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::primitives::ObjectIdentifier;

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

fn rp(object: ObjectIdentifier) -> BytesMut {
    let mut data = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut data);
    data
}

/// The services Table 19-5 maps to READ and WRITE are audited, those it
/// maps elsewhere wait for a later release, and one that doesn't decode is
/// refused rather than sent unreported.
#[test]
fn which_services_are_audited() {
    let device = oid(ObjectType::DEVICE, 10);
    let (operation, targets, _) = decode(ConfirmedServiceChoice::READ_PROPERTY, &rp(device))
        .unwrap()
        .unwrap();
    assert_eq!(operation, AuditOperation::READ);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].object, device);
    for later in [
        ConfirmedServiceChoice::SUBSCRIBE_COV,
        ConfirmedServiceChoice::CREATE_OBJECT,
        ConfirmedServiceChoice::DELETE_OBJECT,
        ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        ConfirmedServiceChoice::REINITIALIZE_DEVICE,
        ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
    ] {
        assert!(decode(later, &[]).is_none(), "{later:?}");
    }
    for audited in [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        ConfirmedServiceChoice::READ_RANGE,
        ConfirmedServiceChoice::ATOMIC_READ_FILE,
        ConfirmedServiceChoice::AUDIT_LOG_QUERY,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
        ConfirmedServiceChoice::ADD_LIST_ELEMENT,
        ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
    ] {
        let refused = decode(audited, &[0x0C]).unwrap().err().unwrap();
        assert!(refused.to_string().contains("must decode"), "{audited:?}");
    }
}

/// A record's Result: a peer's Error as it came, structured or not, Aborts
/// and Rejects as their codes, and the client's own timeout as TIMEOUT.
#[test]
fn a_failed_requests_result() {
    let abort_codes = [56, 51, 52, 53, 54, 136, 135, 127, 124, 125, 30, 123];
    for (reason, code) in abort_codes.into_iter().enumerate() {
        assert_eq!(
            audit_result(&Error::abort(AbortReason::from_raw(reason as u8))),
            (ErrorClass::COMMUNICATION, ErrorCode::from_raw(code)),
            "abort {reason}"
        );
    }
    let reject_codes = [69, 59, 60, 61, 62, 63, 64, 65, 66, 67];
    for (reason, code) in reject_codes.into_iter().enumerate() {
        assert_eq!(
            audit_result(&Error::reject(RejectReason::from_raw(reason as u8))),
            (ErrorClass::COMMUNICATION, ErrorCode::from_raw(code)),
            "reject {reason}"
        );
    }
    let communication = |code| (ErrorClass::COMMUNICATION, code);
    // Reasons the standard leaves unassigned below the proprietary range
    // are OTHER; the proprietary range keeps its own codes, to its top.
    for reason in [12, 63] {
        assert_eq!(
            audit_result(&Error::abort(AbortReason::from_raw(reason))),
            communication(ErrorCode::OTHER),
            "abort {reason}"
        );
    }
    for reason in [10, 11, 63] {
        assert_eq!(
            audit_result(&Error::reject(RejectReason::from_raw(reason))),
            communication(ErrorCode::OTHER),
            "reject {reason}"
        );
    }
    for reason in [64, 255] {
        assert_eq!(
            audit_result(&Error::abort(AbortReason::from_raw(reason))),
            communication(ErrorCode::ABORT_PROPRIETARY),
            "abort {reason}"
        );
        assert_eq!(
            audit_result(&Error::reject(RejectReason::from_raw(reason))),
            communication(ErrorCode::REJECT_PROPRIETARY),
            "reject {reason}"
        );
    }
    assert_eq!(
        audit_result(&Error::Timeout(Duration::from_secs(1))),
        communication(ErrorCode::TIMEOUT)
    );
    assert_eq!(
        audit_result(&Error::RoutedPathTooLong { dnet: 5 }),
        communication(ErrorCode::MESSAGE_TOO_LONG)
    );
    // The client's own refusals record OTHER, as their untyped text did
    // (#1833); a request cut off after it may have gone out is recorded
    // with OTHER too, its outcome unknown.
    for local in [
        Error::RequestCutOff { sent: false },
        Error::RequestCutOff { sent: true },
        Error::invalid_config("local"),
        Error::invalid_argument("local"),
        Error::unsupported("local"),
        Error::limit_reached("local"),
        Error::range("local"),
    ] {
        assert_eq!(audit_result(&local), communication(ErrorCode::OTHER));
    }
    let refused = (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!(
        audit_result(&Error::with_detail(
            refused.0,
            refused.1,
            Some(bacnet_types::error::ErrorDetail::FirstFailedElementNumber(
                2
            ))
        )),
        refused
    );
    assert_eq!(
        audit_result(&Error::protocol(refused.0, refused.1)),
        refused
    );
    // A class or code outside the standard's lists passes through as sent.
    let unknown = (ErrorClass::from_raw(512), ErrorCode::from_raw(513));
    assert_eq!(
        audit_result(&Error::protocol(unknown.0, unknown.1)),
        unknown
    );
}

/// A request reached its target when the device answered, the wait ran
/// out, the transport failed while sending, or a stop cut it off after it
/// may have gone out; not when it was refused before sending, its
/// transport not started or stopped, or a stop cut it off before it went.
#[test]
fn which_requests_reached_their_target() {
    assert!(attempted(&Ok(Bytes::new())));
    for reached in [
        Error::abort(AbortReason::TSM_TIMEOUT),
        Error::Timeout(Duration::from_secs(1)),
        Error::Transport(std::io::Error::other("send failed")),
        Error::RoutedPathTooLong { dnet: 5 },
        // As a request whose retry DeviceCommunicationControl withheld
        // (#1833).
        Error::RequestCutOff { sent: true },
    ] {
        assert!(attempted(&Err(reached)));
    }
    // The client's own refusals, typed (#1833), count as the untyped text
    // they replace did: not attempted. So does a send its transport refused
    // as not started or stopped, which went nowhere (#1833).
    for refused in [
        Error::RequestCutOff { sent: false },
        bacnet_transport::port::stopped(),
        bacnet_transport::port::stopped_for_good(),
        super::seat::seat_closed(),
        Error::state(Lifecycle::Stopping, "the seat's node is stopping"),
        Error::invalid_config("transport not set on ClientBuilder"),
        Error::invalid_argument("routed DNET must be in 1..=65534"),
        Error::unsupported("the node's source Audit carries out this request"),
        Error::limit_reached("routed send lost its active path authorization"),
        Error::range("log counts exceed the log's sequence range"),
        Error::SendTooLong {
            length: 2000,
            limit: 1497,
        },
    ] {
        assert!(!attempted(&Err(refused)));
    }
}

/// A read of the wildcard Device names the device the answer reports, as
/// its Target Device too; an answer for another property answers nothing
/// that was asked.
#[test]
fn a_read_names_what_the_answer_reports() {
    let wildcard = oid(ObjectType::DEVICE, ObjectIdentifier::WILDCARD_INSTANCE);
    let device = oid(ObjectType::DEVICE, 10);
    let request = rp(wildcard);
    let (_, targets, decoded) = decode(ConfirmedServiceChoice::READ_PROPERTY, &request)
        .unwrap()
        .unwrap();
    for (property, answered) in [
        (PropertyIdentifier::OBJECT_NAME, true),
        (PropertyIdentifier::DESCRIPTION, false),
    ] {
        let mut ack = BytesMut::new();
        ReadPropertyACK {
            object_identifier: device,
            property_identifier: property,
            property_array_index: None,
            property_value: vec![0x21, 1],
        }
        .encode(&mut ack)
        .unwrap();
        let (records, target_device) = decoded.records(targets.clone(), &Ok(ack.freeze()));
        assert_eq!(records.len(), 1);
        if answered {
            assert_eq!((records[0].0.object, records[0].1), (device, None));
            assert_eq!(target_device, Some(device));
        } else {
            assert_eq!(records[0].0.object, wildcard);
            assert_eq!(
                records[0].1,
                Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
            );
            assert_eq!(target_device, None);
        }
    }
}

/// A ReadPropertyMultiple answer that doesn't follow the request reference
/// by reference is recorded as it reports; one that doesn't decode fails
/// every requested reference.
#[test]
fn an_rpm_answer_that_does_not_follow_the_request() {
    let object = oid(ObjectType::ANALOG_INPUT, 1);
    let request = ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: object,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::REQUIRED,
                property_array_index: None,
            }],
        }],
    };
    let mut data = BytesMut::new();
    request.encode(&mut data).unwrap();
    let (_, targets, decoded) = decode(ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE, &data)
        .unwrap()
        .unwrap();
    let answer = ReadPropertyMultipleACK {
        list_of_read_access_results: vec![ReadAccessResult {
            object_identifier: object,
            list_of_results: [
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
            ]
            .map(|property_identifier| ReadResultElement {
                property_identifier,
                property_array_index: None,
                property_value: Some(vec![0x21, 1]),
                error: None,
            })
            .to_vec(),
        }],
    };
    let mut ack = BytesMut::new();
    answer.encode(&mut ack).unwrap();
    let (records, target_device) = decoded.records(targets.clone(), &Ok(ack.freeze()));
    assert_eq!(
        records
            .iter()
            .map(|(target, result)| (
                target.property.as_ref().unwrap().property_identifier,
                *result
            ))
            .collect::<Vec<_>>(),
        [
            (PropertyIdentifier::OBJECT_IDENTIFIER, None),
            (PropertyIdentifier::OBJECT_NAME, None),
        ]
    );
    assert_eq!(target_device, None);
    let (records, _) = decoded.records(targets, &Ok(Bytes::from_static(&[0x0E])));
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].0.property.as_ref().unwrap().property_identifier,
        PropertyIdentifier::REQUIRED
    );
    assert_eq!(
        records[0].1,
        Some((ErrorClass::COMMUNICATION, ErrorCode::OTHER))
    );
}

/// A WritePropertyMultiple-Error names a property, not a position: for a
/// property the request writes twice it names the last write, so the
/// first is recorded as made, and that write and every one after it carry
/// the error.
#[test]
fn a_wpm_error_naming_a_property_written_twice_names_its_last_write() {
    use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
    use bacnet_types::constructed::{BACnetObjectPropertyReference, BACnetPropertyValue};
    use bacnet_types::error::ErrorDetail;
    let object = oid(ObjectType::ANALOG_VALUE, 1);
    let write = |property_identifier, priority| BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value: vec![0x21, 1],
        priority,
    };
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: object,
            list_of_properties: vec![
                write(PropertyIdentifier::PRESENT_VALUE, Some(8)),
                write(PropertyIdentifier::PRESENT_VALUE, Some(16)),
                write(PropertyIdentifier::DESCRIPTION, None),
            ],
        }],
    };
    let mut data = BytesMut::new();
    request.encode(&mut data).unwrap();
    let (_, targets, decoded) = decode(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, &data)
        .unwrap()
        .unwrap();
    let denied = (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    let error = Error::with_detail(
        denied.0,
        denied.1,
        Some(ErrorDetail::FirstFailedWriteAttempt(
            BACnetObjectPropertyReference {
                object_identifier: object,
                property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
                property_array_index: None,
            },
        )),
    );
    let (records, _) = decoded.records(targets, &Err(error));
    assert_eq!(
        records
            .iter()
            .map(|(_, result)| *result)
            .collect::<Vec<_>>(),
        [None, Some(denied), Some(denied)]
    );
}
