//! An Event Enrollment's kept parameters across a rebuild (#2331): the cap
//! on what storage keeps of them, storage holding what no write leaves,
//! every choice of the two properties through the file's octets, what a
//! restore does to the enrollment, and an enrollment rebuilt another way
//! than the one that saved the file. The writes, the setters and a
//! request's stage are in `enrollment_parameters_tests`.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, EventType, FaultType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::enrollment_parameters_tests::{
    CAP, built_on, covered, enumerated, event_value, fault_value, monitored, open, range_fault,
    state_parameters, try_open,
};
use super::parameters_tests::{holding, read, write};
use super::reporting_tests::{persistence, refused_with};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, WriteConfigPersistence, WriteConfigSnapshot};
use crate::event_enrollment::{
    EventEnrollmentEvalState, EventEnrollmentObject, EventEnrollmentPending,
};
use crate::snapshot::enrollment_parameters_tests::{
    XorShift, every_event_choice, every_fault_choice, extended_fault, legacy,
};
use crate::snapshot::write_config as format;
use crate::traits::BACnetObject;

/// Storage that keeps the file's octets, as a file backend does, so what
/// it hands back has been through the format.
#[derive(Default)]
struct FileOctets {
    file: Mutex<Option<Vec<u8>>>,
    saves: AtomicUsize,
}

impl FileOctets {
    fn file(&self) -> Vec<u8> {
        self.file.lock().unwrap().clone().expect("a saved file")
    }
}

impl WriteConfigPersistence for FileOctets {
    fn load(&self, object: ObjectIdentifier) -> Result<Option<WriteConfigSnapshot>, Error> {
        match &*self.file.lock().unwrap() {
            Some(octets) => Ok(Some(format::decode(object, octets)?)),
            None => Ok(None),
        }
    }

