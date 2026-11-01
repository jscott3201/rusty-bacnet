//! WritePropertyMultiple through the server and the `Responder`: the same
//! reply octets for each row, and the same values read back after it,
//! including what a failed request kept of its successful prefix.
use super::write::{bv, encoded};
use super::*;
use bacnet_services::wpm::{
    WriteAccessSpecification, WritePropertyMultipleError, WritePropertyMultipleRequest,
};
use bacnet_types::constructed::BACnetPropertyValue;

/// A WritePropertyMultiple body: each (object, property, value) a
/// specification of its own, with no priority.
fn wpm_body(writes: &[(ObjectIdentifier, P, PropertyValue)]) -> BytesMut {
    let request = WritePropertyMultipleRequest {
        list_of_write_access_specs: writes
            .iter()
            .map(|(object, property, value)| WriteAccessSpecification {
                object_identifier: *object,
                list_of_properties: vec![BACnetPropertyValue {
                    property_identifier: *property,
                    property_array_index: None,
                    value: encoded(value),
                    priority: None,
                }],
            })
            .collect(),
    };
    let mut body = BytesMut::new();
    request.encode(&mut body).unwrap();
    body
}

fn wpm(invoke_id: u8, body: &[u8]) -> Bytes {
    confirmed(
        invoke_id,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        body,
        1476,
    )
}

/// The WritePropertyMultiple Error's class and code, and the object and
/// property of the attempt it names.
fn wpm_error_of(reply: Option<Apdu>) -> (ErrorClass, ErrorCode, ObjectIdentifier, u32) {
    let Some(Apdu::Error(error)) = reply else {
        panic!("expected an Error PDU, got {reply:?}");
    };
    let error = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
    let attempt = error.first_failed_write_attempt;
    (
        error.error_class,
        error.error_code,
        attempt.object_identifier,
        attempt.property_identifier,
    )
}

fn is_wpm_ack(reply: &Option<Apdu>) -> bool {
    matches!(reply, Some(Apdu::SimpleAck(ack))
        if ack.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE)
}

#[tokio::test(start_paused = true)]
async fn wpm_success_and_refusals_match() {
    let mut parity = Parity::start_writes().await;
    let active = PropertyValue::Enumerated(1);
    let described = PropertyValue::CharacterString("written".into());

    let body = wpm_body(&[
        (bv(2), P::PRESENT_VALUE, active.clone()),
        (bv(2), P::DESCRIPTION, described.clone()),
    ]);
    let reply = parity
        .same("a WPM that writes everything", wpm(1, &body), 1476)
        .await;
    assert!(is_wpm_ack(&reply), "{reply:?}");
    parity
        .read_back("BV-2 after the WPM", bv(2), P::PRESENT_VALUE)
        .await;
    parity
        .read_back("BV-2's Description after the WPM", bv(2), P::DESCRIPTION)
        .await;

    // BV-1's Present_Value is commandable: both command it for the
    // request's source, at priority 16 (#2209).
    let body = wpm_body(&[(bv(1), P::PRESENT_VALUE, active.clone())]);
    let reply = parity
        .same("a WPM commanding a Present_Value", wpm(5, &body), 1476)
        .await;
    assert!(is_wpm_ack(&reply), "{reply:?}");
    for property in [P::PRESENT_VALUE, P::PRIORITY_ARRAY, P::VALUE_SOURCE] {
        parity
            .read_back("BV-1 after the WPM", bv(1), property)
            .await;
    }

    let body = wpm_body(&[(bv(9), P::PRESENT_VALUE, active.clone())]);
    let reply = parity
        .same("a WPM of an unknown object", wpm(2, &body), 1476)
        .await;
    assert_eq!(
        wpm_error_of(reply),
        (
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
            bv(9),
            P::PRESENT_VALUE.to_raw()
        )
    );

    // An object identifier, then no list of values.
    let malformed = wpm(3, &[0x0C, 0x01, 0x40, 0x00, 0x02]);
    let reply = parity.same("a malformed WPM", malformed, 1476).await;
    assert!(matches!(reply, Some(Apdu::Reject(_))), "{reply:?}");

    let body = wpm_body(&[(bv(2), P::PRESENT_VALUE, PropertyValue::Enumerated(0))]);
    let segmented = confirmed_with(
        4,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        &body,
        1476,
        true,
        false,
    );
    let reply = parity.same("a segmented WPM", segmented, 1476).await;
    assert!(matches!(reply, Some(Apdu::Abort(_))), "{reply:?}");
    // Neither refusal wrote anything.
    parity
        .read_back("BV-2 after the refusals", bv(2), P::PRESENT_VALUE)
        .await;
    parity.server.stop().await.unwrap();
}

/// A WPM whose first attempt succeeds and whose second fails: refused by
/// the object, or cut off inside its value. Both sides keep the first
/// write. The syntax fault comes after the request changed the device, so
/// it is an Error, SERVICES / INVALID_TAG, not a Reject (Clauses 15.10 and
/// 20.1.8).
#[tokio::test(start_paused = true)]
async fn wpm_applied_prefix_then_failure_matches() {
    let mut parity = Parity::start_writes().await;
    let ai = oid(ObjectType::ANALOG_INPUT, 1);

    let body = wpm_body(&[
        (bv(2), P::PRESENT_VALUE, PropertyValue::Enumerated(1)),
        (ai, P::PRESENT_VALUE, PropertyValue::Real(5.0)),
    ]);
    let reply = parity
        .same("a WPM refused after a written prefix", wpm(1, &body), 1476)
        .await;
    assert_eq!(
        wpm_error_of(reply),
        (
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
            ai,
            P::PRESENT_VALUE.to_raw()
        )
    );
    let kept = parity
        .read_back("the prefix a refusal kept", bv(2), P::PRESENT_VALUE)
        .await;

    let first = wpm_body(&[(
        bv(2),
        P::DESCRIPTION,
        PropertyValue::CharacterString("one".into()),
    )]);
    let second = wpm_body(&[(bv(2), P::PRESENT_VALUE, PropertyValue::Enumerated(0))]);
    let mut torn = first.to_vec();
    torn.extend_from_slice(&second[..second.len() - 2]);
    let reply = parity
        .same("a WPM cut off after a written prefix", wpm(2, &torn), 1476)
        .await;
    let (class, code, object, _) = wpm_error_of(reply);
    assert_eq!(
        (class, code, object),
        (ErrorClass::SERVICES, ErrorCode::INVALID_TAG, bv(2))
    );
    parity
        .read_back(
            "the Description the syntax fault kept",
            bv(2),
            P::DESCRIPTION,
        )
        .await;
    // The cut-off attempt wrote nothing: BV-2 is still active.
    let after = parity
        .read_back("BV-2 after the syntax fault", bv(2), P::PRESENT_VALUE)
        .await;
    assert_eq!(after, kept);

    // The same tear with nothing written before it is a Reject.
    let reply = parity
        .same(
            "a WPM cut off before any write",
            wpm(3, &second[..second.len() - 2]),
            1476,
        )
        .await;
    assert!(matches!(reply, Some(Apdu::Reject(_))), "{reply:?}");
    parity.server.stop().await.unwrap();
}
