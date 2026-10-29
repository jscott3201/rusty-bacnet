//! Object-owned trend polling schedule. The caller owns synchronization and timers.

use crate::collections::{HashMap, HashSet};
use crate::prelude::*;
use core::time::Duration;

use bacnet_encoding::constructed::encode_log_multiple_record;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetLogMultipleRecord, BACnetLogRecord, LogData,
    LogDatum, LogValue,
};
use bacnet_types::enums::{
    ErrorClass, ErrorCode, LoggingType, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};
use bytes::BytesMut;

use super::{LocalDevice, ObjectDatabase};
use crate::device_reference::decode_reference;
use crate::log_buffer::ANY_VALUE_MAX_OCTETS;
use crate::traits::{BACnetObject, MonotonicClock};
use alignment::{Alignment, DAY};
use cov::CovBaseline;

mod alignment;
mod cov;

/// Local maximum idle/configuration reconciliation delay and failure backoff.
/// This is a scheduling policy, not a BACnet timing guarantee.
const RECONCILE: Duration = Duration::from_millis(100);

#[derive(PartialEq)]
struct Configuration {
    mode: Mode,
    /// Log_DeviceObjectProperty as read, compared whole for scheduling
    /// ownership.
    reference: PropertyValue,
    /// What each acquisition reads: the one reference of a Trend Log, or every
    /// element of a Trend Log Multiple in array order.
    members: Vec<Member>,
}

/// When a log acquires.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    /// POLLED: every `interval` hundredths, counted from each acquisition.
    Polled { interval: u32 },
    /// POLLED with Align_Intervals (Clauses 12.25.27 and 12.30.14): at each
    /// local time of day `offset` hundredths past a multiple of `interval`,
    /// which divides a day. `offset` is Interval_Offset modulo the interval
    /// (12.25.28 and 12.30.15).
    Aligned { interval: u32, offset: u32 },
    /// TRIGGERED with Trigger TRUE: one acquisition now (Clauses 12.25.29
    /// and 12.30.16).
    Triggered,
    /// A Trend Log's COV: on every pass, one acquisition when the reference
    /// has changed by COV's criteria, held to Client_COV_Increment when it
    /// isn't NULL (Clauses 12.25.11 and 12.25.26, #1480). See the `cov`
    /// module.
    Cov { increment: Option<f32> },
}

/// One monitored reference of a log.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Member {
    /// A Trend Log Multiple element naming object or device instance 4194303,
    /// which Clause 12.30.11 treats as empty, or the unset reference of a
    /// triggered Trend Log, which reads the same way (#1417).
    Unspecified,
    /// A property to read, at one element when `index` is present.
    Reference {
        target: ObjectIdentifier,
        property: P,
        index: Option<u32>,
        /// The reference's optional Device member.
        device: Option<ObjectIdentifier>,
    },
}

struct Schedule {
    configuration: Configuration,
    last_success: Option<Duration>,
    /// Latest failed attempt, independent of the last accepted sample.
    retry_completed: Option<Duration>,
    /// An aligned log's plan, kept on the Device clock.
    alignment: Alignment,
    /// A COV log's last logged value, which the next change is measured
    /// from; `None` until the first is logged.
    cov_baseline: Option<CovBaseline>,
}

impl Schedule {
    fn new(configuration: Configuration) -> Self {
        Self {
            configuration,
            last_success: None,
            retry_completed: None,
            alignment: Alignment::default(),
            cov_baseline: None,
        }
    }

    fn remaining(&self, now: Duration) -> Duration {
        if let Some(failed) = self.retry_completed {
            return RECONCILE.saturating_sub(now.saturating_sub(failed));
        }
        match (self.configuration.mode, self.last_success) {
            (Mode::Polled { interval }, Some(success)) => {
                hundredths(interval).saturating_sub(now.saturating_sub(success))
            }
            (Mode::Aligned { interval, offset }, _) => self.alignment.wait(now, interval, offset),
            // A COV log looks again on the next pass.
            (Mode::Cov { .. }, _) => RECONCILE,
            _ => Duration::ZERO,
        }
    }
}

fn hundredths(value: u32) -> Duration {
    Duration::from_millis(u64::from(value) * 10)
}

