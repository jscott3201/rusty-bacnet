//! The object types the COV criteria table (Clause 13.1, Table 13-1) lists,
//! as this crate builds them (#1061): each takes SubscribeCOV, and the values
//! each reports after its leading value and Status_Flags follow its row.
//! Staging is pinned in `staging/tests.rs` instead, since building one takes
//! a stage ladder.

use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P};

use crate::access_control::{AccessDoorObject, AccessPointObject, CredentialDataInputObject};
use crate::accumulator::PulseConverterObject;
use crate::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::life_safety::{LifeSafetyPointObject, LifeSafetyZoneObject};
use crate::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use crate::load_control::LoadControlObject;
use crate::loop_obj::LoopObject;
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::traits::{BACnetObject, CovReportedProperty};
use crate::value_types::{
    CharacterStringValueObject, DatePatternValueObject, DateTimePatternValueObject,
    DateTimeValueObject, DateValueObject, IntegerValueObject, LargeAnalogValueObject,
    OctetStringValueObject, PositiveIntegerValueObject, TimePatternValueObject, TimeValueObject,
};

/// One object of every Table 13-1 type but Staging.
fn table_13_1_objects() -> Vec<Box<dyn BACnetObject>> {
    vec![
        Box::new(AccessDoorObject::new(1, "door").unwrap()),
        Box::new(AccessPointObject::new(1, "point").unwrap()),
        Box::new(AnalogInputObject::new(1, "ai", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(AnalogOutputObject::new(1, "ao", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(AnalogValueObject::new(1, "av", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(IntegerValueObject::new(1, "iv").unwrap()),
        Box::new(LargeAnalogValueObject::new(1, "lav").unwrap()),
        Box::new(LightingOutputObject::new(1, "lo").unwrap()),
        Box::new(PositiveIntegerValueObject::new(1, "piv").unwrap()),
        Box::new(BinaryInputObject::new(1, "bi").unwrap()),
        Box::new(BinaryLightingOutputObject::new(1, "blo").unwrap()),
        Box::new(BinaryOutputObject::new(1, "bo").unwrap()),
        Box::new(BinaryValueObject::new(1, "bv").unwrap()),
        Box::new(CharacterStringValueObject::new(1, "csv").unwrap()),
        Box::new(DateValueObject::new(1, "dv").unwrap()),
        Box::new(DatePatternValueObject::new(1, "dpv").unwrap()),
        Box::new(DateTimeValueObject::new(1, "dtv").unwrap()),
        Box::new(DateTimePatternValueObject::new(1, "dtpv").unwrap()),
        Box::new(LifeSafetyPointObject::new(1, "lsp").unwrap()),
        Box::new(LifeSafetyZoneObject::new(1, "lsz").unwrap()),
        Box::new(MultiStateInputObject::new(1, "msi", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "mso", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "msv", 3).unwrap()),
        Box::new(OctetStringValueObject::new(1, "osv").unwrap()),
        Box::new(TimeValueObject::new(1, "tv").unwrap()),
        Box::new(TimePatternValueObject::new(1, "tpv").unwrap()),
        Box::new(CredentialDataInputObject::new(1, "cdi").unwrap()),
        Box::new(LoadControlObject::new(1, "lc").unwrap()),
        Box::new(LoopObject::new(1, "loop", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(PulseConverterObject::new(1, "pc", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
    ]
}

#[test]
fn every_table_13_1_object_type_takes_subscribe_cov() {
    for object in table_13_1_objects() {
        let object_type = object.object_identifier().object_type();
        assert!(object.supports_cov(), "{object_type:?}");
        assert!(object.supports_subscribe_cov_property(), "{object_type:?}");
    }
}

#[test]
fn table_13_1_extra_values_follow_each_row() {
    use CovReportedProperty::{Trigger, Value};
    let rows: [(ObjectType, &[CovReportedProperty]); 6] = [
        (ObjectType::ACCESS_DOOR, &[Trigger(P::DOOR_ALARM_STATE)]),
        (
            ObjectType::ACCESS_POINT,
            &[
                Value(P::ACCESS_EVENT_TAG),
                Trigger(P::ACCESS_EVENT_TIME),
                Value(P::ACCESS_EVENT_CREDENTIAL),
                Value(P::ACCESS_EVENT_AUTHENTICATION_FACTOR),
            ],
        ),
        (
            ObjectType::CREDENTIAL_DATA_INPUT,
            &[Trigger(P::UPDATE_TIME)],
        ),
        (
            ObjectType::LOAD_CONTROL,
            &[
                Trigger(P::REQUESTED_SHED_LEVEL),
                Trigger(P::START_TIME),
                Trigger(P::SHED_DURATION),
                Trigger(P::DUTY_WINDOW),
            ],
        ),
        (
            ObjectType::LOOP,
            &[Value(P::SETPOINT), Value(P::CONTROLLED_VARIABLE_VALUE)],
        ),
        (ObjectType::PULSE_CONVERTER, &[Value(P::UPDATE_TIME)]),
    ];
    for object in table_13_1_objects() {
        let object_type = object.object_identifier().object_type();
        let expected = rows
            .iter()
            .find(|(row, _)| *row == object_type)
            .map_or(&[][..], |(_, reported)| *reported);
        assert_eq!(
            object.cov_reported_properties(),
            expected,
            "{object_type:?}"
        );
    }
}
