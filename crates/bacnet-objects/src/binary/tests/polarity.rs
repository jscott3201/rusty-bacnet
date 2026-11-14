//! Polarity and the physical side of the Binary Input and Output (#2580).

use super::*;
use crate::event::test_clock::test_now;
use crate::traits::{EventReporting as _, LocalInputValue};
use bacnet_types::enums::{ErrorClass, ErrorCode, EventState, EventType};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::PropertyValue::{Boolean, Enumerated};

const INACTIVE: u32 = 0;
const ACTIVE: u32 = 1;

fn error_code(error: Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

fn present_value(object: &dyn BACnetObject) -> u32 {
    match object
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
    {
        Enumerated(value) => value,
        other => panic!("Present_Value read {other:?}"),
    }
}

fn set_out_of_service(object: &mut dyn BACnetObject, out_of_service: bool) {
    object
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            Boolean(out_of_service),
            None,
        )
        .unwrap();
}

fn command(bo: &mut BinaryOutputObject, value: u32) {
    bo.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        Enumerated(value),
        Some(8),
        &crate::command_source::test_origin(),
    )
    .unwrap();
}

fn local(object: &mut dyn BACnetObject, input: LocalInputValue) -> Result<(), Error> {
    object
        .local_input()
        .expect("the object takes local inputs")
        .apply_local_input(input)
}

/// The object reads Polarity as configured, refuses a client's write of it
/// and an application value that is neither NORMAL nor REVERSE.
fn assert_polarity_is_local_only(object: &mut dyn BACnetObject) {
    local(object, LocalInputValue::Polarity(Polarity::REVERSE)).unwrap();
    let read = |object: &dyn BACnetObject| {
        object
            .read_property(PropertyIdentifier::POLARITY, None)
            .unwrap()
    };
    assert_eq!(read(object), Enumerated(Polarity::REVERSE.to_raw()));

    let written = object.write_property(
        PropertyIdentifier::POLARITY,
        None,
        Enumerated(Polarity::NORMAL.to_raw()),
        None,
    );
    assert_eq!(
        error_code(written.unwrap_err()),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    let refused = local(object, LocalInputValue::Polarity(Polarity::from_raw(2)));
    assert_eq!(
        error_code(refused.unwrap_err()),
        (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    assert_eq!(read(object), Enumerated(Polarity::REVERSE.to_raw()));
}

// --- Binary Input ---

#[test]
fn bi_present_value_is_the_physical_input_through_polarity() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_physical_input(true);
    assert_eq!(present_value(&bi), ACTIVE);
    bi.set_physical_input(false);
    assert_eq!(present_value(&bi), INACTIVE);

    bi.set_polarity(Polarity::REVERSE).unwrap();
    bi.set_physical_input(true);
    assert_eq!(present_value(&bi), INACTIVE, "ON under REVERSE");
    // The served route takes the same state.
    local(&mut bi, LocalInputValue::PhysicalInput(false)).unwrap();
    assert_eq!(present_value(&bi), ACTIVE, "OFF under REVERSE");
}

#[test]
fn bi_polarity_change_in_service_derives_present_value_again() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    // No reading yet: nothing to derive from, so Present_Value stays.
    bi.set_polarity(Polarity::REVERSE).unwrap();
    assert_eq!(present_value(&bi), INACTIVE);

    bi.set_physical_input(true);
    assert_eq!(present_value(&bi), INACTIVE);
    // A steady ON input reads ACTIVE once Polarity is NORMAL again.
    local(&mut bi, LocalInputValue::Polarity(Polarity::NORMAL)).unwrap();
    assert_eq!(present_value(&bi), ACTIVE);
    bi.set_polarity(Polarity::REVERSE).unwrap();
    assert_eq!(present_value(&bi), INACTIVE);
}

/// A logical value records the physical state it implies, so a later
/// Polarity change derives from that state.
#[test]
fn bi_logical_value_implies_a_physical_state() {
    let mut served = BinaryInputObject::new(1, "BI-1").unwrap();
    served.set_polarity(Polarity::REVERSE).unwrap();
    local(
        &mut served,
        LocalInputValue::present_value(Enumerated(ACTIVE)),
    )
    .unwrap();
    assert_eq!(present_value(&served), ACTIVE);
    served.set_polarity(Polarity::NORMAL).unwrap();
    assert_eq!(present_value(&served), INACTIVE, "the input was OFF");

    let mut unattached = BinaryInputObject::new(2, "BI-2").unwrap();
    unattached.set_polarity(Polarity::REVERSE).unwrap();
    unattached.set_present_value(ACTIVE);
    unattached.set_polarity(Polarity::NORMAL).unwrap();
    assert_eq!(present_value(&unattached), INACTIVE, "the input was OFF");
}

/// Out of service Present_Value is the client's: the input and Polarity
/// still change, and the return to service derives from both.
#[test]
fn bi_out_of_service_decouples_present_value_from_the_input() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    bi.set_physical_input(true);
    set_out_of_service(&mut bi, true);

    local(&mut bi, LocalInputValue::PhysicalInput(false)).unwrap();
    assert_eq!(present_value(&bi), ACTIVE, "the input is decoupled");
    bi.set_polarity(Polarity::REVERSE).unwrap();
    assert_eq!(present_value(&bi), ACTIVE, "so is Polarity");
    bi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        Enumerated(INACTIVE),
        None,
    )
    .unwrap();
    // A low-level logical value is a simulated one now: it implies no state.
    bi.set_present_value(INACTIVE);
    // The logical route stays the client's while out of service.
    let refused = local(&mut bi, LocalInputValue::present_value(Enumerated(ACTIVE)));
    assert_eq!(
        error_code(refused.unwrap_err()),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(present_value(&bi), INACTIVE);

    set_out_of_service(&mut bi, false);
    assert_eq!(present_value(&bi), ACTIVE, "OFF under REVERSE");
}

