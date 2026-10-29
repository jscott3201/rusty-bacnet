//! Averaging (type 18) object per ASHRAE 135-2020 Clause 12.5.
//!
//! Computes minimum, maximum and average statistics over a sliding window of
//! samples taken from a referenced object property. The object keeps the
//! schedule those samples follow; the database reads the referenced property
//! when a sample falls due (`ObjectDatabase::sample_due_averaging_objects`),
//! and the application can feed samples of its own.

use crate::prelude::*;
use alloc::borrow::Cow;
use alloc::sync::Arc;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetObjectPropertyReference,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use core::time::Duration;

use crate::common::{self, read_identity_properties};
use crate::device_reference;
use crate::traits::{BACnetObject, MonotonicClock};

mod metadata;
mod schedule;
mod window;

pub use schedule::MIN_SAMPLE_PERIOD;
pub use window::{DEFAULT_WINDOW_INTERVAL, DEFAULT_WINDOW_SAMPLES, MAX_WINDOW_SAMPLES};

/// BACnet Averaging object (type 18).
///
/// Keeps the most recent Window_Samples sample attempts and serves the
/// minimum, maximum and average of the valid ones. Table 12-5 has no
/// Present_Value, Status_Flags, Event_State, Reliability or Out_Of_Service,
/// so the object serves none of them (#1064).
///
/// Once the object is in a running server and holds an
/// Object_Property_Reference, the server samples that property every
/// Window_Interval / Window_Samples seconds (Clauses 12.5.14 and 12.5.15), at
/// most every [`MIN_SAMPLE_PERIOD`]. A failed read, or a value the object can't
/// average, is recorded as a missed attempt. Without a reference the
/// application feeds the samples (see [`add_sample`](Self::add_sample) and
/// `BACnetServer::add_averaging_sample_local`); with one, an application
/// sample is one more attempt in the window and doesn't move the schedule.
///
/// Changing Window_Interval, Window_Samples or Object_Property_Reference, or
/// writing zero to Attempted_Samples, discards the samples: both counts read
/// zero and the statistics read positive infinity, NaN and negative infinity
/// until the next valid sample. The schedule starts over at the same moment,
/// so the next server sample is one spacing later.
pub struct AveragingObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    window: window::SampleWindow,
    object_property_reference: Option<BACnetObjectPropertyReference>,
}

