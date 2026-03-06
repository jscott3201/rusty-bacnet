//! Object/property shorthand parsing for integrators.
//!
//! Handles full names (kebab-case), common abbreviations, and numeric values
//! for BACnet object types, property identifiers, and write values.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, Time};

/// Abbreviation mappings for object types: (abbreviation, raw_value).
///
/// Raw values per ASHRAE 135-2020 Clause 21:
/// 0=analog-input, 1=analog-output, 2=analog-value, 3=binary-input, 4=binary-output,
/// 5=binary-value, 6=calendar, 7=command, 8=device, 10=file, 12=loop,
/// 13=multi-state-input, 14=multi-state-output, 15=notification-class, 16=program,
/// 17=schedule, 19=multi-state-value, 20=trend-log, 23=accumulator, 24=pulse-converter.
const OBJ_ABBREVS: &[(&str, u32)] = &[
    ("ai", 0),   // analog-input
    ("ao", 1),   // analog-output
    ("av", 2),   // analog-value
    ("bi", 3),   // binary-input
    ("bo", 4),   // binary-output
    ("bv", 5),   // binary-value
    ("dev", 8),  // device
    ("msi", 13), // multi-state-input
    ("mso", 14), // multi-state-output
    ("msv", 19), // multi-state-value
    ("lp", 21),  // life-safety-point
    ("lsp", 21), // life-safety-point
    ("sc", 17),  // schedule
    ("cal", 6),  // calendar
    ("nc", 15),  // notification-class
    ("trn", 20), // trend-log
    ("acc", 23), // accumulator
    ("pi", 24),  // pulse-converter
    ("lo", 12),  // loop
    ("prg", 16), // program
    ("cmd", 7),  // command
];

/// Abbreviation mappings for property identifiers: (abbreviation, raw_value).
const PROP_ABBREVS: &[(&str, u32)] = &[
    ("pv", 85),   // present-value
    ("on", 77),   // object-name
    ("ot", 79),   // object-type
    ("desc", 28), // description
    ("sf", 111),  // status-flags
    ("es", 36),   // event-state
    ("oos", 81),  // out-of-service
    ("pa", 87),   // priority-array
    ("rd", 104),  // relinquish-default
    ("ol", 76),   // object-list
    ("all", 8),   // ALL
];

/// Parse an object type from a string.
///
/// Accepts:
/// - Full names case-insensitive with hyphens: `analog-input`, `ANALOG_INPUT`
/// - Common abbreviations: `ai`, `ao`, `av`, `bi`, `bo`, `bv`, `dev`, etc.
/// - Numeric values: `0`, `1`, etc.
pub fn parse_object_type(s: &str) -> Result<ObjectType, String> {
    let s = s.trim();

    // Try numeric first.
    if let Ok(n) = s.parse::<u32>() {
        return Ok(ObjectType::from_raw(n));
    }

    // Try abbreviations (case-insensitive).
    let lower = s.to_ascii_lowercase();
    for &(abbr, raw) in OBJ_ABBREVS {
        if lower == abbr {
            return Ok(ObjectType::from_raw(raw));
        }
    }

    // Try matching against ALL_NAMED (convert hyphens to underscores, case-insensitive).
    let normalized = lower.replace('-', "_");
    for &(name, val) in ObjectType::ALL_NAMED {
        if name.eq_ignore_ascii_case(&normalized) {
            return Ok(val);
        }
    }

    Err(format!("unknown object type: '{s}'"))
}

/// Parse an object specifier like `analog-input:1` or `ai:1` into (ObjectType, instance).
pub fn parse_object_specifier(s: &str) -> Result<(ObjectType, u32), String> {
    let (type_str, inst_str) = s
        .rsplit_once(':')
        .ok_or_else(|| format!("expected 'type:instance' format, got '{s}'"))?;

    let obj_type = parse_object_type(type_str)?;
    let instance = inst_str
        .parse::<u32>()
        .map_err(|_| format!("invalid instance number: '{inst_str}'"))?;

    Ok((obj_type, instance))
}