#[derive(Default)]
pub(super) struct TrendPollSchedule(HashMap<ObjectIdentifier, Schedule>);

impl TrendPollSchedule {
    pub(super) fn retire(&mut self, oid: &ObjectIdentifier) {
        self.0.remove(oid);
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

impl ObjectDatabase {
    /// Poll every due local Trend Log or Trend Log Multiple synchronously and
    /// return the next wait.
    ///
    /// Each acquisition reads every reference of the log: a Trend Log's one,
    /// or each Log_DeviceObjectProperty element of a Trend Log Multiple, whose
    /// record then carries one value per element in array order (#1203).
    /// Only this database is read. A reference whose Device member names
    /// another device (see [`ObjectDatabase::local_device`]) is never read
    /// here, even when a same-numbered local object exists: its value is a
    /// failure, PROPERTY / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. A Trend Log
    /// Multiple element naming instance 4194303 is empty and fails with
    /// PROPERTY / NO_PROPERTY_SPECIFIED. A reference with an array index reads
    /// that element (#1205). A local read that fails yields the error a
    /// ReadProperty of it would report: OBJECT / UNKNOWN_OBJECT for a missing
    /// object, PROPERTY / PROPERTY_IS_NOT_AN_ARRAY for an index on a property
    /// that isn't an array, or the read's own error, such as INVALID_ARRAY_INDEX
    /// for an index past the end (#1183).
    ///
    /// Only POLLED logs with a nonzero Log_Interval and at least one reference
    /// are polled. A TRIGGERED log whose Trigger reads TRUE makes one
    /// acquisition, and the object clears Trigger once it accepts the record
    /// (Clauses 12.25.29 and 12.30.16). A Trend Log Multiple without members
    /// records no values for it, and a Trend Log without a reference records
    /// PROPERTY / NO_PROPERTY_SPECIFIED, so Trigger never stays TRUE. With
    /// Align_Intervals TRUE and a Log_Interval that divides a day, a POLLED
    /// log acquires when the Device clock's time of day is Interval_Offset
    /// (modulo the interval) past a multiple of the interval (Clauses
    /// 12.25.27-28 and 12.30.14-15); the first acquisition waits for such a
    /// boundary. Such a log is judged against the Device clock on every pass,
    /// so a clock change moves its plan with it (see the `alignment` module).
    /// A Trend Log whose Logging_Type is COV is looked at on every pass and
    /// logs its reference, with the monitored object's Status_Flags, when it
    /// changed by COV's criteria (#1480; see the `cov` module).
    ///
    /// Every pass also lets each Trend Log, Trend Log Multiple and Event Log
    /// look at its Start_Time / Stop_Time window, with or without a
    /// monotonic clock, so one that opens or closes is recorded within a pass
    /// even when no record arrives. An Event Log takes its records from the
    /// application and the device's notifications, never from this poller,
    /// but this pass is the one regular visit every log gets.
    ///
    /// The caller must hold exclusive database access for this whole call. The
    /// bound monotonic clock drives scheduling; the shared Device clock
    /// provides each actual acquisition timestamp. Without either clock an
    /// attempt cannot succeed. A successful call to an object's insertion hook,
    /// including its accepted disabled/count-only outcomes, starts the
    /// configured interval.
    ///
    /// Log_Interval is in hundredths. Deadlines follow actual completion, without
    /// catch-up bursts. Invalid timestamps and insertion errors preserve the last
    /// success and retry after 100 ms. The returned wait is at most 100 ms so a
    /// timer-driven caller also reconciles configuration changes and idle logs.
    /// Slow synchronous work can make another object due; a minimum 1 ms yield
    /// avoids a zero-delay loop. These bounds are local policy, not real-time
    /// guarantees. Clock, object reads and insertion hooks must remain bounded.
    pub fn poll_trend_logs(&mut self) -> Duration {
        let mut logs = self.find_by_type(ObjectType::TREND_LOG);
        logs.extend(self.find_by_type(ObjectType::TREND_LOG_MULTIPLE));
        let trend_logs = logs.len();
        logs.extend(self.find_by_type(ObjectType::EVENT_LOG));
        // Every log looks at its window, Event Logs included, whether or not
        // a monotonic clock drives acquisition.
        for oid in &logs {
            if let Some(log) = self.get_mut(oid) {
                log.refresh_log_window_internal();
            }
        }
        logs.truncate(trend_logs);
        let Some(monotonic) = self.monotonic_clock.clone() else {
            return RECONCILE;
        };
        let mut eligible = HashSet::new();
        let local = self.local_device();
        for oid in logs {
            // Exclusive access prevents structural change after selection.
            let Some(configuration) = self.get(&oid).and_then(configuration) else {
                continue;
            };
            eligible.insert(oid);
            match self.trend_poll.0.get_mut(&oid) {
                Some(entry) if entry.configuration == configuration => {}
                // A COV log whose Client_COV_Increment alone changed keeps
                // its baseline: the next look measures from the last logged
                // value with the new increment, so nothing is logged twice.
                Some(entry) if only_increment_changed(&entry.configuration, &configuration) => {
                    entry.configuration = configuration;
                }
                _ => {
                    self.trend_poll.0.insert(oid, Schedule::new(configuration));
                }
            }
            self.poll_one(oid, local, &*monotonic);
        }
        self.trend_poll.0.retain(|oid, _| eligible.contains(oid));
        let now = monotonic();
        let remaining = self
            .trend_poll
            .0
            .values()
            .map(|entry| entry.remaining(now))
            .min()
            .unwrap_or(RECONCILE)
            .min(RECONCILE);
        if remaining.is_zero() {
            Duration::from_millis(1)
        } else {
            remaining
        }
    }

    /// Acquire for `oid`'s schedule entry when it is due.
    fn poll_one(&mut self, oid: ObjectIdentifier, local: LocalDevice, monotonic: &MonotonicClock) {
        let now = monotonic();
        let entry = &self.trend_poll.0[&oid];
        let mode = entry.configuration.mode;
        if let Mode::Cov { increment } = mode {
            self.poll_cov(oid, local, increment, monotonic);
            return;
        }
        // An aligned log looks at the Device clock on every pass, so its plan
        // follows a clock change; the others wait for their deadline. Both
        // wait out the retry delay after a failed attempt.
        let waiting = match mode {
            Mode::Aligned { .. } => entry
                .retry_completed
                .is_some_and(|failed| now.saturating_sub(failed) < RECONCILE),
            _ => !entry.remaining(now).is_zero(),
        };
        if waiting {
            return;
        }
        let frame = self
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime());
        let entry = self.trend_poll.0.get_mut(&oid).unwrap();
        let mut boundary = None;
        if let Mode::Aligned { interval, offset } = mode {
            // Without a clock there is no boundary to find; look again later.
            let Some(frame) = frame else {
                entry.retry_completed = Some(now);
                return;
            };
            entry.retry_completed = None;
            boundary = entry.alignment.due(now, &frame, interval, offset);
            if boundary.is_none() {
                return;
            }
        }
        let accepted = frame.is_some_and(|frame| {
            let values: Vec<LogValue> = self.trend_poll.0[&oid]
                .configuration
                .members
                .iter()
                .map(|member| self.acquire(local, member))
                .collect();
            let object = self.get_mut(&oid).unwrap();
            let inserted = if oid.object_type() == ObjectType::TREND_LOG_MULTIPLE {
                object.add_trend_multiple_record(BACnetLogMultipleRecord {
                    date: frame.local_date,
                    time: frame.local_time,
                    log_data: LogData::Values(values),
                })
            } else {
                object.add_trend_record(BACnetLogRecord {
                    date: frame.local_date,
                    time: frame.local_time,
                    // A Trend Log has exactly one reference.
                    log_datum: values
                        .into_iter()
                        .next()
                        .map_or(LogDatum::NullValue, LogDatum::from),
                    status_flags: None,
                })
            };
            inserted
                .inspect_err(|error| {
                    log_warn!(object = %oid, %error, "trend-log record insertion failed");
                })
                .is_ok()
        });
        let completed = monotonic();
        if accepted && mode == Mode::Triggered {
            // Served: the next Trigger starts a fresh entry.
            self.trend_poll.0.remove(&oid);
            return;
        }
        let entry = self.trend_poll.0.get_mut(&oid).unwrap();
        if !accepted {
            entry.retry_completed = Some(completed);
            return;
        }
        entry.last_success = Some(completed);
        entry.retry_completed = None;
        if let Some(boundary) = boundary {
            entry.alignment.serve(boundary);
        }
    }

    /// One reference's value, or the failure that stopped its read
    /// (Clause 12.25 Log_Buffer; Clause 12.30.19 for each member).
    fn acquire(&self, local: LocalDevice, member: &Member) -> LogValue {
        match self.read_member(local, member) {
            Ok((value, _)) => property_value_to_log_value(&value),
            Err(failure) => failure,
        }
    }

    /// One reference's value with the object it was read from, or the
    /// failure that stopped its read, as [`acquire`](Self::acquire) logs it.
    fn read_member(
        &self,
        local: LocalDevice,
        member: &Member,
    ) -> Result<(PropertyValue, &dyn BACnetObject), LogValue> {
        let Member::Reference {
            target,
            property,
            index,
            device,
        } = *member
        else {
            return Err(failure(
                ErrorClass::PROPERTY,
                ErrorCode::NO_PROPERTY_SPECIFIED,
            ));
        };
        if !local.is_local(device) {
            // Reading another device's property is not supported here.
            return Err(failure(
                ErrorClass::PROPERTY,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ));
        }
        let Some(target) = self.get(&target) else {
            return Err(failure(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT));
        };
        // ReadProperty's gate, which the Averaging sampler shares: only an
        // array has elements, whatever an object's read arm does with the
        // index.
        if index.is_some() && !target.is_array_property(property) {
            return Err(failure(
                ErrorClass::PROPERTY,
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            ));
        }
        match target.read_property(property, index) {
            Ok(value) => Ok((value, target)),
            Err(
                Error::Remote(RemoteError::Error { class, code })
                | Error::Structured { class, code, .. },
            ) => Err(failure(class, code)),
            // An error with no BACnet class and code reports as ReadProperty
            // would answer it.
            Err(_) => Err(failure(ErrorClass::SERVICES, ErrorCode::OTHER)),
        }
    }
}

fn failure(class: ErrorClass, code: ErrorCode) -> LogValue {
    LogValue::Failure {
        error_class: class,
        error_code: code,
    }
}

/// Whether `new` differs from `old` only in a COV log's increment.
fn only_increment_changed(old: &Configuration, new: &Configuration) -> bool {
    matches!((old.mode, new.mode), (Mode::Cov { .. }, Mode::Cov { .. }))
        && old.reference == new.reference
        && old.members == new.members
}

/// What a log's properties ask of the poller now; `None` when nothing.
fn configuration(object: &dyn BACnetObject) -> Option<Configuration> {
    let read = |property| object.read_property(property, None).ok();
    let logging_type = match read(P::LOGGING_TYPE) {
        Some(PropertyValue::Enumerated(value)) => LoggingType::from_raw(value),
        _ => LoggingType::POLLED,
    };
    let mode = match logging_type {
        // An object without a Trigger row is never triggered.
        LoggingType::TRIGGERED => {
            (read(P::TRIGGER) == Some(PropertyValue::Boolean(true))).then_some(Mode::Triggered)?
        }
        LoggingType::POLLED => {
            let Some(PropertyValue::Unsigned(raw)) = read(P::LOG_INTERVAL) else {
                return None;
            };
            let interval = u32::try_from(raw).ok().filter(|value| *value > 0)?;
            let offset = match read(P::INTERVAL_OFFSET) {
                Some(PropertyValue::Unsigned(offset)) => (offset % u64::from(interval)) as u32,
                _ => 0,
            };
            let aligned = read(P::ALIGN_INTERVALS) == Some(PropertyValue::Boolean(true));
            // Clause 12.30.14 aligns only an interval that goes evenly into
            // one of its clock periods; each of those goes evenly into a day,
            // so going evenly into a day is the same test.
            if aligned && DAY.is_multiple_of(interval) {
                Mode::Aligned { interval, offset }
            } else {
                Mode::Polled { interval }
            }
        }
        // Only a Trend Log takes COV; NULL is the target's own criteria.
        LoggingType::COV => Mode::Cov {
            increment: match read(P::CLIENT_COV_INCREMENT) {
                Some(PropertyValue::Real(increment)) => Some(increment),
                _ => None,
            },
        },
        _ => return None,
    };
    let reference = read(P::LOG_DEVICE_OBJECT_PROPERTY)?;
    let members = if object.object_identifier().object_type() == ObjectType::TREND_LOG_MULTIPLE {
        let PropertyValue::List(elements) = &reference else {
            return None;
        };
        elements.iter().map(member).collect::<Option<Vec<_>>>()?
    } else {
        match member(&reference)? {
            // A log with no reference polls nothing. A Trigger on one is
            // still served, with a failure, so it doesn't stay TRUE.
            Member::Unspecified if mode != Mode::Triggered => return None,
            member => vec![member],
        }
    };
    // A Trigger is served even with no members, so it never stays TRUE.
    if members.is_empty() && mode != Mode::Triggered {
        return None;
    }
    Some(Configuration {
        mode,
        reference,
        members,
    })
}

/// One reference as a read serves it: a single
/// BACnetDeviceObjectPropertyReference in its Clause 21 encoding (#1234).
/// Anything else leaves the log unpolled. An unset reference, its object or
/// Device at instance 4194303, names nothing: an empty Trend Log Multiple
/// element (Clause 12.30.11), or a Trend Log without a reference (#1417).
fn member(value: &PropertyValue) -> Option<Member> {
    let reference: BACnetDeviceObjectPropertyReference = decode_reference(value).ok()?;
    if reference.is_unset() {
        return Some(Member::Unspecified);
    }
    Some(Member::Reference {
        target: reference.object_identifier,
        property: P::from_raw(reference.property_identifier),
        index: reference.property_array_index,
        device: reference.device_identifier,
    })
}

/// A read value as the log stores it: one of the datatypes both record kinds
/// name, or else the any-value alternative holding the value's own encoding,
/// the bytes a ReadProperty of it would carry (Clauses 12.25.14 and
/// 12.30.19).
///
/// An any-value over [`ANY_VALUE_MAX_OCTETS`] logs PROPERTY /
/// VALUE_TOO_LONG instead, so every record stays small enough to page. A
/// value no record could carry (one that fails to encode, a bit string with
/// impossible padding, framed bytes whose tags don't balance) logs SERVICES
/// / OTHER, so the log never refuses what the poller hands it.
fn property_value_to_log_value(value: &PropertyValue) -> LogValue {
    let value = match value {
        PropertyValue::Real(v) => LogValue::RealValue(*v),
        PropertyValue::Unsigned(v) => LogValue::UnsignedValue(*v),
        PropertyValue::Signed(v) => LogValue::SignedValue(i64::from(*v)),
        PropertyValue::Boolean(v) => LogValue::BooleanValue(*v),
        PropertyValue::Enumerated(v) => LogValue::EnumValue(u64::from(*v)),
        PropertyValue::BitString { unused_bits, data } => LogValue::BitstringValue {
            unused_bits: *unused_bits,
            data: data.clone(),
        },
        PropertyValue::Null => LogValue::NullValue,
        other => {
            let mut encoded = BytesMut::new();
            if encode_property_value(&mut encoded, other).is_err() {
                // A value the read could not have carried either.
                return failure(ErrorClass::SERVICES, ErrorCode::OTHER);
            }
            if encoded.len() > ANY_VALUE_MAX_OCTETS {
                return failure(ErrorClass::PROPERTY, ErrorCode::VALUE_TOO_LONG);
            }
            LogValue::AnyValue(encoded.to_vec())
        }
    };
    if !matches!(
        value,
        LogValue::BitstringValue { .. } | LogValue::AnyValue(_)
    ) {
        return value;
    }
    // Only these two alternatives can hold something no record carries;
    // encoding a one-member record tells.
    let probe = BACnetLogMultipleRecord {
        date: Date {
            year: 0,
            month: 1,
            day: 1,
            day_of_week: 1,
        },
        time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_data: LogData::Values(vec![value.clone()]),
    };
    match encode_log_multiple_record(&probe, &mut BytesMut::new()) {
        Ok(()) => value,
        Err(_) => failure(ErrorClass::SERVICES, ErrorCode::OTHER),
    }
}

#[cfg(test)]
mod tests;
