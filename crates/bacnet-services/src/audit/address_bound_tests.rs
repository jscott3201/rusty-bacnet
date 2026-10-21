//! The audit codecs hold every `BACnetAddress` MAC to
//! `BACnetAddress::MAX_MAC_LEN` (18 octets) in both directions (#1156): the
//! AuditLogQuery device-address filters, and an AuditNotification's two
//! recipient fields ([2] and [10]).

use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bacnet_types::enums::{AuditOperation, BACnetSuccessFilter, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;

use super::{
    AuditLogQueryRequest, AuditNotificationRequest, BACnetAuditLogQueryParameters,
    BACnetAuditNotification,
};

const LONGEST: usize = BACnetAddress::MAX_MAC_LEN;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// An address on network 7 whose MAC is `len` octets of 0xA5.
fn address(len: usize) -> BACnetAddress {
    BACnetAddress {
        network_number: 7,
        mac_address: MacAddr::from_slice(&vec![0xA5; len]),
    }
}

/// `encoded` with each 18-octet 0xA5 MAC OCTET STRING one octet longer. The
/// addresses sit inside opening and closing tags, so no outer length changes.
fn lengthen_macs(encoded: &[u8]) -> Vec<u8> {
    let mut longest = vec![0x65, LONGEST as u8];
    longest.extend([0xA5; LONGEST]);
    let mut wire = Vec::new();
    let mut rest = encoded;
    while let Some(at) = rest.windows(longest.len()).position(|w| w == longest) {
        wire.extend_from_slice(&rest[..at]);
        wire.extend([0x65, LONGEST as u8 + 1]);
        wire.extend([0xA5; LONGEST + 1]);
        rest = &rest[at + longest.len()..];
    }
    wire.extend_from_slice(rest);
    assert_ne!(wire, encoded, "no 18-octet MAC to lengthen");
    wire
}

/// Both query choices, each filtering on an address with a `len`-octet MAC.
fn queries(len: usize) -> [AuditLogQueryRequest; 2] {
    let query = |query_parameters| AuditLogQueryRequest {
        audit_log: oid(ObjectType::AUDIT_LOG, 1),
        query_parameters,
        start_at_sequence_number: None,
        requested_count: 5,
    };
    [
        query(BACnetAuditLogQueryParameters::ByTarget {
            target_device_identifier: oid(ObjectType::DEVICE, 2),
            target_device_address: Some(address(len)),
            target_object_identifier: None,
            target_property_identifier: None,
            target_array_index: None,
            target_priority: None,
            operations: None,
            successful_actions_only: BACnetSuccessFilter::ALL,
        }),
        query(BACnetAuditLogQueryParameters::BySource {
            source_device_identifier: oid(ObjectType::DEVICE, 2),
            source_device_address: Some(address(len)),
            source_object_identifier: None,
            operations: None,
            successful_actions_only: BACnetSuccessFilter::ALL,
        }),
    ]
}

#[test]
fn audit_log_query_address_filter_holds_to_the_bacnet_address_bound() {
    for (longest, too_long) in queries(LONGEST).into_iter().zip(queries(LONGEST + 1)) {
        let mut encoded = BytesMut::new();
        longest.try_encode(&mut encoded).unwrap();
        assert_eq!(AuditLogQueryRequest::decode(&encoded).unwrap(), longest);
        let error = AuditLogQueryRequest::decode(&lengthen_macs(&encoded)).unwrap_err();
        assert!(matches!(error, Error::Decoding(_)), "{error:?}");

        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            too_long.try_encode(&mut output),
            Err(Error::Encode(_))
        ));
        assert_eq!(output.as_ref(), b"prefix");
    }
}

#[test]
fn audit_notification_recipients_hold_to_the_bacnet_address_bound() {
    let device = BACnetRecipient::Device(oid(ObjectType::DEVICE, 1));
    let at = |len| BACnetRecipient::Address(address(len));
    let notification = |source_device, target_device| AuditNotificationRequest {
        notifications: vec![BACnetAuditNotification {
            source_timestamp: None,
            target_timestamp: None,
            source_device,
            source_object: None,
            operation: AuditOperation::READ,
            source_comment: None,
            target_comment: None,
            invoke_id: None,
            source_user_id: None,
            source_user_role: None,
            target_device,
            target_object: None,
            target_property: None,
            target_priority: None,
            target_value: None,
            current_value: None,
            result: None,
        }],
    };
    // Each recipient field alone: [2] first, then [10].
    for (longest, too_long) in [
        (
            notification(at(LONGEST), device.clone()),
            notification(at(LONGEST + 1), device.clone()),
        ),
        (
            notification(device.clone(), at(LONGEST)),
            notification(device.clone(), at(LONGEST + 1)),
        ),
    ] {
        let mut encoded = BytesMut::new();
        longest.try_encode(&mut encoded).unwrap();
        assert_eq!(AuditNotificationRequest::decode(&encoded).unwrap(), longest);
        let error = AuditNotificationRequest::decode(&lengthen_macs(&encoded)).unwrap_err();
        assert!(matches!(error, Error::Decoding(_)), "{error:?}");

        let mut output = BytesMut::from(&b"prefix"[..]);
        assert!(matches!(
            too_long.try_encode(&mut output),
            Err(Error::Encode(_))
        ));
        assert_eq!(output.as_ref(), b"prefix");
    }
}
