//! How the low-level client's errors sort into [`OpsError`].

use std::time::Duration;

use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
use bacnet_types::error::{DecodeError, DecodingKind, Error, ErrorDetail, RemoteError};

use super::*;

fn at() -> DeviceAddress {
    DeviceAddress::local(&[5]).unwrap()
}

fn unknown_property() -> RemoteError {
    RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::UNKNOWN_PROPERTY,
    }
}

/// A timeout, the transaction state machine's included, is unreachable,
/// and so is a link failure, which isn't a timeout; none is a refusal.
#[test]
fn a_timeout_is_unreachable_never_refused() {
    let cases = [
        (Error::Timeout(Duration::from_secs(3)), true),
        (Error::abort(AbortReason::TSM_TIMEOUT), true),
        (Error::Transport(std::io::Error::other("link down")), false),
    ];
    for (error, timeout) in cases {
        let text = error.to_string();
        let sorted = OpsError::at(&at(), error);
        assert!(
            matches!(&sorted, OpsError::Unreachable { device, .. } if *device == at()),
            "{sorted:?}"
        );
        assert!(sorted.unreachable(), "{sorted:?}");
        assert_eq!(sorted.timed_out(), timeout, "{sorted:?}");
        assert_eq!(sorted.remote(), None, "{sorted:?}");
        // The text is the client's own.
        assert_eq!(sorted.to_string(), text);
        assert!(std::error::Error::source(&sorted).is_some());
    }
}

/// A device's Error, Reject or other Abort is a refusal holding it; a
/// structured Error PDU keeps its class and code.
#[test]
fn a_device_answer_is_refused() {
    let detail = ErrorDetail::FirstFailedElementNumber(2);
    let cases = [
        (Error::Remote(unknown_property()), unknown_property()),
        (
            Error::reject(RejectReason::UNRECOGNIZED_SERVICE),
            RemoteError::Reject {
                reason: RejectReason::UNRECOGNIZED_SERVICE,
            },
        ),
        (
            Error::abort(AbortReason::SEGMENTATION_NOT_SUPPORTED),
            RemoteError::Abort {
                reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            },
        ),
        (
            Error::with_detail(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
                Some(detail),
            ),
            unknown_property(),
        ),
    ];
    for (error, remote) in cases {
        let sorted = OpsError::at(&at(), error);
        assert!(
            matches!(&sorted, OpsError::Refused { device, error } if *device == at() && *error == remote),
            "{sorted:?}"
        );
        assert!(!sorted.unreachable());
        assert!(!sorted.timed_out());
        assert_eq!(sorted.remote(), Some(remote));
        assert_eq!(sorted.to_string(), remote.to_string());
    }
}

/// Any other client error passes through unchanged, whether or not a
/// device is named.
#[test]
fn other_client_errors_pass_through() {
    let decoding = || {
        Error::Decoding(DecodeError::new(
            DecodingKind::Trailing,
            4,
            "{} trailing byte(s)",
        ))
    };
    let text = decoding().to_string();
    for sorted in [OpsError::at(&at(), decoding()), OpsError::from(decoding())] {
        assert!(
            matches!(&sorted, OpsError::Client(Error::Decoding(e)) if e.kind == DecodingKind::Trailing),
            "{sorted:?}"
        );
        assert!(!sorted.unreachable());
        assert_eq!(sorted.to_string(), text);
    }
    // Without a device, a device's answer stays the client's error, and
    // still says what it is.
    let remote = OpsError::from(Error::Remote(unknown_property()));
    assert!(matches!(remote, OpsError::Client(Error::Remote(_))));
    assert_eq!(remote.remote(), Some(unknown_property()));
    let timeout = OpsError::from(Error::Timeout(Duration::from_secs(1)));
    assert!(timeout.unreachable() && timeout.timed_out());
}

/// The device lookups' errors get their own variants, with the same text.
#[test]
fn lookups_are_unknown_or_ambiguous() {
    let unknown = Error::UnknownDevice {
        device: "2:1234".into(),
    };
    let text = unknown.to_string();
    let sorted = OpsError::at(&at(), unknown);
    assert!(
        matches!(&sorted, OpsError::UnknownDevice { target } if target == "2:1234"),
        "{sorted:?}"
    );
    assert_eq!(sorted.to_string(), text);

    let addresses = [
        "10.0.0.1:47808".parse::<DeviceAddress>().unwrap(),
        DeviceAddress::new(2, &[5]).unwrap(),
    ];
    let ambiguous = Error::AmbiguousDevice {
        instance: 7,
        candidates: addresses.iter().map(BACnetAddress::from).collect(),
    };
    let text = ambiguous.to_string();
    let sorted = OpsError::from(ambiguous);
    let OpsError::Ambiguous {
        instance,
        candidates,
    } = &sorted
    else {
        panic!("{sorted:?}");
    };
    assert_eq!(*instance, 7);
    assert_eq!(candidates, &addresses);
    assert_eq!(sorted.to_string(), text);
    assert!(!sorted.unreachable());
}

/// Refused text reads as the parser's reason.
#[test]
fn a_parse_error_reads_as_its_reason() {
    let error = OpsError::parse("object", "zz:1", "unknown object type: 'zz'".into());
    assert!(matches!(
        &error,
        OpsError::Parse { what: "object", input, .. } if input == "zz:1"
    ));
    assert_eq!(error.to_string(), "unknown object type: 'zz'");
    assert!(!error.unreachable());
}
