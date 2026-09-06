use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, Time};

use super::{BACnetServer, ServerConfig};

const HUNDREDTHS_PER_SECOND: i128 = 100;
const HUNDREDTHS_PER_MINUTE: i128 = 60 * HUNDREDTHS_PER_SECOND;
const HUNDREDTHS_PER_DAY: i128 = 24 * 60 * HUNDREDTHS_PER_MINUTE;

/// Validated civil-time settings for the bundled server clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockConfig {
    utc_offset_minutes: i16,
    daylight_savings_status: bool,
}

impl ClockConfig {
    /// Configure signed minutes west of UTC and daylight-saving status.
    pub fn new(utc_offset_minutes: i16, daylight_savings_status: bool) -> Result<Self, Error> {
        if !(-1440..=1440).contains(&utc_offset_minutes) || utc_offset_minutes % 15 != 0 {
            return Err(Error::Encoding(format!(
                "UTC offset {utc_offset_minutes} must be within -1440..=1440 minutes in 15-minute increments"
            )));
        }
        Ok(Self {
            utc_offset_minutes,
            daylight_savings_status,
        })
    }

    /// Return the configured signed minutes west of UTC.
    pub fn utc_offset_minutes(self) -> i16 {
        self.utc_offset_minutes
    }

    /// Return whether the one-hour daylight-saving adjustment is active.
    pub fn daylight_savings_status(self) -> bool {
        self.daylight_savings_status
    }
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            utc_offset_minutes: 0,
            daylight_savings_status: false,
        }
    }
}

impl<T: TransportPort + 'static> BACnetServer<T> {
    pub(super) async fn start_with_clock_mode(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
        clock_config: Option<ClockConfig>,
    ) -> Result<Self, Error> {
        Self::start_with_clock_mode_and_bindings(config, db, transport, clock_config, Vec::new())
            .await
    }

    /// Start with the default system-UTC Device clock.
    pub async fn start(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
    ) -> Result<Self, Error> {
        Self::start_with_clock(config, db, transport, ClockConfig::default()).await
    }

    /// Start with a validated Device clock configuration.
    pub async fn start_with_clock(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
        clock_config: ClockConfig,
    ) -> Result<Self, Error> {
        Self::start_with_clock_mode(config, db, transport, Some(clock_config)).await
    }

    /// Start without a Device wall clock or time-synchronization execution.
    pub async fn start_clockless(
        config: ServerConfig,
        db: ObjectDatabase,
        transport: T,
    ) -> Result<Self, Error> {
        Self::start_with_clock_mode(config, db, transport, None).await
    }
}

#[derive(Clone, Copy)]
struct ClockState {
    synchronized_utc: Option<(i128, Instant)>,
    config: ClockConfig,
}

/// Mutable server-owned controller behind the object layer's read-only port.
pub(super) struct ServerClock {
    state: Mutex<ClockState>,
}

impl ServerClock {
    pub(super) fn new(config: ClockConfig) -> Self {
        Self {
            state: Mutex::new(ClockState {
                synchronized_utc: None,
                config,
            }),
        }
    }

    /// Apply a fully specified local or UTC synchronization value.
    ///
    /// Validation and conversion finish before the controller is mutated.
    pub(super) fn synchronize(&self, date: Date, time: Time, is_utc: bool) -> Result<(), Error> {
        let supplied_hundredths = date_time_to_hundredths(date, time)?;
        let config = self
            .state
            .lock()
            .map_err(|_| Error::Encoding("Device clock lock poisoned".into()))?
            .config;

        let utc_hundredths = if is_utc {
            supplied_hundredths
        } else {
            supplied_hundredths + i128::from(config.utc_offset_minutes) * HUNDREDTHS_PER_MINUTE
                - if config.daylight_savings_status {
                    60 * HUNDREDTHS_PER_MINUTE
                } else {
                    0
                }
        };

        frame_from_utc_hundredths(utc_hundredths, config).ok_or_else(|| {
            Error::Encoding("synchronized time is outside the BACnet Date range".into())
        })?;

        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::Encoding("Device clock lock poisoned".into()))?;
        state.synchronized_utc = Some((utc_hundredths, Instant::now()));
        Ok(())
    }
}

impl ClockReader for ServerClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        let state = *self.state.lock().ok()?;
        let utc_hundredths = match state.synchronized_utc {
            Some((anchor, monotonic_anchor)) => {
                let elapsed_hundredths =
                    i128::try_from(monotonic_anchor.elapsed().as_millis() / 10).ok()?;
                anchor + elapsed_hundredths
            }
            None => system_utc_hundredths(),
        };
        frame_from_utc_hundredths(utc_hundredths, state.config)
    }
}

