//! What a peer receives for a confirmed request whose contents stop before
//! a member's header says they should (#1303, #1304, #1374), at the top
//! level or inside a constructed frame (#1333).
//!
//! The decoders behind these services report such a member as a short
//! buffer: the data ran out before an argument was complete. The server
//! rejects it as MISSING_REQUIRED_PARAMETER, as it does a request that
//! stops before a member starts (#1446), whatever the service: CreateObject,
//! the list services and SubscribeCOVPropertyMultiple, which answer other
//! refusals with their formal error, included. AV-1 is the Harness's Analog
//! Value.
use super::*;
use crate::server::cov_wire_test_support::Harness;
use bacnet_types::error::DecodingKind;

/// AV-1's object identifier as a primitive `[0]`.
const AV_1: [u8; 5] = [0x0C, 0x00, 0x80, 0x00, 0x01];

/// Whether `apdu` answers the request sent with `invoke_id`.
fn answers(apdu: &Apdu, invoke_id: u8) -> bool {
    match apdu {
        Apdu::SimpleAck(ack) => ack.invoke_id == invoke_id,
        Apdu::ComplexAck(ack) => ack.invoke_id == invoke_id,
        Apdu::Error(error) => error.invoke_id == invoke_id,
        Apdu::Reject(reject) => reject.invoke_id == invoke_id,
        Apdu::Abort(abort) => abort.invoke_id == invoke_id,
        _ => false,
    }
}

/// Send `body` as one confirmed `service` request and return the APDU
/// answering it.
pub(super) async fn answer_to(
    h: &mut Harness,
    service: ConfirmedServiceChoice,
    body: &[u8],
) -> Apdu {
    h.request(service, BytesMut::from(body)).await;
    let invoke_id = h.invoke_id;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let found = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames.iter().position(|apdu| answers(apdu, invoke_id));
                at.map(|at| frames.remove(at))
            };
            match found {
                Some(apdu) => return apdu,
                None => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    })
    .await
    .expect("an answer to the request")
}

/// Send `body` as one confirmed `service` request and require a Reject with
/// `reason` in answer.
pub(super) async fn reject_for(
    h: &mut Harness,
    service: ConfirmedServiceChoice,
    body: &[u8],
    reason: RejectReason,
) {
    reject_case(h, service, "", body, reason).await;
}

/// [`reject_for`] for the case `what`, which a failure names.
pub(super) async fn reject_case(
    h: &mut Harness,
    service: ConfirmedServiceChoice,
    what: &str,
    body: &[u8],
    reason: RejectReason,
) {
    match answer_to(h, service, body).await {
        Apdu::Reject(reject) => {
            assert_eq!(
                reject.reject_reason, reason,
                "{service:?} {what} {body:02X?}"
            );
        }
        other => panic!("{service:?} {what} {body:02X?} drew {other:?}"),
    }
}

/// Send `body` and require the MISSING_REQUIRED_PARAMETER Reject a request
/// cut short draws.
async fn cut_short(h: &mut Harness, service: ConfirmedServiceChoice, body: &[u8]) {
    reject_for(h, service, body, RejectReason::MISSING_REQUIRED_PARAMETER).await;
}

#[tokio::test(start_paused = true)]
async fn read_property_multiple_cut_short_is_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let service = ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE;
    // The object identifier with three of its four octets, then AV-1 with a
    // property identifier that says two octets and holds one.
    let reference_cut = [&AV_1[..], &[0x1E, 0x0A, 0x55]].concat();
    for body in [&AV_1[..4], &reference_cut] {
        cut_short(&mut h, service, body).await;
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn subscribe_cov_property_cut_short_is_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    // Process 1 and AV-1 as `[1]`, then the `[4]` property reference cut
    // short.
    let body = [0x09, 0x01, 0x1C, 0x00, 0x80, 0x00, 0x01, 0x4E, 0x0A, 0x55];
    cut_short(
        &mut h,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
        &body,
    )
    .await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn subscribe_cov_property_multiple_cut_short_is_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    // Process 1, unconfirmed, then one specification for AV-1 whose first
    // `[0]` property reference is cut short.
    let body = [
        &[0x09, 0x01, 0x19, 0x00, 0x4E][..],
        &AV_1,
        &[0x1E, 0x0E, 0x0A, 0x55],
    ]
    .concat();
    let service = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
    cut_short(&mut h, service, &body).await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn confirmed_audit_notification_cut_short_or_misshapen_is_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let service = ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION;
    // `[0]` around one notification: source-device Device 1 in `[2]`, then a
    // source-object `[3]` of three octets, or of four with two present.
    let device = [0x2E, 0x0C, 0x02, 0x00, 0x00, 0x01, 0x2F];
    let three_octets = [&[0x0E][..], &device, &[0x3B, 0x00, 0x80, 0x00, 0x0F]].concat();
    let cut = [&[0x0E][..], &device, &[0x3C, 0x00, 0x80]].concat();
    // A wrong length is an encoding not valid for the datatype.
    reject_for(
        &mut h,
        service,
        &three_octets,
        RejectReason::INVALID_DATA_ENCODING,
    )
    .await;
    cut_short(&mut h, service, &cut).await;
    h.server.stop().await.unwrap();
}

