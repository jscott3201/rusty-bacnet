//! ReadPropertyMultiple through the server and the `Responder`: the same
//! reply octets for ALL, REQUIRED and OPTIONAL on every object, for error
//! rows inside the ACK, and for the refusals and both limits.
use super::*;

/// An object and the references an RPM reads of it.
type Spec<'a> = (ObjectIdentifier, &'a [(P, Option<u32>)]);

/// A ReadPropertyMultiple body of `specs`, each an object and its
/// references.
fn rpm_body(specs: &[Spec<'_>]) -> BytesMut {
    let request = bacnet_services::rpm::ReadPropertyMultipleRequest {
        list_of_read_access_specs: specs
            .iter()
            .map(|(object, references)| ReadAccessSpecification {
                object_identifier: *object,
                list_of_property_references: references
                    .iter()
                    .map(
                        |&(property_identifier, property_array_index)| PropertyReference {
                            property_identifier,
                            property_array_index,
                        },
                    )
                    .collect(),
            })
            .collect(),
    };
    let mut body = BytesMut::new();
    request.encode(&mut body).unwrap();
    body
}

fn rpm(invoke_id: u8, specs: &[Spec<'_>]) -> Bytes {
    let body = rpm_body(specs);
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &body,
        1476,
    )
}

fn is_rpm_ack(reply: &Option<Apdu>) -> bool {
    matches!(reply, Some(Apdu::ComplexAck(ack))
        if ack.service_choice == ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE)
}

fn abort_reason(reply: &Option<Apdu>) -> Option<AbortReason> {
    match reply {
        Some(Apdu::Abort(abort)) if abort.sent_by_server => Some(abort.abort_reason),
        _ => None,
    }
}

/// ALL, REQUIRED and OPTIONAL on each object of the fixture, the Device
/// included, one request each; then all three on every object in one
/// request, which is longer than the requester takes, so both abort it.
#[tokio::test(start_paused = true)]
async fn rpm_all_required_and_optional_match() {
    let mut parity = Parity::start().await;
    let objects: Vec<ObjectIdentifier> = parity.db.iter_objects().map(|(oid, _)| oid).collect();
    assert_eq!(objects.len(), 5, "{objects:?}");
    let mut n = 0u8;
    for &object in &objects {
        for selector in [P::ALL, P::REQUIRED, P::OPTIONAL] {
            n += 1;
            let apdu = rpm(n, &[(object, &[(selector, None)])]);
            let what = format!("RPM {selector:?} of {object:?}");
            let reply = parity.same(&what, apdu, 1476).await;
            assert!(is_rpm_ack(&reply), "{what}: {reply:?}");
        }
    }
    let every = [(P::ALL, None), (P::REQUIRED, None), (P::OPTIONAL, None)];
    let specs: Vec<_> = objects.iter().map(|&object| (object, &every[..])).collect();
    let reply = parity
        .same("RPM of everything", rpm(n + 1, &specs), 1476)
        .await;
    assert_eq!(
        abort_reason(&reply),
        Some(AbortReason::SEGMENTATION_NOT_SUPPORTED),
        "{reply:?}"
    );
    parity.server.stop().await.unwrap();
}

/// Errors an RPM reports inside its ACK: an unknown object and property, an
/// index on a non-array; beside them a state text by index, a Group's
/// Present_Value and both wildcard instances.
#[tokio::test(start_paused = true)]
async fn rpm_error_rows_and_wildcards_match() {
    let mut parity = Parity::start().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let apdu = rpm(
        1,
        &[
            (
                ai,
                &[
                    (P::PRESENT_VALUE, None),
                    (P::PRESENT_VALUE, Some(1)),
                    (P::from_raw(9999), None),
                ],
            ),
            (
                oid(ObjectType::ANALOG_INPUT, 99),
                &[(P::PRESENT_VALUE, None)],
            ),
            (
                oid(ObjectType::MULTI_STATE_VALUE, 1),
                &[(P::STATE_TEXT, Some(2)), (P::STATE_TEXT, Some(9))],
            ),
            (oid(ObjectType::GROUP, 1), &[(P::PRESENT_VALUE, None)]),
            (
                oid(ObjectType::DEVICE, 4_194_303),
                &[(P::OBJECT_IDENTIFIER, None)],
            ),
            (
                oid(ObjectType::NETWORK_PORT, 4_194_303),
                &[(P::OBJECT_NAME, None)],
            ),
        ],
    );
    let reply = parity.same("RPM error rows", apdu, 1476).await;
    let Some(Apdu::ComplexAck(ack)) = reply else {
        panic!("expected a ComplexACK, got {reply:?}");
    };
    let ack = bacnet_services::rpm::ReadPropertyMultipleACK::decode(&ack.service_ack).unwrap();
    let errors: Vec<_> = ack
        .list_of_read_access_results
        .iter()
        .flat_map(|result| &result.list_of_results)
        .filter_map(|row| row.error)
        .collect();
    assert_eq!(
        errors,
        [
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
            (ErrorClass::PROPERTY, ErrorCode::INVALID_ARRAY_INDEX),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ]
    );
    parity.server.stop().await.unwrap();
}

/// Malformed parameters are rejected, and a segmented request is aborted.
#[tokio::test(start_paused = true)]
async fn rpm_refusals_match() {
    let mut parity = Parity::start().await;
    // An object identifier, then no list of references.
    let malformed = confirmed(
        1,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &[0x0C, 0x00, 0x00, 0x00, 0x01],
        1476,
    );
    let reply = parity.same("malformed RPM", malformed, 1476).await;
    assert!(matches!(reply, Some(Apdu::Reject(_))), "{reply:?}");

    let body = rpm_body(&[(
        oid(ObjectType::ANALOG_INPUT, 1),
        &[(P::PRESENT_VALUE, None)],
    )]);
    let segmented = confirmed_with(
        2,
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
        &body,
        1476,
        true,
        false,
    );
    let reply = parity.same("segmented RPM", segmented, 1476).await;
    assert_eq!(
        abort_reason(&reply),
        Some(AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    parity.server.stop().await.unwrap();
}

/// The work limit: three rows, or a Group's Present_Value with its two
/// member rows, past a limit of two draw OUT_OF_RESOURCES on both sides;
/// two rows fit it.
#[tokio::test(start_paused = true)]
async fn rpm_work_limit_matches() {
    let budget = ReadPropertyMultipleBudget {
        max_result_elements: 2,
        ..ReadPropertyMultipleBudget::default()
    };
    let mut parity = Parity::start_with(budget, Segmentation::NO_SEGMENTATION).await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let three = [
        (P::PRESENT_VALUE, None),
        (P::OBJECT_NAME, None),
        (P::UNITS, None),
    ];
    for (what, apdu) in [
        ("RPM past the work limit", rpm(1, &[(ai, &three[..])])),
        (
            "RPM of a Group past the work limit",
            rpm(
                2,
                &[(oid(ObjectType::GROUP, 1), &[(P::PRESENT_VALUE, None)])],
            ),
        ),
    ] {
        let reply = parity.same(what, apdu, 1476).await;
        assert_eq!(
            abort_reason(&reply),
            Some(AbortReason::OUT_OF_RESOURCES),
            "{what}"
        );
    }
    let reply = parity
        .same("RPM at the work limit", rpm(3, &[(ai, &three[..2])]), 1476)
        .await;
    assert!(is_rpm_ack(&reply), "{reply:?}");
    parity.server.stop().await.unwrap();
}

/// The byte limit: an ACK past it draws BUFFER_OVERFLOW on both sides,
/// after the reads; one that fits is sent.
#[tokio::test(start_paused = true)]
async fn rpm_byte_limit_matches() {
    let budget = ReadPropertyMultipleBudget {
        max_service_ack_bytes: 40,
        ..ReadPropertyMultipleBudget::default()
    };
    let mut parity = Parity::start_with(budget, Segmentation::NO_SEGMENTATION).await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let reply = parity
        .same(
            "RPM past the byte limit",
            rpm(1, &[(ai, &[(P::ALL, None)])]),
            1476,
        )
        .await;
    assert_eq!(
        abort_reason(&reply),
        Some(AbortReason::BUFFER_OVERFLOW),
        "{reply:?}"
    );
    let reply = parity
        .same(
            "RPM within the byte limit",
            rpm(2, &[(ai, &[(P::PRESENT_VALUE, None)])]),
            1476,
        )
        .await;
    assert!(is_rpm_ack(&reply), "{reply:?}");
    parity.server.stop().await.unwrap();
}
