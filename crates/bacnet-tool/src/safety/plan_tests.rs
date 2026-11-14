//! Tests of the write preview: occupancy, effects, tiers and refusals.

use bacnet_ops::format::RowError;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::RemoteError;
use bacnet_types::primitives::ObjectIdentifier;

use super::*;

fn encoded(values: &[PropertyValue]) -> Vec<u8> {
    let mut buf = BytesMut::new();
    for value in values {
        encode_property_value(&mut buf, value).unwrap();
    }
    buf.to_vec()
}

/// AO:1 of the lab: 55 at priority 10, Relinquish_Default 0.
fn ao1() -> Vec<RawRow> {
    let mut slots = vec![PropertyValue::Null; 16];
    slots[9] = PropertyValue::Real(55.0);
    vec![
        (
            PropertyIdentifier::PRESENT_VALUE,
            Ok(encoded(&[PropertyValue::Real(55.0)])),
        ),
        (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
        (
            PropertyIdentifier::RELINQUISH_DEFAULT,
            Ok(encoded(&[PropertyValue::Real(0.0)])),
        ),
        (
            PropertyIdentifier::OUT_OF_SERVICE,
            Ok(encoded(&[PropertyValue::Boolean(false)])),
        ),
    ]
}

fn spec(value: Option<&str>, priority: u8) -> WriteSpec {
    WriteSpec {
        device: 100,
        object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
        property: PropertyIdentifier::PRESENT_VALUE,
        index: None,
        value: value.map(str::to_string),
        priority: Some(priority),
    }
}

fn one(spec: WriteSpec) -> Planned {
    let preview = preview(&[spec], &[ao1().into()], |_, _, _| false, false, false).unwrap();
    preview.items.into_iter().next().unwrap()
}

#[test]
fn a_weaker_priority_leaves_present_value_alone() {
    let planned = one(spec(Some("42"), 12));
    assert_eq!(planned.effect, Effect::Unchanged { held_by: Some(10) });
    assert_eq!(planned.occupancy, Occupancy::Empty);
    assert_eq!(planned.gate, Gate::Allowed(Tier::T1));
    // The integer was sent as the REAL the object holds.
    assert_eq!(planned.value.bytes, encoded(&[PropertyValue::Real(42.0)]));
}

#[test]
fn a_preview_keeps_every_digit_of_a_float() {
    // The screens round to six digits; a preview shows what is sent.
    let planned = one(spec(Some("72.12345"), 8));
    assert_eq!(planned.value.text, "72.12345");
    assert_eq!(
        planned.effect,
        Effect::Changes {
            from: "55".into(),
            to: "72.12345".into()
        }
    );
}

#[test]
fn a_binary_preview_names_the_states() {
    let mut slots = vec![PropertyValue::Null; 16];
    slots[7] = PropertyValue::Enumerated(1);
    let rows = vec![
        (
            PropertyIdentifier::PRESENT_VALUE,
            Ok(encoded(&[PropertyValue::Enumerated(1)])),
        ),
        (PropertyIdentifier::PRIORITY_ARRAY, Ok(encoded(&slots))),
        (
            PropertyIdentifier::RELINQUISH_DEFAULT,
            Ok(encoded(&[PropertyValue::Enumerated(0)])),
        ),
    ];
    let spec = WriteSpec {
        object: ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap(),
        value: None,
        ..spec(None, 8)
    };
    let preview = preview(&[spec], &[rows.into()], |_, _, _| true, false, false).unwrap();
    assert_eq!(
        preview.items[0].effect,
        Effect::Changes {
            from: "active".into(),
            to: "inactive".into()
        }
    );
}

#[test]
fn priority_8_changes_present_value_and_needs_t2() {
    let planned = one(spec(Some("42"), 8));
    assert_eq!(
        planned.effect,
        Effect::Changes {
            from: "55".into(),
            to: "42".into()
        }
    );
    assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
    assert_eq!(planned.before, None, "slot 8 was empty");
}

#[test]
fn tiers_follow_the_priority() {
    let tier = |p| {
        gate(
            PropertyIdentifier::PRESENT_VALUE,
            Some(p),
            false,
            Occupancy::Empty,
            false,
        )
    };
    for p in [1, 2, 5] {
        assert_eq!(tier(p), Gate::Allowed(Tier::T3), "priority {p}");
    }
    for p in [3, 4, 7, 8] {
        assert_eq!(tier(p), Gate::Allowed(Tier::T2), "priority {p}");
    }
    for p in 9..=16 {
        assert_eq!(tier(p), Gate::Allowed(Tier::T1), "priority {p}");
    }
    assert!(matches!(tier(6), Gate::Blocked(_)));
    assert_eq!(
        gate(
            PropertyIdentifier::PRESENT_VALUE,
            Some(6),
            false,
            Occupancy::Empty,
            true
        ),
        Gate::Allowed(Tier::T3),
        "dangerous allows it at T3"
    );
    assert_eq!(
        gate(
            PropertyIdentifier::PRESENT_VALUE,
            Some(12),
            true,
            Occupancy::Other,
            false
        ),
        Gate::Allowed(Tier::T2),
        "relinquishing someone else's slot"
    );
    assert_eq!(
        gate(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            false,
            Occupancy::NotCommandable,
            false
        ),
        Gate::Allowed(Tier::T2)
    );
}

#[test]
fn relinquishing_the_slot_in_control_falls_back_and_marks_another_writer() {
    let planned = one(spec(None, 10));
    assert_eq!(planned.occupancy, Occupancy::Other);
    assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
    assert_eq!(
        planned.effect,
        Effect::Changes {
            from: "55".into(),
            to: "0".into()
        }
    );
    assert_eq!(planned.before.unwrap().text, "55");
    let ours = preview(
        &[spec(None, 10)],
        &[ao1().into()],
        |_, _, _| true,
        false,
        false,
    )
    .unwrap();
    assert_eq!(ours.items[0].occupancy, Occupancy::Ours);
    assert_eq!(ours.gate, Gate::Allowed(Tier::T1));
}

/// `spec` for AO:2, a second object with the same rows.
fn other(value: Option<&str>, priority: u8) -> WriteSpec {
    WriteSpec {
        object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 2).unwrap(),
        ..spec(value, priority)
    }
}

