use super::*;
use crate::object_profile::TagsPersistence;
use crate::prelude::*;
use crate::present_value_access::PresentValueAccess;
use crate::property_metadata::PropertyMetadata;
use alloc::sync::Arc;
use bacnet_types::enums::PropertyIdentifier as P;

#[cfg(test)]
mod nonfinite_tests;

// ---------------------------------------------------------------------------
// AnalogValue (type 2)
// ---------------------------------------------------------------------------

/// BACnet Analog Value object.
pub struct AnalogValueObject {
    pub(super) core: IoCore,
    event_detector: OutOfRangeDetector,
    analog: AnalogProperties,
    present_value: f32,
    fault_out_of_range: FaultOutOfRangeState,
    audit_policy: crate::audit::ObjectAuditPolicy,
    access: PresentValueAccess,
    /// 16-level priority array. `None` = no command at that level.
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
    /// Value source tracking.
    value_source: crate::command_source::ValueSourceTracking,
    /// The last writer of a noncommandable Present_Value, once tracked (#1552).
    write_source: crate::command_source::SingleValueSource,
}

impl AnalogValueObject {
    /// Provision independently optional Audit properties before registration.
    /// Raw object configuration bypasses server notification ownership; use
    /// BACnetServer::write_local or WP/WPM for live property mutations.
    pub fn set_audit_policy(&mut self, policy: crate::audit::ObjectAuditPolicy) {
        self.audit_policy = policy;
    }

    /// Create a new Analog Value object with a commandable Present_Value.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
    ) -> Result<Self, Error> {
        Self::with_access(instance, name, units, PresentValueAccess::Commandable)
    }

    /// Create a new Analog Value object whose Present_Value is written as `access` says.
    pub fn with_access(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
        access: PresentValueAccess,
    ) -> Result<Self, Error> {
        Ok(Self {
            core: IoCore::new(ObjectType::ANALOG_VALUE, instance, name.into())?,
            event_detector: OutOfRangeDetector::default(),
            analog: AnalogProperties::new(units),
            present_value: 0.0,
            fault_out_of_range: FaultOutOfRangeState::default(),
            audit_policy: crate::audit::ObjectAuditPolicy::default(),
            access,
            priority_array: [None; 16],
            relinquish_default: 0.0,
            value_source: crate::command_source::ValueSourceTracking::default(),
            write_source: crate::command_source::SingleValueSource::default(),
        })
    }

    /// Build with application-owned Tags storage and the requested Present_Value access.
    /// Saved Tags override configured Tags only while [`Self::set_profile`]
    /// provisions the row. Loaded data must satisfy the shared Tags rules and
    /// the opt-in 1 MiB snapshot limit.
    pub fn with_tags_persistence(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
        access: PresentValueAccess,
        persistence: Arc<dyn TagsPersistence>,
    ) -> Result<Self, Error> {
        let mut object = Self::with_access(instance, name, units, access)?;
        object.core.keep_tags(persistence)?;
        Ok(object)
    }

    /// Track the source of a noncommandable Present_Value (Clause 19.5,
    /// #1552), provisioned before registration like the audit policy.
    ///
    /// On, the object serves Value_Source: the last writer, network or
    /// local, which alone may then correct it. A write must name its writer
    /// (`write_property_from`), and an application update through
    /// its `LocalInput` with no origin leaves no source (NONE). A commandable
    /// object always tracks its sources through Priority_Array, so this
    /// changes nothing on one.
    pub fn set_value_source_tracking(&mut self, enabled: bool) {
        self.write_source.set_enabled(enabled);
    }

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }

    /// Set the Relinquish_Default (#270).
    ///
    /// Validated the same way a commanded Present_Value is (finite Real);
    /// after the store, Present_Value is resolved anew from the priority
    /// array so an empty array falls back to the new default immediately.
    pub fn set_relinquish_default(&mut self, value: f32) -> Result<(), Error> {
        if self.access != PresentValueAccess::Commandable {
            return Err(common::unknown_property_error());
        }
        common::reject_non_finite(value)?;
        self.relinquish_default = value;
        self.recalculate_present_value();
        Ok(())
    }

    /// A noncommandable Present_Value write, network-equivalent.
    fn write_direct_present_value(
        &mut self,
        array_index: Option<u32>,
        value: PropertyValue,
    ) -> Result<(), Error> {
        if array_index.is_some() {
            return Err(common::property_is_not_an_array_error());
        }
        if self.access == PresentValueAccess::ReadOnly && !self.core.out_of_service {
            return Err(common::write_access_denied_error());
        }
        // Clause 19.2: an otherwise permitted noncommandable NULL is a no-op.
        if value == PropertyValue::Null {
            return Ok(());
        }
        self.present_value = Self::checked_present_value(value)?;
        Ok(())
    }

    fn checked_present_value(value: PropertyValue) -> Result<f32, Error> {
        let PropertyValue::Real(v) = value else {
            return Err(common::invalid_data_type_error());
        };
        if !v.is_finite() {
            return Err(common::value_out_of_range_error());
        }
        Ok(v)
    }

    /// The rows before the access projects them: Table 12-4's, then the
    /// audit rows when provisioned, the fault limits when configured, the
    /// bounds that are set and the profile's.
    fn rows(&self) -> Cow<'static, [PropertyMetadata]> {
        let mut rows = Cow::Borrowed(metadata::VALUE);
        if self.audit_policy != crate::audit::ObjectAuditPolicy::default() {
            rows.to_mut().extend(self.audit_policy.metadata());
        }
        self.fault_out_of_range.add_rows(&mut rows);
        self.analog.add_rows(&mut rows);
        self.core.add_profile_rows(&mut rows);
        rows
    }

    /// Whether `property` is absent under the object's Present_Value access.
    fn excludes(&self, property: PropertyIdentifier) -> bool {
        self.access != PresentValueAccess::Commandable
            && self
                .access
                .excludes(&self.rows(), property, self.write_source.is_enabled())
    }
}

