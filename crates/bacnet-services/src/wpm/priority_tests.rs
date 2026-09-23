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
        Err(Error::Decoding { .. })
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
    for priority in [
        &[0x39][..],
        &[0x3a, 1],
        &[0x38],
        &[0x3d, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ] {
        let (bytes, _) = request(priority);
        let error = cursor_error(&bytes);
        assert_eq!(
            error.kind,
            WritePropertyMultipleFailureKind::Syntax(RejectReason::INVALID_DATA_ENCODING)
        );
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
