use super::*;

use bacnet_types::enums::SilencedState;

// ---------------------------------------------------------------------------
// PR-0803 sub-slice 1 (R1 outcome b): operation -> property -> COV pins.
// Event_State is intrinsic-only, so LSO dispatches fan out exactly the
// Silenced / Operation_Expected / Present_Value / Tracking_Value deltas with
// one appended Status_Flags each — no coarse PV blast, no duplicated
// Status_Flags, no Event_State payload, and no invented Zone Tracking_Value.
// ---------------------------------------------------------------------------

/// Decode one unconfirmed COV notification, keeping the subscriber handle so
/// tests can attribute each payload to its subscription.
fn decode_cov_notification(apdu: &Apdu) -> COVNotificationRequest {
    let Apdu::UnconfirmedRequest(request) = apdu else {
        panic!("expected unconfirmed COV notification, got {apdu:?}");
    };
    assert_eq!(
        request.service_choice,
        UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
    );
    COVNotificationRequest::decode(&request.service_request).unwrap()
}

/// Assert a single-notification payload carries its property plus exactly one
/// Status_Flags, and never Event_State.
fn assert_exact_single_payload(
    notification: &COVNotificationRequest,
    property: PropertyIdentifier,
) {
    let properties: Vec<_> = notification
        .list_of_values
        .iter()
        .map(|value| value.property_identifier)
        .collect();
    assert_eq!(
        properties,
        vec![property, PropertyIdentifier::STATUS_FLAGS],
        "exact payload must be [property, Status_Flags] with no duplication"
    );
    assert!(
        !properties.contains(&PropertyIdentifier::EVENT_STATE),
        "Event_State is never part of an LSO COV payload"
    );
}

fn encode_life_safety_operation(
    operation: bacnet_types::enums::LifeSafetyOperation,
    object: ObjectIdentifier,
) -> Bytes {
    let request = LifeSafetyOperationRequest {
        requesting_process_identifier: 9,
        requesting_source: "operator".into(),
        request: operation,
        object_identifier: Some(object),
    };
    let mut encoded = BytesMut::new();
    request.encode(&mut encoded).unwrap();
    encoded.freeze()
}

#[tokio::test]
async fn silence_unsilence_dispatch_fans_out_exact_property_deltas() {
    use bacnet_types::enums::LifeSafetyOperation;

    let cases = [
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE,
            SilencedState::ALL_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_AUDIBLE,
            SilencedState::AUDIBLE_SILENCED,
        ),
        (
            SilencedState::UNSILENCED,
            LifeSafetyOperation::SILENCE_VISUAL,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE,
            SilencedState::UNSILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_AUDIBLE,
            SilencedState::VISIBLE_SILENCED,
        ),
        (
            SilencedState::ALL_SILENCED,
            LifeSafetyOperation::UNSILENCE_VISUAL,
            SilencedState::AUDIBLE_SILENCED,
        ),
    ];

    for (initial, operation, expected) in cases {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_silenced(initial);
        point.set_operation_expected(operation);
        let mut db = clocked_test_database();
        db.add(Box::new(point)).unwrap();
        let fixture = DispatchFixture::new(
            db,
            [
                subscription(
                    Some(PropertyIdentifier::SILENCED),
                    CovNotificationKind::Single,
                    1,
                ),
                subscription(
                    Some(PropertyIdentifier::OPERATION_EXPECTED),
                    CovNotificationKind::Single,
                    2,
                ),
                subscription(None, CovNotificationKind::Single, 3),
            ],
        )
        .await;

        fixture
            .dispatch(
                0x40,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
                encode_life_safety_operation(operation, point_oid()),
            )
            .await;
        let apdus = fixture.take_apdus();

        // Exact post-state: Silenced moved, Event_State still NORMAL.
        {
            let db = fixture.db.read().await;
            let object = db.get(&point_oid()).unwrap();
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::SILENCED, None)
                    .unwrap(),
                PropertyValue::Enumerated(expected.to_raw()),
                "operation {} from state {}",
                operation.to_raw(),
                initial.to_raw()
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::EVENT_STATE, None)
                    .unwrap(),
                PropertyValue::Enumerated(0)
            );
        }

        // ACK first, then exactly the two matching property notifications —
        // the whole-object subscription stays silent (no PV/Status_Flags
        // change means no coarse PV blast).
        assert!(
            matches!(apdus[0], Apdu::SimpleAck(_)),
            "operation {} must ACK first",
            operation.to_raw()
        );
        let notifications: Vec<_> = apdus[1..].iter().map(decode_cov_notification).collect();
        assert_eq!(
            notifications.len(),
            2,
            "operation {} must notify only the two matching property subs",
            operation.to_raw()
        );
        for notification in &notifications {
            assert_eq!(notification.monitored_object_identifier, point_oid());
            assert!(
                !notification
                    .list_of_values
                    .iter()
                    .any(|value| value.property_identifier == PropertyIdentifier::PRESENT_VALUE),
                "Silenced/OE-only operation must not blast Present_Value"
            );
        }
        let by_subscriber: std::collections::HashMap<u32, &COVNotificationRequest> = notifications
            .iter()
            .map(|n| (n.subscriber_process_identifier, n))
            .collect();
        assert_exact_single_payload(by_subscriber[&1], PropertyIdentifier::SILENCED);
        assert_exact_single_payload(by_subscriber[&2], PropertyIdentifier::OPERATION_EXPECTED);
        assert!(
            !by_subscriber.contains_key(&3),
            "whole-object subscription must stay silent without a PV/Status change"
        );
    }
}

