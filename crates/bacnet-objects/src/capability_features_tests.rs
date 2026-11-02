//! The capability features (#2215): with one off, no built-in object lends
//! that capability, through any of its accessors; with it on, some do. The
//! crate's other tests of a capability run only with its feature on.

use crate::catalogue::representatives;
use bacnet_types::enums::ObjectType;

/// The catalogue types that lend through `accessor`.
fn lenders(accessor: fn(&mut dyn crate::traits::BACnetObject) -> bool) -> Vec<ObjectType> {
    representatives()
        .into_iter()
        .filter_map(|mut object| {
            accessor(&mut *object).then(|| object.object_identifier().object_type())
        })
        .collect()
}

#[test]
fn event_reporting_is_lent_only_with_its_feature() {
    let on = cfg!(feature = "event-reporting");
    for (name, found) in [
        (
            "event_reporting",
            lenders(|o| o.event_reporting().is_some()),
        ),
        (
            "event_reporting_mut",
            lenders(|o| o.event_reporting_mut().is_some()),
        ),
        (
            "enrollment_eval",
            lenders(|o| o.enrollment_eval().is_some()),
        ),
        (
            "enrollment_eval_mut",
            lenders(|o| o.enrollment_eval_mut().is_some()),
        ),
    ] {
        assert_eq!(!found.is_empty(), on, "{name}: {found:?}");
    }
}

#[test]
fn durable_writes_are_lent_only_with_their_feature() {
    let found = lenders(|o| o.durable_writes().is_some());
    assert_eq!(
        !found.is_empty(),
        cfg!(feature = "durable-writes"),
        "{found:?}"
    );
}
