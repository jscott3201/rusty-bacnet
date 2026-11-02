use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use super::CovReportedProperty;

/// The Table 13-1 properties a whole-object COV notification carries after
/// its leading value and Status_Flags, behind
/// [`super::BACnetObject::cov_reported_properties`]. A free function, so the
/// default trait method needs no `Self: Sized`, which would break
/// `dyn BACnetObject` dispatch.
///
/// Every row Table 13-1 gives more than Present_Value and Status_Flags is
/// here. Access Point's row leads with Access_Event instead of Present_Value;
/// the server supplies that leading value, so only the rest is listed.
pub(super) fn cov_reported_properties_default(
    object_type: ObjectType,
) -> &'static [CovReportedProperty] {
    use CovReportedProperty::{Trigger, Value};
    use PropertyIdentifier as P;
    match object_type {
        ObjectType::ACCESS_DOOR => &[Trigger(P::DOOR_ALARM_STATE)],
        ObjectType::ACCESS_POINT => &[
            Value(P::ACCESS_EVENT_TAG),
            Trigger(P::ACCESS_EVENT_TIME),
            Value(P::ACCESS_EVENT_CREDENTIAL),
            Value(P::ACCESS_EVENT_AUTHENTICATION_FACTOR),
        ],
        ObjectType::CREDENTIAL_DATA_INPUT => &[Trigger(P::UPDATE_TIME)],
        ObjectType::LOAD_CONTROL => &[
            Trigger(P::REQUESTED_SHED_LEVEL),
            Trigger(P::START_TIME),
            Trigger(P::SHED_DURATION),
            Trigger(P::DUTY_WINDOW),
        ],
        ObjectType::LOOP => &[Value(P::SETPOINT), Value(P::CONTROLLED_VARIABLE_VALUE)],
        ObjectType::PULSE_CONVERTER => &[Value(P::UPDATE_TIME)],
        ObjectType::STAGING => &[Trigger(P::PRESENT_STAGE)],
        _ => &[],
    }
}
