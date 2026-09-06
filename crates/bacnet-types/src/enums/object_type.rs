// ===========================================================================
// ObjectType (Clause 12)
// ===========================================================================

#[cfg(all(feature = "serde", not(feature = "std")))]
use alloc::string::String;

#[cfg(feature = "serde")]
use serde::Deserialize;

bacnet_enum! {
    /// BACnet object types (Clause 12).
    ///
    /// Standard types are 0-63; vendor-proprietary types are 128-1023.
    /// The 10-bit type field allows values 0-1023.
    pub struct ObjectType(u32);

    const ANALOG_INPUT = 0;
    const ANALOG_OUTPUT = 1;
    const ANALOG_VALUE = 2;
    const BINARY_INPUT = 3;
    const BINARY_OUTPUT = 4;
    const BINARY_VALUE = 5;
    const CALENDAR = 6;
    const COMMAND = 7;
    const DEVICE = 8;
    const EVENT_ENROLLMENT = 9;
    const FILE = 10;
    const GROUP = 11;
    const LOOP = 12;
    const MULTI_STATE_INPUT = 13;
    const MULTI_STATE_OUTPUT = 14;
    const NOTIFICATION_CLASS = 15;
    const PROGRAM = 16;
    const SCHEDULE = 17;
    const AVERAGING = 18;
    const MULTI_STATE_VALUE = 19;
    const TREND_LOG = 20;
    const LIFE_SAFETY_POINT = 21;
    const LIFE_SAFETY_ZONE = 22;
    const ACCUMULATOR = 23;
    const PULSE_CONVERTER = 24;
    const EVENT_LOG = 25;
    const GLOBAL_GROUP = 26;
    const TREND_LOG_MULTIPLE = 27;
    const LOAD_CONTROL = 28;
    const STRUCTURED_VIEW = 29;
    const ACCESS_DOOR = 30;
    const TIMER = 31;
    const ACCESS_CREDENTIAL = 32;
    const ACCESS_POINT = 33;
    const ACCESS_RIGHTS = 34;
    const ACCESS_USER = 35;
    const ACCESS_ZONE = 36;
    const CREDENTIAL_DATA_INPUT = 37;
    /// Deprecated in 135-2020 (Clause 24 deleted).
    const NETWORK_SECURITY = 38;
    const BITSTRING_VALUE = 39;
    const CHARACTERSTRING_VALUE = 40;
    const DATEPATTERN_VALUE = 41;
    const DATE_VALUE = 42;
    const DATETIMEPATTERN_VALUE = 43;
    const DATETIME_VALUE = 44;
    const INTEGER_VALUE = 45;
    const LARGE_ANALOG_VALUE = 46;
    const OCTETSTRING_VALUE = 47;
    const POSITIVE_INTEGER_VALUE = 48;
    const TIMEPATTERN_VALUE = 49;
    const TIME_VALUE = 50;
    const NOTIFICATION_FORWARDER = 51;
    const ALERT_ENROLLMENT = 52;
    const CHANNEL = 53;
    const LIGHTING_OUTPUT = 54;
    const BINARY_LIGHTING_OUTPUT = 55;
    const NETWORK_PORT = 56;
    const ELEVATOR_GROUP = 57;
    const ESCALATOR = 58;
    const LIFT = 59;
    /// New in 135-2020.
    const STAGING = 60;
    /// New in 135-2020.
    const AUDIT_LOG = 61;
    /// New in 135-2020.
    const AUDIT_REPORTER = 62;
    /// New in 135-2020.
    const COLOR = 63;
    /// New in 135-2020.
    const COLOR_TEMPERATURE = 64;
}

#[cfg(feature = "serde")]
impl<'de> Deserialize<'de> for ObjectType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string: String = Deserialize::deserialize(deserializer)?;
        string.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::ObjectType;

    fn parse(input: &str) -> ObjectType {
        serde_json::from_str(&format!("\"{input}\"")).expect("deserializes")
    }

    #[test]
    fn accepts_every_supported_case_style() {
        for input in [
            "analoginput",
            "ANALOGINPUT",
            "AnalogInput",
            "analogInput",
            "analog_input",
            "ANALOG_INPUT",
            "analog-input",
            "ANALOG-INPUT",
        ] {
            assert_eq!(parse(input), ObjectType::ANALOG_INPUT, "input: {input}");
        }
    }

    #[test]
    fn covers_the_full_standard_range() {
        assert_eq!(parse("device"), ObjectType::DEVICE);
        assert_eq!(parse("color-temperature"), ObjectType::COLOR_TEMPERATURE);
    }

    /// The hand-written name table this impl replaced still mapped
    /// `audit-reporter` to 61 and `audit-log` to 62, contradicting the
    /// constants (and Clause 21.6). Deriving names from `ALL_NAMED` keeps the
    /// two from drifting apart again.
    #[test]
    fn audit_names_agree_with_the_constants() {
        assert_eq!(parse("audit-log"), ObjectType::AUDIT_LOG);
        assert_eq!(parse("audit-log").to_raw(), 61);
        assert_eq!(parse("audit-reporter"), ObjectType::AUDIT_REPORTER);
        assert_eq!(parse("audit-reporter").to_raw(), 62);
    }

    #[test]
    fn rejects_unknown_names() {
        assert!(serde_json::from_str::<ObjectType>("\"not-an-object\"").is_err());
    }

    #[test]
    fn rejects_non_string_input() {
        assert!(serde_json::from_str::<ObjectType>("8").is_err());
    }
}
