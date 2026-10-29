//! Start_Time and Stop_Time of a log object: the span of local date and time
//! inside which it logs (Clauses 12.25.6-7, 12.27.9-10 and 12.30.9-10).
//!
//! Each end is a BACnetDateTime. One with every field unspecified leaves
//! that side of the span open. Any other value has to name an actual moment:
//! a real day, and a time whose hour and minute are set. Workstations often
//! send the seconds or the hundredths unspecified, so each of those counts as
//! zero; any other partly unspecified end is refused rather than guessed at.
//! The span holds the start and excludes the stop, so a stop at or before the
//! start admits nothing.
//!
//! [`LogWindow`] keeps the configuration and whether the span admitted
//! logging at the last look; [`crate::log_lifecycle::LogLifecycle`] turns a
//! change of that into the LOG_DISABLED record the log keeps.

use bacnet_types::calendar::SpecificDate;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, PropertyValue, Time};

use crate::clock::UNSPECIFIED_DATETIME;
use crate::common;
use crate::property_metadata::{
    PropertyConformance::Optional, PropertyMetadata, PropertyWriteCapability::Always,
};

/// Both clauses make Start_Time and Stop_Time writable when present.
pub(crate) const START_TIME_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::START_TIME, Optional, None, Always);
pub(crate) const STOP_TIME_METADATA: PropertyMetadata =
    PropertyMetadata::new(P::STOP_TIME, Optional, None, Always);

/// One end of the span: open, or an actual local date and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum End {
    Open,
    At(SpecificDate, (u8, u8, u8, u8)),
}

impl End {
    /// The end `value` names, or `None` for a value that is neither wholly
    /// unspecified nor an actual moment. The weekday octet may be left
    /// unspecified or disagree with the date: it never changes the day named.
    /// Unspecified seconds or hundredths count as zero.
    fn of((date, time): (Date, Time)) -> Option<Self> {
        if (date, time) == UNSPECIFIED_DATETIME {
            return Some(Self::Open);
        }
        let weekday = date.day_of_week == Date::UNSPECIFIED || (1..=7).contains(&date.day_of_week);
        let day = SpecificDate::from_date(&date).filter(|_| weekday)?;
        let or_zero = |field: u8| if field == Time::UNSPECIFIED { 0 } else { field };
        let time = Time {
            second: or_zero(time.second),
            hundredths: or_zero(time.hundredths),
            ..time
        };
        time.is_specific().then_some(Self::At(
            day,
            (time.hour, time.minute, time.second, time.hundredths),
        ))
    }
}

/// A log's Start_Time and Stop_Time, and whether they admitted logging at the
/// last look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LogWindow {
    start: (Date, Time),
    stop: (Date, Time),
    /// `None` until the first look after a local change of either end.
    open: Option<bool>,
}

impl Default for LogWindow {
    /// Both ends unspecified: a span that is always open.
    fn default() -> Self {
        Self {
            start: UNSPECIFIED_DATETIME,
            stop: UNSPECIFIED_DATETIME,
            open: Some(true),
        }
    }
}

impl LogWindow {
    /// Serve Start_Time or Stop_Time as a BACnetDateTime, an application Date
    /// then an application Time; `None` for any other property.
    pub(crate) fn read(&self, property: P) -> Option<PropertyValue> {
        let (date, time) = match property {
            P::START_TIME => self.start,
            P::STOP_TIME => self.stop,
            _ => return None,
        };
        Some(PropertyValue::List(vec![
            PropertyValue::Date(date),
            PropertyValue::Time(time),
        ]))
    }

    /// Store a client's Start_Time or Stop_Time; `None` for any other
    /// property. The caller looks at the span again afterwards, so a change
    /// that opens or closes it is recorded at once.
    ///
    /// A value that isn't a Date then a Time is PROPERTY / INVALID_DATA_TYPE,
    /// and one that is neither wholly unspecified nor an actual moment
    /// PROPERTY / VALUE_OUT_OF_RANGE; either leaves the window as it was.
    pub(crate) fn write(
        &mut self,
        property: P,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        if !matches!(property, P::START_TIME | P::STOP_TIME) {
            return None;
        }
        let PropertyValue::List(items) = value else {
            return Some(Err(common::invalid_data_type_error()));
        };
        let [PropertyValue::Date(date), PropertyValue::Time(time)] = items.as_slice() else {
            return Some(Err(common::invalid_data_type_error()));
        };
        Some(self.set(property, (*date, *time)))
    }

