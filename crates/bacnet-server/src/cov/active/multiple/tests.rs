use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryValueObject;
use std::time::Duration;

const MULTIPLE: PropertyIdentifier = PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap()
}

fn direct() -> SubscriberEndpoint {
    SubscriberEndpoint::new(&[0x0A, 0, 0, 5, 0xBA, 0xC0], None)
}

fn routed() -> SubscriberEndpoint {
    SubscriberEndpoint::new(
        &[0x0A, 0, 0, 1, 0xBA, 0xC0],
        Some(&NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[0x33]),
        }),
    )
}

fn context(endpoint: SubscriberEndpoint, process_id: u32, confirmed: bool) -> MultipleContextKey {
    MultipleContextKey {
        endpoint,
        process_id,
        confirmed,
    }
}

fn reference(
    context: &MultipleContextKey,
    property: Option<PropertyIdentifier>,
    expires_at: Option<Instant>,
    kind: CovNotificationKind,
) -> CovSubscription {
    CovSubscription {
        subscriber_mac: context.endpoint.mac.clone(),
        subscriber_network: context.endpoint.network.clone(),
        subscriber_process_identifier: context.process_id,
        monitored_object_identifier: av(1),
        issue_confirmed_notifications: context.confirmed,
        expires_at,
        last_notified_observation: None,
        monitored_property: property,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: kind,
        timestamped: false,
    }
}

#[test]
fn active_cov_multiple_entries_follow_context_refresh_without_purging_or_single() {
    let now = Instant::now();
    let unconfirmed = context(direct(), 2, false);
    let confirmed = context(direct(), 2, true);
    let mut table = CovSubscriptionTable::new();
    let single = reference(&unconfirmed, Some(PV), None, CovNotificationKind::Single);
    table.subscribe(single.clone()).unwrap();
    // Generic admission cannot bypass the context owner.
    let mut bypass = single;
    bypass.notification_kind = CovNotificationKind::Multiple;
    bypass.expires_at = Some(now + Duration::from_secs(60));
    assert!(matches!(table.subscribe(bypass), Err(Error::Encoding(_))));
    assert_eq!(table.len(), 1);

    let first = now + Duration::from_millis(1_500);
    let mut status = reference(
        &unconfirmed,
        Some(PV),
        Some(first),
        CovNotificationKind::Multiple,
    );
    status.monitored_property = Some(PropertyIdentifier::STATUS_FLAGS);
    let present = reference(
        &unconfirmed,
        Some(PV),
        Some(first),
        CovNotificationKind::Multiple,
    );
    table
        .subscribe_multiple(&unconfirmed, first, 5, vec![present, status])
        .unwrap();
    let later = now + Duration::from_secs(60);
    let other_form = reference(
        &confirmed,
        Some(PV),
        Some(later),
        CovNotificationKind::Multiple,
    );
    table
        .subscribe_multiple(&confirmed, later, 7, vec![other_form])
        .unwrap();

    let rows = |table: &CovSubscriptionTable, at| {
        let mut rows: Vec<_> = table
            .active_cov_multiple_entries(at)
            .into_iter()
            .map(|entry| {
                (
                    entry.context.confirmed,
                    entry.property.to_raw(),
                    entry.time_remaining,
                    entry.max_notification_delay,
                )
            })
            .collect();
        rows.sort_unstable();
        rows
    };
    let pv = PV.to_raw();
    let flags = PropertyIdentifier::STATUS_FLAGS.to_raw();
    assert_eq!(
        rows(&table, now),
        vec![(false, pv, 2, 5), (false, flags, 2, 5), (true, pv, 60, 7)],
        "Single excluded; finite rounds up; each form keeps its own delay"
    );

    // An expiry-only renewal refreshes the whole unconfirmed context's
    // lifetime and delay (last write wins) and leaves the other form alone.
    table
        .subscribe_multiple(&unconfirmed, now + Duration::from_secs(10), 9, vec![])
        .unwrap();
    assert_eq!(
        rows(&table, now),
        vec![(false, pv, 10, 9), (false, flags, 10, 9), (true, pv, 60, 7)]
    );
    // The refreshed context's deadline is reached: omitted, but not purged.
    assert_eq!(
        rows(&table, now + Duration::from_secs(10)),
        vec![(true, pv, 50, 7)]
    );
    assert_eq!(table.len(), 4);
}

