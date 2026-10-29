//! What a peer receives for a request with octets after its last member
//! (#1411).
//!
//! The decoders refuse such a request, and the server rejects a confirmed
//! one as TOO_MANY_ARGUMENTS (#1446), with nothing read, written, deleted or
//! changed. A member under a tag the decoder can't take where a member is
//! due is INVALID_TAG instead. A GetAlarmSummary has no parameters at all,
//! so any octet in it is trailing. A confirmed request is refused before its
//! password or the server's policy is looked at. An unconfirmed request
//! with trailing octets is dropped, since nothing can answer it: a Who-Is
//! gets no I-Am, a Who-Has no I-Have, and an I-Am binds no device.
use super::*;
use crate::server::cov_wire_test_support::{Harness, av1};
use crate::server::mistagged_request_wire_tests::{
    FILE_1, FILE_2, READ, WRITE, contents, harness, request,
};
use crate::server::truncated_request_wire_tests::{answer_to, reject_case, reject_for};
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_types::enums::{EnableDisable, ReinitializedState};

/// AV-1 as an application object identifier.
const AV_1: [u8; 5] = [0xC4, 0x00, 0x80, 0x00, 0x01];
/// "secret" as an application CharacterString, where the services want it
/// under a context tag.
const SECRET_APPLICATION: [u8; 9] = [0x75, 0x07, 0x00, b's', b'e', b'c', b'r', b'e', b't'];

#[tokio::test(start_paused = true)]
async fn file_requests_with_trailing_octets_are_rejected() {
    let mut h = harness().await;
    // The well-formed requests these extend are served (see
    // mistagged_file_requests_are_rejected); the writes below carry
    // 0x42, which neither file holds, so any of them carried out would show.
    let before = contents(&h).await;
    let cases: [(ConfirmedServiceChoice, &str, Vec<u8>); 9] = [
        (
            READ,
            "an octet after the stream frame",
            request(FILE_1, &[0x0E, 0x31, 0x05, 0x21, 0x10, 0x0F, 0x00]),
        ),
        (
            READ,
            "a member after the octet count",
            request(FILE_1, &[0x0E, 0x31, 0x05, 0x21, 0x10, 0x21, 0x01, 0x0F]),
        ),
        (
            READ,
            "an octet after the record frame",
            request(FILE_2, &[0x1E, 0x31, 0x00, 0x21, 0x02, 0x1F, 0x00]),
        ),
        (
            READ,
            "a member after the record count",
            request(FILE_2, &[0x1E, 0x31, 0x00, 0x21, 0x02, 0x21, 0x01, 0x1F]),
        ),
        (
            WRITE,
            "an octet after the stream frame",
            request(FILE_1, &[0x0E, 0x31, 0x05, 0x62, 0x00, 0x42, 0x0F, 0x00]),
        ),
        (
            WRITE,
            "a member after the file data",
            request(
                FILE_1,
                &[0x0E, 0x31, 0x05, 0x62, 0x00, 0x42, 0x21, 0x01, 0x0F],
            ),
        ),
        (
            WRITE,
            "a second stream frame",
            request(
                FILE_1,
                &[
                    0x0E, 0x31, 0x05, 0x62, 0x00, 0x42, 0x0F, 0x0E, 0x31, 0x07, 0x62, 0x00, 0x42,
                    0x0F,
                ],
            ),
        ),
        (
            WRITE,
            "an octet after the record frame",
            request(
                FILE_2,
                &[0x1E, 0x31, 0x00, 0x21, 0x01, 0x62, 0x00, 0x42, 0x1F, 0x00],
            ),
        ),
        (
            WRITE,
            "a record frame after the stream frame",
            request(
                FILE_1,
                &[
                    0x0E, 0x31, 0x05, 0x62, 0x00, 0x42, 0x0F, 0x1E, 0x31, 0x00, 0x21, 0x00, 0x1F,
                ],
            ),
        ),
    ];
    for (service, what, body) in &cases {
        reject_case(
            &mut h,
            *service,
            what,
            body,
            RejectReason::TOO_MANY_ARGUMENTS,
        )
        .await;
    }
    assert_eq!(contents(&h).await, before);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn delete_object_with_trailing_octets_is_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let delete = ConfirmedServiceChoice::DELETE_OBJECT;
    for (what, tail) in [
        ("an octet", &[0x00][..]),
        ("a second identifier", &AV_1[..]),
    ] {
        reject_case(
            &mut h,
            delete,
            what,
            &[&AV_1[..], tail].concat(),
            RejectReason::TOO_MANY_ARGUMENTS,
        )
        .await;
        assert!(
            h.server.database().read().await.get(&av1()).is_some(),
            "{what}"
        );
    }
    let answer = answer_to(&mut h, delete, &AV_1).await;
    assert!(matches!(answer, Apdu::SimpleAck(_)), "{answer:?}");
    assert!(h.server.database().read().await.get(&av1()).is_none());
    h.server.stop().await.unwrap();
}