crate::io_core::impl_profile!(AnalogValueObject);
super::impl_analog_properties!(AnalogValueObject);
super::impl_fault_out_of_range!(AnalogValueObject);
crate::write_config::reporting::impl_reporting_persistence!(AnalogValueObject, in core);

impl BACnetObject for AnalogValueObject {
    crate::traits::lend_audit_policy!();

    fn object_identifier(&self) -> ObjectIdentifier {
        self.core.oid
    }

    fn object_name(&self) -> &str {
        &self.core.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        crate::property_metadata::check_read_array_index(self, property, array_index)?;
        if self.excludes(property) {
            return Err(common::unknown_property_error());
        }
        let source = match self.access {
            PresentValueAccess::Commandable => {
                self.value_source
                    .read(property, array_index, &self.priority_array)
            }
            _ => self.write_source.read(property, array_index),
        };
        if let Some(result) = source {
            return result;
        }
        if let Some(result) = self.audit_policy.read(property, array_index) {
            return result;
        }
        if let Some(result) = self.core.read(&self.event_detector, property, array_index) {
            return result;
        }
        if let Some(result) = self.analog.read(property) {
            return result;
        }
        if let Some(value) = self.fault_out_of_range.read_limit(property) {
            return Ok(value);
        }
        match property {
            P::PRESENT_VALUE => Ok(PropertyValue::Real(self.present_value)),
            P::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Real)
            }
            P::RELINQUISH_DEFAULT => Ok(PropertyValue::Real(self.relinquish_default)),
            P::CURRENT_COMMAND_PRIORITY => {
                Ok(common::current_command_priority(&self.priority_array))
            }
            P::PROPERTY_LIST => {
                crate::property_metadata::read_property_list(&self.property_metadata(), array_index)
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property_from(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        origin: &crate::command_source::CommandOrigin,
    ) -> Result<(), Error> {
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        if property == P::VALUE_SOURCE {
            return match self.access {
                PresentValueAccess::Commandable => {
                    self.value_source.correct(value, priority, origin)
                }
                _ => self.write_source.correct(array_index, value, origin),
            };
        }
        if property == P::PRESENT_VALUE {
            return match self.access {
                PresentValueAccess::Commandable => {
                    crate::command_source::write_sourced_priority!(
                        self,
                        value,
                        priority,
                        origin,
                        Self::checked_present_value
                    )
                }
                PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                    self.write_source.admit(origin)?;
                    // A NULL is a no-op (Clause 19.2), so it has no writer.
                    let null = value == PropertyValue::Null;
                    self.write_direct_present_value(array_index, value)?;
                    if !null {
                        self.write_source.record(origin);
                    }
                    Ok(())
                }
            };
        }
        self.write_property(property, array_index, value, priority)
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        crate::write_config::reporting::admit(self, property, array_index, &value)?;
        if self.excludes(property) {
            return Err(common::unknown_property_error());
        }
        if let Some(result) = self
            .audit_policy
            .write(property, array_index, &value, priority)
        {
            return result;
        }
        if property == P::PRESENT_VALUE {
            if self.access == PresentValueAccess::Commandable {
                return Err(common::write_access_denied_error());
            }
            // A tracked source needs the writer (`write_property_from`).
            self.write_source.unsourced()?;
            return self.write_direct_present_value(array_index, value);
        }
        if property == P::VALUE_SOURCE {
            return Err(common::write_access_denied_error());
        }
        if property == P::RELINQUISH_DEFAULT {
            if let PropertyValue::Real(v) = value {
                return self.set_relinquish_default(v);
            }
            return Err(common::invalid_data_type_error());
        }
        // Clause 12.4 requires simulation/test writes while Out_Of_Service is TRUE:
        // Present_Value is writable, as is Reliability when that property exists
        // and supports values beyond NO_FAULT_DETECTED; the core takes the
        // Reliability write. `is_writable_property` stays statically true
        // because it describes capability.
        if let Some(result) =
            self.core
                .write(&mut self.event_detector, property, array_index, &value)
        {
            return result.map(drop);
        }
        if let Some(result) = self.analog.write(property, &value) {
            return result;
        }
        Err(crate::common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
        ))
    }

    #[cfg(feature = "durable-writes")]
    fn durable_writes(&mut self) -> Option<&mut dyn crate::durable::DurableWrites> {
        self.durable_capability()
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.access
            .project(self.rows(), self.write_source.is_enabled())
    }

    crate::traits::lend_cov_reporting!();

    crate::traits::lend_event_reporting!();

    crate::traits::lend_reliability_eval!();

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }
}

