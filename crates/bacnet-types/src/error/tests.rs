use super::*;

/// A remote error reads the same on its own and inside `Error::Remote`, and
/// keeps the text the three variants it replaced had (#1815): class and code
/// by name, a number for a value without one, reasons as numbers.
#[test]
fn remote_error_display_is_unchanged() {
    for (remote, text) in [
        (
            RemoteError::Error {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_OBJECT,
            },
            "BACnet error: property / unknown-object",
        ),
        (
            RemoteError::Error {
                class: ErrorClass::from_raw(999),
                code: ErrorCode::from_raw(65_535),
            },
            "BACnet error: class=999 / code=65535",
        ),
        (
            RemoteError::Reject {
                reason: RejectReason::UNDEFINED_ENUMERATION,
            },
            "BACnet reject: reason=8",
        ),
        (
            RemoteError::Reject {
                reason: RejectReason::from_raw(200),
            },
            "BACnet reject: reason=200",
        ),
        (
            RemoteError::Abort {
                reason: AbortReason::SEGMENTATION_NOT_SUPPORTED,
            },
            "BACnet abort: reason=4",
        ),
        (
            RemoteError::Abort {
                reason: AbortReason::from_raw(255),
            },
            "BACnet abort: reason=255",
        ),
    ] {
        assert_eq!(remote.to_string(), text);
        let error = Error::from(remote);
        assert_eq!(error.to_string(), text);
        assert!(matches!(error, Error::Remote(held) if held == remote));
        assert_eq!(error.remote(), Some(remote));
    }
}