    fn save(&self, object: ObjectIdentifier, snapshot: &WriteConfigSnapshot) -> Result<(), Error> {
        *self.file.lock().unwrap() = Some(format::encode(object, snapshot)?);
        self.saves.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// With storage, Event_Parameters or Fault_Parameters whose encoding is
/// past 64 KiB are refused with RESOURCES / NO_SPACE_TO_WRITE_PROPERTY
/// before anything is saved, and the old value stays; at the cap they are
/// taken and the file holds them. Without storage the enrollment takes
/// both, as it always has.
#[test]
fn a_parameter_past_the_cap_is_refused_before_any_save_with_storage_only() {
    let BACnetEventParameter::Opaque { data: at_cap, .. } = legacy(CAP - 4) else {
        unreachable!()
    };
    let BACnetEventParameter::Opaque { data: past, .. } = legacy(CAP - 3) else {
        unreachable!()
    };
    let strings = BACnetEventParameter::ChangeOfCharacterstring {
        time_delay: 1,
        list_of_alarm_values: vec!["x".repeat(1000); 70],
    };
    let over = [
        (P::EVENT_PARAMETERS, PropertyValue::OctetString(past)),
        (P::EVENT_PARAMETERS, event_value(&strings)),
        (P::FAULT_PARAMETERS, fault_value(&extended_fault(CAP + 1))),
    ];
    let within = [
        (P::EVENT_PARAMETERS, PropertyValue::OctetString(at_cap)),
        (P::FAULT_PARAMETERS, fault_value(&extended_fault(CAP))),
    ];

    let storage = MemoryConfig::new();
    let mut kept = open(Some(&storage));
    let mut plain = open(None);
    for (property, value) in over {
        let before = read(&kept, property);
        let result = kept.write_property(property, None, value.clone(), None);
        assert!(
            refused_with(
                &result,
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
            ),
            "{property:?}: {result:?}"
        );
        assert_eq!(read(&kept, property), before, "{property:?}");
        plain
            .write_property(property, None, value, None)
            .unwrap_or_else(|error| panic!("{property:?} without storage: {error:?}"));
    }
    assert_eq!(storage.saves(), 0);
    assert_eq!(storage.saved(), None);

    let file = Arc::new(FileOctets::default());
    let mut filed = built_on(Some(file.clone())).unwrap();
    for (property, value) in within {
        write(&mut filed, property, value);
    }
    assert_eq!(file.saves.load(Ordering::SeqCst), 2);
    let served = [P::EVENT_PARAMETERS, P::FAULT_PARAMETERS].map(|row| read(&filed, row));
    drop(filed);
    let rebuilt = built_on(Some(file.clone())).unwrap();
    assert_eq!(
        [P::EVENT_PARAMETERS, P::FAULT_PARAMETERS].map(|row| read(&rebuilt, row)),
        served
    );
}

/// Storage holding what no write leaves fails the build, as the codec
/// refuses it: a backend of the application's own may hand over anything.
#[test]
fn storage_holding_what_no_write_leaves_fails_the_build() {
    let none = AlgorithmWriteConfig::default;
    let refused = [
        AlgorithmWriteConfig {
            event_parameters: Some(legacy(CAP - 3)),
            ..none()
        },
        AlgorithmWriteConfig {
            event_parameters: Some(BACnetEventParameter::Opaque {
                tag: 5,
                data: vec![],
            }),
            ..none()
        },
        AlgorithmWriteConfig {
            fault_parameters: Some(extended_fault(CAP + 1)),
            ..none()
        },
        AlgorithmWriteConfig {
            time_delay: Some(1),
            ..none()
        },
    ];
    for stored in refused {
        let built = try_open(Some(&holding(stored)));
        assert!(matches!(built, Err(Error::Encoding(_))));
    }
}

/// Every choice of both properties a write leaves is saved, read back
/// through the file's octets by a rebuilt enrollment, and served as it was
/// written; and a save the rebuilt enrollment makes for another row leaves
/// the parameter's octets in the file as they were.
#[test]
fn every_choice_is_served_again_from_the_file_and_the_file_stays_the_same() {
    let events = every_event_choice()
        .into_iter()
        .map(|parameters| (P::EVENT_PARAMETERS, event_value(&parameters)));
    let faults = every_fault_choice()
        .into_iter()
        .map(|parameters| (P::FAULT_PARAMETERS, fault_value(&parameters)));
    let mut cases = 0;
    for (property, value) in events.chain(faults) {
        cases += 1;
        let file = Arc::new(FileOctets::default());
        let mut enrollment = built_on(Some(file.clone())).unwrap();
        enrollment
            .write_property(property, None, value.clone(), None)
            .unwrap_or_else(|error| panic!("{property:?} {value:?}: {error:?}"));
        assert_eq!(read(&enrollment, property), value, "{property:?}");
        drop(enrollment);
        let first = file.file();

        let mut rebuilt = built_on(Some(file.clone()))
            .unwrap_or_else(|error| panic!("{value:?} doesn't reopen: {error:?}"));
        assert_eq!(read(&rebuilt, property), value, "{property:?} reopened");
        // The rebuilt enrollment saves another row: the mask and the
        // parameter's row, which end the file, are the octets it read.
        write(
            &mut rebuilt,
            P::NOTIFICATION_CLASS,
            PropertyValue::Unsigned(9),
        );
        let with_class = file.file();
        assert_eq!(with_class.len(), first.len() + 4);
        assert!(
            with_class.ends_with(&first[PLAIN_BODY_END..]),
            "{property:?} {value:?}: the row's octets changed"
        );
        drop(rebuilt);
        let again = built_on(Some(file.clone())).unwrap();
        assert_eq!(read(&again, property), value, "{property:?} reopened twice");
    }
    assert_eq!(
        cases,
        every_event_choice().len() + every_fault_choice().len()
    );
}

/// Where the kind 4 body of a file with no reporting row ends, and the
/// mask and the parameters begin: after the header, the kind and seven
/// absent rows.
const PLAIN_BODY_END: usize = 12 + 1 + 7;

/// A flat value the reviewer of #2331 found: change-of-value parameters
/// whose bitmask claims nine unused bits. It has an encoding, which is all
/// the flat form is checked for, and the framed decoder refuses that
/// encoding.
fn nine_unused_bits() -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Unsigned(2),
        PropertyValue::Unsigned(1),
        PropertyValue::BitString {
            unused_bits: 9,
            data: vec![0xFF],
        },
    ])
}

