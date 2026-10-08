//! Table 13-1a-2 reports for source-tracked, noncommandable Value objects.
use super::value_source::decoded;
use super::*;
use bacnet_objects::{
    binary::BinaryValueObject, multistate::MultiStateValueObject,
    present_value_access::PresentValueAccess, traits::BACnetObject,
};

const FIELDS: [PropertyIdentifier; 3] = [
    PropertyIdentifier::PRESENT_VALUE,
    PropertyIdentifier::STATUS_FLAGS,
    PropertyIdentifier::VALUE_SOURCE,
];

fn families(access: PresentValueAccess, tracked: bool) -> Vec<Box<dyn BACnetObject>> {
    let mut av = AnalogValueObject::with_access(3, "av", 95, access).unwrap();
    av.set_value_source_tracking(tracked);
    av.write_property(
        PropertyIdentifier::COV_INCREMENT,
        None,
        PropertyValue::Real(2.0),
        None,
    )
    .unwrap();
    let mut bv = BinaryValueObject::with_access(3, "bv", access).unwrap();
    bv.set_value_source_tracking(tracked);
    let mut msv = MultiStateValueObject::with_access(3, "msv", 3, access).unwrap();
    msv.set_value_source_tracking(tracked);
    vec![Box::new(av), Box::new(bv), Box::new(msv)]
}

async fn fixture(obj: Box<dyn BACnetObject>, hold: bool) -> Fixture {
    let f = Fixture::new(hold);
    let mut db = clocked_test_database();
    db.add(obj).unwrap();
    *f.db.write().await = db;
    f
}

async fn subscribe(
    f: &Fixture,
    oid: ObjectIdentifier,
    kind: CovNotificationKind,
    confirmed: bool,
) -> CovSubscriptionSnapshot {
    let mut sub = proposal(kind, confirmed, PropertyIdentifier::VALUE_SOURCE);
    sub.monitored_object_identifier = oid;
    sub.last_notified_observation = None;
    // This must not override the object's PV criterion.
    sub.cov_increment = Some(1000.0);
    f.table.write().await.admit_for_test(sub, 0).unwrap()
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

async fn write(
    f: &Fixture,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
) {
    f.db.write()
        .await
        .get_mut(&oid)
        .unwrap()
        .write_property_from(
            property,
            None,
            value,
            None,
            &crate::command_source::test_origin(),
        )
        .unwrap();
}

async fn assert_report(f: &Fixture, oid: ObjectIdentifier, kind: CovNotificationKind) {
    let frames = std::mem::take(&mut *f.sent.lock().unwrap());
    assert_eq!(frames.len(), 1, "{oid:?} {kind:?}");
    let values = decoded(frames[0].clone(), kind);
    assert_eq!(values.iter().map(|v| v.0).collect::<Vec<_>>(), FIELDS);
    let db = f.db.read().await;
    for (property, bytes) in values {
        let mut expected = BytesMut::new();
        encode_property_value(
            &mut expected,
            &db.get(&oid).unwrap().read_property(property, None).unwrap(),
        )
        .unwrap();
        assert_eq!(bytes, expected, "{oid:?} {property:?}");
    }
}

#[tokio::test]
async fn value_source_cov_noncommandable_initial_none_and_renewal_three_fields() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
            for obj in families(access, true) {
                let oid = obj.object_identifier();
                // IC 135-2020-32 permits NONE before any Present_Value write.
                assert_eq!(
                    obj.read_property(PropertyIdentifier::VALUE_SOURCE, None)
                        .unwrap(),
                    PropertyValue::ApplicationData(vec![0x08])
                );
                let f = fixture(obj, false).await;
                for _ in 0..2 {
                    let sub = subscribe(&f, oid, kind, false).await;
                    f.fire(true, &[sub]).await;
                    assert_report(&f, oid, kind).await;
                }
                fire(&f, oid).await;
                assert!(f.sent.lock().unwrap().is_empty());
                f.finish(false).await;
            }
        }
    }
}