#[test]
fn constructors_build_the_remote_arms() {
    let (class, code) = (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    assert_eq!(
        Error::protocol(class, code).remote(),
        Some(RemoteError::Error { class, code })
    );
    let reason = RejectReason::INVALID_TAG;
    assert_eq!(
        Error::reject(reason).remote(),
        Some(RemoteError::Reject { reason })
    );
    assert_eq!(Error::reject(reason).reject_reason(), Some(reason));
    let reason = AbortReason::OUT_OF_RESOURCES;
    assert_eq!(
        Error::abort(reason).remote(),
        Some(RemoteError::Abort { reason })
    );
    // An abort is no syntax fault: a responder doesn't turn it into a reject.
    assert_eq!(Error::abort(reason).reject_reason(), None);
    assert_eq!(Error::Timeout(Duration::from_secs(1)).remote(), None);
}

/// A structured error reports its class and code as the remote error, without
/// the detail.
#[cfg(feature = "alloc")]
#[test]
fn remote_reads_a_structured_error_without_its_detail() {
    let (class, code) = (ErrorClass::SERVICES, ErrorCode::LIST_ELEMENT_NOT_FOUND);
    let structured =
        Error::with_detail(class, code, Some(ErrorDetail::FirstFailedElementNumber(3)));
    assert!(matches!(structured, Error::Structured { .. }));
    assert_eq!(
        structured.remote(),
        Some(RemoteError::Error { class, code })
    );
}

#[test]
fn protocol_error_display() {
    let err = Error::protocol(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_OBJECT);
    assert_eq!(err.to_string(), "BACnet error: property / unknown-object");

    // Unknown class/code falls back to numeric
    let err2 = Error::protocol(ErrorClass::from_raw(999), ErrorCode::from_raw(999));
    assert_eq!(err2.to_string(), "BACnet error: class=999 / code=999");
    assert_eq!(
        Error::protocol(ErrorClass::PROPERTY, ErrorCode::from_raw(999)).to_string(),
        "BACnet error: property / code=999"
    );
}

#[cfg(feature = "alloc")]
#[test]
fn structured_error_display_names_the_detail() {
    use crate::enums::ObjectType;
    use crate::primitives::ObjectIdentifier;

    let reference = |index| BACnetObjectPropertyReference {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE.to_raw(),
        property_array_index: index,
    };
    let private_transfer = |error_parameters| ErrorDetail::PrivateTransfer {
        vendor_id: 555,
        service_number: 7,
        error_parameters,
    };
    for (class, code, detail, expected) in [
        (
            5,
            81,
            ErrorDetail::FirstFailedElementNumber(2),
            "BACnet error: services / list-element-not-found, first failed element 2",
        ),
        (
            2,
            40,
            ErrorDetail::FirstFailedWriteAttempt(reference(Some(3))),
            "BACnet error: property / write-access-denied, \
             first failed write attempt ANALOG_VALUE,7 PRESENT_VALUE[3]",
        ),
        (
            2,
            44,
            ErrorDetail::FirstFailedSubscription(reference(None)),
            "BACnet error: property / not-cov-property, \
             first failed subscription ANALOG_VALUE,7 PRESENT_VALUE",
        ),
        (
            5,
            0,
            private_transfer(Some(vec![0x21, 1])),
            "BACnet error: services / other, vendor 555 service 7, \
             2 octets of error parameters",
        ),
        (
            5,
            0,
            private_transfer(None),
            "BACnet error: services / other, vendor 555 service 7",
        ),
        (
            5,
            39,
            ErrorDetail::VtSessionIdentifiers(vec![1, 4]),
            "BACnet error: services / vt-session-termination-failure, \
             VT sessions not closed: 1 4",
        ),
    ] {
        assert_eq!(
            Error::with_detail(
                ErrorClass::from_raw(class),
                ErrorCode::from_raw(code),
                Some(detail)
            )
            .to_string(),
            expected
        );
    }
    assert!(matches!(
        Error::with_detail(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_OBJECT, None),
        Error::Remote(RemoteError::Error {
            class: ErrorClass::PROPERTY,
            code: ErrorCode::UNKNOWN_OBJECT
        })
    ));
}

#[test]
fn decoding_error_display() {
    let err = Error::decoding(42, "unexpected tag");
    assert_eq!(
        err.to_string(),
        "decoding error at offset 42: unexpected tag"
    );
    assert!(matches!(
        err,
        Error::Decoding(DecodeError {
            offset: 42,
            kind: DecodingKind::InvalidEncoding,
            context: "",
            label: "unexpected tag",
            ..
        })
    ));
}

/// A label with numbers keeps its offset and kind (#1776).
#[test]
fn labelled_values_keep_their_fault() {
    let err = Error::from(
        DecodeError::new(DecodingKind::Trailing, 3, "{} trailing byte(s)").with_value(2),
    );
    assert_eq!(
        err.to_string(),
        "decoding error at offset 3: 2 trailing byte(s)"
    );
    assert_eq!(err.decode_fault(), Some((3, DecodingKind::Trailing)));
    assert_eq!(
        Error::trailing(3, "x").decode_fault(),
        Some((3, DecodingKind::Trailing))
    );
    assert_eq!(Error::buffer_too_short(1, 0).decode_fault(), None);
}

#[test]
fn syntax_faults_name_their_reject_reasons() {
    for (error, reason) in [
        (
            Error::decoding(1, "bad"),
            RejectReason::INVALID_DATA_ENCODING,
        ),
        (
            Error::out_of_range(1, "wide"),
            RejectReason::PARAMETER_OUT_OF_RANGE,
        ),
        (Error::overflow(1, "many"), RejectReason::BUFFER_OVERFLOW),
        (
            Error::decoding_kind(DecodingKind::Unsupported, 1, "DBCS"),
            RejectReason::OTHER,
        ),
        (Error::invalid_tag(1, "tag"), RejectReason::INVALID_TAG),
        (
            Error::missing(1, "gone"),
            RejectReason::MISSING_REQUIRED_PARAMETER,
        ),
        (Error::trailing(1, "more"), RejectReason::TOO_MANY_ARGUMENTS),
        (
            Error::buffer_too_short(9, 4),
            RejectReason::MISSING_REQUIRED_PARAMETER,
        ),
        (
            Error::reject(RejectReason::UNDEFINED_ENUMERATION),
            RejectReason::UNDEFINED_ENUMERATION,
        ),
    ] {
        assert_eq!(error.reject_reason(), Some(reason), "{error}");
    }
    for error in [
        Error::protocol(ErrorClass::SERVICES, ErrorCode::OTHER),
        Error::Range(RangeError::new("x")),
        Error::abort(AbortReason::BUFFER_OVERFLOW),
    ] {
        assert_eq!(error.reject_reason(), None, "{error}");
    }
    // A request's decode error becomes its Reject; any other error stays.
    assert!(matches!(
        Error::trailing(3, "more").into_request_reject(),
        Error::Remote(RemoteError::Reject { reason }) if reason == RejectReason::TOO_MANY_ARGUMENTS
    ));
    assert!(matches!(
        Error::Range(RangeError::new("x")).into_request_reject(),
        Error::Range(_)
    ));
    // The kind doesn't show in the message.
    assert_eq!(
        Error::trailing(3, "two trailing byte(s)").to_string(),
        "decoding error at offset 3: two trailing byte(s)"
    );
}

/// The alloc-only variants keep their text and take no Reject reason.
#[cfg(feature = "alloc")]
#[test]
fn owned_variants_take_no_reject_reason() {
    for error in [Error::Encoding("x".into()), Error::OutOfRange("x".into())] {
        assert_eq!(error.reject_reason(), None, "{error}");
    }
    assert_eq!(
        Error::trailing(3, "more")
            .into_request_reject()
            .reject_reason(),
        Some(RejectReason::TOO_MANY_ARGUMENTS)
    );
    assert!(matches!(
        Error::OutOfRange("x".into()).into_request_reject(),
        Error::OutOfRange(_)
    ));
}

/// The static encoder and segmenter refusals take no Reject reason either
/// (#1802), with or without `alloc`.
#[test]
fn static_refusals_take_no_reject_reason() {
    for error in [
        Error::encode("x"),
        Error::Segmentation(SegmentationError::ZeroSegmentSize),
    ] {
        assert_eq!(error.reject_reason(), None, "{error:?}");
        assert!(!matches!(
            error.into_request_reject(),
            Error::Remote(RemoteError::Reject { .. })
        ));
    }
}

#[test]
fn range_error_display() {
    let err = Error::from(RangeError::new("instance number {} exceeds max 4194303").with_value(5));
    assert_eq!(
        err.to_string(),
        "value out of range: instance number 5 exceeds max 4194303"
    );
}

#[test]
fn buffer_too_short_display() {
    let err = Error::buffer_too_short(10, 3);
    assert!(err.to_string().contains("need 10"));
    assert!(err.to_string().contains("have 3"));
}

#[test]
fn timeout_error_display() {
    let err = Error::Timeout(Duration::from_secs(3));
    assert!(err.to_string().contains("3s"));
}

#[test]
fn log_not_advancing_display() {
    assert_eq!(
        Error::LogNotAdvancing {
            requested: 7,
            returned: Some(3)
        }
        .to_string(),
        "log read is not advancing: asked for sequence number 7, the device answered from 3"
    );
    assert_eq!(
        Error::LogNotAdvancing {
            requested: 7,
            returned: None
        }
        .to_string(),
        "log read is not advancing: asked for sequence number 7, which the log's counts \
         say it holds, the device answered with no records"
    );
}

#[test]
fn send_too_long_keeps_the_observed_lengths_without_becoming_an_apdu_reject() {
    let error = Error::SendTooLong {
        length: 1500,
        limit: 480,
    };
    assert_eq!(
        error.to_string(),
        "send refused before transmission: 1500 octets exceed the 480-octet limit"
    );
    assert_eq!(error.reject_reason(), None);
    assert!(matches!(
        error.into_request_reject(),
        Error::SendTooLong {
            length: 1500,
            limit: 480
        }
    ));
}

#[test]
fn routed_path_too_long_display_preserves_dnet() {
    let err = Error::RoutedPathTooLong { dnet: 1234 };
    assert!(err.to_string().contains("1234"));
}

#[test]
fn routed_path_capacity_display_preserves_bound() {
    let err = Error::RoutedPathCapacityExceeded { capacity: 256 };
    assert!(err.to_string().contains("256"));
}

#[test]
fn unsupported_transport_display_names_both_data_links() {
    let err = Error::UnsupportedTransport {
        required: DataLink::Bip,
        actual: DataLink::Mstp,
    };
    assert_eq!(
        err.to_string(),
        "operation requires BACnet/IP; this transport is MS/TP"
    );
}