#[tokio::test]
async fn reset_dispatch_fans_out_exact_pv_delta_without_event_state_blast() {
    use bacnet_types::enums::{LifeSafetyOperation, LifeSafetyState};

    for operation in [
        LifeSafetyOperation::RESET,
        LifeSafetyOperation::RESET_ALARM,
        LifeSafetyOperation::RESET_FAULT,
    ] {
        let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
        point.set_present_value(LifeSafetyState::ALARM.to_raw());
        point.set_operation_expected(operation);
        point.set_reset_executor(Arc::new(|_| {
            Ok(LifeSafetyPointResetCommit {
                present_value: Some(LifeSafetyState::QUIET),
                ..Default::default()
            })
        }));
        let mut db = clocked_test_database();
        db.add(Box::new(point)).unwrap();
        let fixture = DispatchFixture::new(
            db,
            [
                subscription(
                    Some(PropertyIdentifier::PRESENT_VALUE),
                    CovNotificationKind::Single,
                    1,
                ),
                subscription(None, CovNotificationKind::Single, 2),
                subscription(
                    Some(PropertyIdentifier::SILENCED),
                    CovNotificationKind::Single,
                    3,
                ),
            ],
        )
        .await;

        fixture
            .dispatch(
                0x41,
                ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
                encode_life_safety_operation(operation, point_oid()),
            )
            .await;
        let apdus = fixture.take_apdus();

        {
            let db = fixture.db.read().await;
            let object = db.get(&point_oid()).unwrap();
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                    .unwrap(),
                PropertyValue::Enumerated(LifeSafetyState::QUIET.to_raw())
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::EVENT_STATE, None)
                    .unwrap(),
                PropertyValue::Enumerated(0)
            );
        }

        assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
        let notifications: Vec<_> = apdus[1..].iter().map(decode_cov_notification).collect();
        // PV property sub + whole-object sub fire; the untouched SILENCED
        // property sub stays silent.
        assert_eq!(
            notifications.len(),
            2,
            "reset {operation:?} must notify exactly the PV and whole-object subs"
        );
        for notification in &notifications {
            assert_exact_single_payload(notification, PropertyIdentifier::PRESENT_VALUE);
        }
        let subscribers: Vec<u32> = notifications
            .iter()
            .map(|n| n.subscriber_process_identifier)
            .collect();
        assert!(subscribers.contains(&1));
        assert!(subscribers.contains(&2));
        assert!(
            !subscribers.contains(&3),
            "untouched SILENCED property sub must stay silent"
        );
    }
}

#[tokio::test]
async fn zone_silence_dispatch_never_invents_tracking_value() {
    use bacnet_types::enums::LifeSafetyOperation;

    let zone_oid = ObjectIdentifier::new(ObjectType::LIFE_SAFETY_ZONE, 1).unwrap();
    let mut zone = bacnet_objects::life_safety::LifeSafetyZoneObject::new(1, "zone").unwrap();
    zone.set_silenced(SilencedState::UNSILENCED);
    zone.set_operation_expected(LifeSafetyOperation::SILENCE);
    let mut db = clocked_test_database();
    db.add(Box::new(zone)).unwrap();

    let mut silenced_sub = subscription(
        Some(PropertyIdentifier::SILENCED),
        CovNotificationKind::Single,
        1,
    );
    silenced_sub.monitored_object_identifier = zone_oid;
    let mut whole_sub = subscription(None, CovNotificationKind::Single, 2);
    whole_sub.monitored_object_identifier = zone_oid;
    let fixture = DispatchFixture::new(db, [silenced_sub, whole_sub]).await;

    fixture
        .dispatch(
            0x42,
            ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
            encode_life_safety_operation(LifeSafetyOperation::SILENCE, zone_oid),
        )
        .await;
    let apdus = fixture.take_apdus();

    {
        let db = fixture.db.read().await;
        let object = db.get(&zone_oid).unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::SILENCED, None)
                .unwrap(),
            PropertyValue::Enumerated(SilencedState::ALL_SILENCED.to_raw())
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::EVENT_STATE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
        assert!(
            object
                .read_property(PropertyIdentifier::TRACKING_VALUE, None)
                .is_err(),
            "zone must not expose an invented Tracking_Value"
        );
    }

    assert!(matches!(apdus[0], Apdu::SimpleAck(_)));
    let notifications: Vec<_> = apdus[1..].iter().map(decode_cov_notification).collect();
    assert_eq!(
        notifications.len(),
        1,
        "zone Silenced-only change notifies only the SILENCED property sub"
    );
    assert_eq!(notifications[0].subscriber_process_identifier, 1);
    assert_exact_single_payload(&notifications[0], PropertyIdentifier::SILENCED);
    assert!(
        !notifications[0]
            .list_of_values
            .iter()
            .any(|value| value.property_identifier == PropertyIdentifier::TRACKING_VALUE),
        "zone COV must never carry an invented Tracking_Value"
    );
}