/// A request's decode error, turned into its Reject where the handler
/// decodes the request, draws the Reject naming the fault, whatever the
/// service, the formal-error ones included (#1446). The same error met once
/// the service is running is no fault of the request's syntax, so it keeps
/// the Error PDU the service answers other refusals with.
#[test]
fn request_syntax_faults_draw_the_reject_naming_them_for_every_service() {
    let reply = |service, error: &Error| bacnet_device_core::reply::error_apdu(7, service, error);
    for raw in 0..=u8::MAX {
        let service = ConfirmedServiceChoice::from_raw(raw);
        for (error, reason) in [
            (
                Error::decoding(3, "malformed"),
                RejectReason::INVALID_DATA_ENCODING,
            ),
            (
                Error::out_of_range(3, "wide"),
                RejectReason::PARAMETER_OUT_OF_RANGE,
            ),
            (Error::overflow(3, "many"), RejectReason::BUFFER_OVERFLOW),
            (
                Error::decoding_kind(DecodingKind::Unsupported, 3, "DBCS"),
                RejectReason::OTHER,
            ),
            (Error::invalid_tag(3, "tag"), RejectReason::INVALID_TAG),
            (
                Error::missing(3, "missing"),
                RejectReason::MISSING_REQUIRED_PARAMETER,
            ),
            (Error::trailing(3, "more"), RejectReason::TOO_MANY_ARGUMENTS),
            (
                Error::buffer_too_short(9, 4),
                RejectReason::MISSING_REQUIRED_PARAMETER,
            ),
        ] {
            let what = format!("{service:?} {error}");
            assert!(matches!(reply(service, &error), Apdu::Error(_)), "{what}");
            assert_eq!(
                reply(service, &error.into_request_reject()),
                Apdu::Reject(RejectPdu {
                    invoke_id: 7,
                    reject_reason: reason,
                }),
                "{what}"
            );
        }
    }
    // A refusal that isn't about syntax keeps its Error PDU.
    let refusal = Error::OutOfRange("too large".into()).into_request_reject();
    let service = ConfirmedServiceChoice::READ_PROPERTY;
    assert!(matches!(reply(service, &refusal), Apdu::Error(_)));
}

/// A `[0]` Unsigned announcing two contents octets and holding one.
const PROCESS_CUT: [u8; 2] = [0x0A, 0x01];

#[tokio::test(start_paused = true)]
async fn service_parameters_cut_short_are_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    // AV-1, then a `[1]` property identifier cut the same way; and a `[0]`
    // object identifier with two of its four octets.
    let property_cut = [&AV_1[..], &[0x1A, 0x55]].concat();
    let object_cut = [0x0C, 0x00, 0x80];
    let cases: [(ConfirmedServiceChoice, &[u8]); 10] = [
        (ConfirmedServiceChoice::READ_PROPERTY, &property_cut),
        (ConfirmedServiceChoice::WRITE_PROPERTY, &property_cut),
        (ConfirmedServiceChoice::READ_RANGE, &object_cut),
        (ConfirmedServiceChoice::SUBSCRIBE_COV, &PROCESS_CUT),
        (ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY, &PROCESS_CUT),
        (
            ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
            &PROCESS_CUT,
        ),
        (ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, &PROCESS_CUT),
        (ConfirmedServiceChoice::GET_EVENT_INFORMATION, &object_cut),
        (ConfirmedServiceChoice::LIFE_SAFETY_OPERATION, &PROCESS_CUT),
        (ConfirmedServiceChoice::AUDIT_LOG_QUERY, &object_cut),
    ];
    for (service, body) in cases {
        cut_short(&mut h, service, body).await;
    }
    h.server.stop().await.unwrap();
}

