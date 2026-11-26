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

/// Tags storage that never saves.
struct Failing;

impl bacnet_objects::object_profile::TagsPersistence for Failing {
    fn load(
        &self,
        _: ObjectIdentifier,
    ) -> Result<Option<bacnet_objects::object_profile::TagsSnapshot>, Error> {
        Ok(None)
    }
    fn save(
        &self,
        _: ObjectIdentifier,
        _: &bacnet_objects::object_profile::TagsSnapshot,
    ) -> Result<(), Error> {
        Err(Error::from(bacnet_types::error::PersistenceError::new(
            bacnet_types::error::PersistenceErrorKind::BackendFailed,
            "storage unavailable",
        )))
    }
}

/// A refused zone write, and one whose save fails, are audited as failed
/// writes, each with its error; the clock stays.
#[tokio::test]
async fn refused_and_unsaved_zone_writes_are_audited_with_their_errors() {
    // A second Device, whose Tags storage can't save.
    let config = DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(11, 0).unwrap(),
        ..Default::default()
    };
    let unsaved = DeviceObject::with_tags_persistence(config, Arc::new(Failing)).unwrap();
    let mut fixture = try_servers_with(
        vec![configured(
            1,
            Some(vec![Selector::ObjectType(ObjectType::DEVICE)]),
            true,
        )],
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
        (true, 1476, AuditCapture::default()),
        vec![Box::new(unsaved)],
    )
    .await
    .unwrap();
    let kept = oid(ObjectType::DEVICE, 11);
    let clock =
        Arc::new(ServerClock::new(ClockConfig::new(300, false).unwrap()).taking_zone_writes(true));
    fixture
        .server
        .db
        .write()
        .await
        .set_clock_reader(Some(Arc::clone(&clock) as Arc<dyn ClockReader>));
    let device = oid(ObjectType::DEVICE, 10);
    let off_grid = wp(device, PropertyIdentifier::UTC_OFFSET, vec![0x31, 7], None);
    let unsaved = wp(kept, PropertyIdentifier::UTC_OFFSET, vec![0x31, 60], None);
    for request in [off_grid, unsaved] {
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let response = dispatch(&fixture.server, service, request).await;
        assert!(matches!(response, Apdu::Error(_)), "{response:?}");
    }
    assert_eq!(clock.read_clock().unwrap().utc_offset, 300);
    settle().await;
    let records = records(&fixture);
    assert_eq!(records.len(), 2, "{records:?}");
    for record in &records {
        assert_eq!(record.operation, AuditOperation::WRITE);
        assert!(record.result.is_some(), "{record:?}");
    }
    assert_eq!(records[1].target_object, Some(kept));
    fixture.server.stop().await.unwrap();
}
