//! Every built-in object that lends a `LocalInput` refuses each input it
//! doesn't model with `not_supported()`, and leaves its state as it was
//! (#2179). The table names what each lender models, so a lender added to
//! the catalogue, or a refusal arm that starts accepting, fails here.

use crate::access_control::{
    AccessControlInput, AccessEventReport, CredentialReadReport, DoorStateReport,
};
use crate::catalogue::representatives;
use crate::load_control::LoadControlSetting;
use crate::traits::{BACnetObject, LocalInputValue, not_supported};
use bacnet_types::constructed::BACnetAuthenticationFactor;
use bacnet_types::enums::{AccessEvent, AuthenticationFactorType, ObjectType};
use bacnet_types::primitives::PropertyValue;

/// Each kind of input, by name, with a value of its own datatype.
fn inputs() -> Vec<(&'static str, LocalInputValue)> {
    let factor = BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::UNDEFINED,
        format_class: 0,
        value: Vec::new(),
    };
    vec![
        (
            "PresentValue",
            LocalInputValue::present_value(PropertyValue::Real(1.0)),
        ),
        (
            "TrackingValue",
            LocalInputValue::TrackingValue(PropertyValue::Enumerated(1)),
        ),
        (
            "ControlledVariableValue",
            LocalInputValue::ControlledVariableValue(PropertyValue::Real(2.0)),
        ),
        (
            "AveragingSample",
            LocalInputValue::AveragingSample(Some(PropertyValue::Real(3.0))),
        ),
        (
            "AccessEvent",
            LocalInputValue::AccessControl(AccessControlInput::AccessEvent(
                AccessEventReport::new(AccessEvent::GRANTED, 1),
            )),
        ),
        (
            "CredentialRead",
            LocalInputValue::AccessControl(AccessControlInput::CredentialRead(
                CredentialReadReport::new(factor),
            )),
        ),
        (
            "DoorState",
            LocalInputValue::AccessControl(AccessControlInput::DoorState(
                DoorStateReport::default(),
            )),
        ),
        ("Pulses", LocalInputValue::Pulses(5)),
        (
            "ProgramState",
            LocalInputValue::ProgramState(PropertyValue::Enumerated(2)),
        ),
        (
            "LoadControl",
            LocalInputValue::LoadControl(LoadControlSetting::StateDescription(Some(
                "shedding".into(),
            ))),
        ),
    ]
}

/// What each lender models; it refuses every other input.
const MODELS: &[(ObjectType, &[&str])] = &[
    (ObjectType::ANALOG_INPUT, &["PresentValue"]),
    (ObjectType::ANALOG_VALUE, &["PresentValue"]),
    (ObjectType::BINARY_INPUT, &["PresentValue"]),
    (ObjectType::BINARY_VALUE, &["PresentValue"]),
    (ObjectType::MULTI_STATE_INPUT, &["PresentValue"]),
    (ObjectType::MULTI_STATE_VALUE, &["PresentValue"]),
    (ObjectType::INTEGER_VALUE, &["PresentValue"]),
    (
        ObjectType::LOOP,
        &["PresentValue", "ControlledVariableValue"],
    ),
    (
        ObjectType::LIFE_SAFETY_POINT,
        &["PresentValue", "TrackingValue"],
    ),
    (
        ObjectType::LIFE_SAFETY_ZONE,
        &["PresentValue", "TrackingValue"],
    ),
    (ObjectType::AVERAGING, &["AveragingSample"]),
    (ObjectType::ACCESS_POINT, &["AccessEvent"]),
    (ObjectType::CREDENTIAL_DATA_INPUT, &["CredentialRead"]),
    (ObjectType::ACCESS_DOOR, &["DoorState"]),
    (ObjectType::ACCUMULATOR, &["Pulses"]),
    (ObjectType::PROGRAM, &["ProgramState"]),
    (ObjectType::LOAD_CONTROL, &["LoadControl"]),
];

/// Every property the object lists, read whole.
fn state(object: &dyn BACnetObject) -> Vec<String> {
    object
        .property_metadata()
        .iter()
        .map(|row| {
            let read = object.read_property(row.property_identifier, None);
            format!("{:?}: {read:?}", row.property_identifier)
        })
        .collect()
}

#[test]
fn every_lender_refuses_the_inputs_it_does_not_model() {
    let mut lenders = Vec::new();
    for mut object in representatives() {
        let object_type = object.object_identifier().object_type();
        if object.local_input().is_none() {
            continue;
        }
        lenders.push(object_type);
        let models = MODELS
            .iter()
            .find(|(listed, _)| *listed == object_type)
            .unwrap_or_else(|| panic!("{object_type:?} lends LocalInput: list it in MODELS"))
            .1;
        for (name, input) in inputs() {
            if models.contains(&name) {
                continue;
            }
            let before = state(object.as_ref());
            let result = object.local_input().unwrap().apply_local_input(input);
            assert_eq!(
                format!("{result:?}"),
                format!("{:?}", Err::<(), _>(not_supported())),
                "{object_type:?} must refuse {name}"
            );
            assert_eq!(
                state(object.as_ref()),
                before,
                "{object_type:?} changed refusing {name}"
            );
        }
    }
    let listed: Vec<_> = MODELS.iter().map(|(object_type, _)| *object_type).collect();
    for object_type in listed {
        assert!(
            lenders.contains(&object_type),
            "{object_type:?} is in MODELS but lends no LocalInput"
        );
    }
}
