//! How a Trend Log or Trend Log Multiple acquires its records: Logging_Type,
//! Log_Interval, Align_Intervals, Interval_Offset and Trigger (Clauses
//! 12.25.9 and 12.25.26 to 12.25.29; 12.30.12 to 12.30.16). The database's
//! trend poller reads these rows to schedule acquisitions; the object owns
//! their values and write rules.

use bacnet_types::enums::{ErrorClass, ErrorCode, LoggingType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;
use crate::log_persistence::LogAcquisitionSettings;

/// The Log_Interval a trend log takes, in hundredths of a second (one
/// minute), when POLLED is chosen while its Log_Interval is zero. Clauses
/// 12.25.26 and 12.30.12 leave the default to the device.
pub const DEFAULT_LOG_INTERVAL: u32 = 6_000;

/// Which object type's rules an [`Acquisition`] follows where the two
/// differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rules {
    /// Trend Log (Clause 12.25), which logs by COV too (#1480). A
    /// Log_Interval write is how older clients move it between POLLED and
    /// COV (12.25.9): a POLLED log's nonzero interval written to zero turns
    /// it to COV, and a nonzero interval written while COV turns it back to
    /// POLLED at that interval. Any value but POLLED, COV and TRIGGERED is
    /// PROPERTY / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, the answer Clause
    /// 12.25.26 gives for a Logging_Type the object doesn't support.
    TrendLog,
    /// Trend Log Multiple (Clause 12.30), which never logs by COV: COV, like
    /// any value but POLLED and TRIGGERED, is PROPERTY / VALUE_OUT_OF_RANGE
    /// (12.30.12), and a zero Log_Interval just leaves a POLLED log idle.
    TrendLogMultiple,
}

/// A trend log's acquisition settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Acquisition {
    rules: Rules,
    logging_type: LoggingType,
    /// Hundredths of a second; zero while TRIGGERED or COV.
    log_interval: u32,
    align_intervals: bool,
    interval_offset: u32,
    trigger: bool,
}

impl Acquisition {
    /// POLLED with no interval, so nothing is acquired until one is set.
    pub(super) fn new(rules: Rules) -> Self {
        Self {
            rules,
            logging_type: LoggingType::POLLED,
            log_interval: 0,
            align_intervals: false,
            interval_offset: 0,
            trigger: false,
        }
    }

    pub(super) fn logging_type(&self) -> LoggingType {
        self.logging_type
    }

    /// The settings a durable log saves (#1541). Trigger isn't one: a
    /// pending acquisition doesn't outlive a restart.
    pub(super) fn settings(&self) -> LogAcquisitionSettings {
        LogAcquisitionSettings {
            logging_type: self.logging_type,
            log_interval: self.log_interval,
            align_intervals: self.align_intervals,
            interval_offset: self.interval_offset,
        }
    }

    /// Take back saved settings as they were. The snapshot was checked
    /// against these rules when it was read.
    pub(super) fn restore(&mut self, settings: LogAcquisitionSettings) {
        self.logging_type = settings.logging_type;
        self.log_interval = settings.log_interval;
        self.align_intervals = settings.align_intervals;
        self.interval_offset = settings.interval_offset;
        self.trigger = false;
    }

    /// Whether Log_Interval takes writes now: in every mode but TRIGGERED
    /// (Table 12-29 footnote 3, Table 12-35 footnote 2).
    pub(super) fn log_interval_writable(&self) -> bool {
        self.logging_type != LoggingType::TRIGGERED
    }

    /// Serve one of the five rows; `None` for any other property.
    pub(super) fn read(&self, property: P) -> Option<PropertyValue> {
        Some(match property {
            P::LOGGING_TYPE => PropertyValue::Enumerated(self.logging_type.to_raw()),
            P::LOG_INTERVAL => PropertyValue::Unsigned(self.log_interval.into()),
            P::ALIGN_INTERVALS => PropertyValue::Boolean(self.align_intervals),
            P::INTERVAL_OFFSET => PropertyValue::Unsigned(self.interval_offset.into()),
            P::TRIGGER => PropertyValue::Boolean(self.trigger),
            _ => return None,
        })
    }

    /// A write of one of the five rows, a client's or a local process's;
    /// `None` for any other property, and for Log_Interval while it is
    /// read-only, when the caller refuses it as it refuses any read-only row.
    pub(super) fn write(
        &mut self,
        property: P,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        Some(match (property, value) {
            (P::LOGGING_TYPE, PropertyValue::Enumerated(raw)) => {
                self.set_logging_type(LoggingType::from_raw(*raw))
            }
            (P::LOG_INTERVAL, _) if !self.log_interval_writable() => return None,
            (P::LOG_INTERVAL, PropertyValue::Unsigned(raw)) => {
                common::u64_to_u32(*raw).and_then(|hundredths| self.store_log_interval(hundredths))
            }
            (P::INTERVAL_OFFSET, PropertyValue::Unsigned(raw)) => {
                common::u64_to_u32(*raw).map(|hundredths| self.interval_offset = hundredths)
            }
            (P::ALIGN_INTERVALS, PropertyValue::Boolean(align)) => {
                self.align_intervals = *align;
                Ok(())
            }
            (P::TRIGGER, PropertyValue::Boolean(trigger)) => self.write_trigger(*trigger),
            (
                P::LOGGING_TYPE
                | P::LOG_INTERVAL
                | P::INTERVAL_OFFSET
                | P::ALIGN_INTERVALS
                | P::TRIGGER,
                _,
            ) => Err(common::invalid_data_type_error()),
            _ => return None,
        })
    }