#[cfg(test)]
pub(crate) fn clocked_test_database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.set_clock_reader(Some(std::sync::Arc::new(ServerClock::new(
        ClockConfig::default(),
    ))));
    db
}

fn system_utc_hundredths() -> i128 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    i128::from(elapsed.as_secs()) * HUNDREDTHS_PER_SECOND + i128::from(elapsed.subsec_millis() / 10)
}

fn date_time_to_hundredths(date: Date, time: Time) -> Result<i128, Error> {
    let year = date
        .actual_year()
        .ok_or_else(|| invalid_datetime("date contains an unspecified year"))?;
    if !(1..=12).contains(&date.month) {
        return Err(invalid_datetime("date month must be 1..=12"));
    }
    let max_day = days_in_month(year, date.month);
    if date.day == 0 || date.day > max_day {
        return Err(invalid_datetime("date day is invalid for its month"));
    }
    if !(1..=7).contains(&date.day_of_week) {
        return Err(invalid_datetime("date day-of-week must be 1..=7"));
    }

    let days = days_from_civil(i64::from(year), i64::from(date.month), i64::from(date.day));
    let expected_day_of_week = (days + 3).rem_euclid(7) as u8 + 1;
    if date.day_of_week != expected_day_of_week {
        return Err(invalid_datetime("date day-of-week is inconsistent"));
    }
    if time.hour > 23 || time.minute > 59 || time.second > 59 || time.hundredths > 99 {
        return Err(invalid_datetime(
            "time must be fully specified and within its field ranges",
        ));
    }

    let day_hundredths = i128::from(time.hour) * 60 * HUNDREDTHS_PER_MINUTE
        + i128::from(time.minute) * HUNDREDTHS_PER_MINUTE
        + i128::from(time.second) * HUNDREDTHS_PER_SECOND
        + i128::from(time.hundredths);
    Ok(i128::from(days) * HUNDREDTHS_PER_DAY + day_hundredths)
}

fn invalid_datetime(message: &str) -> Error {
    Error::Encoding(format!("invalid time synchronization value: {message}"))
}

