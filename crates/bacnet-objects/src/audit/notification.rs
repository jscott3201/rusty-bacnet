use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

use super::*;

/// Mutable, object-owned receiver for one authorized Audit notification batch.
///
/// Implementations must commit the complete prospective state before exposing
/// any in-memory change. The server uses this type-erased channel instead of
/// downcasting an Audit Log object or taking ownership of its persistence.
pub trait AuditLogNotificationSink: Send + Sync {
    /// Whether this sink currently accepts Audit notifications.
    fn notification_logging_enabled(&self) -> bool;

    /// Merge or create every notification in wire order as one durable batch.
    ///
    /// `apdu_timeout_ms` is the local Device object's configured APDU_Timeout.
    /// It supplies the Clause 12.64 complementary timestamp window.
    fn store_notifications(
        &mut self,
        notifications: &[BACnetAuditNotification],
        apdu_timeout_ms: u32,
    ) -> Result<(), Error>;
}

impl AuditLogObject {
    fn store_notification_batch(
        &mut self,
        notifications: &[BACnetAuditNotification],
        apdu_timeout_ms: u32,
    ) -> Result<(), Error> {
        if !self.log_enable {
            return Err(Error::Protocol {
                class: ErrorClass::SERVICES.to_raw() as u32,
                code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
            });
        }
        if notifications.is_empty() {
            return Err(Error::OutOfRange(
                "Audit notification batch must not be empty".into(),
            ));
        }

        let mut prospective = self.snapshot_for_next_generation()?;
        let mut changed = false;
        for notification in notifications {
            if let Some(index) = matching_record_index(
                &prospective.records,
                notification,
                u64::from(apdu_timeout_ms) * 2,
            ) {
                let BACnetAuditLogDatum::AuditNotification(stored) =
                    &mut prospective.records[index].record.datum
                else {
                    unreachable!("matching_record_index only returns Audit notifications");
                };
                if stored.source_timestamp.is_some() && stored.target_timestamp.is_some() {
                    continue;
                }
                merge_notification(stored, notification);
                validate_record(&prospective.records[index].record)?;
                changed = true;
            } else {
                let timestamp = self.valid_timestamp()?;
                let record = BACnetAuditLogRecord {
                    timestamp,
                    datum: BACnetAuditLogDatum::AuditNotification(notification.clone()),
                };
                validate_record(&record)?;
                append_record(&mut prospective, record);
                changed = true;
            }
        }
        if changed {
            self.commit_and_apply(prospective)?;
        }
        Ok(())
    }
}

impl AuditLogNotificationSink for AuditLogObject {
    fn notification_logging_enabled(&self) -> bool {
        self.log_enable
    }

    fn store_notifications(
        &mut self,
        notifications: &[BACnetAuditNotification],
        apdu_timeout_ms: u32,
    ) -> Result<(), Error> {
        self.store_notification_batch(notifications, apdu_timeout_ms)
    }
}

fn matching_record_index(
    records: &[BACnetAuditLogRecordResult],
    incoming: &BACnetAuditNotification,
    window_ms: u64,
) -> Option<usize> {
    records.iter().rposition(|result| {
        let BACnetAuditLogDatum::AuditNotification(stored) = &result.record.datum else {
            return false;
        };
        notification_identity_matches(stored, incoming)
            && complementary_timestamps_match(stored, incoming, window_ms)
    })
}

fn notification_identity_matches(
    stored: &BACnetAuditNotification,
    incoming: &BACnetAuditNotification,
) -> bool {
    // Clause 12.64 names the exact equality coordinates. Its
    // "operation-source" is the Clause 19.6 client actor represented by
    // Source Device; Source Object is optional extra information. Likewise,
    // Target Property is the BACnetPropertyReference parameter, distinct from
    // the separately defined Target Object parameter. Source Object and Target
    // Object are therefore merged when absent but are not match coordinates.
    stored.source_device == incoming.source_device
        && stored.operation == incoming.operation
        && stored.invoke_id == incoming.invoke_id
        && stored.target_device == incoming.target_device
        && stored.target_property == incoming.target_property
        && optional_agrees(&stored.source_user_id, &incoming.source_user_id)
        && optional_agrees(&stored.source_user_role, &incoming.source_user_role)
        && optional_agrees(&stored.target_value, &incoming.target_value)
}