    /// Set Start_Time or Stop_Time, checked as [`write`](Self::write) checks
    /// it. The caller decides whether the change is recorded.
    pub(crate) fn set(&mut self, property: P, value: (Date, Time)) -> Result<(), Error> {
        End::of(value).ok_or_else(common::value_out_of_range_error)?;
        match property {
            P::START_TIME => self.start = value,
            _ => self.stop = value,
        }
        Ok(())
    }

    /// Set Start_Time or Stop_Time as local configuration, checked as
    /// [`set`](Self::set) checks it. The state of the last look is
    /// forgotten, so the next look only notes where the span stands: a local
    /// change is configuration, not a transition to record.
    pub(crate) fn configure(&mut self, property: P, value: (Date, Time)) -> Result<(), Error> {
        self.set(property, value)?;
        self.open = None;
        Ok(())
    }

    /// Start_Time and Stop_Time, as configured or written.
    pub(crate) fn ends(&self) -> ((Date, Time), (Date, Time)) {
        (self.start, self.stop)
    }

    /// Whether both ends are open, so the span admits every moment and a look
    /// needs no clock.
    pub(crate) fn is_unbounded(&self) -> bool {
        self.start == UNSPECIFIED_DATETIME && self.stop == UNSPECIFIED_DATETIME
    }

    /// Whether `value` names an actual moment, by the rules a window end
    /// follows.
    pub(crate) fn is_moment(value: (Date, Time)) -> bool {
        matches!(End::of(value), Some(End::At(..)))
    }

    /// Whether the span holds the local moment `now`: on or after the start
    /// and before the stop. A `now` that isn't an actual moment is outside.
    pub(crate) fn admits(&self, now: (Date, Time)) -> bool {
        let (Some(start), Some(stop), Some(now @ End::At(..))) =
            (End::of(self.start), End::of(self.stop), End::of(now))
        else {
            return false;
        };
        (start == End::Open || start <= now) && (stop == End::Open || now < stop)
    }

    /// Whether the span admitted logging at the last look; an unknown state
    /// counts as open, so logging isn't held back for want of a clock.
    pub(crate) fn is_open(&self) -> bool {
        self.open != Some(false)
    }

    /// Whether the span admitted logging at the last look, or `None` before
    /// the first.
    pub(crate) fn last(&self) -> Option<bool> {
        self.open
    }

