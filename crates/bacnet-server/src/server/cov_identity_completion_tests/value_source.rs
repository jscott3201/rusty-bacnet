use super::*;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;

pub(super) const FIELDS: [PropertyIdentifier; 5] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::VALUE_SOURCE,
    PropertyIdentifier::LAST_COMMAND_TIME,
    PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
];

pub(super) fn decoded(
    frame: Bytes,
    kind: CovNotificationKind,
) -> Vec<(PropertyIdentifier, Vec<u8>)> {
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!("unconfirmed notification")
    };
    match kind {
        CovNotificationKind::Single => COVNotificationRequest::decode(&request.service_request)
            .unwrap()
            .list_of_values
            .into_iter()
            .map(|v| {
                assert_eq!(v.property_array_index, None);
                (v.property_identifier, v.value)
            })
            .collect(),
        CovNotificationKind::Multiple => {
            let n = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
            assert_eq!(n.list_of_cov_notifications.len(), 1);
            n.list_of_cov_notifications
                .into_iter()
                .next()
                .unwrap()
                .list_of_values
                .into_iter()
                .map(|v| {
                    assert_eq!(v.property_array_index, None);
                    (v.property_identifier, v.value)
                })
                .collect()
        }
    }
}

#[tokio::test]
async fn value_source_cov_initial_wire_five_fields() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let fixture = Fixture::new(false);
        let mut sub = proposal(kind, false, PropertyIdentifier::VALUE_SOURCE);
        sub.last_notified_observation = None;
        let accepted = fixture.table.write().await.admit_for_test(sub, 0).unwrap();
        fixture.fire(true, &[accepted]).await;
        let frames = fixture.sent.lock().unwrap().clone();
        assert_eq!(frames.len(), 1);
        let values = decoded(frames[0].clone(), kind);
        assert_eq!(values.iter().map(|v| v.0).collect::<Vec<_>>(), FIELDS);
        let db = fixture.db.read().await;
        let obj = db.get(&object()).unwrap();
        for (property, bytes) in values {
            let mut expected = BytesMut::new();
            encode_property_value(&mut expected, &obj.read_property(property, None).unwrap())
                .unwrap();
            assert_eq!(bytes, expected);
        }
        drop(db);
        fixture.finish(false).await;
    }
}

use bacnet_objects::{
    analog::AnalogOutputObject,
    binary::{BinaryOutputObject, BinaryValueObject},
    multistate::{MultiStateOutputObject, MultiStateValueObject},
    traits::BACnetObject,
};

fn families() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(AnalogOutputObject::new(3, "ao", 95).unwrap()),
        Box::new(AnalogValueObject::new(3, "av", 95).unwrap()),
        Box::new(BinaryOutputObject::new(3, "bo").unwrap()),
        Box::new(BinaryValueObject::new(3, "bv").unwrap()),
        Box::new(MultiStateOutputObject::new(3, "mso", 3).unwrap()),
        Box::new(MultiStateValueObject::new(3, "msv", 3).unwrap()),
    ]
}
async fn fire(f: &Fixture, oid: ObjectIdentifier) {
    BACnetServer::<TestTransport>::fire_cov_notifications(
        &crate::server::cov_notify_context::CovNotifyContext {
            db: &f.db,
            network: &f.network,
            cov_table: &f.table,
            cov_in_flight: &f.permits,
            notification_transactions: &f.transactions,
            comm_state: &f.comm,
            config: &f.config,
        },
        &oid,
    )
    .await;
}
async fn assert_report(f: &Fixture, oid: ObjectIdentifier, kind: CovNotificationKind) {
    let frames = std::mem::take(&mut *f.sent.lock().unwrap());
    assert_eq!(frames.len(), 1, "{oid:?} {kind:?}");
    let values = decoded(frames[0].clone(), kind);
    assert_eq!(values.iter().map(|v| v.0).collect::<Vec<_>>(), FIELDS);
    let db = f.db.read().await;
    for (p, bytes) in values {
        let mut expected = BytesMut::new();
        encode_property_value(
            &mut expected,
            &db.get(&oid).unwrap().read_property(p, None).unwrap(),
        )
        .unwrap();
        assert_eq!(bytes, expected, "{oid:?} {p:?}");
    }
}
async fn command(f: &Fixture, oid: ObjectIdentifier, value: PropertyValue, priority: u8) {
    f.db.write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            Some(priority),
            &crate::command_source::test_origin(),
        )
        .unwrap();
}
#[tokio::test]
async fn value_source_cov_six_families_initial_renewal_and_triggers() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for mut obj in families() {
            let oid = obj.object_identifier();
            let analog = matches!(
                oid.object_type(),
                ObjectType::ANALOG_OUTPUT | ObjectType::ANALOG_VALUE
            );
            let binary = matches!(
                oid.object_type(),
                ObjectType::BINARY_OUTPUT | ObjectType::BINARY_VALUE
            );
            let first = if analog {
                PropertyValue::Real(10.0)
            } else if binary {
                PropertyValue::Enumerated(0)
            } else {
                PropertyValue::Unsigned(1)
            };
            if analog {
                obj.write_property(
                    PropertyIdentifier::COV_INCREMENT,
                    None,
                    PropertyValue::Real(2.0),
                    None,
                )
                .unwrap();
            }
            obj.write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                first.clone(),
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
            let f = Fixture::new(false);
            let mut db = clocked_test_database();
            db.add(obj).unwrap();
            *f.db.write().await = db;
            let mut sub = proposal(kind, false, PropertyIdentifier::VALUE_SOURCE);
            sub.monitored_object_identifier = oid;
            sub.last_notified_observation = None;
            sub.cov_increment = Some(1000.0); // Never governs specialized object PV.
            for _ in 0..2 {
                let accepted = f
                    .table
                    .write()
                    .await
                    .admit_for_test(sub.clone(), 0)
                    .unwrap();
                f.fire(true, &[accepted]).await;
                assert_report(&f, oid, kind).await;
            }
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty());
            // A correction changes only the claim, with time/PV/priority stable.
            let before =
                f.db.read()
                    .await
                    .get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::LAST_COMMAND_TIME, None)
                    .unwrap();
            f.db.write()
                .await
                .get_mut(&oid)
                .unwrap()
                .write_property_from(
                    PropertyIdentifier::VALUE_SOURCE,
                    None,
                    PropertyValue::ApplicationData(vec![0x08]),
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            assert_eq!(
                f.db.read()
                    .await
                    .get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::LAST_COMMAND_TIME, None)
                    .unwrap(),
                before
            );
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            command(&f, oid, first.clone(), 8).await;
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            // Identical PV/source, new effective priority alone qualifies.
            command(&f, oid, first.clone(), 4).await;
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            if analog {
                command(&f, oid, PropertyValue::Real(10.5), 4).await;
                fire(&f, oid).await;
                assert!(
                    f.sent.lock().unwrap().is_empty(),
                    "subincrement must ignore sequence-time change"
                );
            }
            let changed = if analog {
                PropertyValue::Real(12.0)
            } else if binary {
                PropertyValue::Enumerated(1)
            } else {
                PropertyValue::Unsigned(2)
            };
            command(&f, oid, changed, 4).await;
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            f.db.write()
                .await
                .get_mut(&oid)
                .unwrap()
                .write_property(
                    PropertyIdentifier::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(true),
                    None,
                )
                .unwrap();
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            fire(&f, oid).await;
            assert!(f.sent.lock().unwrap().is_empty());
            f.finish(false).await;
        }
    }
}