/// Parse a property identifier from a string.
///
/// Accepts:
/// - Full names case-insensitive with hyphens: `present-value`, `object-name`
/// - Common abbreviations: `pv`, `on`, `ot`, `desc`, `sf`, `es`, `oos`, `pa`, `rd`, `ol`, `all`
/// - Numeric values: `85`, `77`, etc.
/// - Array index syntax: `object-list[3]` → (OBJECT_LIST, Some(3))
///
/// Returns (PropertyIdentifier, Option<array_index>).
pub fn parse_property(s: &str) -> Result<(PropertyIdentifier, Option<u32>), String> {
    let s = s.trim();

    // Check for array index: property[index]
    let (prop_str, array_index) = if let Some(bracket_pos) = s.find('[') {
        let end = s
            .find(']')
            .ok_or_else(|| format!("missing closing ']' in '{s}'"))?;
        if end <= bracket_pos {
            return Err(format!("malformed array index in '{s}'"));
        }
        let idx_str = &s[bracket_pos + 1..end];
        let idx = idx_str
            .parse::<u32>()
            .map_err(|_| format!("invalid array index: '{idx_str}'"))?;
        (&s[..bracket_pos], Some(idx))
    } else {
        (s, None)
    };

    let prop = parse_property_name(prop_str)?;
    Ok((prop, array_index))
}

/// Parse just the property name (without array index).
fn parse_property_name(s: &str) -> Result<PropertyIdentifier, String> {
    // Try numeric first.
    if let Ok(n) = s.parse::<u32>() {
        return Ok(PropertyIdentifier::from_raw(n));
    }

    // Try abbreviations (case-insensitive).
    let lower = s.to_ascii_lowercase();
    for &(abbr, raw) in PROP_ABBREVS {
        if lower == abbr {
            return Ok(PropertyIdentifier::from_raw(raw));
        }
    }

    // Try matching against ALL_NAMED (convert hyphens to underscores, case-insensitive).
    let normalized = lower.replace('-', "_");
    for &(name, val) in PropertyIdentifier::ALL_NAMED {
        if name.eq_ignore_ascii_case(&normalized) {
            return Ok(val);
        }
    }

    Err(format!("unknown property: '{s}'"))
}

