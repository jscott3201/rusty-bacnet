//! Entries keyed by monitored object first (#2303).
use super::*;
use crate::cov::CovNotificationKind;
use alloc::vec;
use bacnet_time::Duration;
use bacnet_types::MacAddr;
use bacnet_types::enums::ObjectType;

const T0: Instant = Instant::from_secs(1_000);

fn sub(
    mac: u8,
    routed: bool,
    process: u32,
    object: ObjectIdentifier,
    property: Option<PropertyIdentifier>,
) -> CovSubscription {
    CovSubscription {
        subscriber_mac: MacAddr::from_slice(&[mac]),
        subscriber_network: routed.then(|| NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[mac]),
        }),
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: false,
        expires_at: Some(T0 + Duration::from_secs(60)),
        last_notified_observation: None,
        monitored_property: property,
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    }
}

#[test]
fn an_objects_entries_are_found_together_whatever_their_family_or_recipient() {
    let objects = [
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 2).unwrap(),
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 0).unwrap(),
        ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
    ];
    let mut table = CovTable::default();
    for (n, object) in objects.into_iter().enumerate() {
        for (mac, routed, process, property) in [
            (9, true, 0, None),
            (1, false, 7, Some(PropertyIdentifier::PRESENT_VALUE)),
            (0, false, 0, None),
            (3, true, 2, Some(PropertyIdentifier::STATUS_FLAGS)),
        ] {
            let mut proposal = sub(mac, routed, process, object, property);
            proposal.subscriber_process_identifier += n as u32;
            table.subscribe(proposal, T0).unwrap();
        }
        // A Multiple reference too.
        let mut multiple = sub(4, false, 1, object, Some(PropertyIdentifier::PRESENT_VALUE));
        multiple.notification_kind = CovNotificationKind::Multiple;
        let context = multiple.key().unwrap().multiple_context().unwrap().clone();
        let (route, expires) = (multiple.endpoint(), multiple.expires_at.unwrap());
        table
            .subscribe_multiple(&context, &route, expires, 0, None, vec![multiple], T0)
            .unwrap();
    }
    assert_eq!(table.len(), 15);
    for object in objects {
        let found: Vec<_> = table.object_entries(object).collect();
        assert_eq!(found.len(), 5, "{object:?}");
        assert!(found.iter().all(|entry| entry.key().object() == object));
        assert_eq!(table.subscriptions_for(&object, T0).len(), 5);
    }
    let ordered: Vec<_> = table.entries().map(|entry| entry.key().object()).collect();
    let mut sorted = ordered.clone();
    sorted.sort();
    assert_eq!(ordered, sorted, "entries iterate by object");
    table.remove_for_object(objects[0]);
    assert_eq!(table.len(), 10);
    assert_eq!(table.object_entries(objects[0]).count(), 0);
}
