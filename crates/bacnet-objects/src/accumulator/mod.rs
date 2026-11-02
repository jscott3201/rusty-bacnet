//! Accumulator (type 23) and Pulse Converter (type 24) objects.
//!
//! Per ASHRAE 135-2020 §12.61 Table 12-79 (Accumulator) and §12.23 Table 12-27 (PulseConverter).

use crate::prelude::*;
use alloc::borrow::Cow;
use bacnet_encoding::constructed::{encode_prescale, encode_scale};
use bacnet_types::constructed::{BACnetPrescale, BACnetScale};
use bacnet_types::enums::{
    EngineeringUnits, EventState, ObjectType, PropertyIdentifier, Reliability,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use bytes::BytesMut;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;
use crate::traits::{DeadlineWaker, MonotonicClock};
use alloc::sync::Arc;
use pulse_feed::RateWindow;

mod metadata;
#[cfg(test)]
mod present_value_tests;
mod pulse_converter;
mod pulse_feed;
#[cfg(test)]
mod pulse_feed_tests;
#[cfg(test)]
mod pulse_rate_tests;

#[doc(hidden)]
pub(crate) use pulse_converter::InputReading;
pub use pulse_converter::PulseConverterObject;

// ---------------------------------------------------------------------------
// AccumulatorObject (type 23)
// ---------------------------------------------------------------------------

/// BACnet Accumulator object — tracks a pulse count with scaling.
///
/// The application feeds it the input pulses it counts through
/// [`LocalInput`](crate::traits::LocalInput), as
/// [`LocalInputValue::Pulses`](crate::traits::LocalInputValue::Pulses),
/// before or after the object is registered (a server's `add_pulses_local`).
/// Pulses advance Present_Value through Prescale, wrapping past
/// Max_Pres_Value, and Pulse_Rate counts them per Limit_Monitoring_Interval
/// in a window its [`Timed`](crate::traits::Timed) capability closes, which
/// a server drives from its monotonic task.
#[derive(Clone)]
pub struct AccumulatorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u64,
    max_pres_value: u64,
    scale: BACnetScale,
    prescale: Option<BACnetPrescale>,
    /// Prescale's running remainder, below its modulo-divide: the pulses'
    /// share of the next Present_Value step (Clause 12.61.13).
    prescale_carry: u32,
    /// Pulse_Rate: the pulses counted over the last whole
    /// Limit_Monitoring_Interval window (Clause 12.61.20).
    pulse_rate: u64,
    units: EngineeringUnits,
    limit_monitoring_interval: u32,
    /// The window fed pulses are counted in until it closes.
    rate_window: RateWindow,
    /// The monotonic clock a restarted window opens on, once bound.
    monotonic: Option<Arc<MonotonicClock>>,
    /// Wakes the host's pass when a new window's deadline is armed.
    deadline_waker: Option<Arc<DeadlineWaker>>,
    status_flags: StatusFlags,
    event_state: EventState,
    out_of_service: bool,
    reliability: Reliability,
    value_before_change: u64,
    value_set: u64,
}