crate::traits::impl_audit_policy!(AnalogValueObject);

crate::event::impl_builtin_intrinsic_reporting!(
    AnalogValueObject,
    event_detector,
    core.event_history,
    [present_value],
    core.reliability,
    core.event_detection_enable,
    OutOfRangeDetector::ALGORITHM
);

/// The Present_Value the application supplies while it isn't commandable,
/// published as Value_Source from `origin` when the object tracks its
/// source (#1552).
impl crate::traits::LocalInput for AnalogValueObject {
    fn apply_local_input(&mut self, input: crate::traits::LocalInputValue) -> Result<(), Error> {
        let crate::traits::LocalInputValue::PresentValue { value, origin } = input else {
            return Err(crate::traits::not_supported());
        };
        let Some(origin) = origin else {
            return self.apply_local_present_value(value);
        };
        self.write_source.admit(&origin)?;
        self.apply_local_present_value(value)?;
        self.write_source.record(&origin);
        Ok(())
    }
}

impl AnalogValueObject {
    /// The application's Present_Value, refused while it's commandable or
    /// Out_Of_Service is TRUE.
    fn apply_local_present_value(&mut self, value: PropertyValue) -> Result<(), Error> {
        match self.access {
            PresentValueAccess::Commandable => Err(crate::traits::not_supported()),
            _ if self.core.out_of_service => Err(common::write_access_denied_error()),
            PresentValueAccess::ReadOnly | PresentValueAccess::Writable => {
                self.present_value = Self::checked_present_value(value)?;
                self.write_source.forget();
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod detection_enable_reset_tests {
    use super::*;

    /// Regression guard for issue #123: once transitions populate timestamps and messages,
    /// disabling event detection must still restore their Clause 13.2.2.1 initial conditions.
    #[test]
    fn av_disabling_detection_resets_event_history() {
        let mut av = AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
        assert_eq!(
            av.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
        av.event_detector.event_state = bacnet_types::enums::EventState::HIGH_LIMIT;
        av.event_detector.acked_transitions = bacnet_types::bitstring::EventTransitionBits::empty();
        av.event_detector.pending = Some(crate::event::PendingTransition {
            state: bacnet_types::enums::EventState::HIGH_LIMIT,
            due: crate::event::test_clock::in_seconds(2),
        });
        av.event_detector.fault_reliability = Some(bacnet_types::enums::Reliability::NO_SENSOR);
        av.core.event_history.time_stamps = [
            BACnetTimeStamp::SequenceNumber(1),
            BACnetTimeStamp::SequenceNumber(2),
            BACnetTimeStamp::SequenceNumber(3),
        ];
        av.core.event_history.message_texts = ["offnormal".into(), "fault".into(), "normal".into()];

        av.write_property(
            PropertyIdentifier::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
            None,
        )
        .unwrap();

        assert_eq!(
            av.read_property(PropertyIdentifier::EVENT_DETECTION_ENABLE, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        assert_eq!(
            av.event_detector.event_state,
            bacnet_types::enums::EventState::NORMAL
        );
        assert_eq!(
            av.event_detector.acked_transitions,
            bacnet_types::bitstring::EventTransitionBits::all()
        );
        assert!(av.event_detector.pending.is_none());
        assert!(av.event_detector.fault_reliability.is_none());
        assert_eq!(
            av.core.event_history.time_stamps,
            [
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
                BACnetTimeStamp::SequenceNumber(0),
            ]
        );
        assert_eq!(
            av.core.event_history.message_texts,
            [String::new(), String::new(), String::new()]
        );
    }
}

#[cfg(test)]
mod fault_out_of_range_non_finite_tests {
    use super::*;
    use bacnet_types::enums::{EngineeringUnits, ErrorClass, ErrorCode, Reliability};
    use bacnet_types::error::RemoteError;

    fn assert_value_out_of_range(result: Result<ReliabilityEvaluation, Error>) {
        assert!(matches!(
            result,
            Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::VALUE_OUT_OF_RANGE
        ));
    }

    #[test]
    fn av_non_finite_monitored_values_preserve_reliability_status_and_ownership() {
        for non_finite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut normal =
                AnalogValueObject::new(1, "AV-normal", EngineeringUnits::DEGREES_CELSIUS).unwrap();
            normal.configure_fault_out_of_range(10.0, 20.0).unwrap();
            let status_before = normal
                .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap();
            normal.present_value = non_finite;

            assert_value_out_of_range(normal.reliability_eval().unwrap().evaluate_reliability());
            assert_eq!(normal.present_value.to_bits(), non_finite.to_bits());
            assert_eq!(normal.core.reliability, Reliability::NO_FAULT_DETECTED);
            assert_eq!(
                normal
                    .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                    .unwrap(),
                status_before
            );
            assert!(normal.fault_out_of_range.owned_fault.is_none());

            normal.present_value = 9.0;
            assert_eq!(
                normal
                    .reliability_eval()
                    .unwrap()
                    .evaluate_reliability()
                    .unwrap(),
                ReliabilityEvaluation::Changed {
                    old_reliability: Reliability::NO_FAULT_DETECTED,
                    new_reliability: Reliability::UNDER_RANGE,
                }
            );

            let mut owned =
                AnalogValueObject::new(2, "AV-owned", EngineeringUnits::DEGREES_CELSIUS).unwrap();
            owned.configure_fault_out_of_range(10.0, 20.0).unwrap();
            owned.present_value = 9.0;
            owned
                .reliability_eval()
                .unwrap()
                .evaluate_reliability()
                .unwrap();
            let status_before = owned
                .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                .unwrap();
            owned.present_value = non_finite;

            assert_value_out_of_range(owned.reliability_eval().unwrap().evaluate_reliability());
            assert_eq!(owned.present_value.to_bits(), non_finite.to_bits());
            assert_eq!(owned.core.reliability, Reliability::UNDER_RANGE);
            assert_eq!(
                owned
                    .read_property(PropertyIdentifier::STATUS_FLAGS, None)
                    .unwrap(),
                status_before
            );
            assert!(matches!(
                owned.fault_out_of_range.owned_fault,
                Some(OwnedRangeFault::UnderRange)
            ));

            owned.present_value = 21.0;
            assert_eq!(
                owned
                    .reliability_eval()
                    .unwrap()
                    .evaluate_reliability()
                    .unwrap(),
                ReliabilityEvaluation::Changed {
                    old_reliability: Reliability::UNDER_RANGE,
                    new_reliability: Reliability::OVER_RANGE,
                }
            );
        }
    }
}