/// A DeviceCommunicationControl body with password "secret", then `tail`.
fn dcc(mode: EnableDisable, minutes: Option<u16>, tail: &[u8]) -> Vec<u8> {
    let mut body = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: minutes,
        enable_disable: mode,
        password: Some("secret".into()),
    }
    .encode(&mut body)
    .unwrap();
    body.extend_from_slice(tail);
    body.to_vec()
}

/// The server's DCC state and whether a DCC timer is running.
async fn dcc_state(h: &Harness) -> (DccState, bool) {
    (
        h.server.comm_state(),
        !h.server.dcc_timer.lock().await.is_none(),
    )
}

#[tokio::test(start_paused = true)]
async fn dcc_with_trailing_octets_is_rejected_and_changes_nothing() {
    let mut h = Harness::start(ServerConfig {
        dcc_policy: DccPolicy::RequirePassword,
        dcc_password: Some("secret".into()),
        ..Default::default()
    })
    .await;
    let service = ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL;
    let restrict = EnableDisable::DISABLE_INITIATION;
    let (enabled, restricted) = (DccState::Enable, DccState::DisableInitiation);
    // Five minutes of DISABLE_INITIATION with the right password, then an
    // octet or a `[3]`; and the password under its application tag, which
    // leaves it unread.
    let cases: [(&str, Vec<u8>); 3] = [
        (
            "an octet after the password",
            dcc(restrict, Some(5), &[0x00]),
        ),
        (
            "a [3] after the password",
            dcc(restrict, Some(5), &[0x39, 0x07]),
        ),
        (
            "the password as an application CharacterString",
            [&[0x09, 0x05, 0x19, 0x02][..], &SECRET_APPLICATION].concat(),
        ),
    ];
    for (what, body) in &cases {
        reject_case(
            &mut h,
            service,
            what,
            body,
            RejectReason::TOO_MANY_ARGUMENTS,
        )
        .await;
        assert_eq!(dcc_state(&h).await, (enabled, false), "{what}");
    }
    let answer = answer_to(&mut h, service, &dcc(restrict, Some(5), &[])).await;
    assert!(matches!(answer, Apdu::SimpleAck(_)), "{answer:?}");
    assert_eq!(dcc_state(&h).await, (restricted, true));
    // An ENABLE with trailing octets leaves initiation restricted too.
    let enable = EnableDisable::ENABLE;
    reject_for(
        &mut h,
        service,
        &dcc(enable, None, &[0x00]),
        RejectReason::TOO_MANY_ARGUMENTS,
    )
    .await;
    assert_eq!(dcc_state(&h).await, (restricted, true));
    let answer = answer_to(&mut h, service, &dcc(enable, None, &[])).await;
    assert!(matches!(answer, Apdu::SimpleAck(_)), "{answer:?}");
    assert_eq!(dcc_state(&h).await, (enabled, false));
    h.server.stop().await.unwrap();
}

/// A ReinitializeDevice body for WARMSTART with `password`, then `tail`.
fn reinitialize(password: Option<&str>, tail: &[u8]) -> Vec<u8> {
    let mut body = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: ReinitializedState::WARMSTART,
        password: password.map(str::to_owned),
    }
    .encode(&mut body)
    .unwrap();
    body.extend_from_slice(tail);
    body.to_vec()
}

