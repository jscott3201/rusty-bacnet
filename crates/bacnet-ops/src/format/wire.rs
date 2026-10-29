//! Application values as plain text without a property to name them by:
//! the form the capture decode tree and the one-shot CLI's tag views show.

use bacnet_types::primitives::PropertyValue;

/// A `PropertyValue` as plain text, with no property context: the
/// capture decode tree and the CLI's tag views use it; read values go
/// through [`ValueFormat`](super::ValueFormat), which knows the property.
pub fn format_property_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Boolean(b) => b.to_string(),
        PropertyValue::Unsigned(v) => v.to_string(),
        PropertyValue::Signed(v) => v.to_string(),
        PropertyValue::Real(v) => format!("{v}"),
        PropertyValue::Double(v) => format!("{v}"),
        PropertyValue::OctetString(bytes) => {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("[{}]", hex.join(" "))
        }
        PropertyValue::CharacterString(s) => format!("\"{s}\""),
        PropertyValue::BitString { unused_bits, data } => {
            let total_bits = (data.len() * 8).saturating_sub(*unused_bits as usize);
            let mut bits = String::with_capacity(total_bits);
            for i in 0..total_bits {
                let byte_idx = i / 8;
                let bit_idx = 7 - (i % 8);
                if data[byte_idx] & (1 << bit_idx) != 0 {
                    bits.push('1');
                } else {
                    bits.push('0');
                }
            }
            format!("bits({bits})")
        }
        PropertyValue::Enumerated(v) => format!("enumerated({v})"),
        PropertyValue::Date(d) => {
            let year_str = if d.year == 0xFF {
                "*".to_string()
            } else {
                format!("{}", d.year as u16 + 1900)
            };
            let month_str = if d.month == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", d.month)
            };
            let day_str = if d.day == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", d.day)
            };
            format!("{year_str}-{month_str}-{day_str}")
        }
        PropertyValue::Time(t) => {
            let hour_str = if t.hour == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.hour)
            };
            let minute_str = if t.minute == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.minute)
            };
            let second_str = if t.second == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.second)
            };
            let hundredths_str = if t.hundredths == 0xFF {
                "*".to_string()
            } else {
                format!("{:02}", t.hundredths)
            };
            format!("{hour_str}:{minute_str}:{second_str}.{hundredths_str}")
        }
        PropertyValue::ObjectIdentifier(oid) => {
            format!("{}:{}", oid.object_type(), oid.instance_number())
        }
        PropertyValue::List(items) => {
            let formatted: Vec<String> = items.iter().map(format_property_value).collect();
            format!("[{}]", formatted.join(", "))
        }
        PropertyValue::ApplicationData(bytes) => {
            let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
            format!("application-data([{}])", hex.join(" "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_null() {
        assert_eq!(format_property_value(&PropertyValue::Null), "null");
    }

    #[test]
    fn format_boolean() {
        assert_eq!(format_property_value(&PropertyValue::Boolean(true)), "true");
    }

    #[test]
    fn format_unsigned() {
        assert_eq!(format_property_value(&PropertyValue::Unsigned(42)), "42");
    }

    #[test]
    fn format_string() {
        assert_eq!(
            format_property_value(&PropertyValue::CharacterString("hello".to_string())),
            "\"hello\""
        );
    }

    #[test]
    fn format_enumerated() {
        assert_eq!(
            format_property_value(&PropertyValue::Enumerated(3)),
            "enumerated(3)"
        );
    }

    #[test]
    fn format_bitstring_empty_data() {
        // B4: empty data with unused_bits > 0 should not panic.
        let val = PropertyValue::BitString {
            unused_bits: 3,
            data: vec![],
        };
        assert_eq!(format_property_value(&val), "bits()");
    }

    #[test]
    fn format_date_human_readable() {
        use bacnet_types::primitives::Date;
        let d = Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        };
        assert_eq!(format_property_value(&PropertyValue::Date(d)), "2024-03-15");
    }

    #[test]
    fn format_date_unspecified() {
        use bacnet_types::primitives::Date;
        let d = Date {
            year: 0xFF,
            month: 12,
            day: 0xFF,
            day_of_week: 0xFF,
        };
        assert_eq!(format_property_value(&PropertyValue::Date(d)), "*-12-*");
    }

    #[test]
    fn format_time_human_readable() {
        use bacnet_types::primitives::Time;
        let t = Time {
            hour: 14,
            minute: 30,
            second: 0,
            hundredths: 0,
        };
        assert_eq!(
            format_property_value(&PropertyValue::Time(t)),
            "14:30:00.00"
        );
    }
}