impl AccumulatorObject {
    /// Create a new Accumulator object with default values.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        units: EngineeringUnits,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCUMULATOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            max_pres_value: u64::MAX,
            scale: BACnetScale::FloatScale(1.0),
            prescale: None,
            prescale_carry: 0,
            pulse_rate: 0,
            units,
            limit_monitoring_interval: 0,
            rate_window: RateWindow::default(),
            monotonic: None,
            deadline_waker: None,
            status_flags: StatusFlags::empty(),
            event_state: EventState::NORMAL,
            out_of_service: false,
            reliability: Reliability::NO_FAULT_DETECTED,
            value_before_change: 0,
            value_set: 0,
        })
    }

    /// Set the present value (application use).
    pub fn set_present_value(&mut self, value: u64) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the scale.
    pub fn set_scale(&mut self, scale: BACnetScale) {
        self.scale = scale;
    }

    /// Set the prescale. Prescale is optional (Table 12-79), and the object
    /// serves it only once this has set one: until then it is absent from
    /// Property_List and a read is UNKNOWN_PROPERTY, since BACnetPrescale
    /// has no value standing for none. Fed pulses then advance Present_Value
    /// by its ratio, starting from an empty remainder; with a modulo-divide
    /// of zero they leave Present_Value alone.
    pub fn set_prescale(&mut self, prescale: BACnetPrescale) {
        self.prescale = Some(prescale);
        self.prescale_carry = 0;
    }

    /// Set Limit_Monitoring_Interval, the length in seconds of the window
    /// Pulse_Rate counts over, and start a new, empty window. Setting the
    /// interval it already has changes nothing, so a client that rewrites
    /// the same value doesn't keep restarting the window. Zero, the
    /// default, leaves Pulse_Rate to the application and network writes.
    pub fn set_limit_monitoring_interval(&mut self, seconds: u32) {
        if seconds == self.limit_monitoring_interval {
            return;
        }
        self.limit_monitoring_interval = seconds;
        self.restart_rate_window();
    }

    /// Set Pulse_Rate to `pulses`, the input pulses counted over the latest
    /// Limit_Monitoring_Interval, for an application that measures the rate
    /// itself. While Out_Of_Service is TRUE the property is decoupled from
    /// the input (Clause 12.61.10), so the value is dropped and a client's
    /// keeps being served. Being a setter on the concrete object, it works
    /// only before the object is registered; a registered one counts the
    /// rate from fed pulses.
    pub fn set_pulse_rate(&mut self, pulses: u64) {
        if !self.out_of_service {
            self.pulse_rate = pulses;
        }
    }

    /// Take a client's Present_Value, written out of service. The written
    /// count starts afresh: Prescale's remainder is cleared, since it held
    /// a share of a step toward the count being replaced, and once back in
    /// service fed pulses count on from the written value.
    fn write_present_value(&mut self, value: u64) {
        self.present_value = value;
        self.prescale_carry = 0;
    }

    /// Whether Prescale is present, for the metadata.
    fn has_prescale(&self) -> bool {
        self.prescale.is_some()
    }
}

