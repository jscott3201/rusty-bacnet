//! A peer's write of the Device's zone is audited like any other write
//! (#2395): an Audit Reporter monitoring the Device reports it, with the
//! property written and its value.
use super::*;
use crate::server::clock::{ClockConfig, ServerClock};
use bacnet_objects::clock::ClockReader;

#[tokio::test]
async fn a_peer_zone_write_is_audited_like_any_write() {
    let mut fixture = plural(vec![configured(
        1,
        Some(vec![Selector::ObjectType(ObjectType::DEVICE)]),
        true,
    )])
    .await;
    // The fixture's server has no clock: give its Device one that takes
    // the opt-in.
    let clock =
        Arc::new(ServerClock::new(ClockConfig::new(300, false).unwrap()).taking_zone_writes(true));
    fixture
        .server
        .db
        .write()
        .await
        .set_clock_reader(Some(Arc::clone(&clock) as Arc<dyn ClockReader>));
    let device = oid(ObjectType::DEVICE, 10);
    let response = dispatch(
        &fixture.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            device,
            PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS,
            vec![0x11],
            None,
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    assert!(clock.read_clock().unwrap().daylight_savings_status);
    settle().await;
    let records = records(&fixture);
    assert_eq!(records.len(), 1, "{records:?}");
    let record = &records[0];
    assert_eq!(record.operation, AuditOperation::WRITE);
    assert_eq!(record.target_object, Some(device));
    assert_eq!(
        record
            .target_property
            .as_ref()
            .map(|p| p.property_identifier),
        Some(PropertyIdentifier::DAYLIGHT_SAVINGS_STATUS)
    );
    assert_eq!(record.target_value, Some(vec![0x11]));
    assert_eq!(record.result, None);
    fixture.server.stop().await.unwrap();
}