/// Parse a write value from a string.
///
/// Accepts:
/// - `null` → PropertyValue::Null
/// - `true`/`false` → PropertyValue::Boolean
/// - `active` → PropertyValue::Enumerated(1), `inactive` → PropertyValue::Enumerated(0)
/// - Integer without decimal: `42` → PropertyValue::Unsigned, negative → PropertyValue::Signed
/// - Float with decimal: `72.5` → PropertyValue::Real
/// - Quoted string: `"hello"` → PropertyValue::CharacterString
/// - `enumerated:3` → PropertyValue::Enumerated(3)
pub fn parse_value(s: &str) -> Result<PropertyValue, String> {
    let s = s.trim();

    // Null.
    if s.eq_ignore_ascii_case("null") {
        return Ok(PropertyValue::Null);
    }

    // Boolean.
    if s.eq_ignore_ascii_case("true") {
        return Ok(PropertyValue::Boolean(true));
    }
    if s.eq_ignore_ascii_case("false") {
        return Ok(PropertyValue::Boolean(false));
    }

    // Active/Inactive as enumerated.
    if s.eq_ignore_ascii_case("active") {
        return Ok(PropertyValue::Enumerated(1));
    }
    if s.eq_ignore_ascii_case("inactive") {
        return Ok(PropertyValue::Enumerated(0));
    }

    // Date prefix: date:YYYY-MM-DD
    if let Some(rest) = s.strip_prefix("date:") {
        let parts: Vec<&str> = rest.split('-').collect();
        if parts.len() != 3 {
            return Err(format!(
                "invalid date format: '{rest}', expected YYYY-MM-DD"
            ));
        }
        let year = if parts[0] == "*" {
            0xFF
        } else {
            let y = parts[0]
                .parse::<u16>()
                .map_err(|_| format!("invalid year: '{}'", parts[0]))?;
            if y < 1900 {
                return Err(format!("year must be >= 1900, got {y}"));
            }
            (y - 1900) as u8
        };
        let month = if parts[1] == "*" {
            0xFF
        } else {
            parts[1]
                .parse::<u8>()
                .map_err(|_| format!("invalid month: '{}'", parts[1]))?
        };
        let day = if parts[2] == "*" {
            0xFF
        } else {
            parts[2]
                .parse::<u8>()
                .map_err(|_| format!("invalid day: '{}'", parts[2]))?
        };
        return Ok(PropertyValue::Date(Date {
            year,
            month,
            day,
            day_of_week: 0xFF,
        }));
    }

    // Time prefix: time:HH:MM:SS or time:HH:MM:SS.hh
    if let Some(rest) = s.strip_prefix("time:") {
        let (time_part, hundredths_part) = if let Some((tp, hp)) = rest.rsplit_once('.') {
            (tp, Some(hp))
        } else {
            (rest, None)
        };
        let parts: Vec<&str> = time_part.split(':').collect();
        if parts.len() != 3 {
            return Err(format!(
                "invalid time format: '{rest}', expected HH:MM:SS or HH:MM:SS.hh"
            ));
        }
        let hour = if parts[0] == "*" {
            0xFF
        } else {
            parts[0]
                .parse::<u8>()
                .map_err(|_| format!("invalid hour: '{}'", parts[0]))?
        };
        let minute = if parts[1] == "*" {
            0xFF
        } else {
            parts[1]
                .parse::<u8>()
                .map_err(|_| format!("invalid minute: '{}'", parts[1]))?
        };
        let second = if parts[2] == "*" {
            0xFF
        } else {
            parts[2]
                .parse::<u8>()
                .map_err(|_| format!("invalid second: '{}'", parts[2]))?
        };
        let hundredths = match hundredths_part {
            Some("*") => 0xFF,
            Some(h) => h
                .parse::<u8>()
                .map_err(|_| format!("invalid hundredths: '{h}'"))?,
            None => 0,
        };
        return Ok(PropertyValue::Time(Time {
            hour,
            minute,
            second,
            hundredths,
        }));
    }

    // ObjectIdentifier prefix: object:type:instance
    if let Some(rest) = s.strip_prefix("object:") {
        let (obj_type, instance) = parse_object_specifier(rest)?;
        let oid = ObjectIdentifier::new(obj_type, instance)
            .map_err(|e| format!("invalid object identifier: {e}"))?;
        return Ok(PropertyValue::ObjectIdentifier(oid));
    }

    // Enumerated prefix.
    if let Some(rest) = s.strip_prefix("enumerated:") {
        let val = rest
            .parse::<u32>()
            .map_err(|_| format!("invalid enumerated value: '{rest}'"))?;
        return Ok(PropertyValue::Enumerated(val));
    }

    // Quoted string.
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        let inner = &s[1..s.len() - 1];
        return Ok(PropertyValue::CharacterString(inner.to_string()));
    }

    // Numeric — check for decimal point to distinguish integer vs float.
    if s.contains('.') {
        let val = s
            .parse::<f32>()
            .map_err(|_| format!("invalid float value: '{s}'"))?;
        return Ok(PropertyValue::Real(val));
    }

    // Scientific notation (e.g., 1e10, -2.5e3).
    if (s.contains('e') || s.contains('E'))
        && !s.eq_ignore_ascii_case("enumerated")
        && !s.starts_with("enumerated:")
    {
        if let Ok(val) = s.parse::<f32>() {
            return Ok(PropertyValue::Real(val));
        }
    }

    // Try integer — negative is Signed, positive is Unsigned.
    if s.starts_with('-') {
        let val = s
            .parse::<i32>()
            .map_err(|_| format!("invalid integer value: '{s}'"))?;
        return Ok(PropertyValue::Signed(val));
    }

    if let Ok(val) = s.parse::<u64>() {
        return Ok(PropertyValue::Unsigned(val));
    }

    Err(format!("cannot parse value: '{s}'"))
}