#[tokio::test]
async fn value_source_cov_stale_completion_and_cancellation_fences() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            for initial in [false, true] {
                for change in [Change::Renew, Change::Recreate, Change::Remove] {
                    stale_completion(
                        initial,
                        kind,
                        confirmed,
                        change,
                        PropertyIdentifier::VALUE_SOURCE,
                    )
                    .await;
                }
            }
        }
    }
}
#[tokio::test]
async fn value_source_cov_send_failure_and_confirmation_baselines() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            let f = Fixture::new(false);
            let mut p = proposal(kind, confirmed, PropertyIdentifier::VALUE_SOURCE);
            p.last_notified_observation = None;
            let sub = f.table.write().await.admit_for_test(p, 0).unwrap();
            if !confirmed {
                f.fail.store(true, Ordering::Relaxed);
            }
            f.fire(true, std::slice::from_ref(&sub)).await;
            let observation = f
                .table
                .read()
                .await
                .get_subscription(sub.key())
                .unwrap()
                .last_notified_observation
                .clone();
            if confirmed {
                assert!(
                    observation.is_none(),
                    "the baseline waits for the ACK (#896)"
                );
                f.finish(true).await;
                assert!(f
                    .table
                    .read()
                    .await
                    .get_subscription(sub.key())
                    .unwrap()
                    .last_notified_observation
                    .as_ref()
                    .unwrap()
                    .source_companions()
                    .is_some());
            } else {
                assert!(observation.is_none());
                f.fail.store(false, Ordering::Relaxed);
                f.sent.lock().unwrap().clear();
                f.fire(true, std::slice::from_ref(&sub)).await;
                assert!(f
                    .table
                    .read()
                    .await
                    .get_subscription(sub.key())
                    .unwrap()
                    .last_notified_observation
                    .as_ref()
                    .unwrap()
                    .source_companions()
                    .is_some());
                f.finish(false).await;
            }
        }
    }
}

#[tokio::test]
async fn value_source_cov_held_send_commits_exact_delivered_tuple() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let f = Fixture::new(true);
        let mut p = proposal(kind, false, PropertyIdentifier::VALUE_SOURCE);
        p.last_notified_observation = None;
        let sub = f.table.write().await.admit_for_test(p, 0).unwrap();
        let snapshots = [sub.clone()];
        let mut work = Box::pin(f.fire(true, &snapshots));
        assert!(futures_util::poll!(work.as_mut()).is_pending());
        let delivered = decoded(f.sent.lock().unwrap()[0].clone(), kind);
        command(&f, object(), PropertyValue::Real(12.0), 8).await;
        f.release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(2), work)
            .await
            .unwrap();
        let before = f
            .table
            .read()
            .await
            .get_subscription(sub.key())
            .unwrap()
            .last_notified_observation
            .clone()
            .unwrap();
        let (pv, priority) = before.source_companions().unwrap();
        let priority = priority.as_ref().unwrap();
        for (property, sample) in [
            (PropertyIdentifier::PRESENT_VALUE, pv),
            (PropertyIdentifier::VALUE_SOURCE, before.sample()),
            (PropertyIdentifier::CURRENT_COMMAND_PRIORITY, priority),
        ] {
            let mut encoded = BytesMut::new();
            encode_property_value(&mut encoded, sample.value()).unwrap();
            assert_eq!(
                delivered.iter().find(|v| v.0 == property).unwrap().1,
                encoded
            );
        }
        assert_eq!(pv.value(), &PropertyValue::Real(10.0));
        assert_eq!(priority.value(), &PropertyValue::Null);
        f.sent.lock().unwrap().clear();
        f.release.add_permits(1);
        f.fire(false, &[]).await;
        assert_report(&f, object(), kind).await;
        f.finish(false).await;
    }
}
