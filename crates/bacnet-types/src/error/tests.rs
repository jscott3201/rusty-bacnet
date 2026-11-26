use super::*;

/// A remote error reads the same on its own and inside `Error::Remote`, and
/// names class, code and reason in lower case with hyphens, with a number
/// only for a value without a name (#1815, #2325).
#[test]
fn remote_error_display_names_class_code_and_reason() {
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
            "BACnet reject: undefined-enumeration",
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
            "BACnet abort: segmentation-not-supported",
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

/// An ambiguous instance names every claimant: a routed one as its station
/// on its network, and a local one by its MAC's length, in the CLI's target
/// grammar for 1, 3 or 6 octets, which bacnet-ops reads back, and as bare
/// hex for any other length, which the grammar refuses (#2560).
#[cfg(feature = "std")]
#[test]
fn ambiguous_device_names_every_candidate() {
    let at = |network_number, mac: &[u8]| BACnetAddress {
        network_number,
        mac_address: crate::MacAddr::from_slice(mac),
    };
    let error = Error::AmbiguousDevice {
        instance: 1001,
        candidates: vec![
            at(0, &[5]),
            at(0, &[0x00, 0x04, 0xD2]),
            at(0, &[10, 0, 0, 1, 0xBA, 0xC0]),
            at(0, &[1, 2]),
            at(2, &[0x05]),
        ]
        .into(),
    };
    assert_eq!(
        error.to_string(),
        "device 1001 is claimed by 5 addresses: mstp:5, bip6:0x0004D2, \
         10.0.0.1:47808, 0x0102, 2:0x05; name one by its address"
    );
}

/// An unknown device names it as the request did (#2560).
#[cfg(feature = "std")]
#[test]
fn unknown_device_names_the_device() {
    let error = Error::UnknownDevice {
        device: "2:0x05".into(),
    };
    assert_eq!(
        error.to_string(),
        "device 2:0x05 is not known to the client; discover it first"
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

/// A decoding error becomes the Reject its kind names; a range refusal,
/// which is no syntax fault, stays as it is (#1833 removed the untyped
/// text forms this pinned for).
#[test]
fn only_a_decoding_error_becomes_a_reject() {
    assert_eq!(
        Error::trailing(3, "more")
            .into_request_reject()
            .reject_reason(),
        Some(RejectReason::TOO_MANY_ARGUMENTS)
    );
    assert!(matches!(
        Error::range("x").into_request_reject(),
        Error::Range(_)
    ));
}

/// The error is 64 octets on a 64-bit host, its tag in [`DecodeError`]'s
/// niche, with persisted state's [`PersistenceError`] beside it, its
/// context boxed so it is no wider than its static form: removing the two
/// owned-text variants (#1833) left the error as it was, since the decoding
/// error sets the size. A 32-bit target's 48 is checked in `decode.rs`.
#[cfg(target_pointer_width = "64")]
#[test]
fn the_error_is_64_octets_on_a_64_bit_host() {
    use core::mem::size_of;
    assert_eq!(size_of::<Error>(), 64);
    assert_eq!(size_of::<Result<()>>(), 64);
    assert_eq!(size_of::<PersistenceError>(), 56);
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
fn initiation_disabled_says_whether_the_request_went_out() {
    let refused = Error::InitiationDisabled { sent: false }.to_string();
    assert!(refused.contains("DeviceCommunicationControl"), "{refused}");
    assert!(refused.starts_with("not sent"), "{refused}");
    let withheld = Error::InitiationDisabled { sent: true }.to_string();
    assert!(
        withheld.contains("DeviceCommunicationControl"),
        "{withheld}"
    );
    assert!(withheld.starts_with("no retry sent"), "{withheld}");
}

/// A request the client or its link cut off says whether the device may
/// have carried it out (#1833), and no peer sent it; the text is true for a
/// link that lost its connection too.
#[test]
fn a_cut_off_request_says_whether_it_may_have_been_carried_out() {
    let unsent = Error::RequestCutOff { sent: false };
    assert_eq!(
        unsent.to_string(),
        "the request ended without an answer: the client stopped, or its link stopped or lost \
         its connection, before it was sent: the device did not get it"
    );
    let sent = Error::RequestCutOff { sent: true };
    assert_eq!(
        sent.to_string(),
        "the request ended without an answer: the client stopped, or its link stopped or lost \
         its connection, after it went out, and the device may have carried it out"
    );
    for cut_off in [unsent, sent] {
        assert_eq!(cut_off.remote(), None);
        assert_eq!(cut_off.reject_reason(), None);
    }
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

/// A network reject names the router in its link's own address form, the
/// network, and the reason by name beside its number; a reason the
/// standard doesn't name by its number alone (#2724).
#[cfg(feature = "std")]
#[test]
fn network_reject_display_names_router_network_and_reason() {
    use bacnet_npdu::{Mac, RejectMessageReason as R};
    let bip = Mac::from_array([192, 168, 1, 20, 0xBA, 0xC0]);
    let cases = [
        (
            bip,
            Some(DataLink::Bip),
            R::NOT_DIRECTLY_CONNECTED,
            "Reject-Message-To-Network from router 192.168.1.20:47808: no route to network 7 \
             (not-directly-connected, reason 1)",
        ),
        // A loopback or unnamed link writes six octets as B/IP does.
        (
            bip,
            None,
            R::ADDRESSING_ERROR,
            "Reject-Message-To-Network from router 192.168.1.20:47808: an address length it \
             found invalid, for network 7 (addressing-error, reason 6)",
        ),
        (
            bip,
            Some(DataLink::Sc),
            R::OTHER,
            "Reject-Message-To-Network from router c0:a8:01:14:ba:c0: the message for network 7 \
             was refused (other, reason 0)",
        ),
        (
            Mac::from_array([5]),
            Some(DataLink::Mstp),
            R::from_raw(5),
            "Reject-Message-To-Network from router mstp:5: a security refusal for network 7 \
             (reason 5)",
        ),
        (
            Mac::from_array([0x00, 0x04, 0xD2]),
            Some(DataLink::Bip6),
            R::ROUTER_BUSY,
            "Reject-Message-To-Network from router bip6:0x0004D2: busy for network 7 \
             (router-busy, reason 2)",
        ),
        (
            Mac::from_array([0x00, 0x04, 0xD2]),
            Some(DataLink::Bip),
            R::from_raw(200),
            "Reject-Message-To-Network from router 00:04:d2: the message for network 7 was \
             refused (reason 200)",
        ),
    ];
    for (router, link, reason, text) in cases {
        let reject = NetworkReject::new(router, link, 7, reason);
        let later = "; an earlier attempt may have reached the device";
        assert_eq!(reject.what().to_string(), text);
        assert_eq!(reject.to_string(), format!("{text}{later}"));
        assert_eq!(
            Error::NetworkRejected(Box::new(reject)).to_string(),
            format!("{text}{later}")
        );
    }
}

/// Each reason's COMMUNICATION code, as an audit record's Result carries it.
#[cfg(feature = "std")]
#[test]
fn network_reject_error_codes_follow_the_reason() {
    use bacnet_npdu::{Mac, RejectMessageReason as R};
    let code = |reason| NetworkReject::new(Mac::from_array([1]), None, 7, reason).error_code();
    assert_eq!(
        code(R::NOT_DIRECTLY_CONNECTED),
        ErrorCode::NOT_ROUTER_TO_DNET
    );
    assert_eq!(code(R::ROUTER_BUSY), ErrorCode::ROUTER_BUSY);
    assert_eq!(
        code(R::UNKNOWN_MESSAGE_TYPE),
        ErrorCode::UNKNOWN_NETWORK_MESSAGE
    );
    assert_eq!(code(R::MESSAGE_TOO_LONG), ErrorCode::MESSAGE_TOO_LONG);
    assert_eq!(code(R::from_raw(5)), ErrorCode::SECURITY_ERROR);
    assert_eq!(code(R::ADDRESSING_ERROR), ErrorCode::ADDRESSING_ERROR);
    assert_eq!(code(R::OTHER), ErrorCode::OTHER);
    assert_eq!(code(R::from_raw(9)), ErrorCode::OTHER);
}

/// An acknowledgement of the wrong kind names the service and both kinds,
/// and says the request's outcome is unknown (#2721).
#[test]
fn unexpected_ack_display_names_the_service_and_both_kinds() {
    let complex = Error::UnexpectedAck {
        service: ConfirmedServiceChoice::WRITE_PROPERTY,
        complex: true,
    };
    assert_eq!(
        complex.to_string(),
        "the device acknowledged write-property with a Complex-ACK, but that service is \
         acknowledged with a Simple-ACK, with no data; whether it carried out the request is \
         unknown"
    );
    let simple = Error::UnexpectedAck {
        service: ConfirmedServiceChoice::READ_PROPERTY,
        complex: false,
    };
    assert_eq!(
        simple.to_string(),
        "the device acknowledged read-property with a Simple-ACK, but that service is \
         acknowledged with a Complex-ACK carrying its result; whether it carried out the \
         request is unknown"
    );
}

/// A reject for a request's one frame says the device never received it;
/// one relayed from a farther router names that router beside the one it
/// came through; a busy reject a request then timed out after reads as the
/// timeout it is, naming the router (#2724).
#[cfg(feature = "std")]
#[test]
fn network_reject_says_whether_the_device_can_have_the_request() {
    use bacnet_npdu::{Mac, RejectMessageReason as R};
    let reject = NetworkReject::new(
        Mac::from_array([2]),
        Some(DataLink::Mstp),
        7,
        R::NOT_DIRECTLY_CONNECTED,
    );
    assert!(!reject.single_frame);
    let once = reject.after_one_frame();
    assert!(once.single_frame);
    assert_eq!(
        once.to_string(),
        "Reject-Message-To-Network from router mstp:2: no route to network 7 \
         (not-directly-connected, reason 1); the request never reached the device"
    );
    let relayed = once.relayed_from(5, Mac::from_array([0x0A]));
    assert_eq!(relayed.refused_by, Some((5, Mac::from_array([0x0A]))));
    assert_eq!(
        relayed.what().to_string(),
        "Reject-Message-To-Network through router mstp:2, from 5:0x0A: no route to network 7 \
         (not-directly-connected, reason 1)"
    );
    let busy = NetworkReject::new(
        Mac::from_array([2]),
        Some(DataLink::Mstp),
        7,
        R::ROUTER_BUSY,
    );
    assert_eq!(
        Error::RouterBusyTimeout(Box::new(busy)).to_string(),
        "BACnet abort: tsm-timeout; on the way, Reject-Message-To-Network from router mstp:2: \
         busy for network 7 (router-busy, reason 2)"
    );
}