#[test]
fn a_batch_takes_its_strongest_tier_and_any_block() {
    let batch = preview(
        &[spec(Some("1"), 12), other(Some("2"), 13)],
        &[ao1().into(), ao1().into()],
        |_, _, _| false,
        false,
        true,
    )
    .unwrap();
    assert_eq!(batch.gate, Gate::Allowed(Tier::T2));
    let blocked = preview(
        &[spec(Some("1"), 12), other(Some("2"), 6)],
        &[ao1().into(), ao1().into()],
        |_, _, _| false,
        false,
        true,
    )
    .unwrap();
    assert!(matches!(blocked.gate, Gate::Blocked(_)));
}

/// A WPM error names the failed write without its priority, so one
/// property twice (at two priorities) would be ambiguous.
#[test]
fn a_batch_may_not_name_one_property_twice() {
    let error = preview(
        &[spec(Some("1"), 8), spec(Some("2"), 9)],
        &[ao1().into(), ao1().into()],
        |_, _, _| false,
        false,
        true,
    )
    .unwrap_err();
    assert!(error.contains("appears twice"), "{error}");
}

#[test]
fn a_typed_null_is_a_relinquish() {
    // Slot 10 holds another commander's 55: relinquishing it is T2.
    let planned = one(spec(Some("null"), 10));
    assert_eq!(planned.value.bytes, super::super::NULL);
    assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
    let mut description = spec(Some("null"), 12);
    description.property = PropertyIdentifier::DESCRIPTION;
    assert!(
        preview(
            &[description],
            &[ao1().into()],
            |_, _, _| false,
            false,
            false
        )
        .is_err()
    );
}

