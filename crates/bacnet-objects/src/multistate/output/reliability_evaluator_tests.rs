//! The object-owned Reliability evaluator on a Multi-state Output.

use super::*;
use bacnet_types::enums::Reliability;

#[test]
fn configuration_error_dominates_bypassed_invalid_present_value() {
    let mut mso = MultiStateOutputObject::new(1, "MSO-dominance", 2).unwrap();
    mso.present_value = 3;
    mso.feedback_value = 3;

    mso.reliability_eval()
        .unwrap()
        .evaluate_reliability()
        .unwrap();
    assert_eq!(
        mso.core.reliability,
        Reliability::CONFIGURATION_ERROR,
        "invalid configuration must dominate invalid Present_Value"
    );
    mso.feedback_value = 1;
    mso.reliability_eval()
        .unwrap()
        .evaluate_reliability()
        .unwrap();
    assert_eq!(mso.core.reliability, Reliability::MULTI_STATE_OUT_OF_RANGE);
    mso.write_property_from(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Unsigned(1),
        None,
        &crate::command_source::test_origin(),
    )
    .unwrap();
    assert_eq!(
        mso.core.reliability,
        Reliability::NO_FAULT_DETECTED,
        "the central priority recalculation must recover synchronously"
    );
}
