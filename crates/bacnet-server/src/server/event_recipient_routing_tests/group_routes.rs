//! Recipients at a group address that isn't the link's own broadcast MAC,
//! such as a B/IP multicast address or the broadcast IP at another port
//! (#1493). A confirmed notification goes to one device, so a confirmed
//! recipient there gets nothing and counts with the confirmed broadcast
//! recipients, while an unconfirmed one is still sent, as the link's local
//! broadcast, which a group address with no DNET is (#1677). A
//! Device binding to a group address names no device and is unroutable, one
//! routed through this network's own number to a group final MAC included.

use super::super::device_bindings::DeviceBindingTable;
use super::*;

/// An IPv4 multicast address and the subnet broadcast at another port: both
/// reach a group of nodes, neither is the link's own broadcast MAC.
const GROUPS: [[u8; 6]; 2] = [[224, 0, 0, 1, 0xBA, 0xC0], [127, 255, 255, 255, 0xBA, 0xC1]];
const PEER: [u8; 6] = [127, 0, 0, 1, 0xBA, 0xC1];
const THIS_NETWORK: u16 = 7;
/// Configured Devices bound to a group address, locally and as a router.
const GROUP_PEER: u32 = 60;
const GROUP_ROUTER: u32 = 61;
/// A Device bound through a station router on this network's own number, at
/// a multicast final MAC: a local binding at that MAC once the number is
/// known, so it names no device either.
const GROUP_FINAL: u32 = 62;
const ROUTER: [u8; 6] = [127, 0, 0, 3, 0xBA, 0xC0];

/// [`routing_transport`] whose group rule also takes in [`GROUPS`].
fn group_transport() -> (TestTransport, SendLog) {
    let mut builder = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(LITERAL_BROADCAST_MAC);
    for group in GROUPS {
        builder = builder.group_mac(&group);
    }
    let transport = builder.build();
    let sent = transport.sent();
    (transport, sent)
}

fn device(instance: u32) -> BACnetRecipient {
    BACnetRecipient::Device(ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap())
}

/// Bindings to a group address, in a table built without the link's check,
/// as no started server could hold the first two. The third passes that
/// check, its router being a station, and fails only once localized.
fn group_bindings() -> Arc<RwLock<DeviceBindingTable>> {
    let id = |instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
    let mut table = DeviceBindingTable::new();
    for binding in [
        DeviceBinding::local(id(GROUP_PEER), GROUPS[0]),
        DeviceBinding::routed(id(GROUP_ROUTER), 700, [0x33], GROUPS[1]),
        DeviceBinding::routed(id(GROUP_FINAL), THIS_NETWORK, GROUPS[0], ROUTER),
    ] {
        table
            .insert_configured(binding.unwrap(), |_| false)
            .unwrap();
    }
    Arc::new(RwLock::new(table))
}

/// Fire one transition at a class holding `destination` on a link that
/// knows its network number is [`THIS_NETWORK`].
async fn distribute_one(
    destination: BACnetDestination,
) -> (Vec<Bytes>, Vec<UnicastFrame>, EventNotificationCounters) {
    let mut db = clocked_test_database();
    let mut nc = NotificationClass::new(0, "NC-0").unwrap();
    nc.add_destination(destination).unwrap();
    db.add(Box::new(nc)).unwrap();
    let (transport, sent) = group_transport();
    let network = Arc::new(NetworkLayer::new(transport));
    let state = bacnet_types::network_number::NetworkNumber::configured(THIS_NETWORK).unwrap();
    network.local_network_number().publish(state);
    distribute_counted_through(&network, &sent, db, group_bindings(), DccState::Enable).await
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_recipient_at_a_group_address_gets_nothing_and_is_counted() {
    let counted = EventNotificationCounters {
        confirmed_broadcast_recipient: 1,
        ..Default::default()
    };
    for group in GROUPS {
        // On network zero, and on this network by number, which is local.
        for network in [0, THIS_NETWORK] {
            let (broadcasts, unicasts, counters) =
                distribute_one(destination_for(address_recipient(network, &group), true)).await;
            assert!(
                broadcasts.is_empty() && unicasts.is_empty(),
                "{group:?} on {network}: nothing is sent"
            );
            assert_eq!(counters, counted, "{group:?} on {network}");
        }
    }
}

#[tokio::test(start_paused = true)]
async fn an_unconfirmed_recipient_at_a_group_address_is_sent_as_a_local_broadcast() {
    for group in GROUPS {
        for network in [0, THIS_NETWORK] {
            let (broadcasts, unicasts, counters) =
                distribute_one(destination_for(address_recipient(network, &group), false)).await;
            assert_eq!(broadcasts.len(), 1, "{group:?} on {network}");
            assert!(unicasts.is_empty(), "{group:?} on {network}");
            assert_eq!(counters, EventNotificationCounters::default());
        }
    }
}

/// The group check leaves a confirmed recipient at a station's address alone.
#[tokio::test(start_paused = true)]
async fn a_confirmed_recipient_at_a_station_is_still_sent_to() {
    let (_, unicasts, counters) =
        distribute_one(destination_for(address_recipient(0, &PEER), true)).await;
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0, PEER);
    assert_eq!(counters, EventNotificationCounters::default());
}

#[tokio::test(start_paused = true)]
async fn a_device_bound_to_a_group_address_is_unroutable() {
    let unroutable = EventNotificationCounters {
        recipient_unroutable: 1,
        ..Default::default()
    };
    for instance in [GROUP_PEER, GROUP_ROUTER, GROUP_FINAL] {
        for confirmed in [false, true] {
            let (broadcasts, unicasts, counters) =
                distribute_one(destination_for(device(instance), confirmed)).await;
            assert!(
                broadcasts.is_empty() && unicasts.is_empty(),
                "Device {instance}, confirmed {confirmed}"
            );
            assert_eq!(
                counters, unroutable,
                "Device {instance}, confirmed {confirmed}"
            );
        }
    }
}
