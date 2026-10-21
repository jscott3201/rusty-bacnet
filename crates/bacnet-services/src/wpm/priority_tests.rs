use super::*;
use bacnet_types::enums::{ObjectType, PropertyIdentifier as P, RejectReason};

fn request(priority: &[u8]) -> (BytesMut, usize) {
    let mut bytes = BytesMut::new();
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 7).unwrap();
    primitives::encode_ctx_object_id(&mut bytes, 0, &oid);
    tags::encode_opening_tag(&mut bytes, 1);
    primitives::encode_ctx_unsigned(&mut bytes, 0, P::PRESENT_VALUE.to_raw() as u64);
    primitives::encode_ctx_unsigned(&mut bytes, 1, 4);
    tags::encode_opening_tag(&mut bytes, 2);
    primitives::encode_app_real(&mut bytes, 20.0);
    tags::encode_closing_tag(&mut bytes, 2);
    let offset = bytes.len();
    bytes.extend_from_slice(priority);
    (bytes, offset)
}

fn cursor_error(bytes: &[u8]) -> WritePropertyMultipleCursorError {
    let mut cursor = WritePropertyMultipleCursor::new(bytes);
    assert!(matches!(
        cursor.next_event().unwrap(),
        Some(WritePropertyMultipleEvent::ObjectStart(_))
    ));
    let error = cursor.next_event().unwrap_err();
    assert_eq!(
        cursor.next_event().unwrap(),
        None,
        "failure stops the cursor"
    );
    assert!(matches!(
        WritePropertyMultipleRequest::decode(bytes),
        Err(Error::Decoding(_))
    ));
    error
}

#[test]
fn wpm_priority_range_failure_keeps_kind_stage_offset_and_indexed_reference() {
    for priority in [0, 17, 257, u64::MAX] {
        let mut encoded = BytesMut::new();
        primitives::encode_ctx_unsigned(&mut encoded, 3, priority);
        let (mut bytes, offset) = request(&encoded);
        tags::encode_closing_tag(&mut bytes, 1);
        let error = cursor_error(&bytes);
        assert_eq!(
            error.kind,
            WritePropertyMultipleFailureKind::PriorityOutOfRange
        );
        assert_eq!(error.stage, WritePropertyMultipleDecodeStage::Priority);
        assert_eq!(error.offset, tags::decode_tag(&bytes, offset).unwrap().1);
        let reference = error.first_failed_write_attempt.unwrap();
        assert_eq!(
            reference.object_identifier,
            ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 7).unwrap()
        );
        assert_eq!(reference.property_identifier, P::PRESENT_VALUE.to_raw());
        assert_eq!(reference.property_array_index, Some(4));
    }
}

#[test]
fn wpm_priority_malformed_unsigned_is_syntax_not_numeric_range() {
    // A priority cut short is missing octets (#1446); one of no octets or of
    // nine is an invalid encoding.
    for (priority, reason) in [
        (&[0x39][..], RejectReason::MISSING_REQUIRED_PARAMETER),
        (&[0x3a, 1], RejectReason::MISSING_REQUIRED_PARAMETER),
        (&[0x38], RejectReason::INVALID_DATA_ENCODING),
        (
            &[0x3d, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            RejectReason::INVALID_DATA_ENCODING,
        ),
    ] {
        let (bytes, _) = request(priority);
        let error = cursor_error(&bytes);
        assert_eq!(error.kind, WritePropertyMultipleFailureKind::Syntax(reason));
        assert_eq!(error.stage, WritePropertyMultipleDecodeStage::Priority);
        assert_eq!(
            error
                .first_failed_write_attempt
                .unwrap()
                .property_array_index,
            Some(4)
        );
    }
}

#[test]
fn whole_request_decode_keeps_the_cursor_reason() {
    // A cut-short priority, a priority of no octets, and one of 17: the
    // decode error draws the Reject reason the cursor chose (#1446).
    for (priority, reason) in [
        (&[0x3a, 1][..], RejectReason::MISSING_REQUIRED_PARAMETER),
        (&[0x38], RejectReason::INVALID_DATA_ENCODING),
        (&[0x39, 17], RejectReason::PARAMETER_OUT_OF_RANGE),
    ] {
        let (bytes, _) = request(priority);
        let error = WritePropertyMultipleRequest::decode(&bytes).unwrap_err();
        assert_eq!(error.reject_reason(), Some(reason), "{priority:02X?}");
    }
    // Every reason a cursor gives maps back to itself.
    for reason in [
        RejectReason::INVALID_TAG,
        RejectReason::MISSING_REQUIRED_PARAMETER,
        RejectReason::TOO_MANY_ARGUMENTS,
        RejectReason::PARAMETER_OUT_OF_RANGE,
        RejectReason::BUFFER_OVERFLOW,
        RejectReason::INVALID_DATA_ENCODING,
        RejectReason::OTHER,
    ] {
        let kind = super::failure_kind(WritePropertyMultipleFailureKind::Syntax(reason));
        assert_eq!(kind.reject_reason(), reason);
    }
}