#[test]
fn a_slot_is_ours_only_by_its_current_value() {
    let fifty_five = encoded(&[PropertyValue::Real(55.0)]);
    let ours = |_: &WriteSpec, _: Option<u8>, now: &[u8]| now == fifty_five.as_slice();
    let planned = preview(&[spec(None, 10)], &[ao1().into()], ours, false, false).unwrap();
    assert_eq!(planned.items[0].occupancy, Occupancy::Ours);
    let wrote_other = |_: &WriteSpec, _: Option<u8>, now: &[u8]| now == [0x44, 0, 0, 0, 0];
    let planned = preview(
        &[spec(None, 10)],
        &[ao1().into()],
        wrote_other,
        false,
        false,
    )
    .unwrap();
    assert_eq!(planned.items[0].occupancy, Occupancy::Other);
    assert_eq!(planned.gate, Gate::Allowed(Tier::T2));
}

#[test]
fn a_priority_array_that_does_not_decode_refuses_the_write() {
    let mut rows = ao1();
    rows[1].1 = Ok(encoded(&vec![PropertyValue::Null; 4]));
    let error = preview(
        &[spec(Some("1"), 12)],
        &[rows.clone().into()],
        |_, _, _| false,
        false,
        false,
    )
    .unwrap_err();
    assert!(error.contains("Priority_Array"), "{error}");
    rows[1].1 = Ok(vec![0xFF]);
    assert!(
        preview(
            &[spec(Some("1"), 12)],
            &[rows.into()],
            |_, _, _| false,
            false,
            false
        )
        .is_err()
    );
    // An error reading it means not commandable.
    let mut rows = ao1();
    rows[1].1 = Err(RowError::Remote(RemoteError::Error {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::UNKNOWN_PROPERTY,
    }));
    let planned = preview(
        &[spec(Some("1"), 12)],
        &[rows.into()],
        |_, _, _| false,
        false,
        false,
    )
    .unwrap();
    assert!(!planned.items[0].commandable);
}

#[test]
fn a_non_commandable_property_keeps_its_old_value_as_the_pre_image() {
    let mut rows = ao1();
    rows.push((
        PropertyIdentifier::DESCRIPTION,
        Ok(encoded(&[PropertyValue::CharacterString("old".into())])),
    ));
    let mut description = spec(Some("\"new\""), 12);
    description.property = PropertyIdentifier::DESCRIPTION;
    let planned = preview(
        &[description],
        &[rows.into()],
        |_, _, _| false,
        false,
        false,
    )
    .unwrap()
    .items
    .remove(0);
    assert_eq!(planned.priority, None);
    assert_eq!(planned.before.unwrap().text, "\"old\"");
    assert_eq!(planned.effect, Effect::NotPresentValue);
    let mut relinquish = spec(None, 12);
    relinquish.property = PropertyIdentifier::DESCRIPTION;
    assert!(
        preview(
            &[relinquish],
            &[ao1().into()],
            |_, _, _| false,
            false,
            false
        )
        .is_err()
    );
}

/// `state-text[2]` of MSV:1, with `element` as the element's read.
fn state_text(element: Option<Result<Vec<u8>, String>>) -> Planned {
    let spec = WriteSpec {
        object: ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap(),
        property: PropertyIdentifier::STATE_TEXT,
        index: Some(2),
        priority: None,
        ..spec(Some("\"Low\""), 12)
    };
    // The rows hold no State_Text: an indexed write reads its element alone.
    let read = PreRead {
        rows: Vec::new(),
        element,
    };
    preview(&[spec], &[read], |_, _, _| false, false, false)
        .unwrap()
        .items
        .remove(0)
}

#[test]
fn an_indexed_write_keeps_the_element_as_its_pre_image() {
    let old = encoded(&[PropertyValue::CharacterString("State 2".into())]);
    let planned = state_text(Some(Ok(old.clone())));
    let before = planned.before.expect("the element's old value");
    assert_eq!(before.bytes, old);
    assert_eq!(before.text, "\"State 2\"");
    assert!(planned.notes.is_empty(), "{:?}", planned.notes);
}

