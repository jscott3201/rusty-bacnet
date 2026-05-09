// ===========================================================================
// ObjectType (Clause 12)
// ===========================================================================

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
    const AUDIT_REPORTER = 61;
    /// New in 135-2020.
    const AUDIT_LOG = 62;
    /// New in 135-2020.
    const COLOR = 63;
    /// New in 135-2020.
    const COLOR_TEMPERATURE = 64;
}