    /// Choose POLLED, TRIGGERED or (on a Trend Log) COV acquisition
    /// (Clauses 12.25.26 and 12.30.12). Any other value changes nothing and
    /// is refused with the error [`Rules`] gives each object type. POLLED
    /// with a zero Log_Interval sets [`DEFAULT_LOG_INTERVAL`]; TRIGGERED and
    /// COV set Log_Interval to zero. Leaving TRIGGERED drops a Trigger not
    /// yet acted on.
    pub(super) fn set_logging_type(&mut self, logging_type: LoggingType) -> Result<(), Error> {
        match logging_type {
            LoggingType::POLLED => {
                if self.log_interval == 0 {
                    self.log_interval = DEFAULT_LOG_INTERVAL;
                }
                self.trigger = false;
            }
            LoggingType::TRIGGERED => self.log_interval = 0,
            LoggingType::COV if self.rules == Rules::TrendLog => {
                self.log_interval = 0;
                self.trigger = false;
            }
            _ => return Err(self.unsupported_logging_type()),
        }
        self.logging_type = logging_type;
        Ok(())
    }

    /// Set Log_Interval as local configuration, held to the network rules:
    /// while TRIGGERED it is read-only, PROPERTY / WRITE_ACCESS_DENIED, and
    /// on a Trend Log it may switch between POLLED and COV (see
    /// [`Rules::TrendLog`]).
    pub(super) fn set_log_interval(&mut self, hundredths: u32) -> Result<(), Error> {
        if !self.log_interval_writable() {
            return Err(common::write_access_denied_error());
        }
        self.store_log_interval(hundredths)
    }

    /// Store a Log_Interval written while it is writable, with a Trend
    /// Log's two Clause 12.25.9 switches: a POLLED log whose interval goes
    /// from nonzero to zero now logs by COV, and a COV log given a nonzero
    /// interval polls at it. Zero written to a COV log, or to a POLLED log
    /// already at zero, changes no mode.
    fn store_log_interval(&mut self, hundredths: u32) -> Result<(), Error> {
        if self.rules == Rules::TrendLog {
            match self.logging_type {
                LoggingType::POLLED if self.log_interval != 0 && hundredths == 0 => {
                    self.logging_type = LoggingType::COV;
                }
                LoggingType::COV if hundredths != 0 => {
                    self.logging_type = LoggingType::POLLED;
                }
                _ => {}
            }
        }
        self.log_interval = hundredths;
        Ok(())
    }

    /// The refusal of a Logging_Type this object doesn't carry out (see
    /// [`Rules`]).
    fn unsupported_logging_type(&self) -> Error {
        match self.rules {
            Rules::TrendLog => common::protocol_error(
                ErrorClass::PROPERTY,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
            Rules::TrendLogMultiple => common::value_out_of_range_error(),
        }
    }

    pub(super) fn set_align_intervals(&mut self, align: bool) {
        self.align_intervals = align;
    }

    pub(super) fn set_interval_offset(&mut self, hundredths: u32) {
        self.interval_offset = hundredths;
    }

    /// Trigger is set TRUE to ask for one acquisition, which the poller
    /// makes and then clears it (Clauses 12.25.29 and 12.30.16). Only a
    /// TRIGGERED log takes it: TRUE on any other is PROPERTY /
    /// NOT_CONFIGURED_FOR_TRIGGERED_LOGGING. FALSE is accepted and changes
    /// nothing, so an acquisition already asked for still happens.
    fn write_trigger(&mut self, trigger: bool) -> Result<(), Error> {
        if !trigger {
            return Ok(());
        }
        if self.logging_type != LoggingType::TRIGGERED {
            return Err(common::protocol_error(
                ErrorClass::PROPERTY,
                ErrorCode::NOT_CONFIGURED_FOR_TRIGGERED_LOGGING,
            ));
        }
        self.trigger = true;
        Ok(())
    }

    /// Set Trigger TRUE as a local process would.
    pub(super) fn trigger(&mut self) -> Result<(), Error> {
        self.write_trigger(true)
    }

    /// A record was acquired: a pending Trigger has been served.
    pub(super) fn acquired(&mut self) {
        self.trigger = false;
    }
}
