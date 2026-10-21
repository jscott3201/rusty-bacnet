//! The typed protocol errors carry exactly the wire values (#915): every
//! named error class and code, every reject and abort reason, and values
//! without a name (proprietary or reserved) travel from the octets of an
//! Error, Reject or Abort PDU to the `Error` the client returns, and back,
//! with the same numbers.

use super::*;
use crate::tsm::TsmResponse;
use bacnet_encoding::apdu::{AbortPdu, ErrorPdu, RejectPdu};
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};

/// An application-tagged ENUMERATED (tag 9) with the fewest content octets,
/// written out by hand so the test does not lean on the encoder it checks.
fn enumerated(value: u32) -> Vec<u8> {
    let octets = value.to_be_bytes();
    let skip = octets
        .iter()
        .take(3)
        .take_while(|octet| **octet == 0)
        .count();
    let content = &octets[skip..];
    let mut out = vec![0x90 | content.len() as u8];
    out.extend_from_slice(content);
    out
}

/// Each named constant, then values without a name at the edges.
fn classes() -> Vec<ErrorClass> {
    let mut all: Vec<_> = ErrorClass::ALL_NAMED.iter().map(|(_, v)| *v).collect();
    all.extend([8, 63, 64, 255, 256, 65_535].map(ErrorClass::from_raw));
    all
}

fn codes() -> Vec<ErrorCode> {
    let mut all: Vec<_> = ErrorCode::ALL_NAMED.iter().map(|(_, v)| *v).collect();
    all.extend([255, 256, 511, 512, 65_535].map(ErrorCode::from_raw));
    all
}

#[test]
fn every_error_class_and_code_round_trips_through_the_wire() {
    let service = ConfirmedServiceChoice::READ_PROPERTY;
    for class in classes() {
        for code in codes() {
            let mut buf = BytesMut::new();
            apdu::encode_apdu(
                &mut buf,
                &Apdu::Error(ErrorPdu {
                    invoke_id: 7,
                    service_choice: service,
                    error_class: class,
                    error_code: code,
                    error_data: Bytes::new(),
                }),
            )
            .unwrap();
            let mut wire = vec![0x50, 7, service.to_raw()];
            wire.extend(enumerated(u32::from(class.to_raw())));
            wire.extend(enumerated(u32::from(code.to_raw())));
            assert_eq!(&buf[..], &wire[..], "{class:?} / {code:?}");

            let Apdu::Error(pdu) = apdu::decode_apdu(buf.freeze()).unwrap() else {
                panic!("expected an Error PDU");
            };
            let result = confirmed_response_result(TsmResponse::from_error_pdu(&pdu));
            let Err(Error::Protocol { class: c, code: k }) = result else {
                panic!("expected a protocol error, got {result:?}");
            };
            assert_eq!((c.to_raw(), k.to_raw()), (class.to_raw(), code.to_raw()));
            assert_eq!((c, k), (class, code));
        }
    }
}

#[test]
fn every_reject_and_abort_reason_round_trips_through_the_wire() {
    for raw in 0..=u8::MAX {
        let reason = RejectReason::from_raw(raw);
        let mut buf = BytesMut::new();
        apdu::encode_apdu(
            &mut buf,
            &Apdu::Reject(RejectPdu {
                invoke_id: 3,
                reject_reason: reason,
            }),
        )
        .unwrap();
        assert_eq!(&buf[..], &[0x60, 3, raw]);
        let Apdu::Reject(pdu) = apdu::decode_apdu(buf.freeze()).unwrap() else {
            panic!("expected a Reject PDU");
        };
        let result = confirmed_response_result(TsmResponse::Reject {
            reason: pdu.reject_reason,
        });
        let Err(Error::Reject { reason: got }) = result else {
            panic!("expected a reject, got {result:?}");
        };
        assert_eq!(got.to_raw(), raw);
        // A responder answers with the reason it was given.
        assert_eq!(Error::Reject { reason: got }.reject_reason(), Some(reason));

        let reason = AbortReason::from_raw(raw);
        let mut buf = BytesMut::new();
        apdu::encode_apdu(
            &mut buf,
            &Apdu::Abort(AbortPdu {
                sent_by_server: true,
                invoke_id: 4,
                abort_reason: reason,
            }),
        )
        .unwrap();
        assert_eq!(&buf[..], &[0x71, 4, raw]);
        let Apdu::Abort(pdu) = apdu::decode_apdu(buf.freeze()).unwrap() else {
            panic!("expected an Abort PDU");
        };
        let result = confirmed_response_result(TsmResponse::Abort {
            reason: pdu.abort_reason,
        });
        let Err(Error::Abort { reason: got }) = result else {
            panic!("expected an abort, got {result:?}");
        };
        assert_eq!(got.to_raw(), raw);
    }
}

#[test]
fn display_keeps_the_numeric_reasons() {
    assert_eq!(
        Error::Reject {
            reason: RejectReason::UNDEFINED_ENUMERATION
        }
        .to_string(),
        "BACnet reject: reason=8"
    );
    assert_eq!(
        Error::Abort {
            reason: AbortReason::from_raw(200)
        }
        .to_string(),
        "BACnet abort: reason=200"
    );
}