/// An in-process write in the flat form whose encoding wouldn't read back
/// is refused by an enrollment with storage, as octets that are no value
/// of the property are on the wire, and nothing is saved: what is saved
/// always reopens. Without storage the enrollment takes it, as before.
#[test]
fn a_flat_value_that_would_not_reopen_is_refused_with_storage() {
    let undecodable = PropertyValue::ApplicationData(vec![0x2E, 0x09, 0x01]);
    let on_the_wire = open(None)
        .write_property(P::EVENT_PARAMETERS, None, undecodable, None)
        .unwrap_err();
    let file = Arc::new(FileOctets::default());
    let mut kept = built_on(Some(file.clone())).unwrap();
    let before = read(&kept, P::EVENT_PARAMETERS);
    let refusal = kept
        .write_property(P::EVENT_PARAMETERS, None, nine_unused_bits(), None)
        .unwrap_err();
    assert_eq!(refusal.to_string(), on_the_wire.to_string());
    assert_eq!(read(&kept, P::EVENT_PARAMETERS), before);
    assert_eq!(file.saves.load(Ordering::SeqCst), 0);
    assert!(file.file.lock().unwrap().is_none());
    // The row stays the setter's: no write got through for it.
    assert_eq!(kept.written_reporting().algorithm, Default::default());
    drop(kept);
    built_on(Some(file)).unwrap();

    let mut plain = open(None);
    plain
        .write_property(P::EVENT_PARAMETERS, None, nine_unused_bits(), None)
        .unwrap();
}

/// One change somewhere in a flat value: a bit string's unused bits or its
/// octets, a number's bits, or an element swapped for another datatype.
fn tweak(value: &mut PropertyValue, random: &mut XorShift) {
    let pool = [
        PropertyValue::Null,
        PropertyValue::Boolean(true),
        PropertyValue::Unsigned(u64::MAX),
        PropertyValue::Signed(-1),
        PropertyValue::Real(f32::NAN),
        PropertyValue::Double(f64::NEG_INFINITY),
        PropertyValue::Enumerated(u32::MAX),
        PropertyValue::OctetString(vec![0x1E, 0x2F]),
        PropertyValue::CharacterString("é".into()),
        PropertyValue::BitString {
            unused_bits: 200,
            data: vec![],
        },
        PropertyValue::List(vec![]),
    ];
    match value {
        PropertyValue::List(items) if !items.is_empty() && random.below(5) != 0 => {
            let at = random.below(items.len());
            tweak(&mut items[at], random);
        }
        PropertyValue::BitString { unused_bits, data } => match random.below(3) {
            0 => *unused_bits = random.next() as u8,
            1 => data.push(random.next() as u8),
            _ => data.clear(),
        },
        PropertyValue::Real(real) => *real = f32::from_bits(random.next() as u32),
        PropertyValue::Double(double) => *double = f64::from_bits(random.next()),
        PropertyValue::Unsigned(unsigned) => *unsigned = random.next() >> random.below(64),
        PropertyValue::OctetString(octets) => octets.push(random.next() as u8),
        other => *other = pool[random.below(pool.len())].clone(),
    }
}

/// Whatever flat value an enrollment with storage accepts is saved as a
/// file a rebuilt enrollment opens, serving the value as it was served:
/// every choice of both properties in its flat form, and each changed one
/// to three times over, 150 times a choice. A value it refuses saves
/// nothing. Only values the enrollment takes without storage are tried,
/// and some of those are refused with it, the one above among them.
#[test]
fn every_flat_value_that_is_accepted_reopens() {
    let events = every_event_choice()
        .into_iter()
        .map(|parameters| (P::EVENT_PARAMETERS, parameters.encode()));
    let faults = every_fault_choice()
        .into_iter()
        .map(|parameters| (P::FAULT_PARAMETERS, parameters.encode_property_value()));
    let mut random = XorShift(0x2331_F1A7_0BAC_0E77);
    let (mut whole, mut reopened, mut refused) = (0u32, 0u32, 0u32);
    let mut plain = open(None);
    for (property, choice) in events.chain(faults) {
        for round in 0..=150 {
            let mut value = choice.clone();
            if round > 0 {
                for _ in 0..=random.below(3) {
                    tweak(&mut value, &mut random);
                }
            }
            let taken = plain.write_property(property, None, value.clone(), None);
            if round == 0 {
                // Each choice's own flat form is one the enrollment takes.
                whole += u32::from(taken.is_ok());
            }
            if taken.is_err() {
                continue;
            }
            let file = Arc::new(FileOctets::default());
            let mut kept = built_on(Some(file.clone())).unwrap();
            match kept.write_property(property, None, value.clone(), None) {
                Ok(()) => {
                    let served = read(&kept, property);
                    drop(kept);
                    let rebuilt = built_on(Some(file.clone())).unwrap_or_else(|error| {
                        panic!("{value:?} was saved and doesn't reopen: {error:?}")
                    });
                    assert_eq!(read(&rebuilt, property), served, "{value:?}");
                    reopened += 1;
                }
                Err(_) => {
                    assert!(file.file.lock().unwrap().is_none(), "{value:?}");
                    refused += 1;
                }
            }
        }
    }
    let choices = (every_event_choice().len() + every_fault_choice().len()) as u32;
    assert_eq!(whole, choices, "flat forms taken");
    assert!(reopened > 500, "{reopened} reopened");
    assert!(refused > 0, "{refused} refused only with storage");
}

