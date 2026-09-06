//! Strict command-line parsing for the three BACnetTimeStamp choices.

use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

pub(crate) const TIMESTAMP_GRAMMAR: &str = "sequence:<0..65535>, \
time:<hour>,<minute>,<second>,<hundredths>, or \
datetime:<full-year>,<month>,<day>,<day-of-week>;<hour>,<minute>,<second>,<hundredths>";

fn decimal(component: &str, value: &str) -> Result<u32, String> {
    value.parse::<u32>().map_err(|_| {
        format!("invalid BACnetTimeStamp {component} '{value}': expected a decimal integer")
    })
}

fn ranged_or_unspecified(
    component: &str,
    value: &str,
    minimum: u8,
    maximum: u8,
) -> Result<u8, String> {
    let value = decimal(component, value)?;
    if value == u32::from(Time::UNSPECIFIED)
        || (u32::from(minimum)..=u32::from(maximum)).contains(&value)
    {
        return Ok(value as u8);
    }
    Err(format!(
        "BACnetTimeStamp {component} must be {minimum}..={maximum} or 255 (unspecified), got {value}"
    ))
}

fn fields4<'a>(kind: &str, value: &'a str) -> Result<[&'a str; 4], String> {
    let fields: Vec<_> = value.split(',').collect();
    fields.try_into().map_err(|_| {
        format!(
            "invalid BACnetTimeStamp {kind} component count; expected exactly 4 comma-separated values ({TIMESTAMP_GRAMMAR})"
        )
    })
}

fn parse_time(value: &str) -> Result<Time, String> {
    let [hour, minute, second, hundredths] = fields4("time", value)?;
    Ok(Time {
        hour: ranged_or_unspecified("hour", hour, 0, 23)?,
        minute: ranged_or_unspecified("minute", minute, 0, 59)?,
        second: ranged_or_unspecified("second", second, 0, 59)?,
        hundredths: ranged_or_unspecified("hundredths", hundredths, 0, 99)?,
    })
}

fn parse_date(value: &str) -> Result<Date, String> {
    let [year, month, day, day_of_week] = fields4("date", value)?;
    let full_year = decimal("full-year", year)?;
    let year = match full_year {
        255 => Date::UNSPECIFIED,
        1900..=2154 => (full_year - 1900) as u8,
        _ => {
            return Err(format!(
                "BACnetTimeStamp full-year must be 1900..=2154 or 255 (unspecified), got {full_year}"
            ));
        }
    };
    Ok(Date {
        year,
        month: ranged_or_unspecified("month", month, 1, 14)?,
        day: ranged_or_unspecified("day", day, 1, 34)?,
        day_of_week: ranged_or_unspecified("day-of-week", day_of_week, 1, 7)?,
    })
}

fn normalize_outer_double_quotes(spec: &str) -> Result<&str, String> {
    let starts_with_quote = spec.starts_with('"');
    let ends_with_quote = spec.ends_with('"');
    let spec = match (starts_with_quote, ends_with_quote, spec.len()) {
        (true, true, length) if length >= 2 => &spec[1..length - 1],
        (false, false, _) => spec,
        _ => {
            return Err(format!(
                "invalid BACnetTimeStamp quoting in '{spec}'; use no quotes or exactly one matching outer double-quote pair"
            ));
        }
    };

    if spec.contains('"') {
        return Err(format!(
            "invalid BACnetTimeStamp quoting in '{spec}'; embedded or multiple double quotes are not accepted"
        ));
    }
    if spec.contains('\'') {
        return Err(format!(
            "invalid BACnetTimeStamp quoting in '{spec}'; single quotes are not accepted"
        ));
    }
    if spec.trim() != spec {
        return Err(format!(
            "invalid BACnetTimeStamp whitespace in '{spec}'; leading and trailing whitespace are not accepted"
        ));
    }
    Ok(spec)
}

