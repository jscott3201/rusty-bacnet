//! The server answers a typed refusal with the same numbers on the wire
//! (#915): every named error class and code and values without a name in an
//! Error PDU, the plain one and the ChangeList-Error, and every reject
//! reason in a Reject PDU; an abort goes out as SERVICES / OTHER.

use bacnet_device_core::reply::error_apdu;
use bacnet_encoding::apdu::{self, Apdu};
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason,
};
use bacnet_types::error::{Error, ErrorDetail, RemoteError};
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
    apdu::encode_apdu(&mut buf, &error_apdu(9, service, error)).unwrap();
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

/// Every kind of error the server's own refusals now take (#1833), and
/// persisted state, goes out as the untyped text it replaces did: an Error PDU with SERVICES / OTHER,
/// the plain one and each Clause 21 production alike. A refusal is never a
/// syntax fault of the request, so `into_request_reject` leaves each as it
/// is; only a decoding error becomes a Reject there, which is why a site
/// typed as `Decoding` is one no request decode passes through.
#[test]
fn local_refusals_go_out_as_the_text_they_replace() {
    use bacnet_types::error::{
        DecodeError, DecodingKind, Lifecycle, PersistenceError, PersistenceErrorKind, RangeError,
        Refusal,
    };
    let services = [
        ConfirmedServiceChoice::READ_PROPERTY,
        ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
        ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        ConfirmedServiceChoice::ADD_LIST_ELEMENT,
        ConfirmedServiceChoice::CREATE_OBJECT,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ];
    let refusals = || {
        let mut all = vec![
            Error::invalid_config("cov_retry_timeout_ms is 0"),
            Error::invalid_argument("time sync: step cap exceeded"),
            Error::unsupported("Device clock is disabled"),
            Error::LimitReached(Refusal::text("server I-Am broadcast capacity exhausted")),
            Error::from(
                RangeError::new("ConfirmedAuditNotification payload exceeds {} bytes")
                    .with_value(65_536),
            ),
            Error::from(DecodeError::new(
                DecodingKind::OutOfRange,
                0,
                "unknown EnableDisable value",
            )),
            // Persisted state (#1833): damage, an encode refusal and a
            // backend's failure, which were that text too.
            Error::from(
                PersistenceError::new(PersistenceErrorKind::Damaged, "checksum mismatch")
                    .in_format("Tags file"),
            ),
            Error::from(PersistenceError::new(
                PersistenceErrorKind::RefusedOnEncode,
                "more entries than the cap",
            )),
            Error::from(PersistenceError::new(
                PersistenceErrorKind::BackendFailed,
                "save failed; its error was reported",
            )),
        ];
        for state in [
            Lifecycle::AlreadyStarted,
            Lifecycle::NotRunning,
            Lifecycle::Stopping,
            Lifecycle::NotConnected,
            Lifecycle::Closed,
        ] {
            all.push(Error::state(state, "Audit delivery owner is closed"));
        }
        all
    };
    // What the untyped text went out as, which #1833 removed: SERVICES /
    // OTHER, in each service's own production.
    let services_other = |service| {
        wire(
            service,
            &Error::protocol(ErrorClass::SERVICES, ErrorCode::OTHER),
        )
    };
    // Error PDU, invoke ID 9, the service; SERVICES (5) and OTHER (0) as
    // application-tagged ENUMERATED.
    let read = ConfirmedServiceChoice::READ_PROPERTY;
    assert_eq!(
        services_other(read),
        [0x50, 9, read.to_raw(), 0x91, 5, 0x91, 0]
    );
    for service in services {
        let untyped = services_other(service);
        for refusal in refusals() {
            assert_eq!(wire(service, &refusal), untyped, "{service:?}: {refusal:?}");
        }
    }
    for refusal in refusals() {
        let decoding = matches!(refusal, Error::Decoding(_));
        let as_request = refusal.into_request_reject();
        if decoding {
            assert!(
                matches!(as_request, Error::Remote(RemoteError::Reject { .. })),
                "{as_request:?}"
            );
        } else {
            assert_eq!(
                wire(read, &as_request),
                services_other(read),
                "{as_request:?}"
            );
        }
    }
}
