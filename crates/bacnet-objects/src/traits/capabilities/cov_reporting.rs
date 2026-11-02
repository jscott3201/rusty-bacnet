//! The COV reporting capability: which subscriptions an object takes, and
//! what its notifications carry.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

/// One property a whole-object (SubscribeCOV) notification reports after
/// Present_Value and Status_Flags, from the object type's Table 13-1 row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// The built-in objects of these types answer
    /// [`CovReporting::cov_reported_properties`] with it, and a custom object
    /// of a standard type can too.
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
/// The built-in objects that lend it: the Analog, Binary and Multi-state
/// Inputs, Outputs and Values; the Integer, Positive Integer, Large Analog,
/// CharacterString and other value types; the Loop, Staging, Load Control,
/// Accumulator and Pulse Converter; the Lighting and Binary Lighting
/// Outputs, Color and Color Temperature; the Life Safety Point and Zone;
/// the Access Door and Point and the Credential Data Input; and, for
/// property subscriptions only, the Averaging and Command objects.
pub trait CovReporting {
    /// Whether SubscribeCOV, the whole-object form, may subscribe to this
    /// object.
    ///
    /// The default, `true`, suits a type with Table 13-1 criteria. An object
    /// whose type has none answers `false`: it still takes property
    /// subscriptions, which follow Table 13-1a, but the bundled server
    /// refuses SubscribeCOV naming it. The built-in Averaging and Command
    /// objects do this.
    fn supports_subscribe_cov(&self) -> bool {
        true
    }

    /// Whether a property subscription (SubscribeCOVProperty or
    /// SubscribeCOVPropertyMultiple) may monitor `property`.
    ///
    /// The server asks this only of a property that reads; a `false` answer
    /// refuses the subscription with NOT_COV_PROPERTY. The default takes
    /// every property. The built-in Life Safety Point and Zone take only
    /// Present_Value, Status_Flags, Tracking_Value, Silenced and
    /// Operation_Expected, and a custom one should narrow it the same way.
    fn supports_cov_property(&self, _property: PropertyIdentifier) -> bool {
        true
    }

    /// The object's COV_Increment, for an object whose numeric
    /// Present_Value reports only a change at least this large.
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
    /// The default reports nothing more. An object of a type whose Table
    /// 13-1 row lists more answers that row,
    /// [`CovReportedProperty::standard_for`] its type, as the built-in ones
    /// do.
    fn cov_reported_properties(&self) -> &'static [CovReportedProperty] {
        &[]
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