fn optional_agrees<T: PartialEq>(left: &Option<T>, right: &Option<T>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

fn complementary_timestamps_match(
    stored: &BACnetAuditNotification,
    incoming: &BACnetAuditNotification,
    window_ms: u64,
) -> bool {
    match (
        stored.source_timestamp.as_ref(),
        incoming.target_timestamp.as_ref(),
    ) {
        (Some(left), Some(right)) if timestamps_within(left, right, window_ms) => true,
        _ => matches!(
            (
                stored.target_timestamp.as_ref(),
                incoming.source_timestamp.as_ref()
            ),
            (Some(left), Some(right)) if timestamps_within(left, right, window_ms)
        ),
    }
}

/// Compare like timestamp variants only. DateTime uses an absolute civil-time
/// distance, Time uses the shortest circular time-of-day distance (so reports
/// can straddle midnight), and SequenceNumber has no duration scale and must
/// therefore be equal. Mixed variants do not match.
fn timestamps_within(left: &BACnetTimeStamp, right: &BACnetTimeStamp, window_ms: u64) -> bool {
    let window_centiseconds = window_ms / 10;
    match (left, right) {
        (BACnetTimeStamp::Time(left), BACnetTimeStamp::Time(right)) => {
            let (Some(left), Some(right)) = (time_centiseconds(*left), time_centiseconds(*right))
            else {
                return false;
            };
            let distance = left.abs_diff(right);
            distance.min(8_640_000 - distance) <= window_centiseconds
        }
        (
            BACnetTimeStamp::DateTime {
                date: left_date,
                time: left_time,
            },
            BACnetTimeStamp::DateTime {
                date: right_date,
                time: right_time,
            },
        ) => {
            let (Some(left), Some(right)) = (
                datetime_centiseconds(*left_date, *left_time),
                datetime_centiseconds(*right_date, *right_time),
            ) else {
                return false;
            };
            left.abs_diff(right) <= window_centiseconds
        }
        (BACnetTimeStamp::SequenceNumber(left), BACnetTimeStamp::SequenceNumber(right)) => {
            left == right
        }
        _ => false,
    }
}

fn time_centiseconds(time: Time) -> Option<u64> {
    (time.hour <= 23 && time.minute <= 59 && time.second <= 59 && time.hundredths <= 99).then(
        || {
            (((u64::from(time.hour) * 60 + u64::from(time.minute)) * 60 + u64::from(time.second))
                * 100)
                + u64::from(time.hundredths)
        },
    )
}

fn datetime_centiseconds(date: Date, time: Time) -> Option<u64> {
    let year = date.actual_year()?;
    if !(1..=12).contains(&date.month) || !(1..=7).contains(&date.day_of_week) {
        return None;
    }
    let max_day = days_in_month(year, date.month);
    if date.day == 0 || date.day > max_day {
        return None;
    }
    let days = days_from_civil(i64::from(year), i64::from(date.month), i64::from(date.day));
    let expected_day_of_week = (days + 3).rem_euclid(7) as u8 + 1;
    if date.day_of_week != expected_day_of_week {
        return None;
    }
    let time = time_centiseconds(time)?;
    let shifted_days = days.checked_add(719_468)?;
    u64::try_from(shifted_days)
        .ok()?
        .checked_mul(8_640_000)?
        .checked_add(time)
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn merge_notification(stored: &mut BACnetAuditNotification, incoming: &BACnetAuditNotification) {
    let incoming_is_target_report = incoming.target_timestamp.is_some();
    fill_missing(&mut stored.source_timestamp, &incoming.source_timestamp);
    fill_missing(&mut stored.target_timestamp, &incoming.target_timestamp);
    fill_missing(&mut stored.source_object, &incoming.source_object);
    fill_missing(&mut stored.source_comment, &incoming.source_comment);
    fill_missing(&mut stored.target_comment, &incoming.target_comment);
    fill_missing(&mut stored.source_user_id, &incoming.source_user_id);
    fill_missing(&mut stored.source_user_role, &incoming.source_user_role);
    fill_missing(&mut stored.target_object, &incoming.target_object);
    fill_missing(&mut stored.target_property, &incoming.target_property);
    fill_missing(&mut stored.target_priority, &incoming.target_priority);
    fill_missing(&mut stored.target_value, &incoming.target_value);
    fill_missing(&mut stored.result, &incoming.result);
    if incoming_is_target_report && incoming.current_value.is_some() {
        stored.current_value.clone_from(&incoming.current_value);
    } else {
        fill_missing(&mut stored.current_value, &incoming.current_value);
    }
}

fn fill_missing<T: Clone>(stored: &mut Option<T>, incoming: &Option<T>) {
    if stored.is_none() {
        stored.clone_from(incoming);
    }
}