#[tokio::test(start_paused = true)]
async fn reinitialize_device_with_trailing_octets_is_rejected() {
    let mut h = Harness::start(ServerConfig {
        reinit_password: Some("secret".into()),
        ..Default::default()
    })
    .await;
    let service = ConfirmedServiceChoice::REINITIALIZE_DEVICE;
    // The server never reinitializes: a well-formed request with the right
    // password is refused as SERVICE_REQUEST_DENIED.
    match answer_to(&mut h, service, &reinitialize(Some("secret"), &[])).await {
        Apdu::Error(error) => assert_eq!(
            (error.error_class, error.error_code),
            (ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
        ),
        other => panic!("{other:?}"),
    }
    // Each of these is refused for its trailing octets before the password
    // is checked.
    let cases: [(&str, Vec<u8>); 4] = [
        (
            "an octet after the right password",
            reinitialize(Some("secret"), &[0x00]),
        ),
        (
            "an octet after a wrong password",
            reinitialize(Some("wrong"), &[0x00]),
        ),
        (
            "the password as an application CharacterString",
            reinitialize(None, &SECRET_APPLICATION),
        ),
        ("a [2] after the state", reinitialize(None, &[0x29, 0x01])),
    ];
    for (what, body) in &cases {
        reject_case(
            &mut h,
            service,
            what,
            body,
            RejectReason::TOO_MANY_ARGUMENTS,
        )
        .await;
        assert_eq!(dcc_state(&h).await, (DccState::Enable, false), "{what}");
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn get_alarm_summary_with_any_octet_is_rejected() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let service = ConfirmedServiceChoice::GET_ALARM_SUMMARY;
    let answer = answer_to(&mut h, service, &[]).await;
    assert!(matches!(answer, Apdu::ComplexAck(_)), "{answer:?}");
    for body in [&[0x00][..], &[0x09, 0x00]] {
        reject_for(&mut h, service, body, RejectReason::TOO_MANY_ARGUMENTS).await;
    }
    h.server.stop().await.unwrap();
}

/// The server serves no ConfirmedPrivateTransfer, so trailing octets don't
/// change its answer; the decoders' refusal reaches only their callers.
#[tokio::test(start_paused = true)]
async fn confirmed_private_transfer_stays_unrecognized() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let service = ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER;
    for body in [
        &[0x09, 0x07, 0x19, 0x01][..],
        &[0x09, 0x07, 0x19, 0x01, 0x00],
    ] {
        match answer_to(&mut h, service, body).await {
            Apdu::Reject(reject) => assert_eq!(
                reject.reject_reason,
                RejectReason::UNRECOGNIZED_SERVICE,
                "{body:02X?}"
            ),
            other => panic!("{body:02X?}: {other:?}"),
        }
    }
    h.server.stop().await.unwrap();
}

/// Deliver an unconfirmed request from the harness peer, let the server run
/// what it makes ready, and return the services of the unconfirmed requests
/// it sent the peer since.
async fn unconfirmed(
    h: &Harness,
    service_choice: UnconfirmedServiceChoice,
    body: &[u8],
) -> Vec<UnconfirmedServiceChoice> {
    h.respond(Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice,
        service_request: Bytes::copy_from_slice(body),
    }))
    .await;
    h.settle().await;
    let mut sent = Vec::new();
    h.frames.lock().unwrap().retain(|apdu| match apdu {
        Apdu::UnconfirmedRequest(request) => {
            sent.push(request.service_choice);
            false
        }
        _ => true,
    });
    sent
}

#[tokio::test(start_paused = true)]
async fn who_is_and_who_has_with_trailing_octets_are_dropped() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let who_is = UnconfirmedServiceChoice::WHO_IS;
    // Limits 0 to 1000, which take in the harness device (856).
    let limits = [0x09, 0x00, 0x1A, 0x03, 0xE8];
    for (what, body) in [
        ("an octet after the limits", [&limits[..], &[0x00]].concat()),
        ("a [2] alone", vec![0x29, 0x00]),
        (
            "limits as application Unsigneds",
            vec![0x21, 0x00, 0x22, 0x03, 0xE8],
        ),
    ] {
        assert_eq!(unconfirmed(&h, who_is, &body).await, [], "{what}");
    }
    assert_eq!(
        unconfirmed(&h, who_is, &limits).await,
        [UnconfirmedServiceChoice::I_AM]
    );

    let who_has = UnconfirmedServiceChoice::WHO_HAS;
    // AV-1 by name.
    let by_name = [0x3D, 0x05, 0x00, b'A', b'V', b'-', b'1'];
    let trailing = [&by_name[..], &[0x00]].concat();
    assert_eq!(unconfirmed(&h, who_has, &trailing).await, []);
    assert_eq!(
        unconfirmed(&h, who_has, &by_name).await,
        [UnconfirmedServiceChoice::I_HAVE]
    );
    let counters = h.server.discovery_counters();
    assert_eq!(
        (counters.who_is_received, counters.who_has_received),
        (1, 1)
    );
    assert_eq!(counters.malformed_dropped, 4);
    h.server.stop().await.unwrap();
}

