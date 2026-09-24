use super::*;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;

async fn selected_change(kind: CovNotificationKind, property: PropertyIdentifier) {
    for increment in [None, Some(2.0)] {
        let fixture = Fixture::new(false);
        fixture
            .db
            .write()
            .await
            .get_mut(&object())
            .unwrap()
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(10.0),
                Some(8),
            )
            .unwrap();
        let mut sub = proposal(kind, false, property);
        sub.last_notified_observation = None;
        sub.cov_increment = increment;
        let accepted = fixture.table.write().await.subscribe(sub).unwrap();
        fixture.fire(true, &[accepted]).await;
        assert_eq!(fixture.sent.lock().unwrap().len(), 1);
        fixture.sent.lock().unwrap().clear();
        {
            let mut db = fixture.db.write().await;
            let av = db.get_mut(&object()).unwrap();
            let (written, value) = if property == PropertyIdentifier::STATUS_FLAGS {
                (
                    PropertyIdentifier::OUT_OF_SERVICE,
                    PropertyValue::Boolean(true),
                )
            } else {
                (
                    PropertyIdentifier::RELINQUISH_DEFAULT,
                    PropertyValue::Real(5.0),
                )
            };
            av.write_property(written, None, value, None).unwrap();
            assert_eq!(
                av.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                    .unwrap(),
                PropertyValue::Real(10.0)
            );
        }
        fixture.fire(false, &[]).await;
        let frames = fixture.sent.lock().unwrap().clone();
        assert_eq!(frames.len(), 1, "selected {property:?} change kind={kind:?} increment={increment:?} with unchanged PV must report");
        let Apdu::UnconfirmedRequest(request) =
            decode_apdu(decode_npdu(frames[0].clone()).unwrap().payload).unwrap()
        else {
            panic!("notification")
        };
        let (actual_property, value) = match kind {
            CovNotificationKind::Single => {
                let n = COVNotificationRequest::decode(&request.service_request).unwrap();
                assert_eq!(
                    n.list_of_values.len(),
                    if property == PropertyIdentifier::STATUS_FLAGS {
                        1
                    } else {
                        2
                    }
                );
                (
                    n.list_of_values[0].property_identifier,
                    n.list_of_values[0].value.clone(),
                )
            }
            CovNotificationKind::Multiple => {
                let n = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
                assert_eq!(n.list_of_cov_notifications.len(), 1);
                let values = &n.list_of_cov_notifications[0].list_of_values;
                assert_eq!(
                    values.len(),
                    if property == PropertyIdentifier::STATUS_FLAGS {
                        1
                    } else {
                        2
                    }
                );
                (values[0].property_identifier, values[0].value.clone())
            }
        };
        assert_eq!(actual_property, property);
        let expected = fixture
            .db
            .read()
            .await
            .get(&object())
            .unwrap()
            .read_property(property, None)
            .unwrap();
        let mut bytes = BytesMut::new();
        encode_property_value(&mut bytes, &expected).unwrap();
        assert_eq!(value, bytes);
        fixture.finish(false).await;
    }
}

#[tokio::test]
async fn cov_sample_single_status_selected_wire() {
    selected_change(
        CovNotificationKind::Single,
        PropertyIdentifier::STATUS_FLAGS,
    )
    .await;
}
#[tokio::test]
async fn cov_sample_multiple_status_selected_wire() {
    selected_change(
        CovNotificationKind::Multiple,
        PropertyIdentifier::STATUS_FLAGS,
    )
    .await;
}
#[tokio::test]
async fn cov_sample_single_relinquish_selected_wire() {
    selected_change(
        CovNotificationKind::Single,
        PropertyIdentifier::RELINQUISH_DEFAULT,
    )
    .await;
}
#[tokio::test]
async fn cov_sample_multiple_relinquish_selected_wire() {
    selected_change(
        CovNotificationKind::Multiple,
        PropertyIdentifier::RELINQUISH_DEFAULT,
    )
    .await;
}