/// Parse a value with optional `@priority` suffix.
///
/// Examples: `72.5@8` → (Real(72.5), Some(8)), `null@16` → (Null, Some(16)), `42` → (Unsigned(42), None)
pub fn parse_value_with_priority(s: &str) -> Result<(PropertyValue, Option<u8>), String> {
    let s = s.trim();

    // Find the last '@' that separates value from priority.
    // Be careful not to split inside a quoted string.
    let at_pos = if let Some(stripped) = s.strip_prefix('"') {
        // For quoted strings, look for '@' after the closing quote.
        if let Some(end_quote) = stripped.find('"') {
            let after_quote = end_quote + 2; // position after closing quote
            s[after_quote..].find('@').map(|p| p + after_quote)
        } else {
            None
        }
    } else {
        s.rfind('@')
    };

    if let Some(pos) = at_pos {
        let value_str = &s[..pos];
        let priority_str = &s[pos + 1..];
        let priority = priority_str
            .parse::<u8>()
            .map_err(|_| format!("invalid priority: '{priority_str}'"))?;
        if !(1..=16).contains(&priority) {
            return Err(format!("priority must be 1-16, got {priority}"));
        }
        let value = parse_value(value_str)?;
        Ok((value, Some(priority)))
    } else {
        let value = parse_value(s)?;
        Ok((value, None))
    }
}

/// Returns all object type names for tab completion (ALL_NAMED names + abbreviations).
///
/// Full names are converted from `UPPER_SNAKE_CASE` to `kebab-case` for user convenience.
pub fn object_type_completions() -> Vec<String> {
    let mut completions: Vec<String> = Vec::new();

    // Add abbreviations.
    for &(abbr, _) in OBJ_ABBREVS {
        completions.push(abbr.to_string());
    }

    // Add full names from ALL_NAMED, converted to kebab-case.
    for &(name, _) in ObjectType::ALL_NAMED {
        completions.push(name.to_ascii_lowercase().replace('_', "-"));
    }

    completions
}