#[test]
fn an_unreadable_element_leaves_the_pre_image_unknown_and_says_why() {
    let planned = state_text(Some(Err("PROPERTY:INVALID_ARRAY_INDEX".into())));
    assert_eq!(planned.before, None);
    assert!(
        planned
            .notes
            .iter()
            .any(|n| n.contains("INVALID_ARRAY_INDEX") && n.contains("can't be reverted")),
        "{:?}",
        planned.notes
    );
    assert_eq!(state_text(None).before, None, "never read");
}

/// Index 0 is the array's length: a resize, never reverted, even with
/// the old length in hand.
#[test]
fn writing_index_0_is_a_resize_with_no_pre_image() {
    let spec = WriteSpec {
        object: ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap(),
        property: PropertyIdentifier::STATE_TEXT,
        index: Some(0),
        priority: None,
        ..spec(Some("4"), 12)
    };
    let read = PreRead {
        rows: Vec::new(),
        element: Some(Ok(encoded(&[PropertyValue::Unsigned(3)]))),
    };
    let planned = preview(&[spec], &[read], |_, _, _| false, false, false)
        .unwrap()
        .items
        .remove(0);
    assert_eq!(planned.before, None);
    assert!(
        planned.notes.iter().any(|n| n == RESIZE),
        "{:?}",
        planned.notes
    );
}

/// #2098: a DateTime Value's Priority_Array, as the server's object encodes
/// it, is sixteen slots, so a commanded write previews instead of refusing,
/// and names the date-time in control.
#[test]
fn a_date_time_value_previews_its_commanded_slot() {
    use bacnet_objects::traits::BACnetObject;
    use bacnet_objects::value_types::DateTimeValueObject;
    use bacnet_types::primitives::{Date, Time};

    let date = Date {
        year: 126,
        month: 10,
        day: 25,
        day_of_week: Date::UNSPECIFIED,
    };
    let time = Time {
        hour: 7,
        minute: 30,
        second: 0,
        hundredths: 0,
    };
    let set = PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)]);
    let mut object = DateTimeValueObject::new(1, "DTV-1").unwrap();
    object
        .write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            set.clone(),
            Some(10),
        )
        .unwrap();
    let read = |property| {
        let value = object.read_property(property, None).unwrap();
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, &value).unwrap();
        (property, Ok(buf.to_vec()))
    };
    let plain = {
        let mut buf = BytesMut::new();
        encode_property_value(&mut buf, &set).unwrap();
        buf.to_vec()
    };
    let rows: Vec<RawRow> = vec![
        read(PropertyIdentifier::PRESENT_VALUE),
        read(PropertyIdentifier::PRIORITY_ARRAY),
        read(PropertyIdentifier::RELINQUISH_DEFAULT),
    ];
    let dtv = |text, priority| WriteSpec {
        object: ObjectIdentifier::new(ObjectType::DATETIME_VALUE, 1).unwrap(),
        ..spec(Some(text), priority)
    };
    let plan = |spec| {
        let read = PreRead::from(rows.clone());
        preview(
            &[spec],
            &[read],
            |_, _, held: &[u8]| held == plain,
            false,
            false,
        )
        .unwrap()
        .items
        .remove(0)
    };
    let planned = plan(dtv("2026-10-26 08:00", 8));
    assert!(planned.commandable);
    assert_eq!(
        planned.effect,
        Effect::Changes {
            from: "2026-10-25 07:30".into(),
            to: "2026-10-26 08:00".into()
        }
    );
    assert_eq!(planned.occupancy, Occupancy::Empty);
    // Over the held slot: the pre-image is the bare Date and Time a revert
    // writes back to Present_Value, not the slot's context-tagged choice.
    let planned = plan(dtv("2026-10-26 08:00", 10));
    let before = planned.before.expect("the slot is held");
    assert_eq!(before.bytes, plain);
    assert_eq!(before.text, "2026-10-25 07:30");
    assert_eq!(planned.occupancy, Occupancy::Ours);
}
