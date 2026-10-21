//! A Trend Log's COV settings (Table 12-29, #1480): Client_COV_Increment,
//! which the trend poller's COV acquisition applies, and
//! COV_Resubscription_Interval, which is held for the remote COV
//! subscriptions this device doesn't make yet.

use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;

/// The COV_Resubscription_Interval a new Trend Log holds, in seconds (one
/// hour). Clause 12.25.10 asks only that it be nonzero.
pub const DEFAULT_COV_RESUBSCRIPTION_INTERVAL: u32 = 3_600;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CovSettings {
    /// `None` is the NULL choice: the referenced property's own COV
    /// criteria (Clause 12.25.11).
    client_cov_increment: Option<f32>,
    /// Seconds; never zero.
    resubscription_interval: u32,
}

impl Default for CovSettings {
    fn default() -> Self {
        Self {
            client_cov_increment: None,
            resubscription_interval: DEFAULT_COV_RESUBSCRIPTION_INTERVAL,
        }
    }
}

impl CovSettings {
    /// Serve one of the two rows; `None` for any other property.
    pub(super) fn read(&self, property: P) -> Option<PropertyValue> {
        Some(match property {
            P::CLIENT_COV_INCREMENT => self
                .client_cov_increment
                .map_or(PropertyValue::Null, PropertyValue::Real),
            P::COV_RESUBSCRIPTION_INTERVAL => {
                PropertyValue::Unsigned(self.resubscription_interval.into())
            }
            _ => return None,
        })
    }

    /// A write of one of the two rows; `None` for any other property.
    pub(super) fn write(
        &mut self,
        property: P,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        Some(match (property, value) {
            (P::CLIENT_COV_INCREMENT, PropertyValue::Null) => {
                self.client_cov_increment = None;
                Ok(())
            }
            (P::CLIENT_COV_INCREMENT, PropertyValue::Real(increment)) => {
                self.set_client_cov_increment(Some(*increment))
            }
            (P::COV_RESUBSCRIPTION_INTERVAL, PropertyValue::Unsigned(raw)) => {
                common::u64_to_u32(*raw)
                    .and_then(|seconds| self.set_resubscription_interval(seconds))
            }
            (P::CLIENT_COV_INCREMENT | P::COV_RESUBSCRIPTION_INTERVAL, _) => {
                Err(common::invalid_data_type_error())
            }
            _ => return None,
        })
    }

    /// Set Client_COV_Increment; `None` is NULL. An increment that is
    /// negative or not finite is PROPERTY / VALUE_OUT_OF_RANGE and changes
    /// nothing.
    pub(super) fn set_client_cov_increment(&mut self, increment: Option<f32>) -> Result<(), Error> {
        if increment.is_some_and(|increment| !increment.is_finite() || increment < 0.0) {
            return Err(common::value_out_of_range_error());
        }
        self.client_cov_increment = increment;
        Ok(())
    }

    /// Set COV_Resubscription_Interval in seconds; zero is PROPERTY /
    /// VALUE_OUT_OF_RANGE, since Clause 12.25.10 keeps it nonzero.
    pub(super) fn set_resubscription_interval(&mut self, seconds: u32) -> Result<(), Error> {
        if seconds == 0 {
            return Err(common::value_out_of_range_error());
        }
        self.resubscription_interval = seconds;
        Ok(())
    }
}
