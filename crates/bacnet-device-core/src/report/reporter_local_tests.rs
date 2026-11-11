//! The [`Reporter`]'s local writes (#2446): each typed call is
//! `apply_local` of its change, a change that wrote reports what it changed,
//! and one that changed nothing or was refused reports nothing and drops its
//! Life Safety capture. `bacnet-server`'s parity tests
//! (`device_core_parity_tests/cov/local_write`) hold `apply_local` to the
//! server's local writes.

use super::*;
use crate::report::LocalValue;
use crate::write::Applied;
use crate::write::local::{LocalChange, LocalCommandSource};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::LocalInputValue;
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_types::enums::{BinaryPV, ErrorCode, Reliability};
use bacnet_types::error::RemoteError;

fn bv2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 2).unwrap()
}

fn msv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap()
}

fn iv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::INTEGER_VALUE, 1).unwrap()
}

fn lsp1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()
}

/// The reporter's fixture with a commandable BV-2 and MSV-1 (three
/// states), a read-only IV-1 and LSP-1, one unconfirmed subscriber on
/// each object, every first notification sent.
fn device() -> Device {
    let mut device = Device::new(OutboxLimits::default());
    let more: [Box<dyn bacnet_objects::traits::BACnetObject>; 4] = [
        Box::new(BinaryValueObject::new(2, "BV-2").unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(IntegerValueObject::with_access(1, "IV-1", PresentValueAccess::ReadOnly).unwrap()),
        Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()),
    ];
    for object in more {
        device.db.add(object).unwrap();
    }
    for (process, object) in (1..).zip([ai(), bv2(), msv1(), iv1(), lsp1()]) {
        device.subscribe_to(object, process, false, T0);
    }
    assert_eq!(sent(&mut device).len(), 5, "the first notifications");
    device
}

/// Every APDU `pop` hands out at [`T0`], each reported sent.
fn sent(device: &mut Device) -> Vec<Vec<u8>> {
    let mut sent = Vec::new();
    loop {
        let mut out = BytesMut::new();
        let reporter = &mut device.reporter;
        let Some(notification) = reporter.pop(&NoBindings, &mut device.ids, T0, &mut out) else {
            return sent;
        };
        reporter.delivered(notification.token, Delivery::Sent, &mut device.ids, T0);
        sent.push(out.to_vec());
    }
}

fn read(device: &Device, object: ObjectIdentifier, property: P) -> PropertyValue {
    let stored = device.db.get(&object).unwrap();
    stored.read_property(property, None).unwrap()
}