/// Returns all property names for tab completion (ALL_NAMED names + abbreviations).
///
/// Full names are converted from `UPPER_SNAKE_CASE` to `kebab-case` for user convenience.
pub fn property_completions() -> Vec<String> {
    let mut completions: Vec<String> = Vec::new();

    // Add abbreviations.
    for &(abbr, _) in PROP_ABBREVS {
        completions.push(abbr.to_string());
    }

    // Add full names from ALL_NAMED, converted to kebab-case.
    for &(name, _) in PropertyIdentifier::ALL_NAMED {
        completions.push(name.to_ascii_lowercase().replace('_', "-"));
    }

    completions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_object_type_full_name() {
        assert_eq!(
            parse_object_type("analog-input").unwrap().to_raw(),
            ObjectType::ANALOG_INPUT.to_raw()
        );
        assert_eq!(
            parse_object_type("ANALOG_INPUT").unwrap().to_raw(),
            ObjectType::ANALOG_INPUT.to_raw()
        );
        assert_eq!(
            parse_object_type("Binary-Value").unwrap().to_raw(),
            ObjectType::BINARY_VALUE.to_raw()
        );
    }

    #[test]
    fn parse_object_type_abbreviation() {
        assert_eq!(parse_object_type("ai").unwrap().to_raw(), 0);
        assert_eq!(parse_object_type("bo").unwrap().to_raw(), 4);
        assert_eq!(parse_object_type("dev").unwrap().to_raw(), 8);
        assert_eq!(parse_object_type("msv").unwrap().to_raw(), 19);
        assert_eq!(parse_object_type("AI").unwrap().to_raw(), 0);
    }

    #[test]
    fn parse_object_type_numeric() {
        assert_eq!(parse_object_type("0").unwrap().to_raw(), 0);
        assert_eq!(parse_object_type("8").unwrap().to_raw(), 8);
        assert_eq!(parse_object_type("42").unwrap().to_raw(), 42);
    }

    #[test]
    fn parse_object_type_error() {
        assert!(parse_object_type("nonexistent").is_err());
    }

    #[test]
    fn parse_object_specifier_works() {
        let (ot, inst) = parse_object_specifier("ai:1").unwrap();
        assert_eq!(ot.to_raw(), 0);
        assert_eq!(inst, 1);

        let (ot, inst) = parse_object_specifier("analog-input:100").unwrap();
        assert_eq!(ot.to_raw(), 0);
        assert_eq!(inst, 100);

        let (ot, inst) = parse_object_specifier("dev:1234").unwrap();
        assert_eq!(ot.to_raw(), 8);
        assert_eq!(inst, 1234);
    }

    #[test]
    fn parse_object_specifier_errors() {
        assert!(parse_object_specifier("ai").is_err()); // missing instance
        assert!(parse_object_specifier("ai:abc").is_err()); // bad instance
    }

    #[test]
    fn parse_property_full_name() {
        let (p, idx) = parse_property("present-value").unwrap();
        assert_eq!(p.to_raw(), 85);
        assert_eq!(idx, None);
    }

    #[test]
    fn parse_property_abbreviation() {
        let (p, _) = parse_property("pv").unwrap();
        assert_eq!(p.to_raw(), 85);
        let (p, _) = parse_property("on").unwrap();
        assert_eq!(p.to_raw(), 77);
        let (p, _) = parse_property("all").unwrap();
        assert_eq!(p.to_raw(), 8);
    }

    #[test]
    fn parse_property_numeric() {
        let (p, _) = parse_property("85").unwrap();
        assert_eq!(p.to_raw(), 85);
    }

    #[test]
    fn parse_property_array_index() {
        let (p, idx) = parse_property("object-list[3]").unwrap();
        assert_eq!(p.to_raw(), 76);
        assert_eq!(idx, Some(3));

        let (p, idx) = parse_property("ol[0]").unwrap();
        assert_eq!(p.to_raw(), 76);
        assert_eq!(idx, Some(0));
    }

    #[test]
    fn parse_value_null() {
        assert!(matches!(parse_value("null").unwrap(), PropertyValue::Null));
        assert!(matches!(parse_value("NULL").unwrap(), PropertyValue::Null));
    }

    #[test]
    fn parse_value_boolean() {
        assert!(matches!(
            parse_value("true").unwrap(),
            PropertyValue::Boolean(true)
        ));
        assert!(matches!(
            parse_value("false").unwrap(),
            PropertyValue::Boolean(false)
        ));
    }

    #[test]
    fn parse_value_active_inactive() {
        assert!(matches!(
            parse_value("active").unwrap(),
            PropertyValue::Enumerated(1)
        ));
        assert!(matches!(
            parse_value("inactive").unwrap(),
            PropertyValue::Enumerated(0)
        ));
    }

    #[test]
    fn parse_value_unsigned() {
        match parse_value("42").unwrap() {
            PropertyValue::Unsigned(v) => assert_eq!(v, 42),
            other => panic!("expected Unsigned, got {other:?}"),
        }
    }

    #[test]
    fn parse_value_signed() {
        match parse_value("-5").unwrap() {
            PropertyValue::Signed(v) => assert_eq!(v, -5),
            other => panic!("expected Signed, got {other:?}"),
        }
    }

    #[test]
    fn parse_value_real() {
        match parse_value("72.5").unwrap() {
            PropertyValue::Real(v) => assert!((v - 72.5).abs() < f32::EPSILON),
            other => panic!("expected Real, got {other:?}"),
        }
    }

    #[test]
    fn parse_value_string() {
        match parse_value("\"hello world\"").unwrap() {
            PropertyValue::CharacterString(s) => assert_eq!(s, "hello world"),
            other => panic!("expected CharacterString, got {other:?}"),
        }
    }

    #[test]
    fn parse_value_enumerated() {
        match parse_value("enumerated:3").unwrap() {
            PropertyValue::Enumerated(v) => assert_eq!(v, 3),
            other => panic!("expected Enumerated, got {other:?}"),
        }
    }

    #[test]
    fn parse_value_with_priority_works() {
        let (val, pri) = parse_value_with_priority("72.5@8").unwrap();
        assert!(matches!(val, PropertyValue::Real(_)));
        assert_eq!(pri, Some(8));

        let (val, pri) = parse_value_with_priority("null@16").unwrap();
        assert!(matches!(val, PropertyValue::Null));
        assert_eq!(pri, Some(16));

        let (val, pri) = parse_value_with_priority("42").unwrap();
        assert!(matches!(val, PropertyValue::Unsigned(42)));
        assert_eq!(pri, None);
    }

    #[test]
    fn parse_value_with_priority_invalid() {
        assert!(parse_value_with_priority("42@0").is_err()); // priority 0 invalid
        assert!(parse_value_with_priority("42@17").is_err()); // priority 17 invalid
    }

    #[test]
    fn completions_not_empty() {
        assert!(!object_type_completions().is_empty());
        assert!(!property_completions().is_empty());
    }

    #[test]
    fn lp_maps_to_life_safety_point() {
        assert_eq!(parse_object_type("lp").unwrap().to_raw(), 21);
        assert_eq!(parse_object_type("lsp").unwrap().to_raw(), 21);
        assert_eq!(parse_object_type("cal").unwrap().to_raw(), 6);
    }

    #[test]
    fn bracket_ordering_panic() {
        assert!(parse_property("pv]3[").is_err());
    }

    #[test]
    fn scientific_notation_real() {
        match parse_value("1e10").unwrap() {
            PropertyValue::Real(v) => assert!((v - 1e10).abs() < 1e5),
            other => panic!("expected Real, got {other:?}"),
        }
        match parse_value("-2.5e3").unwrap() {
            PropertyValue::Real(v) => assert!((v - (-2500.0)).abs() < f32::EPSILON),
            other => panic!("expected Real, got {other:?}"),
        }
    }

    #[test]
    fn parse_date_value() {
        match parse_value("date:2024-03-15").unwrap() {
            PropertyValue::Date(d) => {
                assert_eq!(d.year, 124); // 2024 - 1900
                assert_eq!(d.month, 3);
                assert_eq!(d.day, 15);
                assert_eq!(d.day_of_week, 0xFF);
            }
            other => panic!("expected Date, got {other:?}"),
        }
    }

    #[test]
    fn parse_time_value() {
        match parse_value("time:14:30:00").unwrap() {
            PropertyValue::Time(t) => {
                assert_eq!(t.hour, 14);
                assert_eq!(t.minute, 30);
                assert_eq!(t.second, 0);
                assert_eq!(t.hundredths, 0);
            }
            other => panic!("expected Time, got {other:?}"),
        }
    }

    #[test]
    fn parse_object_identifier_value() {
        match parse_value("object:ai:1").unwrap() {
            PropertyValue::ObjectIdentifier(oid) => {
                assert_eq!(oid.object_type().to_raw(), 0);
                assert_eq!(oid.instance_number(), 1);
            }
            other => panic!("expected ObjectIdentifier, got {other:?}"),
        }
    }

    #[test]
    fn completions_are_kebab_case() {
        let ot = object_type_completions();
        // Full names should be kebab-case (no underscores, lowercase).
        let full_names: Vec<&String> = ot.iter().filter(|s| s.contains('-')).collect();
        assert!(!full_names.is_empty());
        for name in &full_names {
            assert!(!name.contains('_'), "found underscore in '{name}'");
            assert_eq!(name.to_ascii_lowercase(), **name, "not lowercase: '{name}'");
        }
        assert!(ot.iter().any(|s| s == "analog-input"));
    }

    #[test]
    fn empty_string_with_priority() {
        let (val, pri) = parse_value_with_priority("\"\"@8").unwrap();
        match val {
            PropertyValue::CharacterString(s) => assert_eq!(s, ""),
            other => panic!("expected CharacterString, got {other:?}"),
        }
        assert_eq!(pri, Some(8));
    }
}
