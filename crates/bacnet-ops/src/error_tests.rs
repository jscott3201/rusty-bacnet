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
    }
}

/// A router's network reject is unreachable, like a timeout, and never a
/// refusal by the device, which never saw the request; it isn't a timeout
/// either. Its text is the client's, naming the router, the network and
/// the reason (#2724).
#[test]
fn a_network_reject_is_unreachable_never_refused_or_timed_out() {
    use bacnet_npdu::{Mac, RejectMessageReason};
    use bacnet_types::error::NetworkReject;
    let error = || {
        Error::NetworkRejected(Box::new(NetworkReject::new(
            Mac::from_array([2]),
            Some(bacnet_types::data_link::DataLink::Mstp),
            7,
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        )))
    };
    let text = error().to_string();
    assert!(crate::client::unreachable(&error()));
    assert!(!crate::client::timed_out(&error()));
    for sorted in [OpsError::at(&at(), error()), OpsError::from(error())] {
        assert!(sorted.unreachable(), "{sorted:?}");
        assert!(!sorted.timed_out(), "{sorted:?}");
        assert_eq!(sorted.remote(), None, "{sorted:?}");
        assert_eq!(sorted.to_string(), text);
    }
    assert!(matches!(
        OpsError::at(&at(), error()),
        OpsError::Unreachable { device, source: Error::NetworkRejected(_) } if device == at()
    ));
    assert_eq!(
        text,
        "Reject-Message-To-Network from router mstp:2: no route to network 7 \
         (not-directly-connected, reason 1); an earlier attempt may have reached the device"
    );
}

/// A timeout after a router said it was busy is a timeout, unreachable as
/// any is, naming the router in its text (#2724).
#[test]
fn a_busy_timeout_is_a_timeout() {
    use bacnet_npdu::{Mac, RejectMessageReason};
    use bacnet_types::error::NetworkReject;
    let busy = NetworkReject::new(
        Mac::from_array([2]),
        None,
        7,
        RejectMessageReason::ROUTER_BUSY,
    );
    let error = Error::RouterBusyTimeout(Box::new(busy));
    let text = error.to_string();
    assert!(crate::client::timed_out(&error));
    let sorted = OpsError::at(&at(), error);
    assert!(
        matches!(&sorted, OpsError::Unreachable { .. }),
        "{sorted:?}"
    );
    assert!(sorted.unreachable() && sorted.timed_out());
    assert_eq!(sorted.remote(), None);
    assert_eq!(sorted.to_string(), text);
}

/// A device's Error, Reject or other Abort is a refusal holding it; a
/// structured Error PDU keeps its class and code.
#[test]
fn a_device_answer_is_refused() {
    let detail = ErrorDetail::FirstFailedElementNumber(2);
    let cases = [
        (Error::Remote(unknown_property()), unknown_property(), None),
        (
            Error::reject(RejectReason::UNRECOGNIZED_SERVICE),
            RemoteError::Reject {
                reason: RejectReason::UNRECOGNIZED_SERVICE,
            },
            None,
        ),
        (
            Error::abort(AbortReason::SEGMENTATION_NOT_SUPPORTED),
            RemoteError::Abort {
                reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            },
            None,
        ),
        // A structured Error PDU keeps its extra fields.
        (
            Error::with_detail(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
                Some(detail.clone()),
            ),
            unknown_property(),
            Some(detail),
        ),
    ];
    for (error, remote, kept) in cases {
        let text = error.to_string();
        let sorted = OpsError::at(&at(), error);
        let OpsError::Refused {
            device,
            error,
            detail,
        } = &sorted
        else {
            panic!("{sorted:?}");
        };
        assert_eq!((device, *error), (&at(), remote));
        assert_eq!(detail.as_deref(), kept.as_ref());
        assert!(!sorted.unreachable());
        assert!(!sorted.timed_out());
        assert_eq!(sorted.remote(), Some(remote));
        // The text is the client's own, the detail's included.
        assert_eq!(sorted.to_string(), text);
        assert!(std::error::Error::source(&sorted).is_none());
    }
}

/// The texts of `error` and each source after it.
fn chain(error: &(dyn std::error::Error + 'static)) -> Vec<String> {
    let mut texts = vec![error.to_string()];
    let mut next = error.source();
    while let Some(cause) = next {
        texts.push(cause.to_string());
        next = cause.source();
    }
    texts
}

/// A chain prints each cause once: an error's text is the client's
/// error's, and its source that error's own source, so the chain reads as
/// the client error's does, with no link repeated.
#[test]
fn a_chain_prints_its_cause_once() {
    let errors = || {
        [
            Error::Transport(std::io::Error::other("cable out")),
            Error::Timeout(Duration::from_secs(3)),
            Error::Remote(unknown_property()),
        ]
    };
    for (error, client) in errors().into_iter().zip(errors()) {
        let sorted = OpsError::at(&at(), error);
        assert_eq!(chain(&sorted), chain(&client), "{sorted:?}");
    }
    let timeout = OpsError::at(&at(), Error::Timeout(Duration::from_secs(3)));
    assert_eq!(chain(&timeout).len(), 1);
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
        matches!(&sorted, OpsError::UnknownDevice { target, asked: None } if target == "2:1234"),
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