    /// Note where the span stands now.
    pub(crate) fn note(&mut self, open: bool) {
        self.open = Some(open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    use bacnet_types::error::RemoteError;

    fn at(day: u8, hour: u8) -> (Date, Time) {
        (
            Date {
                year: 126,
                month: 10,
                day,
                day_of_week: Date::UNSPECIFIED,
            },
            Time {
                hour,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
        )
    }

    fn window(start: (Date, Time), stop: (Date, Time)) -> LogWindow {
        let mut window = LogWindow::default();
        window.set(P::START_TIME, start).unwrap();
        window.set(P::STOP_TIME, stop).unwrap();
        window
    }

    #[test]
    fn the_span_holds_its_start_and_excludes_its_stop() {
        let span = window(at(3, 10), at(3, 12));
        assert!(!span.admits(at(3, 9)));
        assert!(span.admits(at(3, 10)));
        assert!(span.admits(at(3, 11)));
        assert!(!span.admits(at(3, 12)));
        // An unspecified end is open on its side.
        assert!(window(UNSPECIFIED_DATETIME, at(3, 12)).admits(at(1, 0)));
        assert!(window(at(3, 10), UNSPECIFIED_DATETIME).admits(at(30, 23)));
        assert!(LogWindow::default().admits(at(1, 0)));
        assert!(LogWindow::default().is_unbounded());
        // A stop at or before the start admits nothing.
        for stop in [at(3, 10), at(3, 9)] {
            let empty = window(at(3, 10), stop);
            assert!((0..24).all(|hour| !empty.admits(at(3, hour))));
        }
        // A moment that isn't actual is never inside.
        assert!(!LogWindow::default().admits(UNSPECIFIED_DATETIME));
    }

    #[test]
    fn unspecified_seconds_and_hundredths_count_as_zero() {
        let (date, time) = at(3, 10);
        let loose = (
            date,
            Time {
                second: Time::UNSPECIFIED,
                hundredths: Time::UNSPECIFIED,
                ..time
            },
        );
        let mut span = window(loose, UNSPECIFIED_DATETIME);
        let just_before = Time {
            hour: 9,
            minute: 59,
            second: 59,
            hundredths: 99,
        };
        assert!(!span.admits((date, just_before)));
        assert!(span.admits(at(3, 10)));
        // Served back as written.
        assert_eq!(
            span.read(P::START_TIME).unwrap(),
            PropertyValue::List(vec![
                PropertyValue::Date(loose.0),
                PropertyValue::Time(loose.1)
            ])
        );
        // Either alone too, at a stop.
        for stop in [
            Time {
                second: Time::UNSPECIFIED,
                ..time
            },
            Time {
                hundredths: Time::UNSPECIFIED,
                ..time
            },
        ] {
            span.set(P::STOP_TIME, (date, Time { hour: 11, ..stop }))
                .unwrap();
            assert!(span.admits((
                date,
                Time {
                    hour: 10,
                    minute: 59,
                    ..just_before
                }
            )));
            assert!(!span.admits(at(3, 11)));
        }
    }

    #[test]
    fn writes_take_a_date_and_time_that_is_actual_or_wholly_unspecified() {
        let mut span = LogWindow::default();
        let value = |(date, time): (Date, Time)| {
            PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
        };
        span.write(P::START_TIME, &value(at(3, 10)))
            .unwrap()
            .unwrap();
        assert_eq!(span.read(P::START_TIME).unwrap(), value(at(3, 10)));
        assert_eq!(
            span.read(P::STOP_TIME).unwrap(),
            value(UNSPECIFIED_DATETIME)
        );
        assert!(span.write(P::LOG_INTERVAL, &value(at(3, 10))).is_none());

        let refused = |value: PropertyValue, code: ErrorCode| {
            let mut copy = span;
            let error = copy.write(P::STOP_TIME, &value).unwrap().unwrap_err();
            assert!(
                matches!(error, Error::Remote(RemoteError::Error { class, code: c })
                    if class == ErrorClass::PROPERTY
                        && c == code),
                "{error:?}"
            );
            assert_eq!(copy, span);
        };
        let (date, time) = at(3, 10);
        for partly in [
            (
                Date {
                    year: Date::UNSPECIFIED,
                    ..date
                },
                time,
            ),
            (Date { day: 32, ..date }, time),
            (
                Date {
                    day_of_week: 8,
                    ..date
                },
                time,
            ),
            (
                date,
                Time {
                    minute: Time::UNSPECIFIED,
                    ..time
                },
            ),
            (
                date,
                Time {
                    hour: Time::UNSPECIFIED,
                    second: Time::UNSPECIFIED,
                    hundredths: Time::UNSPECIFIED,
                    ..time
                },
            ),
            (date, UNSPECIFIED_DATETIME.1),
            (UNSPECIFIED_DATETIME.0, time),
        ] {
            refused(value(partly), ErrorCode::VALUE_OUT_OF_RANGE);
        }
        for shape in [
            PropertyValue::Date(date),
            PropertyValue::List(vec![PropertyValue::Time(time), PropertyValue::Date(date)]),
            PropertyValue::List(vec![PropertyValue::Date(date)]),
            PropertyValue::Null,
        ] {
            refused(shape, ErrorCode::INVALID_DATA_TYPE);
        }
    }
}
