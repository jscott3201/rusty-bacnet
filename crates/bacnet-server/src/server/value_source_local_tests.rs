//! Local writes to objects that track the source of a noncommandable
//! Present_Value (#1552): Clause 19.5 has a write the local device makes
//! publish this Device, or the local object that started it, and the Device
//! owns the source then. Device 856 is `02 00 03 58`.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::color::ColorObject;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_types::enums::ObjectType;

const VS: PropertyIdentifier = PropertyIdentifier::VALUE_SOURCE;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

fn av2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
}

fn color() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap()
}

/// Value_Source naming the object `oid` encodes to.
fn names(oid: [u8; 4]) -> PropertyValue {
    PropertyValue::ApplicationData([&[0x1E, 0x1C][..], &oid, &[0x1F]].concat())
}

const DEVICE: [u8; 4] = [0x02, 0x00, 0x03, 0x58];

async fn harness() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        let mut value =
            AnalogValueObject::with_access(2, "AV-2", 62, PresentValueAccess::ReadOnly).unwrap();
        value.set_value_source_tracking(true);
        db.add(Box::new(value)).unwrap();
        let mut color = ColorObject::new(1, "CLR-1").unwrap();
        color.set_value_source_tracking(true);
        db.add(Box::new(color)).unwrap();
    })
    .await
}

async fn source(h: &Harness, oid: ObjectIdentifier) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(VS, None)
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn an_application_update_publishes_this_device() {
    let h = harness().await;
    h.server
        .set_present_value_local(&av2(), PropertyValue::Real(3.5))
        .await
        .unwrap();
    assert_eq!(source(&h, av2()).await, names(DEVICE));
    // The Device owns it, whichever local initiator corrects it.
    h.server
        .write_local(
            &av2(),
            VS,
            None,
            PropertyValue::ApplicationData(vec![0x08]),
            None,
            crate::LocalCommandSource::Object(av1()),
        )
        .await
        .unwrap();
    assert_eq!(
        source(&h, av2()).await,
        PropertyValue::ApplicationData(vec![0x08])
    );
}

#[tokio::test(start_paused = true)]
async fn a_local_write_publishes_its_initiator() {
    let h = harness().await;
    let xy = PropertyValue::List(vec![PropertyValue::Real(0.25), PropertyValue::Real(0.5)]);
    h.server
        .write_local(
            &color(),
            PV,
            None,
            xy.clone(),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(source(&h, color()).await, names(DEVICE));
    h.server
        .write_local(
            &color(),
            PV,
            None,
            xy,
            None,
            crate::LocalCommandSource::Object(av1()),
        )
        .await
        .unwrap();
    // AV-1 is 00 80 00 01.
    assert_eq!(source(&h, color()).await, names([0x00, 0x80, 0x00, 0x01]));
}
