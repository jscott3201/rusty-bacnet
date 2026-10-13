//! A matched destination skipped while its route is resolved moves its own
//! undelivered-notification counter once, and a delivered one moves none
//! (#1160).

use super::super::device_bindings::{DeviceBindingTable, ObservationOutcome, OBSERVED_BINDING_TTL};
use super::*;

const PEER: [u8; 6] = [127, 0, 0, 1, 0xBA, 0xC1];
const ROUTER: [u8; 6] = [127, 0, 0, 3, 0xBA, 0xC0];
/// The observed binding of this Device has expired.
const STALE: u32 = 40;
/// Bound to the link's broadcast MAC. Startup refuses such a binding, so
/// only a table built without the link's broadcast check holds one.
const BROADCAST_BOUND: u32 = 41;
/// No binding at all.
const UNKNOWN: u32 = 42;
const BOUND_LOCAL: u32 = 50;
const BOUND_ROUTED: u32 = 51;

fn device(instance: u32) -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap())
}

fn bindings() -> Arc<RwLock<DeviceBindingTable>> {
    let id = |instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
    let mut table = DeviceBindingTable::new();
    let expired = runtime_clock::now() - OBSERVED_BINDING_TTL;
    assert_eq!(
        table.observe_i_am_at(id(STALE), &PEER, None, expired, |_| false),
        ObservationOutcome::Inserted
    );
    for binding in [
        DeviceBinding::local(id(BROADCAST_BOUND), LITERAL_BROADCAST_MAC),
        DeviceBinding::local(id(BOUND_LOCAL), PEER),
        DeviceBinding::routed(id(BOUND_ROUTED), 700, [0x33], ROUTER),
    ] {
        table
            .insert_configured(binding.unwrap(), |_| false)
            .unwrap();
    }
    Arc::new(RwLock::new(table))
}

/// Fire one transition at a Notification Class holding exactly
/// `destinations`, against [`bindings`].
async fn distribute(
    destinations: Vec<BACnetDestination>,
) -> (Vec<Bytes>, Vec<UnicastFrame>, EventNotificationCounters) {
    let mut db = clocked_test_database();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    for destination in destinations {
        nc.add_destination(destination).unwrap();
    }
    db.add(Box::new(nc)).unwrap();
    distribute_counted(db, bindings(), DccState::Enable).await
}

#[tokio::test]
async fn each_route_skip_moves_only_its_own_counter_once() {
    let none = EventNotificationCounters::default();
    let unbound = EventNotificationCounters {
        device_recipient_unbound: 1,
        ..none
    };
    let unroutable = EventNotificationCounters {
        recipient_unroutable: 1,
        ..none
    };
    let confirmed_broadcast = EventNotificationCounters {
        confirmed_broadcast_recipient: 1,
        ..none
    };
    let not_a_device =
        BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::ANALOG_INPUT, UNKNOWN).unwrap());
    for (case, recipient, confirmed, expected) in [
        ("unknown Device", device(UNKNOWN), false, unbound),
        // An unbound Device is not also a confirmed-broadcast skip.
        ("unknown Device, confirmed", device(UNKNOWN), true, unbound),
        ("expired Device binding", device(STALE), false, unbound),
        (
            "broadcast-bound Device",
            device(BROADCAST_BOUND),
            false,
            unroutable,
        ),
        ("non-Device identifier", not_a_device, false, unroutable),
        (
            "MAC on network 65535",
            address_recipient(65535, &PEER),
            false,
            unroutable,
        ),
        (
            "MAC on network 65535, confirmed",
            address_recipient(65535, &PEER),
            true,
            unroutable,
        ),
        (
            "confirmed local broadcast",
            address_recipient(0, &[]),
            true,
            confirmed_broadcast,
        ),
        (
            "confirmed literal broadcast MAC",
            address_recipient(0, LITERAL_BROADCAST_MAC),
            true,
            confirmed_broadcast,
        ),
        (
            "confirmed remote broadcast",
            address_recipient(1000, &[]),
            true,
            confirmed_broadcast,
        ),
        (
            "confirmed global broadcast",
            address_recipient(65535, &[]),
            true,
            confirmed_broadcast,
        ),
    ] {
        let (broadcasts, unicasts, counters) =
            distribute(vec![destination_for(recipient, confirmed)]).await;
        // An unbound Device is looked for with one Who-Is first (#1368).
        let (who_is, broadcasts) = split_who_is(broadcasts);
        assert_eq!(who_is.len(), usize::from(expected == unbound), "{case}");
        assert!(broadcasts.is_empty() && unicasts.is_empty(), "{case}");
        assert_eq!(counters, expected, "{case}");
    }
}

/// Route skips count per destination: two unbound Devices count two, and the
/// transition's deliverable destinations are still served.
#[tokio::test]
async fn one_transition_counts_each_skipped_destination_and_serves_the_rest() {
    let (broadcasts, unicasts, counters) = distribute(vec![
        destination_for(device(UNKNOWN), false),
        destination_for(device(STALE), true),
        destination_for(address_recipient(65535, &PEER), false),
        destination_for(address_recipient(0, &[]), true),
        destination_for(address_recipient(0, &PEER), false),
        destination_for(address_recipient(0, &[]), false),
    ])
    .await;
    assert_eq!(
        counters,
        EventNotificationCounters {
            device_recipient_unbound: 2,
            recipient_unroutable: 1,
            confirmed_broadcast_recipient: 1,
            ..Default::default()
        }
    );
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0, PEER);
    // One Who-Is for each unbound Device (#1368), and the one notification.
    let (who_is, broadcasts) = split_who_is(broadcasts);
    assert_eq!(who_is.len(), 2);
    assert_eq!(broadcasts.len(), 1);
}

#[tokio::test]
async fn every_deliverable_route_moves_no_counter() {
    for (case, recipient, confirmed) in [
        ("local unicast", address_recipient(0, &PEER), false),
        ("confirmed local unicast", address_recipient(0, &PEER), true),
        ("local broadcast", address_recipient(0, &[]), false),
        (
            "literal broadcast MAC",
            address_recipient(0, LITERAL_BROADCAST_MAC),
            false,
        ),
        ("remote broadcast", address_recipient(1000, &[]), false),
        ("global broadcast", address_recipient(65535, &[]), false),
        ("remote unicast", address_recipient(1000, &PEER), false),
        (
            "confirmed remote unicast",
            address_recipient(1000, &PEER),
            true,
        ),
        ("bound local Device", device(BOUND_LOCAL), false),
        ("confirmed bound local Device", device(BOUND_LOCAL), true),
        ("bound routed Device", device(BOUND_ROUTED), false),
        ("confirmed bound routed Device", device(BOUND_ROUTED), true),
    ] {
        let (broadcasts, unicasts, counters) =
            distribute(vec![destination_for(recipient, confirmed)]).await;
        assert_eq!(broadcasts.len() + unicasts.len(), 1, "{case} is sent");
        assert_eq!(counters, EventNotificationCounters::default(), "{case}");
    }
}