/// Parse an exact BACnetTimeStamp CLI value without component normalization.
pub(crate) fn parse_bacnet_timestamp(spec: &str) -> Result<BACnetTimeStamp, String> {
    let spec = normalize_outer_double_quotes(spec)?;
    let (kind, value) = spec
        .split_once(':')
        .ok_or_else(|| format!("invalid BACnetTimeStamp '{spec}'; expected {TIMESTAMP_GRAMMAR}"))?;
    match kind {
        "sequence" => {
            let sequence = decimal("sequence number", value)?;
            let sequence = u16::try_from(sequence).map_err(|_| {
                format!("BACnetTimeStamp sequence number must be 0..=65535, got {sequence}")
            })?;
            Ok(BACnetTimeStamp::SequenceNumber(sequence))
        }
        "time" => Ok(BACnetTimeStamp::Time(parse_time(value)?)),
        "datetime" => {
            let (date, time) = value.split_once(';').ok_or_else(|| {
                format!("invalid BACnetTimeStamp datetime '{value}'; expected {TIMESTAMP_GRAMMAR}")
            })?;
            Ok(BACnetTimeStamp::DateTime {
                date: parse_date(date)?,
                time: parse_time(time)?,
            })
        }
        _ => Err(format!(
            "unknown BACnetTimeStamp kind '{kind}'; expected sequence, time, or datetime ({TIMESTAMP_GRAMMAR})"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_all_choices_boundaries_patterns_and_unspecified_fields() {
        assert_eq!(
            parse_bacnet_timestamp("sequence:0").unwrap(),
            BACnetTimeStamp::SequenceNumber(0)
        );
        assert_eq!(
            parse_bacnet_timestamp("sequence:65535").unwrap(),
            BACnetTimeStamp::SequenceNumber(65_535)
        );
        assert_eq!(
            parse_bacnet_timestamp("time:23,59,59,99").unwrap(),
            BACnetTimeStamp::Time(Time {
                hour: 23,
                minute: 59,
                second: 59,
                hundredths: 99,
            })
        );
        assert_eq!(
            parse_bacnet_timestamp("datetime:2154,14,34,255;255,0,59,255").unwrap(),
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 254,
                    month: 14,
                    day: 34,
                    day_of_week: 255,
                },
                time: Time {
                    hour: 255,
                    minute: 0,
                    second: 59,
                    hundredths: 255,
                },
            }
        );
        assert_eq!(
            parse_bacnet_timestamp("datetime:255,255,255,255;255,255,255,255").unwrap(),
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 255,
                    month: 255,
                    day: 255,
                    day_of_week: 255,
                },
                time: Time {
                    hour: 255,
                    minute: 255,
                    second: 255,
                    hundredths: 255,
                },
            }
        );
    }

    #[test]
    fn accepts_exactly_one_outer_double_quote_pair_for_time_and_datetime() {
        assert_eq!(
            parse_bacnet_timestamp("\"time:1,2,3,4\"").unwrap(),
            BACnetTimeStamp::Time(Time {
                hour: 1,
                minute: 2,
                second: 3,
                hundredths: 4,
            })
        );
        assert_eq!(
            parse_bacnet_timestamp("\"datetime:2026,9,2,3;5,6,7,8\"").unwrap(),
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 126,
                    month: 9,
                    day: 2,
                    day_of_week: 3,
                },
                time: Time {
                    hour: 5,
                    minute: 6,
                    second: 7,
                    hundredths: 8,
                },
            }
        );
    }

    #[test]
    fn rejects_unmatched_embedded_multiple_single_quotes_and_outer_whitespace() {
        assert_eq!(
            parse_bacnet_timestamp("\"time:1,2,3,4").unwrap_err(),
            "invalid BACnetTimeStamp quoting in '\"time:1,2,3,4'; use no quotes or exactly one matching outer double-quote pair"
        );
        assert_eq!(
            parse_bacnet_timestamp("time:1,2,3,4\"").unwrap_err(),
            "invalid BACnetTimeStamp quoting in 'time:1,2,3,4\"'; use no quotes or exactly one matching outer double-quote pair"
        );
        for invalid in [
            "\"time:1,\"2\",3,4\"",
            "\"\"time:1,2,3,4\"\"",
            "'time:1,2,3,4'",
            " time:1,2,3,4",
            "time:1,2,3,4 ",
            "\" time:1,2,3,4\"",
            "\"time:1,2,3,4 \"",
        ] {
            assert!(
                parse_bacnet_timestamp(invalid).is_err(),
                "accepted invalid timestamp quoting {invalid}"
            );
        }
    }

    #[test]
    fn rejects_malformed_unknown_trailing_range_and_lossy_values() {
        for invalid in [
            "sequence",
            "Sequence:1",
            "unknown:1",
            "sequence:-1",
            "sequence:true",
            "sequence:65536",
            "sequence:1,2",
            "time:1,2,3",
            "time:24,0,0,0",
            "time:0,60,0,0",
            "time:0,0,60,0",
            "time:0,0,0,100",
            "time:0,0,0,0,trailing",
            "datetime:2026,1,1,1,0,0,0,0",
            "datetime:1899,1,1,1;0,0,0,0",
            "datetime:2155,1,1,1;0,0,0,0",
            "datetime:2026,0,1,1;0,0,0,0",
            "datetime:2026,15,1,1;0,0,0,0",
            "datetime:2026,1,0,1;0,0,0,0",
            "datetime:2026,1,35,1;0,0,0,0",
            "datetime:2026,1,1,0;0,0,0,0",
            "datetime:2026,1,1,8;0,0,0,0",
            "datetime:2026,1,1,1;0,0,0,0;trailing",
        ] {
            assert!(
                parse_bacnet_timestamp(invalid).is_err(),
                "accepted invalid timestamp {invalid}"
            );
        }
    }

    #[test]
    fn errors_are_stable_and_actionable() {
        assert_eq!(
            parse_bacnet_timestamp("time:24,0,0,0").unwrap_err(),
            "BACnetTimeStamp hour must be 0..=23 or 255 (unspecified), got 24"
        );
        assert_eq!(
            parse_bacnet_timestamp("datetime:2155,1,1,1;0,0,0,0").unwrap_err(),
            "BACnetTimeStamp full-year must be 1900..=2154 or 255 (unspecified), got 2155"
        );
    }
}