#[tokio::test]
async fn value_source_cov_noncommandable_same_writer_pv_uses_object_criterion() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
            for mut obj in families(access, true) {
                let oid = obj.object_identifier();
                let analog = oid.object_type() == ObjectType::ANALOG_VALUE;
                let binary = oid.object_type() == ObjectType::BINARY_VALUE;
                if access == PresentValueAccess::ReadOnly {
                    obj.write_property(
                        PropertyIdentifier::OUT_OF_SERVICE,
                        None,
                        PropertyValue::Boolean(true),
                        None,
                    )
                    .unwrap();
                }
                let first = if analog {
                    PropertyValue::Real(10.0)
                } else if binary {
                    PropertyValue::Enumerated(0)
                } else {
                    PropertyValue::Unsigned(1)
                };
                obj.write_property_from(
                    PropertyIdentifier::PRESENT_VALUE,
                    None,
                    first,
                    None,
                    &crate::command_source::test_origin(),
                )
                .unwrap();
                let source = obj
                    .read_property(PropertyIdentifier::VALUE_SOURCE, None)
                    .unwrap();
                let f = fixture(obj, false).await;
                let sub = subscribe(&f, oid, kind, false).await;
                f.fire(true, &[sub]).await;
                f.sent.lock().unwrap().clear();
                if analog {
                    for value in [10.5, 11.0, 11.5] {
                        write(
                            &f,
                            oid,
                            PropertyIdentifier::PRESENT_VALUE,
                            PropertyValue::Real(value),
                        )
                        .await;
                        fire(&f, oid).await;
                        assert!(
                            f.sent.lock().unwrap().is_empty(),
                            "subincrement changed source baseline"
                        );
                    }
                }
                let changed = if analog {
                    PropertyValue::Real(12.0)
                } else if binary {
                    PropertyValue::Enumerated(1)
                } else {
                    PropertyValue::Unsigned(2)
                };
                write(&f, oid, PropertyIdentifier::PRESENT_VALUE, changed).await;
                assert_eq!(
                    f.db.read()
                        .await
                        .get(&oid)
                        .unwrap()
                        .read_property(PropertyIdentifier::VALUE_SOURCE, None)
                        .unwrap(),
                    source
                );
                fire(&f, oid).await;
                assert_report(&f, oid, kind).await;
                fire(&f, oid).await;
                assert!(f.sent.lock().unwrap().is_empty());
                f.finish(false).await;
            }
        }
    }
}

#[tokio::test]
async fn value_source_cov_noncommandable_flags_and_final_correction() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for obj in families(PresentValueAccess::Writable, true) {
            let oid = obj.object_identifier();
            let analog = oid.object_type() == ObjectType::ANALOG_VALUE;
            let binary = oid.object_type() == ObjectType::BINARY_VALUE;
            let first = if analog {
                PropertyValue::Real(10.0)
            } else if binary {
                PropertyValue::Enumerated(0)
            } else {
                PropertyValue::Unsigned(1)
            };
            let f = fixture(obj, false).await;
            write(&f, oid, PropertyIdentifier::PRESENT_VALUE, first.clone()).await;
            let sub = subscribe(&f, oid, kind, false).await;
            f.fire(true, &[sub]).await;
            assert_report(&f, oid, kind).await;

            write(
                &f,
                oid,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyValue::Boolean(true),
            )
            .await;
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            // Only the source claim changes; PV is still the first value.
            let corrected = PropertyValue::ApplicationData(vec![0x08]);
            write(&f, oid, PropertyIdentifier::VALUE_SOURCE, corrected.clone()).await;
            assert_eq!(
                f.db.read()
                    .await
                    .get(&oid)
                    .unwrap()
                    .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                    .unwrap(),
                first
            );
            fire(&f, oid).await;
            assert_report(&f, oid, kind).await;
            write(&f, oid, PropertyIdentifier::VALUE_SOURCE, corrected.clone()).await;
            fire(&f, oid).await;
            assert!(
                f.sent.lock().unwrap().is_empty(),
                "unchanged correction notified"
            );

            // One preparation follows both writes. Final VS equals the last
            // report, so this notification depends on PV; it must not carry
            // the writer's intermediate source instead of its correction.
            let changed = if analog {
                PropertyValue::Real(12.0)
            } else if binary {
                PropertyValue::Enumerated(1)
            } else {
                PropertyValue::Unsigned(2)
            };
            write(&f, oid, PropertyIdentifier::PRESENT_VALUE, changed).await;
            write(&f, oid, PropertyIdentifier::VALUE_SOURCE, corrected.clone()).await;
            fire(&f, oid).await;
            let values = decoded(f.sent.lock().unwrap()[0].clone(), kind);
            assert_eq!(
                values
                    .iter()
                    .find(|(p, _)| *p == PropertyIdentifier::VALUE_SOURCE)
                    .unwrap()
                    .1,
                [0x08]
            );
            assert_report(&f, oid, kind).await;
            f.finish(false).await;
        }
    }
}