impl AveragingObject {
    /// Create a new Averaging object; fails if `instance` exceeds the object-identifier range.
    ///
    /// The window starts at [`DEFAULT_WINDOW_SAMPLES`] samples over
    /// [`DEFAULT_WINDOW_INTERVAL`] seconds, with no samples taken.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::AVERAGING, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            window: window::SampleWindow::new(),
            object_property_reference: None,
        })
    }

    /// Add a sample value as the newest slot of the window, dropping the
    /// oldest once the window is full, and recompute the statistics.
    ///
    /// NaN or an infinity fails with PROPERTY / VALUE_OUT_OF_RANGE and leaves
    /// the window unchanged. Like other direct setters this bypasses the
    /// server's COV fanout; once the object is in a running server, its
    /// application calls `BACnetServer::add_averaging_sample_local` instead.
    pub fn add_sample(&mut self, value: f32) -> Result<(), Error> {
        common::reject_non_finite(value)?;
        self.window.push(Some(value));
        Ok(())
    }

    /// Record a sample attempt that produced no value, such as a failed read
    /// of the referenced property.
    ///
    /// The attempt takes a slot in the window, so it counts toward
    /// Attempted_Samples but not Valid_Samples, and the statistics cover the
    /// valid slots left.
    pub fn add_missed_sample(&mut self) {
        self.window.push(None);
    }

    /// Window_Interval: the period, in seconds, the window is expected to span.
    pub fn window_interval(&self) -> u32 {
        self.window.interval()
    }

    /// Window_Samples: how many sample attempts the window holds.
    pub fn window_samples(&self) -> u32 {
        self.window.capacity()
    }

    /// Set Window_Interval and discard the samples, as a network write does.
    ///
    /// Zero fails with PROPERTY / VALUE_OUT_OF_RANGE and changes nothing.
    pub fn set_window_interval(&mut self, seconds: u32) -> Result<(), Error> {
        self.window.set_interval(seconds)
    }

    /// Set Window_Samples and discard the samples, as a network write does.
    ///
    /// Zero, or more than [`MAX_WINDOW_SAMPLES`], fails with PROPERTY /
    /// VALUE_OUT_OF_RANGE and changes nothing.
    pub fn set_window_samples(&mut self, samples: u32) -> Result<(), Error> {
        self.window.set_capacity(samples)
    }

    /// Set the object property reference (the property being averaged) and
    /// discard the samples taken from the previous one. `None`, or a
    /// reference to the reserved instance 4194303, leaves the object without
    /// one (#1417).
    pub fn set_object_property_reference(
        &mut self,
        reference: Option<BACnetObjectPropertyReference>,
    ) {
        self.object_property_reference = reference.filter(|reference| !reference.is_unset());
        self.window.reset();
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

/// Convert an application sample to the REAL the statistics are kept in.
///
/// Clause 12.5 names five datatypes a sampled property may have (BOOLEAN,
/// INTEGER, Unsigned, Enumerated and REAL) and computes in REAL. FALSE and
/// TRUE count as 0 and 1, and an Enumerated counts as its Unsigned value.
/// Anything else, Double included, fails with PROPERTY / INVALID_DATA_TYPE.
fn sample_as_real(value: &PropertyValue) -> Result<f32, Error> {
    match *value {
        PropertyValue::Boolean(active) => Ok(if active { 1.0 } else { 0.0 }),
        PropertyValue::Signed(v) => Ok(v as f32),
        PropertyValue::Unsigned(v) => Ok(v as f32),
        PropertyValue::Enumerated(v) => Ok(v as f32),
        PropertyValue::Real(v) => Ok(v),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// The Unsigned a window-property write carries, as a u32. Another datatype
/// fails with PROPERTY / INVALID_DATA_TYPE and a value past u32 with PROPERTY
/// / VALUE_OUT_OF_RANGE.
fn unsigned_u32(value: &PropertyValue) -> Result<u32, Error> {
    match *value {
        PropertyValue::Unsigned(v) => common::u64_to_u32(v),
        _ => Err(common::invalid_data_type_error()),
    }
}

impl BACnetObject for AveragingObject {
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
        if let Some(result) = read_identity_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::AVERAGING.to_raw()))
            }
            p if p == PropertyIdentifier::MINIMUM_VALUE => {
                Ok(PropertyValue::Real(self.window.minimum()))
            }
            p if p == PropertyIdentifier::MAXIMUM_VALUE => {
                Ok(PropertyValue::Real(self.window.maximum()))
            }
            p if p == PropertyIdentifier::AVERAGE_VALUE => {
                Ok(PropertyValue::Real(self.window.average()))
            }
            p if p == PropertyIdentifier::ATTEMPTED_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.window.attempted().into()))
            }
            p if p == PropertyIdentifier::VALID_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.window.valid().into()))
            }
            p if p == PropertyIdentifier::WINDOW_INTERVAL => {
                Ok(PropertyValue::Unsigned(self.window.interval().into()))
            }
            p if p == PropertyIdentifier::WINDOW_SAMPLES => {
                Ok(PropertyValue::Unsigned(self.window.capacity().into()))
            }
            // Table 12-5 types the property as a
            // BACnetDeviceObjectPropertyReference (#1182). The object keeps
            // only local references, so what it serves never has a Device
            // member; with no reference it reads as the unset form, an
            // Analog Input's Present_Value at the reserved instance (#1417).
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                Ok(device_reference::optional_reference_value(
                    self.object_property_reference
                        .as_ref()
                        .map(device_reference::local_property_reference)
                        .as_ref(),
                    ObjectType::ANALOG_INPUT,
                ))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        // Footnote 1 of Table 12-5 attaches the same reset to a write of any
        // of the four rows below: each accepted write discards the samples,
        // even when the value doesn't change, and a refused one leaves the
        // window alone.
        match property {
            // Clause 12.5 Table 12-5 types Object_Property_Reference as
            // BACnetDeviceObjectPropertyReference, and 12.5.13 leaves
            // sampling a property on another BACnet device optional. This
            // implementation samples local objects only, so the shared
            // helpers decode the reference and refuse a Device member: one
            // that isn't a Device identifier with VALUE_OUT_OF_RANGE (#1182),
            // any other with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED rather than
            // silently dropping the device. The object can't tell which
            // Device holds it; the bundled server drops a Device member naming
            // its own Device before the value gets here (#1153). The unset
            // form, whatever its Device member, clears the reference (#1417);
            // Null and the flat application-tagged list are INVALID_DATA_TYPE.
            p if p == PropertyIdentifier::OBJECT_PROPERTY_REFERENCE => {
                let reference: BACnetDeviceObjectPropertyReference =
                    device_reference::decode_reference(&value)?;
                device_reference::check_device_member(reference.device_identifier)?;
                let reference = device_reference::set_or_unset(reference)
                    .map(device_reference::into_local_property_reference)
                    .transpose()?;
                self.set_object_property_reference(reference);
                Ok(())
            }
            p if p == PropertyIdentifier::WINDOW_INTERVAL => {
                self.window.set_interval(unsigned_u32(&value)?)
            }
            p if p == PropertyIdentifier::WINDOW_SAMPLES => {
                self.window.set_capacity(unsigned_u32(&value)?)
            }
            // Zero is the only value Clause 12.5.11 lets a client write.
            p if p == PropertyIdentifier::ATTEMPTED_SAMPLES => match value {
                PropertyValue::Unsigned(0) => {
                    self.window.reset();
                    Ok(())
                }
                PropertyValue::Unsigned(_) => Err(common::value_out_of_range_error()),
                _ => Err(common::invalid_data_type_error()),
            },
            _ => Err(common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                _array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn supports_subscribe_cov_property(&self) -> bool {
        // Table 13-1 has no Averaging row, so SubscribeCOV stays refused
        // (supports_cov is false). A property subscription uses the Table
        // 13-1a criterion for its datatype; the object has no Status_Flags to
        // add to the report.
        true
    }

    fn add_averaging_sample_internal(
        &mut self,
        sample: Option<PropertyValue>,
    ) -> Result<(), Error> {
        match sample {
            Some(value) => self.add_sample(sample_as_real(&value)?),
            None => {
                self.add_missed_sample();
                Ok(())
            }
        }
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.window.bind_clock(clock);
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        // Only an object with a reference has anything for the server to read.
        self.object_property_reference.as_ref()?;
        self.window.next_due()
    }

    fn take_due_averaging_sample_internal(
        &mut self,
        now: Duration,
    ) -> Option<BACnetObjectPropertyReference> {
        let reference = self.object_property_reference.clone()?;
        self.window.take_due(now).then_some(reference)
    }
}

#[cfg(test)]
mod sample_tests;

#[cfg(test)]
mod schedule_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod window_tests;