impl BACnetObject for AccumulatorObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCUMULATOR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value))
            }
            p if p == PropertyIdentifier::MAX_PRES_VALUE => {
                Ok(PropertyValue::Unsigned(self.max_pres_value))
            }
            // Both are constructed productions (Clause 21): Scale a CHOICE of
            // a float [0] or an integer [1], Prescale its two Unsigneds under
            // [0] and [1], served as their context-tagged octets (#1487).
            p if p == PropertyIdentifier::SCALE => {
                let mut encoded = BytesMut::new();
                encode_scale(&mut encoded, &self.scale);
                Ok(PropertyValue::ApplicationData(encoded.to_vec()))
            }
            p if p == PropertyIdentifier::PRESCALE => match &self.prescale {
                Some(prescale) => {
                    let mut encoded = BytesMut::new();
                    encode_prescale(&mut encoded, prescale);
                    Ok(PropertyValue::ApplicationData(encoded.to_vec()))
                }
                None => Err(common::unknown_property_error()),
            },
            p if p == PropertyIdentifier::PULSE_RATE => {
                Ok(PropertyValue::Unsigned(self.pulse_rate))
            }
            p if p == PropertyIdentifier::UNITS => {
                Ok(PropertyValue::Enumerated(self.units.to_raw()))
            }
            p if p == PropertyIdentifier::LIMIT_MONITORING_INTERVAL => Ok(PropertyValue::Unsigned(
                self.limit_monitoring_interval as u64,
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state.to_raw()))
            }
            p if p == PropertyIdentifier::VALUE_BEFORE_CHANGE => {
                Ok(PropertyValue::Unsigned(self.value_before_change))
            }
            p if p == PropertyIdentifier::VALUE_SET => Ok(PropertyValue::Unsigned(self.value_set)),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        crate::property_metadata::check_write_array_index(&*self, property, array_index)?;
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            // Writable only while Out_Of_Service is TRUE (Table 12-79
            // footnote 1, Clause 12.61.4), so a client can set the count a
            // meter shows or simulate one; in service the input owns it.
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if !self.out_of_service {
                    return Err(common::write_access_denied_error());
                }
                let PropertyValue::Unsigned(v) = value else {
                    return Err(common::invalid_data_type_error());
                };
                // The value stays within zero through Max_Pres_Value.
                if v > self.max_pres_value {
                    return Err(common::value_out_of_range_error());
                }
                self.write_present_value(v);
                Ok(())
            }
            p if p == PropertyIdentifier::MAX_PRES_VALUE => {
                if let PropertyValue::Unsigned(v) = value {
                    self.max_pres_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            // Writable only while Out_Of_Service is TRUE (Clause 12.61.20),
            // for simulation; in service the input owns it.
            p if p == PropertyIdentifier::PULSE_RATE => {
                if !self.out_of_service {
                    return Err(common::write_access_denied_error());
                }
                if let PropertyValue::Unsigned(v) = value {
                    self.pulse_rate = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::LIMIT_MONITORING_INTERVAL => {
                if let PropertyValue::Unsigned(v) = value {
                    self.set_limit_monitoring_interval(common::u64_to_u32(v)?);
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
            )),
        }
    }

    fn timed(&mut self) -> Option<&mut dyn crate::traits::Timed> {
        Some(self)
    }

    fn local_input(&mut self) -> Option<&mut dyn crate::traits::LocalInput> {
        Some(self)
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_accumulator_object(self)
    }

    crate::traits::lend_cov_reporting!();
}

impl crate::traits::CovReporting for AccumulatorObject {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::constructed::BACnetPrescale;
    use bacnet_types::error::RemoteError;

    // --- AccumulatorObject ---

    #[test]
    fn accumulator_create_and_read_defaults() {
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        assert_eq!(acc.object_name(), "ACC-1");
        assert_eq!(
            acc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(0)
        );
        assert_eq!(
            acc.read_property(PropertyIdentifier::UNITS, None).unwrap(),
            PropertyValue::Enumerated(95)
        );
    }

    #[test]
    fn accumulator_read_present_value() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_present_value(42);
        assert_eq!(
            acc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Unsigned(42)
        );
    }

    #[test]
    fn accumulator_read_scale_float() {
        // float-scale [0], a four-octet REAL (#1487).
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(
            val,
            PropertyValue::ApplicationData(vec![0x0C, 0x3F, 0x80, 0x00, 0x00])
        );
    }

    #[test]
    fn accumulator_read_scale_integer() {
        // integer-scale [1], a signed INTEGER (#1487).
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_scale(BACnetScale::IntegerScale(10));
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(val, PropertyValue::ApplicationData(vec![0x19, 0x0A]));
        acc.set_scale(BACnetScale::IntegerScale(-2));
        let val = acc.read_property(PropertyIdentifier::SCALE, None).unwrap();
        assert_eq!(val, PropertyValue::ApplicationData(vec![0x19, 0xFE]));
    }

    #[test]
    fn accumulator_without_prescale_does_not_serve_it() {
        // Optional, and BACnetPrescale has no NULL: absent until set.
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        let err = acc
            .read_property(PropertyIdentifier::PRESCALE, None)
            .unwrap_err();
        assert!(
            matches!(err, Error::Remote(RemoteError::Error { class, code })
                if class == bacnet_types::enums::ErrorClass::PROPERTY
                    && code == bacnet_types::enums::ErrorCode::UNKNOWN_PROPERTY),
            "{err:?}"
        );
        assert!(!crate::property_metadata::has_property(
            &acc.property_metadata(),
            PropertyIdentifier::PRESCALE
        ));
    }

    #[test]
    fn accumulator_read_prescale_set() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_prescale(BACnetPrescale {
            multiplier: 5,
            modulo_divide: 100,
        });
        let val = acc
            .read_property(PropertyIdentifier::PRESCALE, None)
            .unwrap();
        // The multiplier under [0], the modulo divide under [1] (#1487).
        assert_eq!(
            val,
            PropertyValue::ApplicationData(vec![0x09, 0x05, 0x19, 0x64])
        );
    }

    #[test]
    fn accumulator_object_type() {
        let acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        let val = acc
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::Enumerated(ObjectType::ACCUMULATOR.to_raw())
        );
    }

    #[test]
    fn accumulator_property_list() {
        let mut acc = AccumulatorObject::new(1, "ACC-1", EngineeringUnits::NO_UNITS).unwrap();
        acc.set_prescale(BACnetPrescale {
            multiplier: 1,
            modulo_divide: 1,
        });
        let list = crate::test_rows::listed(&acc.property_metadata());
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::SCALE));
        assert!(list.contains(&PropertyIdentifier::PRESCALE));
        assert!(list.contains(&PropertyIdentifier::MAX_PRES_VALUE));
        assert!(list.contains(&PropertyIdentifier::PULSE_RATE));
    }
}