#[test]
fn value_source_cov_noncommandable_admission_requires_tracking() {
    use bacnet_services::cov::SubscribeCOVPropertyRequest;
    use bacnet_services::cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    };
    use bacnet_types::constructed::PropertyReference;
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for tracked in [false, true] {
            for access in [PresentValueAccess::Writable, PresentValueAccess::ReadOnly] {
                for obj in families(access, tracked) {
                    let oid = obj.object_identifier();
                    let mut db = clocked_test_database();
                    db.add(obj).unwrap();
                    let mut table = CovSubscriptionTable::new();
                    let mut wire = BytesMut::new();
                    let result = match kind {
                        CovNotificationKind::Single => {
                            SubscribeCOVPropertyRequest {
                                subscriber_process_identifier: 1,
                                monitored_object_identifier: oid,
                                issue_confirmed_notifications: Some(false),
                                lifetime: Some(300),
                                monitored_property_identifier: PropertyIdentifier::VALUE_SOURCE,
                                monitored_property_array_index: None,
                                cov_increment: None,
                            }
                            .encode(&mut wire)
                            .unwrap();
                            crate::handlers::handle_subscribe_cov_property(
                                &mut table,
                                &db,
                                &[1],
                                &wire,
                            )
                        }
                        CovNotificationKind::Multiple => {
                            SubscribeCOVPropertyMultipleRequest {
                                subscriber_process_identifier: 1,
                                issue_confirmed_notifications: false,
                                lifetime: Some(300),
                                max_notification_delay: Some(0),
                                list_of_cov_subscription_specifications: vec![
                                    COVSubscriptionSpecification {
                                        monitored_object_identifier: oid,
                                        list_of_cov_references: vec![COVReference {
                                            monitored_property: PropertyReference {
                                                property_identifier:
                                                    PropertyIdentifier::VALUE_SOURCE,
                                                property_array_index: None,
                                            },
                                            cov_increment: None,
                                            timestamped: false,
                                        }],
                                    },
                                ],
                            }
                            .encode(&mut wire)
                            .unwrap();
                            crate::handlers::handle_subscribe_cov_property_multiple(
                                &mut table,
                                &db,
                                &[1],
                                &wire,
                            )
                        }
                    };
                    if tracked {
                        result.unwrap();
                        assert_eq!(table.len(), 1);
                    } else {
                        assert!(
                            matches!(result.unwrap_err(), Error::Protocol { class, code }
                            | Error::Structured { class, code, .. }
                            if class == ErrorClass::PROPERTY.to_raw() as u32
                                && code == ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
                        );
                        assert!(table.is_empty());
                    }
                }
            }
        }
    }
}

