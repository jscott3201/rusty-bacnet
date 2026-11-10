//! `set_alarm_values` on a Multi-state Input and a Multi-state Value
//! checks its list as a write of it is checked (#2415): a zero, a state
//! past Number_Of_States and a list past 1,024 entries are refused with the
//! error the write gets, and the list held stays. What it takes, a peer can
//! read and write back. `set_number_of_states` still shrinks the count
//! under a held list and leaves the list as it is.

use super::super::*;
use PropertyIdentifier as P;

fn states(values: &[u32]) -> PropertyValue {
    let values = values
        .iter()
        .map(|state| PropertyValue::Unsigned(u64::from(*state)));
    PropertyValue::List(values.collect())
}

/// Run `$body` on a Multi-state Input and a Multi-state Value of four
/// states, each holding Alarm_Values [2]: `$object` names it.
macro_rules! on_both {
    (|$object:ident| $body:block) => {{
        {
            let mut $object = MultiStateInputObject::new(1, "MSI", 4).unwrap();
            $object.set_alarm_values(vec![2]).unwrap();
            $body
        }
        {
            let mut $object = MultiStateValueObject::new(1, "MSV", 4).unwrap();
            $object.set_alarm_values(vec![2]).unwrap();
            $body
        }
    }};
}

/// Each list a write refuses, the setter refuses with the same error, the
/// same element named, and the list held stays.
#[test]
fn a_list_a_write_refuses_the_setter_refuses_with_the_write_s_error() {
    let over: Vec<u32> = (0..=MAX_ALARM_VALUES as u32).map(|at| at % 4 + 1).collect();
    let refused = [
        vec![0],
        vec![1, 0],
        vec![5],
        vec![3, 4, 5],
        vec![u32::MAX],
        over,
    ];
    on_both!(|object| {
        for list in &refused {
            let mut plain = MultiStateInputObject::new(9, "plain", 4).unwrap();
            let written = plain
                .write_property(P::ALARM_VALUES, None, states(list), None)
                .unwrap_err();
            let set = object.set_alarm_values(list.clone()).unwrap_err();
            assert_eq!(format!("{set:?}"), format!("{written:?}"), "{list:?}");
            assert_eq!(
                object.read_property(P::ALARM_VALUES, None).unwrap(),
                states(&[2])
            );
        }
    });
}

/// A list the setter takes is one a peer can write back as it reads it:
/// empty, the edges of the count, duplicates, and the full 1,024 entries.
#[test]
fn a_list_the_setter_takes_a_peer_can_read_and_write_back() {
    let full: Vec<u32> = (0..MAX_ALARM_VALUES as u32).map(|at| at % 4 + 1).collect();
    let taken = [vec![], vec![1], vec![4], vec![3, 3, 1], full];
    on_both!(|object| {
        for list in &taken {
            object.set_alarm_values(list.clone()).unwrap();
            let read = object.read_property(P::ALARM_VALUES, None).unwrap();
            assert_eq!(read, states(list));
            object
                .write_property(P::ALARM_VALUES, None, read.clone(), None)
                .unwrap();
            assert_eq!(object.read_property(P::ALARM_VALUES, None).unwrap(), read);
        }
    });
}

/// The setter checks against the count the object has now: a list naming
/// a state a later grow adds is refused until the grow.
#[test]
fn the_setter_checks_against_the_count_the_object_has_now() {
    on_both!(|object| {
        assert!(object.set_alarm_values(vec![6]).is_err());
        object.set_number_of_states(6).unwrap();
        object.set_alarm_values(vec![6]).unwrap();
    });
}

/// `set_number_of_states` shrinking the count under a held list keeps the
/// list (#2415): a local change repairs nothing. The Value reports the
/// stranded entry as CONFIGURATION_ERROR; the setter then refuses the list
/// it holds, as a write does.
#[test]
fn a_local_shrink_under_a_held_list_keeps_the_list() {
    on_both!(|object| {
        object.set_alarm_values(vec![2, 4]).unwrap();
        object.set_number_of_states(3).unwrap();
        assert_eq!(
            object.read_property(P::ALARM_VALUES, None).unwrap(),
            states(&[2, 4])
        );
        assert!(object.set_alarm_values(vec![2, 4]).is_err());
        assert_eq!(
            object.read_property(P::NUMBER_OF_STATES, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
    });
    let mut value = MultiStateValueObject::new(1, "MSV", 4).unwrap();
    value.set_alarm_values(vec![4]).unwrap();
    value.set_number_of_states(3).unwrap();
    assert_eq!(
        value.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::CONFIGURATION_ERROR.to_raw())
    );
}
