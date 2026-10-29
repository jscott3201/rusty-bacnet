use super::device_bindings::{
    DeviceBindingTable, MAX_DEVICE_BINDINGS, OBSERVED_BINDING_TTL, ObservationOutcome,
};
use super::event_recipient_routing_tests::{
    LITERAL_BROADCAST_MAC, destination_for, distribute_counted,
    distribute_from_database_with_bindings, npdu_destination, split_who_is,
};
use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::EngineeringUnits;

const LOCAL_PEER: &[u8] = &[127, 0, 0, 2, 0xBA, 0xC0];
const ROUTER: &[u8] = &[127, 0, 0, 3, 0xBA, 0xC0];
const FINAL_PEER: &[u8] = &[0x33, 0x44, 0x55];

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

fn binding_database(recipients: &[(ObjectIdentifier, bool)]) -> ObjectDatabase {
    let mut db = clocked_test_database();
    let mut class = NotificationClass::new(0, "NC-0").unwrap();
    for (identifier, confirmed) in recipients {
        class
            .add_destination(destination_for(
                BACnetRecipient::Device(*identifier),
                *confirmed,
            ))
            .unwrap();
    }
    db.add(Box::new(class)).unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    db
}

async fn distribute(
    recipients: &[(ObjectIdentifier, bool)],
    table: DeviceBindingTable,
) -> (Vec<bytes::Bytes>, Vec<(Vec<u8>, bytes::Bytes)>) {
    distribute_from_database_with_bindings(
        binding_database(recipients),
        Arc::new(RwLock::new(table)),
    )
    .await
}

#[tokio::test]
async fn configured_local_device_recipient_is_direct_unicast_confirmed_and_unconfirmed() {
    for confirmed in [false, true] {
        let identifier = device(20 + u32::from(confirmed));
        let mut table = DeviceBindingTable::new();
        table
            .insert_configured(
                DeviceBinding::local(identifier, LOCAL_PEER).unwrap(),
                |_| false,
            )
            .unwrap();
        let (broadcasts, unicasts) = distribute(&[(identifier, confirmed)], table).await;
        assert!(broadcasts.is_empty(), "Device unicast never broadcasts");
        assert_eq!(unicasts.len(), 1);
        assert_eq!(unicasts[0].0.as_slice(), LOCAL_PEER);
        assert_eq!(npdu_destination(&unicasts[0].1), None);
    }
}

#[tokio::test]
async fn configured_and_observed_routed_device_recipients_use_exact_router_and_final_address() {
    for configured in [true, false] {
        let identifier = device(30 + u32::from(configured));
        let mut table = DeviceBindingTable::new();
        if configured {
            table
                .insert_configured(
                    DeviceBinding::routed(identifier, 700, FINAL_PEER, ROUTER).unwrap(),
                    |_| false,
                )
                .unwrap();
        } else {
            assert_eq!(
                table.observe_i_am_at(
                    identifier,
                    ROUTER,
                    Some(&NpduAddress {
                        network: 700,
                        mac_address: MacAddr::from_slice(FINAL_PEER),
                    }),
                    Instant::now(),
                    |_| false,
                ),
                ObservationOutcome::Inserted
            );
        }

        let (broadcasts, unicasts) = distribute(&[(identifier, false)], table).await;
        assert!(broadcasts.is_empty());
        assert_eq!(unicasts.len(), 1);
        assert_eq!(unicasts[0].0.as_slice(), ROUTER);
        assert_eq!(
            npdu_destination(&unicasts[0].1),
            Some((700, FINAL_PEER.to_vec()))
        );
    }
}

#[tokio::test]
async fn unknown_stale_invalid_and_capacity_rejected_devices_emit_zero_frames() {
    let now = Instant::now();
    let stale = device(40);
    let invalid = device(41);
    let unknown = device(42);
    let rejected = device(900_000);
    let mut table = DeviceBindingTable::new();
    assert_eq!(
        table.observe_i_am_at(stale, LOCAL_PEER, None, now, |_| false),
        ObservationOutcome::Inserted
    );
    table
        .insert_configured(
            DeviceBinding::local(invalid, LITERAL_BROADCAST_MAC).unwrap(),
            |_| false,
        )
        .unwrap();
    for instance in 1000..1000 + MAX_DEVICE_BINDINGS as u32 - 2 {
        assert_eq!(
            table.observe_i_am_at(device(instance), LOCAL_PEER, None, now, |_| false),
            ObservationOutcome::Inserted
        );
    }
    assert_eq!(table.len(), MAX_DEVICE_BINDINGS);
    assert_eq!(
        table.observe_i_am_at(rejected, LOCAL_PEER, None, now, |_| false),
        ObservationOutcome::RejectedCapacity
    );
    assert_eq!(
        table.observe_i_am_at(stale, LOCAL_PEER, None, now - OBSERVED_BINDING_TTL, |_| {
            false
        },),
        ObservationOutcome::Refreshed
    );

    let (broadcasts, unicasts, counters) = distribute_counted(
        binding_database(&[
            (unknown, false),
            (stale, false),
            (invalid, false),
            (rejected, false),
        ]),
        Arc::new(RwLock::new(table)),
        DccState::Enable,
    )
    .await;
    // The three unbound Devices are looked for, each with one Who-Is
    // (#1368); none answers, and no notification goes anywhere.
    let (who_is, broadcasts) = split_who_is(broadcasts);
    assert_eq!(who_is.len(), 3);
    assert!(broadcasts.is_empty());
    assert!(unicasts.is_empty());
    // A Device the full table refused is as unbound as one never seen (#1160).
    assert_eq!(
        counters,
        EventNotificationCounters {
            device_recipient_unbound: 3,
            recipient_unroutable: 1,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn mixed_valid_and_unresolved_device_recipients_preserve_valid_delivery() {
    let valid = device(50);
    let mut table = DeviceBindingTable::new();
    table
        .insert_configured(DeviceBinding::local(valid, LOCAL_PEER).unwrap(), |_| false)
        .unwrap();
    let (broadcasts, unicasts) = distribute(&[(device(51), false), (valid, false)], table).await;
    // Device 51 is looked for with one Who-Is (#1368).
    let (who_is, broadcasts) = split_who_is(broadcasts);
    assert_eq!(who_is.len(), 1);
    assert!(broadcasts.is_empty());
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0.as_slice(), LOCAL_PEER);
}