fn frame_from_utc_hundredths(utc_hundredths: i128, config: ClockConfig) -> Option<ClockFrame> {
    let dst_minutes = if config.daylight_savings_status {
        60
    } else {
        0
    };
    let local_hundredths = utc_hundredths
        - i128::from(config.utc_offset_minutes) * HUNDREDTHS_PER_MINUTE
        + i128::from(dst_minutes) * HUNDREDTHS_PER_MINUTE;
    let days = local_hundredths.div_euclid(HUNDREDTHS_PER_DAY);
    let within_day = local_hundredths.rem_euclid(HUNDREDTHS_PER_DAY);
    let days = i64::try_from(days).ok()?;
    let (year, month, day) = civil_from_days(days);
    let encoded_year = year.checked_sub(1900)?;
    if !(0..=254).contains(&encoded_year) {
        return None;
    }

    let total_seconds = within_day / HUNDREDTHS_PER_SECOND;
    Some(ClockFrame {
        local_date: Date {
            year: encoded_year as u8,
            month: month as u8,
            day: day as u8,
            day_of_week: (days + 3).rem_euclid(7) as u8 + 1,
        },
        local_time: Time {
            hour: (total_seconds / 3600) as u8,
            minute: ((total_seconds % 3600) / 60) as u8,
            second: (total_seconds % 60) as u8,
            hundredths: (within_day % HUNDREDTHS_PER_SECOND) as u8,
        },
        utc_offset: config.utc_offset_minutes,
        daylight_savings_status: config.daylight_savings_status,
    })
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
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

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: u16, month: u8, day: u8, day_of_week: u8) -> Date {
        Date {
            year: (year - 1900) as u8,
            month,
            day,
            day_of_week,
        }
    }

    fn time(hour: u8, minute: u8, second: u8, hundredths: u8) -> Time {
        Time {
            hour,
            minute,
            second,
            hundredths,
        }
    }

    #[test]
    fn clock_config_rejects_invalid_offsets() {
        assert!(ClockConfig::new(-1440, false).is_ok());
        assert!(ClockConfig::new(1440, true).is_ok());
        assert!(ClockConfig::new(14, false).is_err());
        assert!(ClockConfig::new(1441, false).is_err());
    }

    #[test]
    fn civil_conversion_covers_offsets_dst_and_rollover() {
        let utc = date_time_to_hundredths(date(2024, 3, 1, 5), time(0, 30, 0, 25)).unwrap();

        let west = frame_from_utc_hundredths(utc, ClockConfig::new(60, false).unwrap()).unwrap();
        assert_eq!(west.local_date, date(2024, 2, 29, 4));
        assert_eq!(west.local_time, time(23, 30, 0, 25));

        let east = frame_from_utc_hundredths(utc, ClockConfig::new(-60, false).unwrap()).unwrap();
        assert_eq!(east.local_date, date(2024, 3, 1, 5));
        assert_eq!(east.local_time, time(1, 30, 0, 25));

        let dst = frame_from_utc_hundredths(utc, ClockConfig::new(60, true).unwrap()).unwrap();
        assert_eq!(dst.local_date, date(2024, 3, 1, 5));
        assert_eq!(dst.local_time, time(0, 30, 0, 25));
    }

    #[test]
    fn synchronization_validation_rejects_wildcards_calendar_errors_and_inconsistent_days() {
        let clock = ServerClock::new(ClockConfig::default());
        let anchor = clock.state.lock().unwrap().synchronized_utc;
        for invalid_date in [
            date(2023, 2, 29, 3),
            date(2024, 2, 29, 5),
            Date {
                year: Date::UNSPECIFIED,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
        ] {
            assert!(clock
                .synchronize(invalid_date, time(12, 0, 0, 0), true)
                .is_err());
        }
        assert!(clock
            .synchronize(
                date(2024, 2, 29, 4),
                Time {
                    hour: Time::UNSPECIFIED,
                    minute: 0,
                    second: 0,
                    hundredths: 0,
                },
                true,
            )
            .is_err());
        assert!(clock
            .synchronize(date(2024, 2, 29, 4), time(24, 0, 0, 0), true)
            .is_err());
        assert_eq!(clock.state.lock().unwrap().synchronized_utc, anchor);
    }

    #[test]
    fn local_and_utc_synchronization_share_the_configured_frame() {
        let clock = ServerClock::new(ClockConfig::new(300, true).unwrap());
        clock
            .synchronize(date(2024, 7, 4, 4), time(9, 15, 0, 0), false)
            .unwrap();
        let local = clock.read_clock().unwrap();
        assert_eq!(local.local_date, date(2024, 7, 4, 4));
        assert_eq!((local.local_time.hour, local.local_time.minute), (9, 15));

        clock
            .synchronize(date(2024, 7, 4, 4), time(13, 15, 0, 0), true)
            .unwrap();
        let utc = clock.read_clock().unwrap();
        assert_eq!(utc.local_date, date(2024, 7, 4, 4));
        assert_eq!((utc.local_time.hour, utc.local_time.minute), (9, 15));
        assert_eq!(utc.utc_offset, 300);
        assert!(utc.daylight_savings_status);
    }

    #[test]
    fn device_schedule_recipient_and_cov_use_the_same_frame() {
        use bacnet_objects::database::ObjectDatabase;
        use bacnet_objects::device::{DeviceConfig, DeviceObject};
        use bacnet_objects::traits::BACnetObject;
        use bacnet_types::enums::PropertyIdentifier;
        use bacnet_types::primitives::PropertyValue;
        use std::sync::Arc;

        let clock = Arc::new(ServerClock::new(ClockConfig::new(300, true).unwrap()));
        clock
            .synchronize(date(2024, 7, 4, 4), time(9, 15, 0, 0), false)
            .unwrap();

        let device = DeviceObject::new(DeviceConfig::default()).unwrap();
        let device_oid = device.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(device)).unwrap();
        db.set_clock_reader(Some(
            Arc::clone(&clock) as Arc<dyn bacnet_objects::clock::ClockReader>
        ));

        let frame = db.clock_frame().unwrap();
        assert_eq!(
            db.get(&device_oid)
                .unwrap()
                .read_property(PropertyIdentifier::LOCAL_TIME, None)
                .unwrap(),
            PropertyValue::Time(frame.local_time)
        );
        assert_eq!(
            crate::schedule::current_time_components(frame),
            Some((3, 9, 15))
        );
        assert_eq!(frame.day_of_week_bit(), Some(0x08));
        assert_eq!(
            crate::server::cov_clock::cov_multiple_datetime(frame),
            (frame.local_date, frame.local_time)
        );
    }
}
