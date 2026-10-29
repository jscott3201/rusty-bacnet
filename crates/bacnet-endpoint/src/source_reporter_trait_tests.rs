//! SourceReporter hands every `BACnetObject` method to the object it wraps
//! (#1097). The rows and the probe are shared with DeviceReadView's tests in
//! bacnet-server, and the trait's methods are read from its source: a method
//! added there fails `every_bacnet_object_method_has_a_forwarding_row` until
//! it has a row, and `every_method_reaches_the_wrapped_object` until
//! SourceReporter forwards it, arguments and answer unchanged.

use super::*;
#[path = "../../bacnet-objects/tests/support/forwarding/mod.rs"]
mod forwarding;

use bacnet_types::enums::ObjectType;
use forwarding::fixtures::oid;
use forwarding::probe::{CallLog, Probe, take};
use forwarding::rows::rows;

/// What one row answered, and the calls the probe logged for it.
type Outcome = (&'static str, String, Vec<String>);

#[test]
fn every_bacnet_object_method_has_a_forwarding_row() {
    forwarding::assert_rows_cover_the_trait();
    forwarding::assert_probe_answers_every_row();
}

/// Run every row on `object`, the probe's log beside each answer.
fn outcomes(object: &mut dyn BACnetObject, log: &CallLog) -> Vec<Outcome> {
    rows()
        .map(|(name, row)| (name, row.call(object), take(log)))
        .collect()
}

#[test]
fn every_method_reaches_the_wrapped_object() {
    // The method an active source answers without asking the wrapped object.
    const OWN_ANSWERS: [&str; 1] = ["is_deleteable"];
    let log = CallLog::default();
    let probe = Probe::new(ObjectType::ANALOG_VALUE, log.clone());
    let mut object: Box<dyn BACnetObject> = Box::new(probe);
    let direct = outcomes(object.as_mut(), &log);
    // Installing moves the box, not the probe, so the capabilities it lends
    // keep their addresses.
    let owner = AuditOwnership::for_source(
        oid(ObjectType::DEVICE, 123),
        oid(ObjectType::AUDIT_REPORTER, 1),
    );
    install(&mut object, &owner).unwrap();
    let active = outcomes(object.as_mut(), &log);
    owner.seal();
    let sealed = outcomes(object.as_mut(), &log);
    for ((expected, active), sealed) in direct.iter().zip(&active).zip(&sealed) {
        let name = expected.0;
        assert_eq!(
            sealed, expected,
            "{name}: SourceReporter must forward it, arguments and answer unchanged"
        );
        if OWN_ANSWERS.contains(&name) {
            assert_ne!(active, expected, "{name}: an active source answers it");
        } else {
            assert_eq!(
                active, expected,
                "{name}: SourceReporter must forward it, arguments and answer unchanged"
            );
        }
    }
}