/// A Who-Is carrying one limit without the other is malformed (#1447), so
/// the server drops it rather than answering it as a Who-Is for every
/// device.
#[tokio::test(start_paused = true)]
async fn a_who_is_with_one_limit_is_dropped() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let who_is = UnconfirmedServiceChoice::WHO_IS;
    // Low limit 0, then high limit 1000: either alone would take in the
    // harness device (856) if read as unbounded.
    for (what, body) in [
        ("only the low limit", &[0x09, 0x00][..]),
        ("only the high limit", &[0x1A, 0x03, 0xE8]),
    ] {
        assert_eq!(unconfirmed(&h, who_is, body).await, [], "{what}");
    }
    assert_eq!(
        unconfirmed(&h, who_is, &[0x09, 0x00, 0x1A, 0x03, 0xE8]).await,
        [UnconfirmedServiceChoice::I_AM]
    );
    let counters = h.server.discovery_counters();
    assert_eq!(
        (counters.who_is_received, counters.malformed_dropped),
        (1, 2)
    );
    h.server.stop().await.unwrap();
}

/// A Who-Has carrying one limit without the other is malformed too (#1483),
/// and so is one whose low limit is above its high one, so the server drops
/// each and sends no I-Have, where one limit used to read as no range.
#[tokio::test(start_paused = true)]
async fn a_who_has_with_one_limit_is_dropped() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let who_has = UnconfirmedServiceChoice::WHO_HAS;
    // AV-1 by name, behind limits 0 and 1000, which take in the harness
    // device (856); either limit alone would too if read as unbounded.
    let by_name = [0x3D, 0x05, 0x00, b'A', b'V', b'-', b'1'];
    let with = |limits: &[u8]| [limits, &by_name[..]].concat();
    for (what, limits) in [
        ("only the low limit", &[0x09, 0x00][..]),
        ("only the high limit", &[0x1A, 0x03, 0xE8]),
        ("low limit above high", &[0x0A, 0x03, 0xE8, 0x19, 0x00]),
    ] {
        assert_eq!(unconfirmed(&h, who_has, &with(limits)).await, [], "{what}");
    }
    assert_eq!(
        unconfirmed(&h, who_has, &with(&[0x09, 0x00, 0x1A, 0x03, 0xE8])).await,
        [UnconfirmedServiceChoice::I_HAVE]
    );
    let counters = h.server.discovery_counters();
    assert_eq!(
        (counters.who_has_received, counters.malformed_dropped),
        (1, 3)
    );
    h.server.stop().await.unwrap();
}

/// Some devices send a high limit past the highest instance, 4194303, to
/// mean every device. The decoders take it as written (#1483), so the
/// server answers such a Who-Is and Who-Has.
#[tokio::test(start_paused = true)]
async fn a_high_limit_past_the_highest_instance_is_answered() {
    let mut h = Harness::start(ServerConfig::default()).await;
    // Low limit 0, high limit 4294967295.
    let limits = [0x09, 0x00, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF];
    assert_eq!(
        unconfirmed(&h, UnconfirmedServiceChoice::WHO_IS, &limits).await,
        [UnconfirmedServiceChoice::I_AM]
    );
    // AV-1 by name.
    let by_name = [0x3D, 0x05, 0x00, b'A', b'V', b'-', b'1'];
    assert_eq!(
        unconfirmed(
            &h,
            UnconfirmedServiceChoice::WHO_HAS,
            &[&limits[..], &by_name[..]].concat()
        )
        .await,
        [UnconfirmedServiceChoice::I_HAVE]
    );
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_i_am_with_trailing_octets_binds_nothing() {
    let mut h = Harness::start(ServerConfig::default()).await;
    use crate::server::binding_probes::WhoIsScope;
    let i_am = UnconfirmedServiceChoice::I_AM;
    // Device `instance`, max APDU 1476, no segmentation, vendor 42.
    let body = |instance: u16| {
        let [high, low] = instance.to_be_bytes();
        [
            0xC4, 0x02, 0x00, high, low, 0x22, 0x05, 0xC4, 0x91, 0x03, 0x21, 0x2A,
        ]
    };
    let device = |instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
    let bindings = h.server.device_bindings.read().await.len();
    // Device 1235 with a trailing octet, then a well-formed I-Am from 1234,
    // so a late bind of the first can't pass for the second's.
    unconfirmed(&h, i_am, &[&body(1235)[..], &[0x00]].concat()).await;
    assert_eq!(h.server.device_bindings.read().await.len(), bindings);
    unconfirmed(&h, i_am, &body(1234)).await;
    let table = h.server.device_bindings.read().await;
    assert_eq!(table.len(), bindings + 1);
    assert!(matches!(
        table.who_is_scope(&device(1234)),
        WhoIsScope::Local
    ));
    assert!(matches!(
        table.who_is_scope(&device(1235)),
        WhoIsScope::Global
    ));
    drop(table);
    h.server.stop().await.unwrap();
}
