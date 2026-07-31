// ===========================================================================
// ObjectType (Clause 12)
// ===========================================================================

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

impl<'de> Deserialize<'de> for ObjectType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string: String = Deserialize::deserialize(deserializer)?;

        // Stripping separators and lowercasing collapses all supported
        // case styles (lowercase, UPPERCASE, PascalCase, camelCase,
        // snake_case, SCREAMING_SNAKE_CASE, kebab-case, SCREAMING-KEBAB-CASE)
        // onto the same normalized form.
        let normalized: String = string
            .chars()
            .filter(|c| *c != '_' && *c != '-')
            .flat_map(|c| c.to_lowercase())
            .collect();

        let object_type = match normalized.as_str() {
            "analoginput" => Self(0),
            "analogoutput" => Self(1),
            "analogvalue" => Self(2),
            "binaryinput" => Self(3),
            "binaryoutput" => Self(4),
            "binaryvalue" => Self(5),
            "calendar" => Self(6),
            "command" => Self(7),
            "device" => Self(8),
            "eventenrollment" => Self(9),
            "file" => Self(10),
            "group" => Self(11),
            "loop" => Self(12),
            "multistateinput" => Self(13),
            "multistateoutput" => Self(14),
            "notificationclass" => Self(15),
            "program" => Self(16),
            "schedule" => Self(17),
            "averaging" => Self(18),
            "multistatevalue" => Self(19),
            "trendlog" => Self(20),
            "lifesafetypoint" => Self(21),
            "lifesafetyzone" => Self(22),
            "accumulator" => Self(23),
            "pulseconverter" => Self(24),
            "eventlog" => Self(25),
            "globalgroup" => Self(26),
            "trendlogmultiple" => Self(27),
            "loadcontrol" => Self(28),
            "structuredview" => Self(29),
            "accessdoor" => Self(30),
            "timer" => Self(31),
            "accesscredential" => Self(32),
            "accesspoint" => Self(33),
            "accessrights" => Self(34),
            "accessuser" => Self(35),
            "accesszone" => Self(36),
            "credentialdatainput" => Self(37),
            "networksecurity" => Self(38),
            "bitstringvalue" => Self(39),
            "characterstringvalue" => Self(40),
            "datepatternvalue" => Self(41),
            "datevalue" => Self(42),
            "datetimepatternvalue" => Self(43),
            "datetimevalue" => Self(44),
            "integervalue" => Self(45),
            "largeanalogvalue" => Self(46),
            "octetstringvalue" => Self(47),
            "positiveintegervalue" => Self(48),
            "timepatternvalue" => Self(49),
            "timevalue" => Self(50),
            "notificationforwarder" => Self(51),
            "alertenrollment" => Self(52),
            "channel" => Self(53),
            "lightingoutput" => Self(54),
            "binarylightingoutput" => Self(55),
            "networkport" => Self(56),
            "elevatorgroup" => Self(57),
            "escalator" => Self(58),
            "lift" => Self(59),
            "staging" => Self(60),
            "auditreporter" => Self(61),
            "auditlog" => Self(62),
            "color" => Self(63),
            "colortemperature" => Self(64),
            _ => return Err(serde::de::Error::custom("invalid ObjectType.")),
        };

        Ok(object_type)
    }
}