/// An Event Enrollment the built-in constructor makes, as CreateObject
/// does, has no storage: it lends no staging, takes parameters past the
/// cap, and its setters always apply.
#[test]
fn a_created_enrollment_keeps_nothing() {
    use bacnet_types::enums::ObjectType;
    let constructor = crate::creation::builtin_constructor(ObjectType::EVENT_ENROLLMENT).unwrap();
    let mut made = constructor(3, "made".into(), &[]).unwrap();
    for (property, value, _) in covered() {
        made.write_property(property, None, value.clone(), None)
            .unwrap();
        assert_eq!(read(made.as_ref(), property), value);
    }
    let BACnetEventParameter::Opaque { data, .. } = legacy(CAP - 3) else {
        unreachable!()
    };
    let long = PropertyValue::OctetString(data);
    made.write_property(P::EVENT_PARAMETERS, None, long, None)
        .unwrap();
    #[cfg(feature = "durable-writes")]
    assert!(made.durable_writes().is_none());
}

/// The enrollment with an alarm standing, a delay counting toward NORMAL
/// and both baselines set, as the server's pass leaves one.
fn in_alarm() -> EventEnrollmentObject {
    let mut enrollment = open(None);
    enrollment.set_event_state(EventState::HIGH_LIMIT);
    let eval = enrollment.enrollment_eval_mut().unwrap();
    eval.set_eval_state(EventEnrollmentEvalState {
        pending: Some(EventEnrollmentPending {
            state: EventState::NORMAL,
            remaining: 3,
            condition: 0,
            params_fingerprint: 42,
        }),
        cov_baseline: Some(PropertyValue::Real(20.0)),
        last_offnormal_value: Some(7),
    })
    .unwrap();
    enrollment
}

/// What a write or a restore can touch beyond the row itself.
fn left(enrollment: &EventEnrollmentObject) -> (Vec<PropertyValue>, EventEnrollmentEvalState) {
    let served = [
        P::EVENT_STATE,
        P::RELIABILITY,
        P::STATUS_FLAGS,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::OBJECT_PROPERTY_REFERENCE,
    ]
    .map(|property| enrollment.read_property(property, None).unwrap());
    let eval = enrollment.enrollment_eval().unwrap().eval_state();
    (served.to_vec(), eval)
}