/// A return to service with no reading ever supplied keeps the value a
/// client left: there is no physical state to derive from.
#[test]
fn bi_return_to_service_without_a_reading_keeps_present_value() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    set_out_of_service(&mut bi, true);
    bi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        Enumerated(ACTIVE),
        None,
    )
    .unwrap();
    set_out_of_service(&mut bi, false);
    assert_eq!(present_value(&bi), ACTIVE);
}

/// CHANGE_OF_STATE watches the logical Present_Value: an OFF input under
/// REVERSE is the default Alarm_Value, ACTIVE.
#[test]
fn bi_alarm_follows_the_logical_state() {
    let mut on = BinaryInputObject::new(1, "BI-1").unwrap();
    on.set_polarity(Polarity::REVERSE).unwrap();
    on.set_physical_input(true);
    assert_eq!(on.evaluate_intrinsic_reporting(test_now()), None);

    let mut off = BinaryInputObject::new(2, "BI-2").unwrap();
    off.set_polarity(Polarity::REVERSE).unwrap();
    off.set_physical_input(false);
    let outcome = off
        .evaluate_intrinsic_reporting(test_now())
        .expect("ACTIVE is the default Alarm_Value");
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::CHANGE_OF_STATE);
}

#[test]
fn bi_polarity_is_read_only_on_the_wire() {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_polarity_is_local_only(&mut bi);
}

// --- Binary Output ---

#[test]
fn bo_physical_output_is_present_value_through_polarity() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    assert!(!bo.physical_output(), "INACTIVE under NORMAL is OFF");
    command(&mut bo, ACTIVE);
    assert!(bo.physical_output());

    bo.set_polarity(Polarity::REVERSE).unwrap();
    assert!(!bo.physical_output(), "ACTIVE under REVERSE is OFF");
    assert_eq!(present_value(&bo), ACTIVE, "Polarity leaves Present_Value");
    command(&mut bo, INACTIVE);
    assert!(bo.physical_output(), "INACTIVE under REVERSE is ON");

    let mut reverse = BinaryOutputObject::new(2, "BO-2").unwrap();
    reverse.set_polarity(Polarity::REVERSE).unwrap();
    assert!(reverse.physical_output(), "a new REVERSE output drives ON");
}