fn code(result: Result<Applied, Error>) -> ErrorCode {
    match result {
        Err(Error::Remote(RemoteError::Error { code, .. })) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// A Present_Value write at `priority` as the local Device.
fn commanded(value: PropertyValue, priority: u8) -> LocalChange {
    LocalChange::Property {
        property: P::PRESENT_VALUE,
        array_index: None,
        value,
        priority: Some(priority),
        source: LocalCommandSource::ServerDevice,
    }
}

/// One typed call on a device.
type Typed = fn(&mut Device) -> Result<Applied, Error>;

/// Each typed call and the change it stands for, made on twin devices:
/// the same result, the same notifications and the same Present_Value,
/// Priority_Array and Reliability afterwards. The values go out as their
/// application tags: a REAL, an Enumerated, an Unsigned and a Signed.
#[test]
fn each_typed_call_is_apply_local_of_its_change() {
    let input = |value| LocalChange::Input(LocalInputValue::present_value(value));
    let cases: [(Typed, ObjectIdentifier, LocalChange); 9] = [
        (
            |d| d.reporter.update(&mut d.db, ai(), 12.5.into(), T0),
            ai(),
            input(PropertyValue::Real(12.5)),
        ),
        (
            |d| d.reporter.update(&mut d.db, iv1(), (-7).into(), T0),
            iv1(),
            input(PropertyValue::Signed(-7)),
        ),
        (
            |d| {
                d.reporter
                    .command(&mut d.db, bv2(), 8, BinaryPV::ACTIVE.into(), T0)
            },
            bv2(),
            commanded(PropertyValue::Enumerated(1), 8),
        ),
        (
            |d| {
                d.reporter
                    .command(&mut d.db, msv1(), 6, LocalValue::State(3), T0)
            },
            msv1(),
            commanded(PropertyValue::Unsigned(3), 6),
        ),
        (
            |d| d.reporter.relinquish(&mut d.db, msv1(), 6, T0),
            msv1(),
            commanded(PropertyValue::Null, 6),
        ),
        (
            |d| d.reporter.relinquish(&mut d.db, bv2(), 8, T0),
            bv2(),
            commanded(PropertyValue::Null, 8),
        ),
        (
            |d| {
                d.reporter
                    .set_reliability(&mut d.db, ai(), Reliability::NO_SENSOR, T0)
            },
            ai(),
            LocalChange::Reliability(Reliability::NO_SENSOR),
        ),
        // Refused as the object refuses the write: an Input in service
        // takes no command, and a binary state is no Integer Value's.
        (
            |d| d.reporter.command(&mut d.db, ai(), 8, 1.0.into(), T0),
            ai(),
            commanded(PropertyValue::Real(1.0), 8),
        ),
        (
            |d| {
                d.reporter
                    .update(&mut d.db, iv1(), BinaryPV::ACTIVE.into(), T0)
            },
            iv1(),
            input(PropertyValue::Enumerated(1)),
        ),
    ];
    let (mut typed, mut change) = (device(), device());
    for (n, (call, object, equivalent)) in cases.into_iter().enumerate() {
        let left = call(&mut typed);
        let right = change
            .reporter
            .apply_local(&mut change.db, object, equivalent, T0);
        assert_eq!(format!("{left:?}"), format!("{right:?}"), "case {n}");
        let notified = sent(&mut typed);
        assert_eq!(notified, sent(&mut change), "case {n}: the notifications");
        assert_eq!(notified.len(), usize::from(left.is_ok()), "case {n}");
        for property in [P::PRESENT_VALUE, P::PRIORITY_ARRAY, P::RELIABILITY] {
            let row = |d: &Device| d.db.get(&object).unwrap().read_property(property, None);
            let rows = (row(&typed), row(&change));
            assert_eq!(format!("{:?}", rows.0), format!("{:?}", rows.1), "case {n}");
        }
    }
}

/// A Reliability the application sets reports Status_Flags with FAULT, and
/// clearing it reports them again without.
#[test]
fn a_reliability_reports_the_fault_and_its_end() {
    let mut device = device();
    for (reliability, flags) in [
        (Reliability::OPEN_LOOP, 0b0100),
        (Reliability::NO_FAULT_DETECTED, 0),
    ] {
        let set = device
            .reporter
            .set_reliability(&mut device.db, ai(), reliability, T0);
        assert_eq!(set.unwrap(), Applied::Written);
        let [apdu] = &sent(&mut device)[..] else {
            panic!("one notification");
        };
        let Apdu::UnconfirmedRequest(request) = decode_apdu(Bytes::copy_from_slice(apdu)).unwrap()
        else {
            panic!("an unconfirmed notification");
        };
        let values = COVNotificationRequest::decode(&request.service_request)
            .unwrap()
            .list_of_values;
        let status = values
            .iter()
            .find(|value| value.property_identifier == P::STATUS_FLAGS)
            .expect("Status_Flags reported");
        assert_eq!(status.value[2] >> 4, flags, "{reliability:?}");
    }
    // The same Reliability again writes, and reports nothing new.
    let again =
        device
            .reporter
            .set_reliability(&mut device.db, ai(), Reliability::NO_FAULT_DETECTED, T0);
    assert_eq!(again.unwrap(), Applied::Written);
    assert!(sent(&mut device).is_empty());
}

/// A refused change and a NULL the property left as it was report nothing,
/// and leave no Life Safety capture behind; a change that wrote reports the
/// Life Safety values it changed.
#[test]
fn a_change_that_changed_nothing_reports_nothing_and_drops_its_capture() {
    let mut device = device();
    // A REAL is no BACnetLifeSafetyState.
    let refused = device
        .reporter
        .update(&mut device.db, lsp1(), 1.0.into(), T0);
    assert_eq!(code(refused), ErrorCode::INVALID_DATA_TYPE);
    assert!(!device.reporter.snapshots.holds(lsp1()), "refused");
    // Direct_Reading, a REAL, has no NULL in its datatype (#1396).
    let null = LocalChange::Property {
        property: P::DIRECT_READING,
        array_index: None,
        value: PropertyValue::Null,
        priority: None,
        source: LocalCommandSource::ServerDevice,
    };
    let unchanged = device
        .reporter
        .apply_local(&mut device.db, lsp1(), null, T0);
    assert_eq!(unchanged.unwrap(), Applied::Unchanged);
    assert!(!device.reporter.snapshots.holds(lsp1()), "unchanged");
    assert!(sent(&mut device).is_empty());
    // ALARM: a Present_Value change, which the subscriber hears.
    let alarm = PropertyValue::Enumerated(2);
    let input = LocalChange::Input(LocalInputValue::present_value(alarm.clone()));
    let written = device
        .reporter
        .apply_local(&mut device.db, lsp1(), input, T0);
    assert_eq!(written.unwrap(), Applied::Written);
    assert!(!device.reporter.snapshots.holds(lsp1()), "reported");
    assert_eq!(sent(&mut device).len(), 1);
    assert_eq!(read(&device, lsp1(), P::PRESENT_VALUE), alarm);
}