/// A restore leaves the enrollment as a write of the same row does. Either
/// drops a delay counting under the old Event_Parameters or
/// Time_Delay_Normal and nothing else: Event_State, Reliability, the
/// acknowledgments and the algorithms' baselines stay, and nothing is
/// evaluated. Fault_Parameters, written or restored, leaves the delay
/// counting too.
#[test]
fn a_restore_leaves_the_enrollment_as_a_write_does() {
    for (property, value, stored) in covered() {
        let mut written = in_alarm();
        let before = left(&written);
        assert!(before.1.pending.is_some());
        write(&mut written, property, value.clone());
        let restored = in_alarm()
            .with_reporting_persistence(persistence(&holding(stored)))
            .unwrap();
        assert_eq!(read(&restored, property), value, "{property:?}");
        assert_eq!(left(&restored), left(&written), "{property:?}");
        let after = left(&restored);
        assert_eq!(after.0, before.0, "{property:?}: nothing served moved");
        assert_eq!(after.1.cov_baseline, before.1.cov_baseline);
        assert_eq!(after.1.last_offnormal_value, before.1.last_offnormal_value);
        let keeps_counting = property == P::FAULT_PARAMETERS;
        assert_eq!(after.1.pending.is_some(), keeps_counting, "{property:?}");
    }
    // On a fresh enrollment there is nothing to drop: it starts in NORMAL
    // with no fault and no private state, whatever was saved.
    let all = AlgorithmWriteConfig {
        event_parameters: Some(state_parameters()),
        fault_parameters: Some(range_fault()),
        time_delay_normal: Some(9),
        ..Default::default()
    };
    let fresh = open(Some(&holding(all)));
    let (served, eval) = left(&fresh);
    assert_eq!(served[0], enumerated(EventState::NORMAL.to_raw()));
    assert_eq!(
        served[1],
        enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(eval, EventEnrollmentEvalState::default());
    // Storage holding no parameter restores none: the delay goes on.
    let untouched = in_alarm()
        .with_reporting_persistence(persistence(&MemoryConfig::new()))
        .unwrap();
    assert_eq!(left(&untouched), left(&in_alarm()));
}

/// What isn't kept can differ on the enrollment rebuilt on a file: the
/// constructor's event type, which is the Event_Type served for the legacy
/// octet layout and for a proprietary alternative, and
/// Object_Property_Reference. No
/// write of the parameters is checked against either, so the saved value
/// is one the rebuilt enrollment takes on the wire too: it opens, serves
/// the value as saved, and Event_Type is the rebuilt enrollment's own for
/// those two forms and the parameters' for every other.
#[test]
fn saved_parameters_open_on_an_enrollment_built_another_way() {
    let legacy_octets = PropertyValue::OctetString(vec![0, 0, 0x20, 0x42, 0, 0, 0xA0, 0x40]);
    let proprietary = event_value(&BACnetEventParameter::Opaque {
        tag: 200,
        data: vec![0x09, 0x01],
    });
    let cases = [
        (legacy_octets, Some(EventType::OUT_OF_RANGE)),
        (proprietary, Some(EventType::OUT_OF_RANGE)),
        (event_value(&state_parameters()), None),
    ];
    for (value, follows_constructor) in cases {
        let storage = MemoryConfig::new();
        let mut enrollment = open(Some(&storage));
        write(&mut enrollment, P::EVENT_PARAMETERS, value.clone());
        write(
            &mut enrollment,
            P::FAULT_PARAMETERS,
            fault_value(&range_fault()),
        );
        let served = read(&enrollment, P::EVENT_PARAMETERS);
        let event_type = read(&enrollment, P::EVENT_TYPE);
        if let Some(constructor) = follows_constructor {
            assert_eq!(event_type, enumerated(constructor.to_raw()));
        }
        drop(enrollment);

        // Another constructor type, another monitored property, and none.
        for reference in [Some(monitored(2)), None] {
            let mut other = EventEnrollmentObject::new(1, "EE", EventType::FLOATING_LIMIT).unwrap();
            other.set_object_property_reference(reference).unwrap();
            let mut other = other
                .with_reporting_persistence(persistence(&storage))
                .unwrap_or_else(|error| panic!("{value:?} doesn't open: {error:?}"));
            assert_eq!(read(&other, P::EVENT_PARAMETERS), served);
            let expected = match follows_constructor {
                Some(_) => enumerated(EventType::FLOATING_LIMIT.to_raw()),
                None => event_type.clone(),
            };
            assert_eq!(read(&other, P::EVENT_TYPE), expected);
            assert_eq!(
                read(&other, P::FAULT_TYPE),
                enumerated(FaultType::FAULT_OUT_OF_RANGE.to_raw())
            );
            // And the rebuilt enrollment takes a write of the same value.
            other
                .write_property(P::EVENT_PARAMETERS, None, value.clone(), None)
                .unwrap();
        }
    }
}

/// The reporting rows and Description are kept beside the parameters as
/// before, in the one file: kind 5 around kind 6.
#[test]
fn the_reporting_rows_are_kept_beside_the_parameters() {
    let file = Arc::new(FileOctets::default());
    let mut enrollment = built_on(Some(file.clone())).unwrap();
    for (property, value, _) in covered() {
        write(&mut enrollment, property, value);
    }
    write(
        &mut enrollment,
        P::NOTIFICATION_CLASS,
        PropertyValue::Unsigned(9),
    );
    let label = PropertyValue::CharacterString("lobby".into());
    write(&mut enrollment, P::DESCRIPTION, label.clone());
    let kept = enrollment.written_reporting();
    drop(enrollment);
    let octets = file.file();
    assert_eq!(octets[12], format::KIND_WITH_DESCRIPTION);
    assert_eq!(octets[12 + 5 + 5], format::KIND_REPORTING_PARAMETERS);
    let rebuilt = built_on(Some(file.clone())).unwrap();
    assert_eq!(rebuilt.written_reporting(), kept);
    assert_eq!(read(&rebuilt, P::DESCRIPTION), label);
    assert_eq!(
        read(&rebuilt, P::NOTIFICATION_CLASS),
        PropertyValue::Unsigned(9)
    );
    for (property, value, _) in covered() {
        assert_eq!(read(&rebuilt, property), value, "{property:?}");
    }
}