async fn observation(
    f: &Fixture,
    sub: &CovSubscriptionSnapshot,
) -> Option<crate::cov::CovObservation> {
    f.table
        .read()
        .await
        .get_subscription(sub.key())
        .unwrap()
        .last_notified_observation
        .clone()
}

#[tokio::test]
async fn value_source_cov_noncommandable_failed_send_and_confirmation_baselines() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for confirmed in [false, true] {
            let obj = families(PresentValueAccess::Writable, true).remove(0);
            let f = fixture(obj, false).await;
            write(
                &f,
                object(),
                PropertyIdentifier::PRESENT_VALUE,
                PropertyValue::Real(10.0),
            )
            .await;
            let sub = subscribe(&f, object(), kind, confirmed).await;
            if !confirmed {
                f.fail.store(true, Ordering::Relaxed);
            }
            f.fire(true, std::slice::from_ref(&sub)).await;
            assert!(observation(&f, &sub).await.is_none());
            if confirmed {
                // Capture has finished once the frame is handed to transport;
                // a later write must not become the ACK's delivered baseline.
                tokio::time::timeout(Duration::from_secs(2), f.entered.notified())
                    .await
                    .unwrap();
                f.entered.notify_one(); // finish consumes the same send evidence.
                write(
                    &f,
                    object(),
                    PropertyIdentifier::PRESENT_VALUE,
                    PropertyValue::Real(12.0),
                )
                .await;
                f.finish(true).await;
            } else {
                f.fail.store(false, Ordering::Relaxed);
                f.sent.lock().unwrap().clear();
                f.fire(true, std::slice::from_ref(&sub)).await;
                f.finish(false).await;
            }
            let baseline = observation(&f, &sub).await.unwrap();
            let (pv, priority) = baseline.source_companions().unwrap();
            assert_eq!(pv.value(), &PropertyValue::Real(10.0));
            assert!(priority.is_none(), "noncommandable baseline has priority");
        }
    }
}

#[tokio::test]
async fn value_source_cov_noncommandable_held_send_keeps_snapshot_and_renewal_fence() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for renew in [false, true] {
            let obj = families(PresentValueAccess::Writable, true).remove(0);
            let f = fixture(obj, true).await;
            write(
                &f,
                object(),
                PropertyIdentifier::PRESENT_VALUE,
                PropertyValue::Real(10.0),
            )
            .await;
            let sub = subscribe(&f, object(), kind, false).await;
            let snapshots = [sub.clone()];
            let mut older = Box::pin(f.fire(true, &snapshots));
            assert!(futures_util::poll!(older.as_mut()).is_pending());
            let delivered = decoded(f.sent.lock().unwrap()[0].clone(), kind);
            write(
                &f,
                object(),
                PropertyIdentifier::PRESENT_VALUE,
                PropertyValue::Real(12.0),
            )
            .await;
            if renew {
                subscribe(&f, object(), kind, false).await;
            }
            f.release.add_permits(1);
            older.await;
            let baseline = observation(&f, &sub).await;
            if renew {
                assert!(baseline.is_none(), "older send overwrote renewed baseline");
            } else {
                let baseline = baseline.unwrap();
                let (pv, priority) = baseline.source_companions().unwrap();
                assert_eq!(pv.value(), &PropertyValue::Real(10.0));
                assert!(priority.is_none());
                for (property, sample) in [
                    (PropertyIdentifier::PRESENT_VALUE, pv),
                    (PropertyIdentifier::VALUE_SOURCE, baseline.sample()),
                ] {
                    let mut encoded = BytesMut::new();
                    encode_property_value(&mut encoded, sample.value()).unwrap();
                    assert_eq!(
                        delivered.iter().find(|v| v.0 == property).unwrap().1,
                        encoded
                    );
                }
            }
            f.sent.lock().unwrap().clear();
            f.release.add_permits(1);
            fire(&f, object()).await;
            assert_report(&f, object(), kind).await;
            f.finish(false).await;
        }
    }
}
