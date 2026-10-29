//! The server answers a typed refusal with the same numbers on the wire
//! (#915): every named error class and code and values without a name in an
//! Error PDU, the plain one and the ChangeList-Error, and every reject
//! reason in a Reject PDU; an abort goes out as SERVICES / OTHER.

use super::error_apdu_from_error;
use bacnet_encoding::apdu::{self, Apdu};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason,
};
use bacnet_types::error::{Error, ErrorDetail};
use bytes::BytesMut;

/// An application-tagged ENUMERATED (tag 9) with the fewest content octets,
/// written out by hand so the test does not lean on the encoder.
fn enumerated(value: u16) -> Vec<u8> {
    let [high, low] = value.to_be_bytes();
    if high == 0 {
        vec![0x91, low]
    } else {
        vec![0x92, high, low]
    }
}

fn wire(service: ConfirmedServiceChoice, error: &Error) -> Vec<u8> {
    let mut buf = BytesMut::new();
    apdu::encode_apdu(&mut buf, &error_apdu_from_error(9, service, error)).unwrap();
    buf.to_vec()
}

fn classes() -> Vec<ErrorClass> {
    let mut all: Vec<_> = ErrorClass::ALL_NAMED.iter().map(|(_, v)| *v).collect();
    all.extend([8, 64, 256, 65_535].map(ErrorClass::from_raw));
    all
}

fn codes() -> Vec<ErrorCode> {
    let mut all: Vec<_> = ErrorCode::ALL_NAMED.iter().map(|(_, v)| *v).collect();
    all.extend([256, 512, 65_535].map(ErrorCode::from_raw));
    all
}

#[test]
fn every_protocol_error_goes_out_with_its_class_and_code() {
    let read = ConfirmedServiceChoice::READ_PROPERTY;
    let add = ConfirmedServiceChoice::ADD_LIST_ELEMENT;
    for class in classes() {
        for code in codes() {
            let pair = [enumerated(class.to_raw()), enumerated(code.to_raw())].concat();

            let plain = wire(read, &Error::protocol(class, code));
            assert_eq!(plain, [&[0x50, 9, read.to_raw()][..], &pair].concat());

            // ChangeList-Error: the pair inside tag 0, then element 2 as
            // context tag 1.
            let structured =
                Error::with_detail(class, code, Some(ErrorDetail::FirstFailedElementNumber(2)));
            let expected = [&[0x50, 9, add.to_raw(), 0x0E][..], &pair, &[0x0F, 0x19, 2]].concat();
            assert_eq!(wire(add, &structured), expected, "{class:?} / {code:?}");
        }
    }
}

/// A handler's abort is no answer the server sends as such: like any refusal
/// other than an Error or a Reject it goes out as an Error PDU with
/// SERVICES / OTHER, for every reason (pinned by #1815).
#[test]
fn an_abort_refusal_goes_out_as_services_other() {
    let read = ConfirmedServiceChoice::READ_PROPERTY;
    let pair = [
        enumerated(ErrorClass::SERVICES.to_raw()),
        enumerated(ErrorCode::OTHER.to_raw()),
    ]
    .concat();
    for raw in 0..=u8::MAX {
        let bytes = wire(read, &Error::abort(AbortReason::from_raw(raw)));
        assert_eq!(bytes, [&[0x50, 9, read.to_raw()][..], &pair].concat());
    }
}

#[test]
fn every_reject_reason_goes_out_unchanged() {
    let read = ConfirmedServiceChoice::READ_PROPERTY;
    for raw in 0..=u8::MAX {
        let reason = RejectReason::from_raw(raw);
        let bytes = wire(read, &Error::reject(reason));
        assert_eq!(bytes, [0x60, 9, raw]);
        let Apdu::Reject(pdu) = apdu::decode_apdu(bytes.into()).unwrap() else {
            panic!("expected a Reject PDU");
        };
        assert_eq!(pdu.reject_reason, reason);
    }
}
