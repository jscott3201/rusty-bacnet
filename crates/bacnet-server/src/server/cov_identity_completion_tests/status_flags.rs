use super::*;
use bacnet_services::{cov::COVNotificationRequest, cov_multiple::COVNotificationMultipleRequest};

pub(super) fn values(
    frame: Bytes,
    kind: CovNotificationKind,
) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!("notification")
    };
    match kind {
        CovNotificationKind::Single => COVNotificationRequest::decode(&request.service_request)
            .unwrap()
            .list_of_values
            .into_iter()
            .map(|v| (v.property_identifier, v.value))
            .collect(),
        CovNotificationKind::Multiple => {
            COVNotificationMultipleRequest::decode(&request.service_request)
                .unwrap()
                .list_of_cov_notifications
                .into_iter()
                .flat_map(|i| i.list_of_values)
                .map(|v| (v.property_identifier, v.value))
                .collect()
        }
    }
}

async fn status_change(kind: CovNotificationKind, ordinary: bool, initial_companion: bool) {
    let fixture = Fixture::new(false);
    let mut sub = proposal(kind, false, PropertyIdentifier::RELINQUISH_DEFAULT);
    sub.last_notified_observation = None;
    sub.cov_increment = Some(2.0);
    if ordinary {
        sub.monitored_property = None;
    }
    let accepted = fixture.table.write().await.subscribe(sub).unwrap();
    fixture.fire(true, &[accepted]).await;
    let initial = values(fixture.sent.lock().unwrap().pop().unwrap(), kind);
    if initial_companion {
        assert_eq!(
            initial
                .iter()
                .filter(|v| v.0 == PropertyIdentifier::STATUS_FLAGS)
                .count(),
            1,
            "initial companion"
        );
    }
    fixture
        .db
        .write()
        .await
        .get_mut(&object())
        .unwrap()
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    fixture.fire(false, &[]).await;
    let frames = fixture.sent.lock().unwrap().clone();
    assert_eq!(
        frames.len(),
        1,
        "fixed selected value with changed flags must report"
    );
    let changed = values(frames[0].clone(), kind);
    assert_eq!(changed.len(), 2);
    assert_eq!(
        changed
            .iter()
            .filter(|v| v.0 == PropertyIdentifier::STATUS_FLAGS)
            .count(),
        1
    );
    fixture.sent.lock().unwrap().clear();
    fixture.fire(false, &[]).await;
    assert!(
        fixture.sent.lock().unwrap().is_empty(),
        "identical numeric value and flags are stable"
    );
    fixture.finish(false).await;
}
#[tokio::test]
async fn cov_status_ordinary_flags_only_wire() {
    status_change(CovNotificationKind::Single, true, true).await;
}
#[tokio::test]
async fn cov_status_single_flags_only_wire() {
    status_change(CovNotificationKind::Single, false, false).await;
}
#[tokio::test]
async fn cov_status_multiple_flags_only_wire() {
    status_change(CovNotificationKind::Multiple, false, false).await;
}
#[tokio::test]
async fn cov_status_single_initial_companion_wire() {
    status_change(CovNotificationKind::Single, false, true).await;
}
#[tokio::test]
async fn cov_status_multiple_initial_companion_wire() {
    status_change(CovNotificationKind::Multiple, false, true).await;
}
