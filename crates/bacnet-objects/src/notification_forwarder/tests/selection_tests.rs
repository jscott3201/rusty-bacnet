//! Which notifications a forwarder takes, and where they go (Clause 12.51).

use super::*;
use crate::database::ObjectDatabase;
use bacnet_types::enums::{EventState, PropertyIdentifier as P};

const NOON: Time = Time {
    hour: 12,
    minute: 0,
    second: 0,
    hundredths: 0,
};

fn input(process_identifier: u32) -> ForwardingInput<'static> {
    ForwardingInput {
        process_identifier,
        to_state: EventState::HIGH_LIMIT,
        locally_initiated: false,
        receiving_port: Some(0),
        today: DaysOfWeek::MONDAY,
        current_time: &NOON,
    }
}

/// A forwarder holding one Recipient_List destination (process 10) and one
/// subscription (process 20).
fn forwarder(instance: u32) -> NotificationForwarderObject {
    let mut nf = NotificationForwarderObject::new(instance, format!("NF-{instance}")).unwrap();
    nf.add_destination(destination(address(0, &[instance as u8]), 10, true))
        .unwrap();
    nf.write_property(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        framed_subscriptions(&[subscription(device(100 + instance), 20, 30)]),
        None,
    )
    .unwrap();
    nf
}

fn database(forwarders: Vec<NotificationForwarderObject>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    for nf in forwarders {
        db.add(Box::new(nf)).unwrap();
    }
    db
}

fn recipients(
    db: &ObjectDatabase,
    input: &ForwardingInput<'_>,
) -> Vec<(BACnetRecipient, u32, bool)> {
    forwarding_targets(db, input, &[]).recipients
}

#[test]
fn forwarder_sends_to_its_recipient_list_and_its_subscriptions() {
    let db = database(vec![forwarder(1)]);
    let selected = forwarding_targets(&db, &input(5), &[]);
    assert_eq!(
        selected.forwarders,
        [ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap()]
    );
    assert_eq!(
        selected.recipients,
        [(address(0, &[1]), 10, true), (device(101), 20, false)]
    );
}

#[test]
fn process_identifier_filter_selects_the_notifications_taken() {
    let mut only_seven = forwarder(1);
    only_seven.set_process_identifier_filter(Some(7));
    let db = database(vec![only_seven]);
    assert!(recipients(&db, &input(5)).is_empty());
    assert_eq!(recipients(&db, &input(7)).len(), 2);
}

#[test]
fn local_forwarding_only_takes_just_this_devices_notifications() {
    let mut local_only = forwarder(1);
    local_only.set_local_forwarding_only(true);
    let db = database(vec![local_only]);
    assert!(recipients(&db, &input(5)).is_empty());
    let local = ForwardingInput {
        locally_initiated: true,
        receiving_port: None,
        ..input(5)
    };
    assert_eq!(recipients(&db, &local).len(), 2);
}

#[test]
fn out_of_service_forwarder_takes_nothing() {
    let mut idle = forwarder(1);
    idle.write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    let db = database(vec![idle]);
    assert!(
        forwarding_targets(&db, &input(5), &[])
            .forwarders
            .is_empty()
    );
}

#[test]
fn port_filter_drops_notifications_from_a_disabled_port() {
    let mut filtered = forwarder(1);
    filtered.set_port_filter(Some(vec![BACnetPortPermission {
        port_id: 0,
        enabled: false,
    }]));
    let db = database(vec![filtered]);
    assert!(recipients(&db, &input(5)).is_empty());
    // A port the array does not name is not enabled either.
    let other_port = ForwardingInput {
        receiving_port: Some(3),
        ..input(5)
    };
    assert!(recipients(&db, &other_port).is_empty());
    // Port_Filter governs only notifications received through a port.
    let local = ForwardingInput {
        locally_initiated: true,
        receiving_port: None,
        ..input(5)
    };
    assert_eq!(recipients(&db, &local).len(), 2);

    let mut enabled = forwarder(1);
    enabled.set_port_filter(Some(vec![BACnetPortPermission {
        port_id: 0,
        enabled: true,
    }]));
    assert_eq!(recipients(&database(vec![enabled]), &input(5)).len(), 2);
}

#[test]
fn recipient_list_filters_by_day_time_and_transition() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    let mut weekend = destination(address(0, &[1]), 1, false);
    weekend.valid_days = DaysOfWeek::SATURDAY | DaysOfWeek::SUNDAY;
    let mut mornings = destination(address(0, &[2]), 2, false);
    mornings.to_time = Time {
        hour: 11,
        ..mornings.to_time
    };
    let mut to_normal = destination(address(0, &[3]), 3, false);
    to_normal.transitions = EventTransitionBits::TO_NORMAL;
    let open = destination(address(0, &[4]), 4, false);
    for entry in [weekend, mornings, to_normal, open] {
        nf.add_destination(entry).unwrap();
    }
    let db = database(vec![nf]);
    assert_eq!(recipients(&db, &input(5)), [(address(0, &[4]), 4, false)]);
    let back_to_normal = ForwardingInput {
        to_state: EventState::NORMAL,
        ..input(5)
    };
    assert_eq!(
        recipients(&db, &back_to_normal),
        [(address(0, &[3]), 3, false), (address(0, &[4]), 4, false)]
    );
}

#[test]
fn a_lapsed_subscription_is_not_a_destination() {
    let mut nf = NotificationForwarderObject::new(1, "NF").unwrap();
    let (clock, set) = manual_clock();
    nf.write_property(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        framed_subscriptions(&[subscription(device(7), 1, 1), subscription(device(8), 1, 2)]),
        None,
    )
    .unwrap();
    let mut db = database(vec![nf]);
    db.set_monotonic_clock_internal(Some(clock));
    assert_eq!(recipients(&db, &input(5)).len(), 2);
    // The first entry's minute is up; it is no longer served even before the
    // operation task drops it.
    set(MINUTE);
    assert_eq!(recipients(&db, &input(5)), [(device(8), 1, false)]);
}

#[test]
fn shared_destinations_go_once_and_skipped_forwarders_take_nothing() {
    let mut twin = forwarder(2);
    twin.add_destination(destination(address(0, &[1]), 10, true))
        .unwrap();
    let db = database(vec![forwarder(1), twin]);
    let selected = forwarding_targets(&db, &input(5), &[]);
    assert_eq!(selected.forwarders.len(), 2);
    assert_eq!(
        selected.recipients,
        [
            (address(0, &[1]), 10, true),
            (device(101), 20, false),
            (address(0, &[2]), 10, true),
            (device(102), 20, false),
        ]
    );
    let first = ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, 1).unwrap();
    let rest = forwarding_targets(&db, &input(5), &[first]);
    assert_eq!(
        rest.recipients,
        [
            (address(0, &[2]), 10, true),
            (address(0, &[1]), 10, true),
            (device(102), 20, false),
        ]
    );
}