/// The services whose members moved onto the shared readers with #1374, each
/// cut short in the member named.
#[tokio::test(start_paused = true)]
async fn inline_members_cut_short_are_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let object_cut = [0x0C, 0x00, 0x80];
    let cases: [(ConfirmedServiceChoice, &str, &[u8]); 9] = [
        (
            ConfirmedServiceChoice::READ_PROPERTY,
            "[0] object",
            &object_cut,
        ),
        (
            ConfirmedServiceChoice::WRITE_PROPERTY,
            "[0] object",
            &object_cut,
        ),
        (
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            "[1] enable-disable",
            &[0x09, 0x05, 0x1A, 0x00],
        ),
        (
            ConfirmedServiceChoice::REINITIALIZE_DEVICE,
            "[0] state",
            &[0x0A, 0x00],
        ),
        (
            ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE,
            "[0] source device",
            &[0x0C, 0x02, 0x00],
        ),
        (
            ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
            "[0] acknowledgment filter",
            &[0x0A, 0x00],
        ),
        (
            ConfirmedServiceChoice::DELETE_OBJECT,
            "object",
            &[0xC4, 0x00, 0x80],
        ),
        (
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            "file",
            &[0xC4, 0x02, 0x80],
        ),
        (
            ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            "file",
            &[0xC4, 0x02, 0x80],
        ),
    ];
    for (service, member, body) in cases {
        match answer_to(&mut h, service, body).await {
            Apdu::Reject(reject) => assert_eq!(
                reject.reject_reason,
                RejectReason::MISSING_REQUIRED_PARAMETER,
                "{service:?} {member}"
            ),
            other => panic!("{service:?} {member} drew {other:?}"),
        }
    }
    h.server.stop().await.unwrap();
}

/// A member cut short inside a constructed frame, which the decoders report
/// as a short buffer too (#1333), draws the same reply.
#[tokio::test(start_paused = true)]
async fn members_cut_short_inside_a_frame_are_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let file = [0xC4, 0x02, 0x80, 0x00, 0x01];
    let elements_cut = [&AV_1[..], &[0x19, 0x55, 0x3E, 0x22, 0x01]].concat();
    let cases: [(ConfirmedServiceChoice, &str, Vec<u8>); 9] = [
        (
            ConfirmedServiceChoice::WRITE_PROPERTY,
            "a REAL in the [3] value",
            [&AV_1[..], &[0x19, 0x55, 0x3E, 0x44, 0x42, 0x90]].concat(),
        ),
        (
            ConfirmedServiceChoice::READ_RANGE,
            "the reference index in [3] by-position",
            [&AV_1[..], &[0x19, 0x55, 0x3E, 0x22, 0x01]].concat(),
        ),
        (
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            "the octet count in [0] stream access",
            [&file[..], &[0x0E, 0x31, 0x00, 0x22, 0x01]].concat(),
        ),
        (
            ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            "the file data in [0] stream access",
            [&file[..], &[0x0E, 0x31, 0x00, 0x63, 0x01, 0x02]].concat(),
        ),
        (
            ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
            "the recipient device in the [1] enrollment filter",
            vec![0x09, 0x00, 0x1E, 0x0E, 0x0C, 0x02, 0x00],
        ),
        (
            ConfirmedServiceChoice::AUDIT_LOG_QUERY,
            "the target device in [1] by-target",
            vec![0x0C, 0x0F, 0x40, 0x00, 0x01, 0x1E, 0x0E, 0x0C, 0x02, 0x00],
        ),
        // The formal-error services: an initial value's property identifier
        // in CreateObject's [1] list, and a list element in [3].
        (
            ConfirmedServiceChoice::CREATE_OBJECT,
            "an initial value's property identifier in [1]",
            vec![0x0E, 0x09, 0x02, 0x0F, 0x1E, 0x0A, 0x00],
        ),
        (
            ConfirmedServiceChoice::ADD_LIST_ELEMENT,
            "an element in [3]",
            elements_cut.clone(),
        ),
        (
            ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
            "an element in [3]",
            elements_cut,
        ),
    ];
    for (service, member, body) in cases {
        match answer_to(&mut h, service, &body).await {
            Apdu::Reject(reject) => assert_eq!(
                reject.reject_reason,
                RejectReason::MISSING_REQUIRED_PARAMETER,
                "{service:?} {member}"
            ),
            other => panic!("{service:?} {member} drew {other:?}"),
        }
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn create_object_and_list_requests_cut_short_are_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    // A [0] object type with one of two octets inside the specifier.
    cut_short(
        &mut h,
        ConfirmedServiceChoice::CREATE_OBJECT,
        &[0x0E, 0x0A, 0x02],
    )
    .await;
    // AV-1, then a [1] property identifier with one of two octets.
    let property_cut = [&AV_1[..], &[0x1A, 0x55]].concat();
    for service in [
        ConfirmedServiceChoice::ADD_LIST_ELEMENT,
        ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
    ] {
        cut_short(&mut h, service, &property_cut).await;
    }
    h.server.stop().await.unwrap();
}