#[test]
fn active_cov_multiple_projection_groups_contexts_and_omits_deleted_objects() {
    let mut db = ObjectDatabase::new();
    let mut analog = AnalogValueObject::new(1, "AV-1", 62).unwrap();
    analog
        .write_property(
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(2.0),
            None,
        )
        .unwrap();
    db.add(Box::new(analog)).unwrap();
    let bv = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    let entry =
        |context: &MultipleContextKey, object, property, index, explicit_increment, timestamped| {
            ActiveCovMultipleEntry {
                context: context.clone(),
                object,
                property,
                index,
                time_remaining: 30 + context.process_id,
                max_notification_delay: context.process_id,
                explicit_increment,
                timestamped,
            }
        };
    let routed_two = context(routed(), 2, false);
    let direct_three = context(direct(), 3, true);
    let deleted_only = context(direct(), 3, false);
    let direct_one = context(direct(), 1, false);
    let array = PropertyIdentifier::PRIORITY_ARRAY;
    let entries = vec![
        entry(&routed_two, av(1), PV, None, Some(0.5), true),
        entry(&direct_three, bv, PV, None, None, false),
        entry(&direct_three, av(1), array, Some(0), Some(1.0), false),
        // Deleted before table cleanup: terminated, so omitted, and a context
        // left without references is removed entirely.
        entry(&deleted_only, av(2), PV, None, None, false),
        entry(&direct_one, av(2), PV, None, None, false),
        entry(&direct_three, av(1), PV, None, None, false),
        entry(&direct_one, av(1), PV, None, None, false),
    ];
    let projected = ActiveCovMultipleSubscriptions::project(&db, device(), entries);

    let cov_ref = |property, index, cov_increment, timestamped| BACnetCOVReference {
        property_identifier: property,
        property_array_index: index,
        cov_increment,
        timestamped,
    };
    let spec = |object, list_of_cov_references| BACnetCOVSubscriptionSpecification {
        monitored_object_identifier: object,
        list_of_cov_references,
    };
    let expected = |context: &MultipleContextKey, specs| BACnetCOVMultipleSubscription {
        recipient: BACnetRecipientProcess {
            recipient: recipient(&context.endpoint),
            process_identifier: context.process_id,
        },
        issue_confirmed_notifications: context.confirmed,
        time_remaining: 30 + context.process_id,
        max_notification_delay: context.process_id,
        list_of_cov_subscription_specifications: specs,
    };
    let mut encoded = BytesMut::new();
    encode_cov_multiple_subscription_list(
        &mut encoded,
        &[
            // Numeric Present_Value inherits the object's increment in use.
            expected(
                &direct_one,
                vec![spec(av(1), vec![cov_ref(PV, None, Some(2.0), false)])],
            ),
            // Index zero is the non-numeric array size: no increment even
            // with an explicit override; a binary value has none either.
            expected(
                &direct_three,
                vec![
                    spec(
                        av(1),
                        vec![
                            cov_ref(PV, None, Some(2.0), false),
                            cov_ref(array, Some(0), None, false),
                        ],
                    ),
                    spec(bv, vec![cov_ref(PV, None, None, false)]),
                ],
            ),
            // Routed contexts follow direct ones and report the remote source;
            // an explicit override wins over the object's increment.
            expected(
                &routed_two,
                vec![spec(av(1), vec![cov_ref(PV, None, Some(0.5), true)])],
            ),
        ],
    );
    let routed_recipient = BACnetRecipient::Address(BACnetAddress {
        network_number: 7,
        mac_address: MacAddr::from_slice(&[0x33]),
    });
    assert_eq!(recipient(&routed_two.endpoint), routed_recipient);
    assert_eq!(
        projected.resolve(device(), MULTIPLE),
        Some(PropertyValue::ApplicationData(encoded.to_vec()))
    );
    assert_eq!(
        projected.resolve(device(), PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS),
        None
    );
    let other_device = ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap();
    assert_eq!(projected.resolve(other_device, MULTIPLE), None);
    assert_eq!(
        ActiveCovMultipleSubscriptions::stopped(device()).resolve(device(), MULTIPLE),
        Some(PropertyValue::ApplicationData(Vec::new()))
    );
}

#[test]
fn live_device_cov_resolves_only_selected_lists() {
    let db = ObjectDatabase::new();
    let table = CovSubscriptionTable::new();
    let empty = Some(PropertyValue::ApplicationData(Vec::new()));
    let active = PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS;
    for (selected_active, selected_multiple) in [(true, false), (false, true), (true, true)] {
        let selection = LiveCovSelection {
            device: device(),
            active: selected_active,
            multiple: selected_multiple,
        };
        let entries = table.live_cov_entries(selection, Instant::now());
        for live in [
            LiveDeviceCov::project(&db, selection, entries),
            LiveDeviceCov::stopped(selection),
        ] {
            let expect = |selected: bool| if selected { empty.clone() } else { None };
            assert_eq!(live.resolve(device(), active), expect(selected_active));
            assert_eq!(live.resolve(device(), MULTIPLE), expect(selected_multiple));
            assert_eq!(
                live.resolve(device(), PropertyIdentifier::OBJECT_NAME),
                None
            );
        }
    }
}
