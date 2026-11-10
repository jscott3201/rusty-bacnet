//! The event algorithm parameter keywords of the analog, binary and
//! multi-state builders (#2410) and of `add_event_enrollment` (#2426): the
//! values the application configures for the parameters a peer may write
//! and `storage_path` keeps (#2331).
//!
//! The builder writes each given value into the object before storage
//! attaches, through the object's own `write_property`, so it is checked as
//! a peer's write would be and a refusal raises from the builder. Storage
//! attaching afterwards puts a saved peer write over the keyword value, the
//! keyword value itself is never saved, and on a clearing start the build
//! runs again on the cleared file, so the keyword applies again.
//!
//! An Event Enrollment's Event_Parameters and Fault_Parameters arrive as the
//! `PropertyValue` a `write_property_local` takes, and go the way that
//! write does: encoded, then decoded for the property as a peer's octets
//! are, then written.

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::types::PyBitString;

/// The longest encoding of Event_Parameters or Fault_Parameters an Event
/// Enrollment with storage takes: the cap `bacnet-objects` holds a written
/// one to before saving it (#2331).
const MAX_KEPT_OCTETS: usize = 64 * 1024;

/// One keyword's value: typed, written as it is, or the encoding of a
/// `PropertyValue` the application gave, decoded for the property first.
#[derive(Clone, Debug)]
enum Keyword {
    Typed(PropertyValue),
    Encoded(Vec<u8>),
}

/// The parameter values one builder call was given, in the order they are
/// written; an absent keyword leaves the object's own default.
#[derive(Clone, Debug, Default)]
pub(crate) struct AlgorithmKeywords(Vec<(PropertyIdentifier, Keyword)>);

impl AlgorithmKeywords {
    /// Time_Delay and Time_Delay_Normal, which every one of these builders
    /// takes.
    pub(crate) fn delays(time_delay: Option<u32>, time_delay_normal: Option<u32>) -> Self {
        let mut keywords = Self::default();
        keywords.add_delays(time_delay, time_delay_normal);
        keywords
    }

    /// High_Limit, Low_Limit, Deadband and Limit_Enable, then the delays:
    /// the analog builders' keywords.
    pub(crate) fn analog(
        high_limit: Option<f32>,
        low_limit: Option<f32>,
        deadband: Option<f32>,
        limit_enable: Option<PyBitString>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
    ) -> Self {
        let mut keywords = Self::default();
        keywords.add(
            PropertyIdentifier::HIGH_LIMIT,
            high_limit.map(PropertyValue::Real),
        );
        keywords.add(
            PropertyIdentifier::LOW_LIMIT,
            low_limit.map(PropertyValue::Real),
        );
        keywords.add(
            PropertyIdentifier::DEADBAND,
            deadband.map(PropertyValue::Real),
        );
        keywords.add(
            PropertyIdentifier::LIMIT_ENABLE,
            limit_enable.map(|bits| bits.to_value()),
        );
        keywords.add_delays(time_delay, time_delay_normal);
        keywords
    }

    /// Alarm_Value (0 for INACTIVE, 1 for ACTIVE), then the delays: the
    /// Binary Input and Value keywords.
    pub(crate) fn binary(
        alarm_value: Option<u32>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
    ) -> Self {
        let mut keywords = Self::default();
        keywords.add(
            PropertyIdentifier::ALARM_VALUE,
            alarm_value.map(PropertyValue::Enumerated),
        );
        keywords.add_delays(time_delay, time_delay_normal);
        keywords
    }

    /// Alarm_Values, in list order, then the delays: the Multi-state Input
    /// and Value keywords.
    pub(crate) fn multistate(
        alarm_values: Option<Vec<u32>>,
        time_delay: Option<u32>,
        time_delay_normal: Option<u32>,
    ) -> Self {
        let mut keywords = Self::default();
        keywords.add(
            PropertyIdentifier::ALARM_VALUES,
            alarm_values.map(|states| {
                PropertyValue::List(
                    states
                        .into_iter()
                        .map(|state| PropertyValue::Unsigned(state.into()))
                        .collect(),
                )
            }),
        );
        keywords.add_delays(time_delay, time_delay_normal);
        keywords
    }

    /// Event_Parameters, Fault_Parameters, then Time_Delay_Normal: the
    /// Event Enrollment keywords. An Event Enrollment has no Time_Delay of
    /// its own; it is a member of Event_Parameters.
    pub(crate) fn enrollment(
        event_parameters: Option<&PropertyValue>,
        fault_parameters: Option<&PropertyValue>,
        time_delay_normal: Option<u32>,
    ) -> Result<Self, Error> {
        let mut keywords = Self::default();
        for (property, value) in [
            (PropertyIdentifier::EVENT_PARAMETERS, event_parameters),
            (PropertyIdentifier::FAULT_PARAMETERS, fault_parameters),
        ] {
            if let Some(value) = value {
                let mut octets = bytes::BytesMut::new();
                bacnet_encoding::primitives::encode_property_value(&mut octets, value)?;
                keywords
                    .0
                    .push((property, Keyword::Encoded(octets.to_vec())));
            }
        }
        keywords.add_delays(None, time_delay_normal);
        Ok(keywords)
    }

    fn add(&mut self, property: PropertyIdentifier, value: Option<PropertyValue>) {
        if let Some(value) = value {
            self.0.push((property, Keyword::Typed(value)));
        }
    }

    fn add_delays(&mut self, time_delay: Option<u32>, time_delay_normal: Option<u32>) {
        let unsigned = |seconds: u32| PropertyValue::Unsigned(seconds.into());
        self.add(PropertyIdentifier::TIME_DELAY, time_delay.map(unsigned));
        self.add(
            PropertyIdentifier::TIME_DELAY_NORMAL,
            time_delay_normal.map(unsigned),
        );
    }

    /// Write the values into `object`, which has no storage attached yet,
    /// stopping at the first the object refuses.
    pub(crate) fn apply(&self, object: &mut dyn BACnetObject) -> Result<(), Error> {
        for (property, keyword) in &self.0 {
            let value = match keyword {
                Keyword::Typed(value) => value.clone(),
                Keyword::Encoded(octets) => bacnet_device_core::write::gate_and_decode_write(
                    object, *property, None, octets,
                )?,
            };
            object.write_property(*property, None, value, None)?;
        }
        Ok(())
    }

    /// Refuse, as an enrollment with storage refuses the write, an
    /// Event_Parameters or Fault_Parameters keyword whose encoding is past
    /// the cap storage holds a written one to: RESOURCES /
    /// NO_SPACE_TO_WRITE_PROPERTY. The object has no storage attached yet
    /// when the keywords apply, so its own write doesn't check this.
    pub(crate) fn check_kept_size(&self, object: &dyn BACnetObject) -> Result<(), Error> {
        for (property, keyword) in &self.0 {
            if !matches!(keyword, Keyword::Encoded(_)) {
                continue;
            }
            if let PropertyValue::ApplicationData(octets) = object.read_property(*property, None)?
                && octets.len() > MAX_KEPT_OCTETS
            {
                return Err(Error::protocol(
                    ErrorClass::RESOURCES,
                    ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "algorithm_keywords_tests.rs"]
mod tests;