/// Out of service the output stays where it was driven: commands move
/// Present_Value without it, and a Polarity change waits for the return.
#[test]
fn bo_out_of_service_holds_the_physical_output() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    command(&mut bo, ACTIVE);
    set_out_of_service(&mut bo, true);
    command(&mut bo, INACTIVE);
    assert_eq!(present_value(&bo), INACTIVE, "still commanded");
    assert!(bo.physical_output(), "held ON");
    // Writing TRUE again holds the same state.
    set_out_of_service(&mut bo, true);
    assert!(bo.physical_output(), "still held ON");
    set_out_of_service(&mut bo, false);
    assert!(!bo.physical_output(), "INACTIVE under NORMAL");

    set_out_of_service(&mut bo, true);
    local(&mut bo, LocalInputValue::Polarity(Polarity::REVERSE)).unwrap();
    assert!(!bo.physical_output(), "Polarity waits");
    set_out_of_service(&mut bo, false);
    assert!(bo.physical_output(), "INACTIVE under REVERSE");

    // The held state is the one Polarity gave the output, not Present_Value.
    let mut reverse = BinaryOutputObject::new(2, "BO-2").unwrap();
    reverse.set_polarity(Polarity::REVERSE).unwrap();
    set_out_of_service(&mut reverse, true);
    assert!(reverse.physical_output(), "INACTIVE under REVERSE, held");
    command(&mut reverse, ACTIVE);
    assert!(reverse.physical_output(), "still held ON");
}

/// COMMAND_FAILURE compares the logical Present_Value with a logical
/// Feedback_Value, whatever Polarity does to the output.
#[test]
fn bo_feedback_value_compares_with_the_logical_state() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.set_polarity(Polarity::REVERSE).unwrap();
    bo.write_property(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        None,
        Boolean(true),
        None,
    )
    .unwrap();
    let feedback = |bo: &mut BinaryOutputObject, value| {
        bo.write_property(
            PropertyIdentifier::FEEDBACK_VALUE,
            None,
            Enumerated(value),
            None,
        )
        .unwrap();
    };
    command(&mut bo, ACTIVE);
    assert!(!bo.physical_output());
    feedback(&mut bo, ACTIVE);
    assert_eq!(bo.evaluate_intrinsic_reporting(test_now()), None);

    feedback(&mut bo, INACTIVE);
    let outcome = bo
        .evaluate_intrinsic_reporting(test_now())
        .expect("the device didn't follow the command");
    assert_eq!(outcome.change.to, EventState::OFFNORMAL);
    assert_eq!(outcome.event_type, EventType::COMMAND_FAILURE);
}

#[test]
fn bo_polarity_is_read_only_on_the_wire_and_its_only_local_input() {
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    assert_polarity_is_local_only(&mut bo);
    for input in [
        LocalInputValue::present_value(Enumerated(ACTIVE)),
        LocalInputValue::PhysicalInput(true),
    ] {
        let refused = local(&mut bo, input);
        assert_eq!(
            error_code(refused.unwrap_err()),
            (
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
            )
        );
    }
}

#[test]
fn database_gives_a_binary_output_physical_output() {
    let mut db = crate::database::ObjectDatabase::new();
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    bo.set_polarity(Polarity::REVERSE).unwrap();
    let bo_oid = bo.object_identifier();
    let bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let bi_oid = bi.object_identifier();
    db.add(Box::new(bo)).unwrap();
    db.add(Box::new(bi)).unwrap();

    assert_eq!(db.physical_output(&bo_oid), Some(true));
    assert_eq!(db.physical_output(&bi_oid), None);
    let unknown = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 9).unwrap();
    assert_eq!(db.physical_output(&unknown), None);
}
