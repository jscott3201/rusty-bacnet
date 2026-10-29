//! Each refusal PDU maps to its [`RemoteError`] arm, and no other PDU maps
//! to one (#2026).

use super::*;
use bacnet_types::error::RemoteError;

#[test]
fn error_pdu_gives_its_class_and_code() {
    let pdu = ErrorPdu {
        invoke_id: 7,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        error_class: ErrorClass::PROPERTY,
        error_code: ErrorCode::UNKNOWN_PROPERTY,
        // A structured body doesn't change the class and code it reports.
        error_data: Bytes::from_static(&[0x1f]),
    };
    let expected = RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::UNKNOWN_PROPERTY,
    };
    assert_eq!(pdu.remote_error(), expected);
    assert_eq!(Apdu::Error(pdu).remote_error(), Some(expected));
}

#[test]
fn reject_pdu_gives_its_reason() {
    let pdu = RejectPdu {
        invoke_id: 3,
        reject_reason: RejectReason::UNRECOGNIZED_SERVICE,
    };
    let expected = RemoteError::Reject {
        reason: RejectReason::UNRECOGNIZED_SERVICE,
    };
    assert_eq!(pdu.remote_error(), expected);
    assert_eq!(Apdu::Reject(pdu).remote_error(), Some(expected));
}

#[test]
fn abort_pdu_gives_its_reason_from_either_side() {
    for sent_by_server in [true, false] {
        let pdu = AbortPdu {
            sent_by_server,
            invoke_id: 9,
            abort_reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
        };
        let expected = RemoteError::Abort {
            reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
        };
        assert_eq!(pdu.remote_error(), expected);
        assert_eq!(Apdu::Abort(pdu).remote_error(), Some(expected));
    }
}

#[test]
fn other_pdus_are_not_refusals() {
    let others = [
        Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 1,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_request: Bytes::new(),
        }),
        Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
            service_request: Bytes::new(),
        }),
        Apdu::SimpleAck(SimpleAck {
            invoke_id: 1,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
        Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id: 1,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::new(),
        }),
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: 1,
            sequence_number: 0,
            actual_window_size: 1,
        }),
    ];
    for apdu in others {
        assert_eq!(apdu.remote_error(), None, "{apdu:?}");
    }
}
