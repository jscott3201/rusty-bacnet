//! COV acquisition for a Trend Log (#1480).
//!
//! A Trend Log whose Logging_Type is COV logs its referenced property the way
//! a COV notification would report it (Clause 12.25.26). The reference is to
//! this device, so no subscription is made: on each pass the poller reads the
//! property and the monitored object's Status_Flags and logs a record when
//! they changed by the test the server's COV notifications apply (Clause
//! 13.1, [`crate::cov_change`]), with Client_COV_Increment as the increment
//! when it isn't NULL (Clause 12.25.11). The record carries those
//! Status_Flags, and its timestamp is the Device clock when the change was
//! seen, the moment a local notification would carry (Clause 12.25.14).
//!
//! The first look logs the current value, as a new subscription's first
//! notification does. While Enable is FALSE or the Start_Time / Stop_Time
//! window is shut nothing is read, and the next look once collection resumes
//! starts over with the current value, as Clause 12.25.10 has a remote
//! subscription restart when Enable goes TRUE. Collection is judged once per
//! pass, so Enable turned off and on again (or a window shut and reopened)
//! within one pass, about 100 ms, isn't seen and starts nothing over. A
//! change of Client_COV_Increment alone keeps the last logged value as the
//! baseline: the next look measures from it with the new increment. A
//! reference this device can't read (another device's, or a missing object)
//! logs its failure once, and again only when the failure changes.

use bacnet_types::bitstring::status_flags_from_bacnet;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum, LogValue};
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};

use super::{LocalDevice, Member, ObjectDatabase, RECONCILE, property_value_to_log_value};
use crate::cov_change::{effective_increment, is_numeric, reports_change};
use crate::traits::{BACnetObject, MonotonicClock};

/// What one look at a COV log's reference found.
#[derive(Debug, Clone)]
pub(super) struct CovBaseline {
    /// The value read, or the failure logged for it.
    value: Result<PropertyValue, LogValue>,
    /// Whether the increment applies to `value` (see
    /// [`is_numeric`]), and the increment then in use.
    numeric: bool,
    increment: Option<f64>,
    /// The monitored object's Status_Flags, when it has them.
    flags: Option<StatusFlags>,
}

impl CovBaseline {
    /// Whether this look is a change from `previous` worth a record: the
    /// value moved by the COV test, the read now fails differently or not
    /// at all, or the Status_Flags present now differ.
    fn reports(&self, previous: &Self) -> bool {
        let value = match (&self.value, &previous.value) {
            (Ok(current), Ok(previous)) => {
                reports_change(current, previous, self.increment, self.numeric)
            }
            (Err(current), Err(previous)) => current != previous,
            _ => true,
        };
        value || (self.flags.is_some() && self.flags != previous.flags)
    }

    /// The record this look logs, stamped `date` and `time`.
    fn record(&self, date: Date, time: Time) -> BACnetLogRecord {
        let datum = match &self.value {
            Ok(value) => property_value_to_log_value(value),
            Err(failure) => failure.clone(),
        };
        BACnetLogRecord {
            date,
            time,
            log_datum: LogDatum::from(datum),
            status_flags: self.flags,
        }
    }
}

/// The monitored object's Status_Flags, when its Property_List names them
/// and they read as the four-bit string they are.
fn status_flags(object: &dyn BACnetObject) -> Option<StatusFlags> {
    if !object.property_list().contains(&P::STATUS_FLAGS) {
        return None;
    }
    match object.read_property(P::STATUS_FLAGS, None) {
        Ok(PropertyValue::BitString {
            unused_bits: 4,
            data,
        }) if data.len() == 1 && data[0] & 0x0f == 0 => Some(status_flags_from_bacnet(&data)),
        _ => None,
    }
}

impl ObjectDatabase {
    /// One pass of a COV Trend Log `oid`: look at its reference and log a
    /// record when it changed (see the module docs).
    pub(super) fn poll_cov(
        &mut self,
        oid: ObjectIdentifier,
        local: LocalDevice,
        increment: Option<f32>,
        monotonic: &MonotonicClock,
    ) {
        let now = monotonic();
        let entry = &self.trend_poll.0[&oid];
        if entry
            .retry_completed
            .is_some_and(|failed| now.saturating_sub(failed) < RECONCILE)
        {
            return;
        }
        if !self
            .get(&oid)
            .is_some_and(|log| log.log_collecting_internal())
        {
            // Nothing is kept now; start over when collection resumes.
            let entry = self.trend_poll.0.get_mut(&oid).unwrap();
            entry.cov_baseline = None;
            entry.retry_completed = None;
            return;
        }
        // A Trend Log has exactly one member, and a COV one never an empty one.
        let member: Member = entry.configuration.members[0];
        let look = match self.read_member(local, &member) {
            Ok((value, target)) => {
                let Member::Reference {
                    property, index, ..
                } = member
                else {
                    unreachable!("a read member names a property");
                };
                let numeric = is_numeric(target, property, index, &value);
                CovBaseline {
                    increment: effective_increment(target, property, numeric, increment),
                    numeric,
                    flags: status_flags(target),
                    value: Ok(value),
                }
            }
            Err(failure) => CovBaseline {
                value: Err(failure),
                numeric: false,
                increment: None,
                flags: None,
            },
        };
        if entry
            .cov_baseline
            .as_ref()
            .is_some_and(|previous| !look.reports(previous))
        {
            return;
        }
        let frame = self
            .clock_frame()
            .filter(|frame| frame.is_valid_actual_datetime());
        let accepted = frame.is_some_and(|frame| {
            let record = look.record(frame.local_date, frame.local_time);
            self.get_mut(&oid)
                .unwrap()
                .add_trend_record(record)
                .inspect_err(|error| {
                    log_warn!(object = %oid, %error, "trend-log COV record insertion failed");
                })
                .is_ok()
        });
        let completed = monotonic();
        let entry = self.trend_poll.0.get_mut(&oid).unwrap();
        if accepted {
            entry.cov_baseline = Some(look);
            entry.last_success = Some(completed);
            entry.retry_completed = None;
        } else {
            // Without a clock, or with the record refused, the change is
            // looked at again after the retry delay.
            entry.retry_completed = Some(completed);
        }
    }
}
