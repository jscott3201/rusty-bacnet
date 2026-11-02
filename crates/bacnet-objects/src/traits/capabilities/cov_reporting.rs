//! The COV reporting capability: which subscriptions an object takes, and
//! what its notifications carry.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

/// One property a whole-object (SubscribeCOV) notification reports after
/// Present_Value and Status_Flags, from the object type's Table 13-1 row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CovReportedProperty {
    /// Carried in each notification; a change of it alone sends none.
    Value(PropertyIdentifier),
    /// Carried in each notification, and any change of it sends one.
    Trigger(PropertyIdentifier),
}

impl CovReportedProperty {
    /// The reported property.
    pub const fn property(self) -> PropertyIdentifier {
        match self {
            Self::Value(property) | Self::Trigger(property) => property,
        }
    }

    /// Whether any change of the property triggers a notification.
    pub const fn triggers(self) -> bool {
        matches!(self, Self::Trigger(_))
    }

    /// The properties `object_type`'s Table 13-1 row reports after its
    /// leading value and Status_Flags, in report order; empty for a type
    /// whose row reports nothing more, or that has no row.
    ///
    /// The rows with more:
    ///
    /// - Access Door: Door_Alarm_State, a trigger.
    /// - Access Point: Access_Event_Tag, Access_Event_Time (a trigger),
    ///   Access_Event_Credential and Access_Event_Authentication_Factor.
    ///   Its row leads with Access_Event instead of Present_Value; the
    ///   server supplies that leading value, so only the rest is listed.
    /// - Credential Data Input: Update_Time, a trigger.
    /// - Load Control: Requested_Shed_Level, Start_Time, Shed_Duration and
    ///   Duty_Window, all triggers.
    /// - Loop: Setpoint and Controlled_Variable_Value.
    /// - Pulse Converter: Update_Time.
    /// - Staging: Present_Stage, a trigger.
    ///
    /// The bundled server reports this row for an object whose
    /// [`CovReporting::cov_reported_properties`] answers `None`, the default.
    pub fn standard_for(object_type: ObjectType) -> &'static [Self] {
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
}

/// An object that reports changes of value (Clause 13.1).
///
/// The object returns itself from
/// [`BACnetObject::cov`](crate::traits::BACnetObject::cov); an object that
/// lends none takes no COV subscription of any kind, and the bundled server
/// refuses SubscribeCOV, SubscribeCOVProperty and
/// SubscribeCOVPropertyMultiple naming it with OBJECT /
/// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Hosts only read through it, under a
/// shared database guard (subscription checks and the notification fan-out
/// both hold one), so it has one `&self` accessor.
///
/// Every method has a default, so an object that only has to say it takes
/// subscriptions writes `impl CovReporting for MyObject {}`. What the host
/// reports comes from the object's properties: the leading value (by object
/// type, Present_Value or an Access Point's Access_Event), Status_Flags
/// when the object has it, then
/// [`cov_reported_properties`](Self::cov_reported_properties).
///
/// Calls on a concrete type need `use bacnet_objects::traits::CovReporting`.
/// The crate's `cov_reporting_tests` pin which built-in types lend it.
pub trait CovReporting {
    /// Whether SubscribeCOV, the whole-object form, may subscribe to this
    /// object.
    ///
    /// The default, `true`, suits a type with Table 13-1 criteria. An object
    /// whose type has none answers `false`: it still takes property
    /// subscriptions, which follow Table 13-1a, but the bundled server
    /// refuses SubscribeCOV naming it with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. The built-in Averaging and
    /// Command objects do this. The reverse, SubscribeCOV without property
    /// subscriptions, can't be expressed: an object that lends this takes
    /// property subscriptions, narrowed only per property through
    /// [`supports_cov_property`](Self::supports_cov_property).
    fn supports_subscribe_cov(&self) -> bool {
        true
    }

    /// Whether a property subscription (SubscribeCOVProperty or
    /// SubscribeCOVPropertyMultiple) may monitor `property`.
    ///
    /// The server asks this only of a property that reads; a `false` answer
    /// refuses the subscription with PROPERTY / NOT_COV_PROPERTY (in
    /// SubscribeCOVPropertyMultiple, naming that reference). The default
    /// takes every property. The built-in Life Safety Point and Zone take
    /// only [`COV_PROPERTIES`](crate::life_safety::COV_PROPERTIES), and a
    /// custom one must narrow its own set the same way: the default doesn't
    /// read the object type.
    fn supports_cov_property(&self, _property: PropertyIdentifier) -> bool {
        true
    }

    /// The object's COV_Increment, for an object whose numeric
    /// Present_Value reports only a change at least this large.
    ///
    /// The server reads it at each evaluation, so return the current
    /// value: a write to COV_Increment takes effect at the next change.
    ///
    /// The built-in Analog Inputs, Outputs and Values, Loop, Staging, Pulse
    /// Converter, Lighting Output and the Integer, Positive Integer and Large
    /// Analog Value types answer theirs. A property subscription's own
    /// increment wins; a property subscription without one inherits this
    /// only for a numeric Present_Value, and other properties report on
    /// their own increment or on any change.
    ///
    /// It is an `f64` so every COV_Increment datatype fits without rounding:
    /// a REAL widens exactly, a Large Analog Value's Double is kept as is,
    /// and an Unsigned increment is exact up to 2^53. The default, `None`,
    /// reports any change, as a binary or multi-state object does.
    fn cov_increment(&self) -> Option<f64> {
        None
    }

    /// The properties a whole-object (SubscribeCOV) notification reports
    /// after its leading value and Status_Flags, in report order.
    ///
    /// A [`CovReportedProperty::Trigger`]'s change sends a notification by
    /// itself; a [`CovReportedProperty::Value`] only rides along. The server
    /// leaves out a listed property the object's Property_List lacks.
    /// Property subscriptions report their own property instead.
    ///
    /// The default, `None`, reports the object type's Table 13-1 row,
    /// [`CovReportedProperty::standard_for`] it, as every built-in object
    /// does. `Some(list)` reports exactly `list`, an empty one nothing
    /// more; a proprietary type, which has no row, may name its own.
    fn cov_reported_properties(&self) -> Option<&'static [CovReportedProperty]> {
        None
    }
}

/// The accessor of an object that lends itself as its [`CovReporting`],
/// for inside its `BACnetObject` impl.
macro_rules! lend_cov_reporting {
    () => {
        fn cov(&self) -> Option<&dyn $crate::traits::CovReporting> {
            Some(self)
        }
    };
}

pub(crate) use lend_cov_reporting;
