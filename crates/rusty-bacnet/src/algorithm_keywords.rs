//! The event algorithm parameter keywords of the analog, binary and
//! multi-state builders (#2410): the values the application configures for
//! the parameters a peer may write and `storage_path` keeps (#2331).
//!
//! The builder writes each given value into the object before storage
//! attaches, through the object's own `write_property`, so it is checked as
//! a peer's write would be and a refusal raises from the builder. Storage
//! attaching afterwards puts a saved peer write over the keyword value, the
//! keyword value itself is never saved, and on a clearing start the build
//! runs again on the cleared file, so the keyword applies again.

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::types::PyBitString;

/// The parameter values one builder call was given, in the order they are
/// written; an absent keyword leaves the object's own default.
#[derive(Clone, Debug, Default)]
pub(crate) struct AlgorithmKeywords(Vec<(PropertyIdentifier, PropertyValue)>);

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

    fn add(&mut self, property: PropertyIdentifier, value: Option<PropertyValue>) {
        if let Some(value) = value {
            self.0.push((property, value));
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
        for (property, value) in &self.0 {
            object.write_property(*property, None, value.clone(), None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "algorithm_keywords_tests.rs"]
mod tests;
