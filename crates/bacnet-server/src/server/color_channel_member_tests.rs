//! The colour alternatives of a Channel value (Addendum 135-2020ca, #1474):
//! an xy colour written to a Channel reaches a Color object's Present_Value,
//! and a colour command reaches a Color Temperature object's Color_Command,
//! which carries it out. Each goes to a member of its own datatype only
//! (Table 12-63).
//!
//! CH-1 (channel 11) has one member, CLR-1's Present_Value; CH-2 (channel
//! 12) has one, CT-1's Color_Command. Neither has a delay. The Channel
//! writes go over the wire; the members are read from the database.
//!
//! CH-21 (channel 41) and CH-22 (channel 42) name the same two properties
//! in Device 9, bound to the harness peer, whose answers are given by hand.
//! A colour member's datatype follows from its object type and property, so
//! the server never reads it first.
//!
//! The clock is paused.
use super::channel_remote_write_tests::{next_request, reliability, sent_reads, start_with};
use super::channel_wire_tests::{ch, channel, member, settled, write_channel};
use super::command_action_wire_tests::{read_db, read_wire};
use super::command_remote_write_tests::{ack, device, sent_writes};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{Reliability, WriteStatus};

/// x 0.5 (0x3F000000) and y 0.25 (0x3E800000), framed in context tag 1.
const XY: [u8; 12] = [
    0x1E, 0x44, 0x3F, 0x00, 0x00, 0x00, 0x44, 0x3E, 0x80, 0x00, 0x00, 0x1F,
];

/// STEP_UP_CCT (4) by 100 K (0x64), framed in context tag 2.
const STEP_UP: [u8; 6] = [0x2E, 0x09, 0x04, 0x59, 0x64, 0x2F];

fn clr1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap()
}

fn ct1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COLOR_TEMPERATURE, 1).unwrap()
}

fn objects(db: &mut ObjectDatabase) {
    db.add(Box::new(ColorObject::new(1, "CLR-1").unwrap()))
        .unwrap();
    db.add(Box::new(ColorTemperatureObject::new(1, "CT-1").unwrap()))
        .unwrap();
    let pv = PropertyIdentifier::PRESENT_VALUE;
    db.add(Box::new(channel(1, 11, vec![(member(clr1(), pv), 0)])))
        .unwrap();
    let cc = PropertyIdentifier::COLOR_COMMAND;
    db.add(Box::new(channel(2, 12, vec![(member(ct1(), cc), 0)])))
        .unwrap();
}

fn octets(value: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(value.to_vec())
}

#[tokio::test(start_paused = true)]
async fn an_xy_colour_reaches_a_colour_present_value() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    write_channel(&mut h, 1, &octets(&XY), None).await.unwrap();
    assert_eq!(settled(&mut h, 1).await, WriteStatus::SUCCESSFUL);
    // The Channel serves the value as written, frame and all.
    assert_eq!(
        read_wire(&mut h, ch(1), PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap(),
        XY
    );
    // The member has the colour, and Transition NONE puts it there at once.
    let colour = PropertyValue::List(vec![PropertyValue::Real(0.5), PropertyValue::Real(0.25)]);
    for p in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::TRACKING_VALUE,
    ] {
        assert_eq!(read_db(&h, clr1(), p, None).await, colour);
    }
}

#[tokio::test(start_paused = true)]
async fn a_colour_command_reaches_a_color_command_and_is_carried_out() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    write_channel(&mut h, 2, &octets(&STEP_UP), None)
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 2).await, WriteStatus::SUCCESSFUL);
    // The member holds the SEQUENCE without the channel value's frame, and
    // stepped 4000 K up to 4100 K.
    assert_eq!(
        read_db(&h, ct1(), PropertyIdentifier::COLOR_COMMAND, None).await,
        octets(&STEP_UP[1..STEP_UP.len() - 1])
    );
    assert_eq!(
        read_db(&h, ct1(), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::Unsigned(4_100)
    );
}

#[tokio::test(start_paused = true)]
async fn each_colour_value_goes_to_its_own_datatype_only() {
    let mut h = Harness::start_with(ServerConfig::default(), objects).await;
    // An xy colour can't go to a Color_Command, a colour command can't go
    // to an xy colour, and a REAL can't go to either: each distribution
    // fails and leaves its member as it was.
    for (instance, value) in [
        (2, octets(&XY)),
        (1, octets(&STEP_UP)),
        (1, PropertyValue::Real(0.5)),
    ] {
        write_channel(&mut h, instance, &value, None).await.unwrap();
        assert_eq!(settled(&mut h, instance).await, WriteStatus::FAILED);
    }
    assert_eq!(
        read_db(&h, ct1(), PropertyIdentifier::COLOR_COMMAND, None).await,
        octets(&[0x09, 0x00])
    );
    assert_eq!(
        read_db(&h, clr1(), PropertyIdentifier::PRESENT_VALUE, None).await,
        PropertyValue::List(vec![
            PropertyValue::Real(0.3127),
            PropertyValue::Real(0.3290)
        ])
    );
}

/// `object`'s `property` in Device 9.
fn remote(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        device_identifier: Some(device(9)),
        ..member(object, property)
    }
}

async fn start_remote() -> Harness {
    start_with(|db| {
        let pv = PropertyIdentifier::PRESENT_VALUE;
        db.add(Box::new(channel(21, 41, vec![(remote(clr1(), pv), 0)])))
            .unwrap();
        let cc = PropertyIdentifier::COLOR_COMMAND;
        db.add(Box::new(channel(22, 42, vec![(remote(ct1(), cc), 0)])))
            .unwrap();
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn colour_values_reach_members_in_another_device_unread() {
    let mut h = start_remote().await;
    // The xy colour goes to CLR-1's Present_Value as its two REALs, and the
    // colour command to CT-1's Color_Command without its frame. Neither
    // write follows a read.
    for (instance, value, object) in [(21, &XY[..], clr1()), (22, &STEP_UP[..], ct1())] {
        write_channel(&mut h, instance, &octets(value), None)
            .await
            .unwrap();
        let (invoke_id, written) = next_request(&h, object).await;
        assert_eq!(written, value[1..value.len() - 1]);
        assert!(sent_reads(&h).is_empty());
        h.respond(ack(invoke_id)).await;
        assert_eq!(settled(&mut h, instance).await, WriteStatus::SUCCESSFUL);
    }
}

#[tokio::test(start_paused = true)]
async fn a_real_for_a_colour_member_in_another_device_fails_unsent() {
    let mut h = start_remote().await;
    // A REAL fits neither member, and their datatypes need no read to say
    // so: nothing is sent, and the distribution fails as a configuration
    // error.
    for instance in [21, 22] {
        write_channel(&mut h, instance, &PropertyValue::Real(0.5), None)
            .await
            .unwrap();
        assert_eq!(settled(&mut h, instance).await, WriteStatus::FAILED);
        assert_eq!(
            reliability(&mut h, instance).await,
            Reliability::CONFIGURATION_ERROR
        );
    }
    assert!(sent_reads(&h).is_empty());
    assert!(sent_writes(&h).is_empty());
}
