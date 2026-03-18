//! BACnet enumeration types per ASHRAE 135-2020.
//!
//! Uses newtype wrappers (e.g. `ObjectType(u32)`) with associated constants
//! rather than Rust enums so that vendor-proprietary values pass through
//! without panicking. Every type provides `from_raw` / `to_raw` for
//! wire-level conversion and a human-readable `Display` impl.

#[cfg(not(feature = "std"))]
use alloc::format;

// ---------------------------------------------------------------------------
// Macro to reduce boilerplate for newtype enum wrappers
// ---------------------------------------------------------------------------

/// Generates a newtype wrapper struct with associated constants, `from_raw`,
/// `to_raw`, `Display`, and optional `Debug` that shows the symbolic name.
macro_rules! bacnet_enum {
    (
        $(#[$meta:meta])*
        $vis:vis struct $Name:ident($inner:ty);
        $(
            $(#[$vmeta:meta])*
            const $VARIANT:ident = $val:expr;
        )*
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        $vis struct $Name($inner);

        impl $Name {
            $(
                $(#[$vmeta])*
                pub const $VARIANT: Self = Self($val);
            )*

            /// Create from a raw wire value.
            #[inline]
            pub const fn from_raw(value: $inner) -> Self {
                Self(value)
            }

            /// Return the raw wire value.
            #[inline]
            pub const fn to_raw(self) -> $inner {
                self.0
            }

            /// All named constants as (name, value) pairs.
            pub const ALL_NAMED: &[(&str, Self)] = &[
                $( (stringify!($VARIANT), Self($val)), )*
            ];
        }

        impl core::fmt::Debug for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.0 {
                    $( $val => f.write_str(concat!(stringify!($Name), "::", stringify!($VARIANT))), )*
                    other => write!(f, "{}({})", stringify!($Name), other),
                }
            }
        }

        impl core::fmt::Display for $Name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.0 {
                    $( $val => f.write_str(stringify!($VARIANT)), )*
                    other => write!(f, "{}", other),
                }
            }
        }
    };
}

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
}

// ===========================================================================
// PropertyIdentifier (Clause 21)
// ===========================================================================

bacnet_enum! {
    /// BACnet property identifiers (Clause 21).
    ///
    /// Standard properties are 0-511; vendor-proprietary IDs are 512+.
    /// The 22-bit property field allows values 0-4,194,303.
    pub struct PropertyIdentifier(u32);

    // 0-50
    const ACKED_TRANSITIONS = 0;
    const ACK_REQUIRED = 1;
    const ACTION = 2;
    const ACTION_TEXT = 3;
    const ACTIVE_TEXT = 4;
    const ACTIVE_VT_SESSIONS = 5;
    const ALARM_VALUE = 6;
    const ALARM_VALUES = 7;
    const ALL = 8;
    const ALL_WRITES_SUCCESSFUL = 9;
    const APDU_SEGMENT_TIMEOUT = 10;
    const APDU_TIMEOUT = 11;
    const APPLICATION_SOFTWARE_VERSION = 12;
    const ARCHIVE = 13;
    const BIAS = 14;
    const CHANGE_OF_STATE_COUNT = 15;
    const CHANGE_OF_STATE_TIME = 16;
    const NOTIFICATION_CLASS = 17;
    // 18: deleted
    const CONTROLLED_VARIABLE_REFERENCE = 19;
    const CONTROLLED_VARIABLE_UNITS = 20;
    const CONTROLLED_VARIABLE_VALUE = 21;
    const COV_INCREMENT = 22;
    const DATE_LIST = 23;
    const DAYLIGHT_SAVINGS_STATUS = 24;
    const DEADBAND = 25;
    const DERIVATIVE_CONSTANT = 26;
    const DERIVATIVE_CONSTANT_UNITS = 27;
    const DESCRIPTION = 28;
    const DESCRIPTION_OF_HALT = 29;
    const DEVICE_ADDRESS_BINDING = 30;
    const DEVICE_TYPE = 31;
    const EFFECTIVE_PERIOD = 32;
    const ELAPSED_ACTIVE_TIME = 33;
    const ERROR_LIMIT = 34;
    const EVENT_ENABLE = 35;
    const EVENT_STATE = 36;
    const EVENT_TYPE = 37;
    const EXCEPTION_SCHEDULE = 38;
    const FAULT_VALUES = 39;
    const FEEDBACK_VALUE = 40;
    const FILE_ACCESS_METHOD = 41;
    const FILE_SIZE = 42;
    const FILE_TYPE = 43;
    const FIRMWARE_REVISION = 44;
    const HIGH_LIMIT = 45;
    const INACTIVE_TEXT = 46;
    const IN_PROCESS = 47;
    const INSTANCE_OF = 48;
    const INTEGRAL_CONSTANT = 49;
    const INTEGRAL_CONSTANT_UNITS = 50;

    // 51-100
    const ISSUE_CONFIRMED_NOTIFICATIONS = 51;
    const LIMIT_ENABLE = 52;
    const LIST_OF_GROUP_MEMBERS = 53;
    const LIST_OF_OBJECT_PROPERTY_REFERENCES = 54;
    // 55: deleted
    const LOCAL_DATE = 56;
    const LOCAL_TIME = 57;
    const LOCATION = 58;
    const LOW_LIMIT = 59;
    const MANIPULATED_VARIABLE_REFERENCE = 60;
    const MAXIMUM_OUTPUT = 61;
    const MAX_APDU_LENGTH_ACCEPTED = 62;
    const MAX_INFO_FRAMES = 63;
    const MAX_MASTER = 64;
    const MAX_PRES_VALUE = 65;
    const MINIMUM_OFF_TIME = 66;
    const MINIMUM_ON_TIME = 67;
    const MINIMUM_OUTPUT = 68;
    const MIN_PRES_VALUE = 69;
    const MODEL_NAME = 70;
    const MODIFICATION_DATE = 71;
    const NOTIFY_TYPE = 72;
    const NUMBER_OF_APDU_RETRIES = 73;
    const NUMBER_OF_STATES = 74;
    const OBJECT_IDENTIFIER = 75;
    const OBJECT_LIST = 76;
    const OBJECT_NAME = 77;
    const OBJECT_PROPERTY_REFERENCE = 78;
    const OBJECT_TYPE = 79;
    const OPTIONAL = 80;
    const OUT_OF_SERVICE = 81;
    const OUTPUT_UNITS = 82;
    const EVENT_PARAMETERS = 83;
    const POLARITY = 84;
    const PRESENT_VALUE = 85;
    const PRIORITY = 86;
    const PRIORITY_ARRAY = 87;
    const PRIORITY_FOR_WRITING = 88;
    const PROCESS_IDENTIFIER = 89;
    const PROGRAM_CHANGE = 90;
    const PROGRAM_LOCATION = 91;
    const PROGRAM_STATE = 92;
    const PROPORTIONAL_CONSTANT = 93;
    const PROPORTIONAL_CONSTANT_UNITS = 94;
    // 95: deleted
    const PROTOCOL_OBJECT_TYPES_SUPPORTED = 96;
    const PROTOCOL_SERVICES_SUPPORTED = 97;
    const PROTOCOL_VERSION = 98;
    const READ_ONLY = 99;
    const REASON_FOR_HALT = 100;

    // 101-200
    // 101: deleted
    const RECIPIENT_LIST = 102;
    const RELIABILITY = 103;
    const RELINQUISH_DEFAULT = 104;
    const REQUIRED = 105;
    const RESOLUTION = 106;
    const SEGMENTATION_SUPPORTED = 107;
    const SETPOINT = 108;
    const SETPOINT_REFERENCE = 109;
    const STATE_TEXT = 110;
    const STATUS_FLAGS = 111;
    const SYSTEM_STATUS = 112;
    const TIME_DELAY = 113;
    const TIME_OF_ACTIVE_TIME_RESET = 114;
    const TIME_OF_STATE_COUNT_RESET = 115;
    const TIME_SYNCHRONIZATION_RECIPIENTS = 116;
    const UNITS = 117;
    const UPDATE_INTERVAL = 118;
    const UTC_OFFSET = 119;
    const VENDOR_IDENTIFIER = 120;
    const VENDOR_NAME = 121;
    const VT_CLASSES_SUPPORTED = 122;
    const WEEKLY_SCHEDULE = 123;
    const ATTEMPTED_SAMPLES = 124;
    const AVERAGE_VALUE = 125;
    const BUFFER_SIZE = 126;
    const CLIENT_COV_INCREMENT = 127;
    const COV_RESUBSCRIPTION_INTERVAL = 128;
    // 129: deleted
    const EVENT_TIME_STAMPS = 130;
    const LOG_BUFFER = 131;
    const LOG_DEVICE_OBJECT_PROPERTY = 132;
    const LOG_ENABLE = 133;
    const LOG_INTERVAL = 134;
    const MAXIMUM_VALUE = 135;
    const MINIMUM_VALUE = 136;
    const NOTIFICATION_THRESHOLD = 137;
    // 138: deleted
    const PROTOCOL_REVISION = 139;
    const RECORDS_SINCE_NOTIFICATION = 140;
    const RECORD_COUNT = 141;
    const START_TIME = 142;
    const STOP_TIME = 143;
    const STOP_WHEN_FULL = 144;
    const TOTAL_RECORD_COUNT = 145;
    const VALID_SAMPLES = 146;
    const WINDOW_INTERVAL = 147;
    const WINDOW_SAMPLES = 148;
    const MAXIMUM_VALUE_TIMESTAMP = 149;
    const MINIMUM_VALUE_TIMESTAMP = 150;
    const VARIANCE_VALUE = 151;
    const ACTIVE_COV_SUBSCRIPTIONS = 152;
    const BACKUP_FAILURE_TIMEOUT = 153;
    const CONFIGURATION_FILES = 154;
    const DATABASE_REVISION = 155;
    const DIRECT_READING = 156;
    const LAST_RESTORE_TIME = 157;
    const MAINTENANCE_REQUIRED = 158;
    const MEMBER_OF = 159;
    const MODE = 160;
    const OPERATION_EXPECTED = 161;
    const SETTING = 162;
    const SILENCED = 163;
    const TRACKING_VALUE = 164;
    const ZONE_MEMBERS = 165;
    const LIFE_SAFETY_ALARM_VALUES = 166;
    const MAX_SEGMENTS_ACCEPTED = 167;
    const PROFILE_NAME = 168;
    const AUTO_SLAVE_DISCOVERY = 169;
    const MANUAL_SLAVE_ADDRESS_BINDING = 170;
    const SLAVE_ADDRESS_BINDING = 171;
    const SLAVE_PROXY_ENABLE = 172;
    const LAST_NOTIFY_RECORD = 173;
    const SCHEDULE_DEFAULT = 174;
    const ACCEPTED_MODES = 175;
    const ADJUST_VALUE = 176;
    const COUNT = 177;
    const COUNT_BEFORE_CHANGE = 178;
    const COUNT_CHANGE_TIME = 179;
    const COV_PERIOD = 180;
    const INPUT_REFERENCE = 181;
    const LIMIT_MONITORING_INTERVAL = 182;
    const LOGGING_OBJECT = 183;
    const LOGGING_RECORD = 184;
    const PRESCALE = 185;
    const PULSE_RATE = 186;
    const SCALE = 187;
    const SCALE_FACTOR = 188;
    const UPDATE_TIME = 189;
    const VALUE_BEFORE_CHANGE = 190;
    const VALUE_SET = 191;
    const VALUE_CHANGE_TIME = 192;
    const ALIGN_INTERVALS = 193;
    // 194: unassigned
    const INTERVAL_OFFSET = 195;
    const LAST_RESTART_REASON = 196;
    const LOGGING_TYPE = 197;
    // 198-201: unassigned

    // 202-235
    const RESTART_NOTIFICATION_RECIPIENTS = 202;
    const TIME_OF_DEVICE_RESTART = 203;
    const TIME_SYNCHRONIZATION_INTERVAL = 204;
    const TRIGGER = 205;
    const UTC_TIME_SYNCHRONIZATION_RECIPIENTS = 206;
    const NODE_SUBTYPE = 207;
    const NODE_TYPE = 208;
    const STRUCTURED_OBJECT_LIST = 209;
    const SUBORDINATE_ANNOTATIONS = 210;
    const SUBORDINATE_LIST = 211;
    const ACTUAL_SHED_LEVEL = 212;
    const DUTY_WINDOW = 213;
    const EXPECTED_SHED_LEVEL = 214;
    const FULL_DUTY_BASELINE = 215;
    // 216-217: unassigned
    const REQUESTED_SHED_LEVEL = 218;
    const SHED_DURATION = 219;
    const SHED_LEVEL_DESCRIPTIONS = 220;
    const SHED_LEVELS = 221;
    const STATE_DESCRIPTION = 222;
    // 223-225: unassigned
    const DOOR_ALARM_STATE = 226;
    const DOOR_EXTENDED_PULSE_TIME = 227;
    const DOOR_MEMBERS = 228;
    const DOOR_OPEN_TOO_LONG_TIME = 229;
    const DOOR_PULSE_TIME = 230;
    const DOOR_STATUS = 231;
    const DOOR_UNLOCK_DELAY_TIME = 232;
    const LOCK_STATUS = 233;
    const MASKED_ALARM_VALUES = 234;
    const SECURED_STATUS = 235;

    // 244-323 (access control)
    const ABSENTEE_LIMIT = 244;
    const ACCESS_ALARM_EVENTS = 245;
    const ACCESS_DOORS = 246;
    const ACCESS_EVENT = 247;
    const ACCESS_EVENT_AUTHENTICATION_FACTOR = 248;
    const ACCESS_EVENT_CREDENTIAL = 249;
    const ACCESS_EVENT_TIME = 250;
    const ACCESS_TRANSACTION_EVENTS = 251;
    const ACCOMPANIMENT = 252;
    const ACCOMPANIMENT_TIME = 253;
    const ACTIVATION_TIME = 254;
    const ACTIVE_AUTHENTICATION_POLICY = 255;
    const ASSIGNED_ACCESS_RIGHTS = 256;
    const AUTHENTICATION_FACTORS = 257;
    const AUTHENTICATION_POLICY_LIST = 258;
    const AUTHENTICATION_POLICY_NAMES = 259;
    const AUTHENTICATION_STATUS = 260;
    const AUTHORIZATION_MODE = 261;
    const BELONGS_TO = 262;
    const CREDENTIAL_DISABLE = 263;
    const CREDENTIAL_STATUS = 264;
    const CREDENTIALS = 265;
    const CREDENTIALS_IN_ZONE = 266;
    const DAYS_REMAINING = 267;
    const ENTRY_POINTS = 268;
    const EXIT_POINTS = 269;
    const EXPIRATION_TIME = 270;
    const EXTENDED_TIME_ENABLE = 271;
    const FAILED_ATTEMPT_EVENTS = 272;
    const FAILED_ATTEMPTS = 273;
    const FAILED_ATTEMPTS_TIME = 274;
    const LAST_ACCESS_EVENT = 275;
    const LAST_ACCESS_POINT = 276;
    const LAST_CREDENTIAL_ADDED = 277;
    const LAST_CREDENTIAL_ADDED_TIME = 278;
    const LAST_CREDENTIAL_REMOVED = 279;
    const LAST_CREDENTIAL_REMOVED_TIME = 280;
    const LAST_USE_TIME = 281;
    const LOCKOUT = 282;
    const LOCKOUT_RELINQUISH_TIME = 283;
    // 284: deleted
    const MAX_FAILED_ATTEMPTS = 285;
    const MEMBERS = 286;
    const MUSTER_POINT = 287;
    const NEGATIVE_ACCESS_RULES = 288;
    const NUMBER_OF_AUTHENTICATION_POLICIES = 289;
    const OCCUPANCY_COUNT = 290;
    const OCCUPANCY_COUNT_ADJUST = 291;
    const OCCUPANCY_COUNT_ENABLE = 292;
    // 293: deleted
    const OCCUPANCY_LOWER_LIMIT = 294;
    const OCCUPANCY_LOWER_LIMIT_ENFORCED = 295;
    const OCCUPANCY_STATE = 296;
    const OCCUPANCY_UPPER_LIMIT = 297;
    const OCCUPANCY_UPPER_LIMIT_ENFORCED = 298;
    // 299: deleted
    const PASSBACK_MODE = 300;
    const PASSBACK_TIMEOUT = 301;
    const POSITIVE_ACCESS_RULES = 302;
    const REASON_FOR_DISABLE = 303;
    const SUPPORTED_FORMATS = 304;
    const SUPPORTED_FORMAT_CLASSES = 305;
    const THREAT_AUTHORITY = 306;
    const THREAT_LEVEL = 307;
    const TRACE_FLAG = 308;
    const TRANSACTION_NOTIFICATION_CLASS = 309;
    const USER_EXTERNAL_IDENTIFIER = 310;
    const USER_INFORMATION_REFERENCE = 311;
    // 312-316: unassigned
    const USER_NAME = 317;
    const USER_TYPE = 318;
    const USES_REMAINING = 319;
    const ZONE_FROM = 320;
    const ZONE_TO = 321;
    const ACCESS_EVENT_TAG = 322;
    const GLOBAL_IDENTIFIER = 323;

    // 326-398
    const VERIFICATION_TIME = 326;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const BASE_DEVICE_SECURITY_POLICY = 327;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const DISTRIBUTION_KEY_REVISION = 328;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const DO_NOT_HIDE = 329;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const KEY_SETS = 330;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const LAST_KEY_SERVER = 331;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const NETWORK_ACCESS_SECURITY_POLICIES = 332;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const PACKET_REORDER_TIME = 333;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const SECURITY_PDU_TIMEOUT = 334;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const SECURITY_TIME_WINDOW = 335;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const SUPPORTED_SECURITY_ALGORITHMS = 336;
    /// Deprecated: removed with Clause 24 in 135-2020.
    const UPDATE_KEY_SET_TIMEOUT = 337;
    const BACKUP_AND_RESTORE_STATE = 338;
    const BACKUP_PREPARATION_TIME = 339;
    const RESTORE_COMPLETION_TIME = 340;
    const RESTORE_PREPARATION_TIME = 341;
    const BIT_MASK = 342;
    const BIT_TEXT = 343;
    const IS_UTC = 344;
    const GROUP_MEMBERS = 345;
    const GROUP_MEMBER_NAMES = 346;
    const MEMBER_STATUS_FLAGS = 347;
    const REQUESTED_UPDATE_INTERVAL = 348;
    const COVU_PERIOD = 349;
    const COVU_RECIPIENTS = 350;
    const EVENT_MESSAGE_TEXTS = 351;
    const EVENT_MESSAGE_TEXTS_CONFIG = 352;
    const EVENT_DETECTION_ENABLE = 353;
    const EVENT_ALGORITHM_INHIBIT = 354;
    const EVENT_ALGORITHM_INHIBIT_REF = 355;
    const TIME_DELAY_NORMAL = 356;
    const RELIABILITY_EVALUATION_INHIBIT = 357;
    const FAULT_PARAMETERS = 358;
    const FAULT_TYPE = 359;
    const LOCAL_FORWARDING_ONLY = 360;
    const PROCESS_IDENTIFIER_FILTER = 361;
    const SUBSCRIBED_RECIPIENTS = 362;
    const PORT_FILTER = 363;
    const AUTHORIZATION_EXEMPTIONS = 364;
    const ALLOW_GROUP_DELAY_INHIBIT = 365;
    const CHANNEL_NUMBER = 366;
    const CONTROL_GROUPS = 367;
    const EXECUTION_DELAY = 368;
    const LAST_PRIORITY = 369;
    const WRITE_STATUS = 370;
    const PROPERTY_LIST = 371;
    const SERIAL_NUMBER = 372;
    const BLINK_WARN_ENABLE = 373;
    const DEFAULT_FADE_TIME = 374;
    const DEFAULT_RAMP_RATE = 375;
    const DEFAULT_STEP_INCREMENT = 376;
    const EGRESS_TIME = 377;
    const IN_PROGRESS = 378;
    const INSTANTANEOUS_POWER = 379;
    const LIGHTING_COMMAND = 380;
    const LIGHTING_COMMAND_DEFAULT_PRIORITY = 381;
    const MAX_ACTUAL_VALUE = 382;
    const MIN_ACTUAL_VALUE = 383;
    const POWER = 384;
    const TRANSITION = 385;
    const EGRESS_ACTIVE = 386;
    const INTERFACE_VALUE = 387;
    const FAULT_HIGH_LIMIT = 388;
    const FAULT_LOW_LIMIT = 389;
    const LOW_DIFF_LIMIT = 390;
    const STRIKE_COUNT = 391;
    const TIME_OF_STRIKE_COUNT_RESET = 392;
    const DEFAULT_TIMEOUT = 393;
    const INITIAL_TIMEOUT = 394;
    const LAST_STATE_CHANGE = 395;
    const STATE_CHANGE_VALUES = 396;
    const TIMER_RUNNING = 397;
    const TIMER_STATE = 398;

    // 399-429 (NetworkPort, Clause 12.56)
    const APDU_LENGTH = 399;
    const IP_ADDRESS = 400;
    const IP_DEFAULT_GATEWAY = 401;
    const IP_DHCP_ENABLE = 402;
    const IP_DHCP_LEASE_TIME = 403;
    const IP_DHCP_LEASE_TIME_REMAINING = 404;
    const IP_DHCP_SERVER = 405;
    const IP_DNS_SERVER = 406;
    const BACNET_IP_GLOBAL_ADDRESS = 407;
    const BACNET_IP_MODE = 408;
    const BACNET_IP_MULTICAST_ADDRESS = 409;
    const BACNET_IP_NAT_TRAVERSAL = 410;
    const IP_SUBNET_MASK = 411;
    const BACNET_IP_UDP_PORT = 412;
    const BBMD_ACCEPT_FD_REGISTRATIONS = 413;
    const BBMD_BROADCAST_DISTRIBUTION_TABLE = 414;
    const BBMD_FOREIGN_DEVICE_TABLE = 415;
    const CHANGES_PENDING = 416;
    const COMMAND_NP = 417;
    const FD_BBMD_ADDRESS = 418;
    const FD_SUBSCRIPTION_LIFETIME = 419;
    const LINK_SPEED = 420;
    const LINK_SPEEDS = 421;
    const LINK_SPEED_AUTONEGOTIATE = 422;
    const MAC_ADDRESS = 423;
    const NETWORK_INTERFACE_NAME = 424;
    const NETWORK_NUMBER = 425;
    const NETWORK_NUMBER_QUALITY = 426;
    const NETWORK_TYPE = 427;
    const ROUTING_TABLE = 428;
    const VIRTUAL_MAC_ADDRESS_TABLE = 429;

    // 430-446 (commandable + IPv6)
    const COMMAND_TIME_ARRAY = 430;
    const CURRENT_COMMAND_PRIORITY = 431;
    const LAST_COMMAND_TIME = 432;
    const VALUE_SOURCE = 433;
    const VALUE_SOURCE_ARRAY = 434;
    const BACNET_IPV6_MODE = 435;
    const IPV6_ADDRESS = 436;
    const IPV6_PREFIX_LENGTH = 437;
    const BACNET_IPV6_UDP_PORT = 438;
    const IPV6_DEFAULT_GATEWAY = 439;
    const BACNET_IPV6_MULTICAST_ADDRESS = 440;
    const IPV6_DNS_SERVER = 441;
    const IPV6_AUTO_ADDRESSING_ENABLE = 442;
    const IPV6_DHCP_LEASE_TIME = 443;
    const IPV6_DHCP_LEASE_TIME_REMAINING = 444;
    const IPV6_DHCP_SERVER = 445;
    const IPV6_ZONE_INDEX = 446;

    // 447-480 (lift/escalator, Clause 12.58-12.60)
    const ASSIGNED_LANDING_CALLS = 447;
    const CAR_ASSIGNED_DIRECTION = 448;
    const CAR_DOOR_COMMAND = 449;
    const CAR_DOOR_STATUS = 450;
    const CAR_DOOR_TEXT = 451;
    const CAR_DOOR_ZONE = 452;
    const CAR_DRIVE_STATUS = 453;
    const CAR_LOAD = 454;
    const CAR_LOAD_UNITS = 455;
    const CAR_MODE = 456;
    const CAR_MOVING_DIRECTION = 457;
    const CAR_POSITION = 458;
    const ELEVATOR_GROUP = 459;
    const ENERGY_METER = 460;
    const ENERGY_METER_REF = 461;
    const ESCALATOR_MODE = 462;
    const FAULT_SIGNALS = 463;
    const FLOOR_TEXT = 464;
    const GROUP_ID = 465;
    // 466: unassigned
    const GROUP_MODE = 467;
    const HIGHER_DECK = 468;
    const INSTALLATION_ID = 469;
    const LANDING_CALLS = 470;
    const LANDING_CALL_CONTROL = 471;
    const LANDING_DOOR_STATUS = 472;
    const LOWER_DECK = 473;
    const MACHINE_ROOM_ID = 474;
    const MAKING_CAR_CALL = 475;
    const NEXT_STOPPING_FLOOR = 476;
    const OPERATION_DIRECTION = 477;
    const PASSENGER_ALARM = 478;
    const POWER_MODE = 479;
    const REGISTERED_CAR_CALL = 480;

    // 481-507 (misc + staging + audit)
    const ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS = 481;
    const PROTOCOL_LEVEL = 482;
    const REFERENCE_PORT = 483;
    const DEPLOYED_PROFILE_LOCATION = 484;
    const PROFILE_LOCATION = 485;
    const TAGS = 486;
    const SUBORDINATE_NODE_TYPES = 487;
    const SUBORDINATE_TAGS = 488;
    const SUBORDINATE_RELATIONSHIPS = 489;
    const DEFAULT_SUBORDINATE_RELATIONSHIP = 490;
    const REPRESENTS = 491;
    const DEFAULT_PRESENT_VALUE = 492;
    const PRESENT_STAGE = 493;
    const STAGES = 494;
    const STAGE_NAMES = 495;
    const TARGET_REFERENCES = 496;
    const AUDIT_SOURCE_REPORTER = 497;
    const AUDIT_LEVEL = 498;
    const AUDIT_NOTIFICATION_RECIPIENT = 499;
    const AUDIT_PRIORITY_FILTER = 500;
    const AUDITABLE_OPERATIONS = 501;
    const DELETE_ON_FORWARD = 502;
    const MAXIMUM_SEND_DELAY = 503;
    const MONITORED_OBJECTS = 504;
    const SEND_NOW = 505;
    const FLOOR_NUMBER = 506;
    const DEVICE_UUID = 507;
}

// ===========================================================================
// Protocol enums (PDU types, services, error classes/codes)
// ===========================================================================

bacnet_enum! {
    /// APDU PDU type identifiers (Clause 20.1).
    pub struct PduType(u8);

    const CONFIRMED_REQUEST = 0;
    const UNCONFIRMED_REQUEST = 1;
    const SIMPLE_ACK = 2;
    const COMPLEX_ACK = 3;
    const SEGMENT_ACK = 4;
    const ERROR = 5;
    const REJECT = 6;
    const ABORT = 7;
}

bacnet_enum! {
    /// Confirmed service request types (Clause 21).
    pub struct ConfirmedServiceChoice(u8);

    const ACKNOWLEDGE_ALARM = 0;
    const CONFIRMED_COV_NOTIFICATION = 1;
    const CONFIRMED_EVENT_NOTIFICATION = 2;
    const GET_ALARM_SUMMARY = 3;
    const GET_ENROLLMENT_SUMMARY = 4;
    const SUBSCRIBE_COV = 5;
    const ATOMIC_READ_FILE = 6;
    const ATOMIC_WRITE_FILE = 7;
    const ADD_LIST_ELEMENT = 8;
    const REMOVE_LIST_ELEMENT = 9;
    const CREATE_OBJECT = 10;
    const DELETE_OBJECT = 11;
    const READ_PROPERTY = 12;
    // 13: reserved
    const READ_PROPERTY_MULTIPLE = 14;
    const WRITE_PROPERTY = 15;
    const WRITE_PROPERTY_MULTIPLE = 16;
    const DEVICE_COMMUNICATION_CONTROL = 17;
    const CONFIRMED_PRIVATE_TRANSFER = 18;
    const CONFIRMED_TEXT_MESSAGE = 19;
    const REINITIALIZE_DEVICE = 20;
    const VT_OPEN = 21;
    const VT_CLOSE = 22;
    const VT_DATA = 23;
    // 24-25: reserved
    const READ_RANGE = 26;
    const LIFE_SAFETY_OPERATION = 27;
    const SUBSCRIBE_COV_PROPERTY = 28;
    const GET_EVENT_INFORMATION = 29;
    const SUBSCRIBE_COV_PROPERTY_MULTIPLE = 30;
    const CONFIRMED_COV_NOTIFICATION_MULTIPLE = 31;
    const CONFIRMED_AUDIT_NOTIFICATION = 32;
    const AUDIT_LOG_QUERY = 33;
}

bacnet_enum! {
    /// Unconfirmed service request types (Clause 21).
    pub struct UnconfirmedServiceChoice(u8);

    const I_AM = 0;
    const I_HAVE = 1;
    const UNCONFIRMED_COV_NOTIFICATION = 2;
    const UNCONFIRMED_EVENT_NOTIFICATION = 3;
    const UNCONFIRMED_PRIVATE_TRANSFER = 4;
    const UNCONFIRMED_TEXT_MESSAGE = 5;
    const TIME_SYNCHRONIZATION = 6;
    const WHO_HAS = 7;
    const WHO_IS = 8;
    const UTC_TIME_SYNCHRONIZATION = 9;
    const WRITE_GROUP = 10;
    const UNCONFIRMED_COV_NOTIFICATION_MULTIPLE = 11;
    const UNCONFIRMED_AUDIT_NOTIFICATION = 12;
    const WHO_AM_I = 13;
    const YOU_ARE = 14;
}

bacnet_enum! {
    /// BACnet error classes (Clause 18.1.1).
    pub struct ErrorClass(u16);

    const DEVICE = 0;
    const OBJECT = 1;
    const PROPERTY = 2;
    const RESOURCES = 3;
    const SECURITY = 4;
    const SERVICES = 5;
    const VT = 6;
    const COMMUNICATION = 7;
}

bacnet_enum! {
    /// BACnet error codes (Clause 18).
    pub struct ErrorCode(u16);

    const OTHER = 0;
    const AUTHENTICATION_FAILED = 1;
    const CONFIGURATION_IN_PROGRESS = 2;
    const DEVICE_BUSY = 3;
    const DYNAMIC_CREATION_NOT_SUPPORTED = 4;
    const FILE_ACCESS_DENIED = 5;
    const INCOMPATIBLE_SECURITY_LEVELS = 6;
    const INCONSISTENT_PARAMETERS = 7;
    const INCONSISTENT_SELECTION_CRITERION = 8;
    const INVALID_DATA_TYPE = 9;
    const INVALID_FILE_ACCESS_METHOD = 10;
    const INVALID_FILE_START_POSITION = 11;
    const INVALID_OPERATOR_NAME = 12;
    const INVALID_PARAMETER_DATA_TYPE = 13;
    const INVALID_TIME_STAMP = 14;
    const KEY_GENERATION_ERROR = 15;
    const MISSING_REQUIRED_PARAMETER = 16;
    const NO_OBJECTS_OF_SPECIFIED_TYPE = 17;
    const NO_SPACE_FOR_OBJECT = 18;
    const NO_SPACE_TO_ADD_LIST_ELEMENT = 19;
    const NO_SPACE_TO_WRITE_PROPERTY = 20;
    const NO_VT_SESSIONS_AVAILABLE = 21;
    const PROPERTY_IS_NOT_A_LIST = 22;
    const OBJECT_DELETION_NOT_PERMITTED = 23;
    const OBJECT_IDENTIFIER_ALREADY_EXISTS = 24;
    const OPERATIONAL_PROBLEM = 25;
    const PASSWORD_FAILURE = 26;
    const READ_ACCESS_DENIED = 27;
    const SECURITY_NOT_SUPPORTED = 28;
    const SERVICE_REQUEST_DENIED = 29;
    const TIMEOUT = 30;
    const UNKNOWN_OBJECT = 31;
    const UNKNOWN_PROPERTY = 32;
    // 33: removed
    const UNKNOWN_VT_CLASS = 34;
    const UNKNOWN_VT_SESSION = 35;
    const UNSUPPORTED_OBJECT_TYPE = 36;
    const VALUE_OUT_OF_RANGE = 37;
    const VT_SESSION_ALREADY_CLOSED = 38;
    const VT_SESSION_TERMINATION_FAILURE = 39;
    const WRITE_ACCESS_DENIED = 40;
    const CHARACTER_SET_NOT_SUPPORTED = 41;
    const INVALID_ARRAY_INDEX = 42;
    const COV_SUBSCRIPTION_FAILED = 43;
    const NOT_COV_PROPERTY = 44;
    const OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED = 45;
    const INVALID_CONFIGURATION_DATA = 46;
    const DATATYPE_NOT_SUPPORTED = 47;
    const DUPLICATE_NAME = 48;
    const DUPLICATE_OBJECT_ID = 49;
    const PROPERTY_IS_NOT_AN_ARRAY = 50;
    const ABORT_BUFFER_OVERFLOW = 51;
    const ABORT_INVALID_APDU_IN_THIS_STATE = 52;
    const ABORT_PREEMPTED_BY_HIGHER_PRIORITY_TASK = 53;
    const ABORT_SEGMENTATION_NOT_SUPPORTED = 54;
    const ABORT_PROPRIETARY = 55;
    const ABORT_OTHER = 56;
    const INVALID_TAG = 57;
    const NETWORK_DOWN = 58;
    const REJECT_BUFFER_OVERFLOW = 59;
    const REJECT_INCONSISTENT_PARAMETERS = 60;
    const REJECT_INVALID_PARAMETER_DATA_TYPE = 61;
    const REJECT_INVALID_TAG = 62;
    const REJECT_MISSING_REQUIRED_PARAMETER = 63;
    const REJECT_PARAMETER_OUT_OF_RANGE = 64;
    const REJECT_TOO_MANY_ARGUMENTS = 65;
    const REJECT_UNDEFINED_ENUMERATION = 66;
    const REJECT_UNRECOGNIZED_SERVICE = 67;
    const REJECT_PROPRIETARY = 68;
    const REJECT_OTHER = 69;
    const UNKNOWN_DEVICE = 70;
    const UNKNOWN_ROUTE = 71;
    const VALUE_NOT_INITIALIZED = 72;
    const INVALID_EVENT_STATE = 73;
    const NO_ALARM_CONFIGURED = 74;
    const LOG_BUFFER_FULL = 75;
    const LOGGED_VALUE_PURGED = 76;
    const NO_PROPERTY_SPECIFIED = 77;
    const NOT_CONFIGURED_FOR_TRIGGERED_LOGGING = 78;
    const UNKNOWN_SUBSCRIPTION = 79;
    const PARAMETER_OUT_OF_RANGE = 80;
    const LIST_ELEMENT_NOT_FOUND = 81;
    const BUSY = 82;
    const COMMUNICATION_DISABLED = 83;
    const SUCCESS = 84;
    const ACCESS_DENIED = 85;
    const BAD_DESTINATION_ADDRESS = 86;
    const BAD_DESTINATION_DEVICE_ID = 87;
    const BAD_SIGNATURE = 88;
    const BAD_SOURCE_ADDRESS = 89;
    const BAD_TIMESTAMP = 90;
    const CANNOT_USE_KEY = 91;
    const CANNOT_VERIFY_MESSAGE_ID = 92;
    const CORRECT_KEY_REVISION = 93;
    const DESTINATION_DEVICE_ID_REQUIRED = 94;
    const DUPLICATE_MESSAGE = 95;
    const ENCRYPTION_NOT_CONFIGURED = 96;
    const ENCRYPTION_REQUIRED = 97;
    const INCORRECT_KEY = 98;
    const INVALID_KEY_DATA = 99;
    const KEY_UPDATE_IN_PROGRESS = 100;
    const MALFORMED_MESSAGE = 101;
    const NOT_KEY_SERVER = 102;
    const SECURITY_NOT_CONFIGURED = 103;
    const SOURCE_SECURITY_REQUIRED = 104;
    const TOO_MANY_KEYS = 105;
    const UNKNOWN_AUTHENTICATION_TYPE = 106;
    const UNKNOWN_KEY = 107;
    const UNKNOWN_KEY_REVISION = 108;
    const UNKNOWN_SOURCE_MESSAGE = 109;
    const NOT_ROUTER_TO_DNET = 110;
    const ROUTER_BUSY = 111;
    const UNKNOWN_NETWORK_MESSAGE = 112;
    const MESSAGE_TOO_LONG = 113;
    const SECURITY_ERROR = 114;
    const ADDRESSING_ERROR = 115;
    const WRITE_BDT_FAILED = 116;
    const READ_BDT_FAILED = 117;
    const REGISTER_FOREIGN_DEVICE_FAILED = 118;
    const READ_FDT_FAILED = 119;
    const DELETE_FDT_ENTRY_FAILED = 120;
    const DISTRIBUTE_BROADCAST_FAILED = 121;
    const UNKNOWN_FILE_SIZE = 122;
    const ABORT_APDU_TOO_LONG = 123;
    const ABORT_APPLICATION_EXCEEDED_REPLY_TIME = 124;
    const ABORT_OUT_OF_RESOURCES = 125;
    const ABORT_TSM_TIMEOUT = 126;
    const ABORT_WINDOW_SIZE_OUT_OF_RANGE = 127;
    const FILE_FULL = 128;
    const INCONSISTENT_CONFIGURATION = 129;
    const INCONSISTENT_OBJECT_TYPE = 130;
    const INTERNAL_ERROR = 131;
    const NOT_CONFIGURED = 132;
    const OUT_OF_MEMORY = 133;
    const VALUE_TOO_LONG = 134;
    const ABORT_INSUFFICIENT_SECURITY = 135;
    const ABORT_SECURITY_ERROR = 136;
    const DUPLICATE_ENTRY = 137;
    const INVALID_VALUE_IN_THIS_STATE = 138;
}

bacnet_enum! {
    /// BACnet abort reasons (Clause 20.1.9).
    pub struct AbortReason(u8);

    const OTHER = 0;
    const BUFFER_OVERFLOW = 1;
    const INVALID_APDU_IN_THIS_STATE = 2;
    const PREEMPTED_BY_HIGHER_PRIORITY_TASK = 3;
    const SEGMENTATION_NOT_SUPPORTED = 4;
    const SECURITY_ERROR = 5;
    const INSUFFICIENT_SECURITY = 6;
    const WINDOW_SIZE_OUT_OF_RANGE = 7;
    const APPLICATION_EXCEEDED_REPLY_TIME = 8;
    const OUT_OF_RESOURCES = 9;
    const TSM_TIMEOUT = 10;
    const APDU_TOO_LONG = 11;
}

bacnet_enum! {
    /// BACnet reject reasons (Clause 20.1.8).
    pub struct RejectReason(u8);

    const OTHER = 0;
    const BUFFER_OVERFLOW = 1;
    const INCONSISTENT_PARAMETERS = 2;
    const INVALID_PARAMETER_DATA_TYPE = 3;
    const INVALID_TAG = 4;
    const MISSING_REQUIRED_PARAMETER = 5;
    const PARAMETER_OUT_OF_RANGE = 6;
    const TOO_MANY_ARGUMENTS = 7;
    const UNDEFINED_ENUMERATION = 8;
    const UNRECOGNIZED_SERVICE = 9;
}

bacnet_enum! {
    /// Segmentation support options (Clause 20.1.2.4).
    pub struct Segmentation(u8);

    const BOTH = 0;
    const TRANSMIT = 1;
    const RECEIVE = 2;
    const NONE = 3;
}

// ===========================================================================
// Network layer enums (Clause 6)
// ===========================================================================

bacnet_enum! {
    /// NPDU network priority levels (Clause 6.2.2).
    pub struct NetworkPriority(u8);

    const NORMAL = 0;
    const URGENT = 1;
    const CRITICAL_EQUIPMENT = 2;
    const LIFE_SAFETY = 3;
}

bacnet_enum! {
    /// Network layer message types (Clause 6.2.4).
    pub struct NetworkMessageType(u8);

    const WHO_IS_ROUTER_TO_NETWORK = 0x00;
    const I_AM_ROUTER_TO_NETWORK = 0x01;
    const I_COULD_BE_ROUTER_TO_NETWORK = 0x02;
    const REJECT_MESSAGE_TO_NETWORK = 0x03;
    const ROUTER_BUSY_TO_NETWORK = 0x04;
    const ROUTER_AVAILABLE_TO_NETWORK = 0x05;
    const INITIALIZE_ROUTING_TABLE = 0x06;
    const INITIALIZE_ROUTING_TABLE_ACK = 0x07;
    const ESTABLISH_CONNECTION_TO_NETWORK = 0x08;
    const DISCONNECT_CONNECTION_TO_NETWORK = 0x09;
    const CHALLENGE_REQUEST = 0x0A;
    const SECURITY_PAYLOAD = 0x0B;
    const SECURITY_RESPONSE = 0x0C;
    const REQUEST_KEY_UPDATE = 0x0D;
    const UPDATE_KEY_SET = 0x0E;
    const UPDATE_DISTRIBUTION_KEY = 0x0F;
    const REQUEST_MASTER_KEY = 0x10;
    const SET_MASTER_KEY = 0x11;
    const WHAT_IS_NETWORK_NUMBER = 0x12;
    const NETWORK_NUMBER_IS = 0x13;
}

bacnet_enum! {
    /// Reject-Message-To-Network reason codes (Clause 6.4.4).
    pub struct RejectMessageReason(u8);

    const OTHER = 0;
    const NOT_DIRECTLY_CONNECTED = 1;
    const ROUTER_BUSY = 2;
    const UNKNOWN_MESSAGE_TYPE = 3;
    const MESSAGE_TOO_LONG = 4;
    /// Removed per 135-2020
    const REMOVED_5 = 5;
    const ADDRESSING_ERROR = 6;
}

// ===========================================================================
// BVLC enums (Annex J / Annex U)
// ===========================================================================

bacnet_enum! {
    /// BACnet/IPv4 BVLC function codes (Annex J).
    pub struct BvlcFunction(u8);

    const BVLC_RESULT = 0x00;
    const WRITE_BROADCAST_DISTRIBUTION_TABLE = 0x01;
    const READ_BROADCAST_DISTRIBUTION_TABLE = 0x02;
    const READ_BROADCAST_DISTRIBUTION_TABLE_ACK = 0x03;
    const FORWARDED_NPDU = 0x04;
    const REGISTER_FOREIGN_DEVICE = 0x05;
    const READ_FOREIGN_DEVICE_TABLE = 0x06;
    const READ_FOREIGN_DEVICE_TABLE_ACK = 0x07;
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY = 0x08;
    const DISTRIBUTE_BROADCAST_TO_NETWORK = 0x09;
    const ORIGINAL_UNICAST_NPDU = 0x0A;
    const ORIGINAL_BROADCAST_NPDU = 0x0B;
    const SECURE_BVLL = 0x0C;
}

bacnet_enum! {
    /// BACnet/IPv4 BVLC-Result codes (Annex J.2).
    pub struct BvlcResultCode(u16);

    const SUCCESSFUL_COMPLETION = 0x0000;
    const WRITE_BROADCAST_DISTRIBUTION_TABLE_NAK = 0x0010;
    const READ_BROADCAST_DISTRIBUTION_TABLE_NAK = 0x0020;
    const REGISTER_FOREIGN_DEVICE_NAK = 0x0030;
    const READ_FOREIGN_DEVICE_TABLE_NAK = 0x0040;
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK = 0x0050;
    const DISTRIBUTE_BROADCAST_TO_NETWORK_NAK = 0x0060;
}

bacnet_enum! {
    /// BACnet/IPv6 BVLC function codes (Annex U, Table U-1).
    pub struct Bvlc6Function(u8);

    const BVLC_RESULT = 0x00;
    const ORIGINAL_UNICAST_NPDU = 0x01;
    const ORIGINAL_BROADCAST_NPDU = 0x02;
    const ADDRESS_RESOLUTION = 0x03;
    const FORWARDED_ADDRESS_RESOLUTION = 0x04;
    const ADDRESS_RESOLUTION_ACK = 0x05;
    const VIRTUAL_ADDRESS_RESOLUTION = 0x06;
    const VIRTUAL_ADDRESS_RESOLUTION_ACK = 0x07;
    const FORWARDED_NPDU = 0x08;
    const REGISTER_FOREIGN_DEVICE = 0x09;
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY = 0x0A;
    // 0x0B is removed per Table U-1
    const DISTRIBUTE_BROADCAST_TO_NETWORK = 0x0C;
}

bacnet_enum! {
    /// BACnet/IPv6 BVLC-Result codes (Annex U.2.1.1).
    pub struct Bvlc6ResultCode(u16);

    const SUCCESSFUL_COMPLETION = 0x0000;
    const ADDRESS_RESOLUTION_NAK = 0x0030;
    const VIRTUAL_ADDRESS_RESOLUTION_NAK = 0x0060;
    const REGISTER_FOREIGN_DEVICE_NAK = 0x0090;
    const DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK = 0x00A0;
    const DISTRIBUTE_BROADCAST_TO_NETWORK_NAK = 0x00C0;
}

// ===========================================================================
// Object-level enums (Clause 12, 21)
// ===========================================================================

bacnet_enum! {
    /// BACnet event state (Clause 12).
    pub struct EventState(u32);

    const NORMAL = 0;
    const FAULT = 1;
    const OFFNORMAL = 2;
    const HIGH_LIMIT = 3;
    const LOW_LIMIT = 4;
    const LIFE_SAFETY_ALARM = 5;
}

bacnet_enum! {
    /// BACnet binary present value (Clause 21).
    pub struct BinaryPV(u32);

    const INACTIVE = 0;
    const ACTIVE = 1;
}

bacnet_enum! {
    /// BACnet polarity (Clause 12).
    pub struct Polarity(u32);

    const NORMAL = 0;
    const REVERSE = 1;
}

bacnet_enum! {
    /// BACnet reliability (Clause 12).
    pub struct Reliability(u32);

    const NO_FAULT_DETECTED = 0;
    const NO_SENSOR = 1;
    const OVER_RANGE = 2;
    const UNDER_RANGE = 3;
    const OPEN_LOOP = 4;
    const SHORTED_LOOP = 5;
    const NO_OUTPUT = 6;
    const UNRELIABLE_OTHER = 7;
    const PROCESS_ERROR = 8;
    const MULTI_STATE_FAULT = 9;
    const CONFIGURATION_ERROR = 10;
    // 11: removed from standard
    const COMMUNICATION_FAILURE = 12;
    const MEMBER_FAULT = 13;
    const MONITORED_OBJECT_FAULT = 14;
    const TRIPPED = 15;
    const LAMP_FAILURE = 16;
    const ACTIVATION_FAILURE = 17;
    const RENEW_DHCP_FAILURE = 18;
    const RENEW_FD_REGISTRATION_FAILURE = 19;
    const RESTART_AUTO_NEGOTIATION_FAILURE = 20;
    const RESTART_FAILURE = 21;
    const PROPRIETARY_COMMAND_FAILURE = 22;
    const FAULTS_LISTED = 23;
    const REFERENCED_OBJECT_FAULT = 24;
}

bacnet_enum! {
    /// BACnet device status (Clause 12.11.9).
    pub struct DeviceStatus(u32);

    const OPERATIONAL = 0;
    const OPERATIONAL_READ_ONLY = 1;
    const DOWNLOAD_REQUIRED = 2;
    const DOWNLOAD_IN_PROGRESS = 3;
    const NON_OPERATIONAL = 4;
    const BACKUP_IN_PROGRESS = 5;
}

bacnet_enum! {
    /// BACnet enable/disable (Clause 16.4).
    pub struct EnableDisable(u32);

    const ENABLE = 0;
    /// Deprecated in revision 20; use DISABLE_INITIATION instead.
    const DISABLE = 1;
    const DISABLE_INITIATION = 2;
}

bacnet_enum! {
    /// BACnet reinitialized state of device (Clause 16.5).
    pub struct ReinitializedState(u32);

    const COLDSTART = 0;
    const WARMSTART = 1;
    const START_BACKUP = 2;
    const END_BACKUP = 3;
    const START_RESTORE = 4;
    const END_RESTORE = 5;
    const ABORT_RESTORE = 6;
    const ACTIVATE_CHANGES = 7;
}

bacnet_enum! {
    /// BACnet file access method (Clause 12.12).
    pub struct FileAccessMethod(u32);

    const STREAM_ACCESS = 0;
    const RECORD_ACCESS = 1;
}

bacnet_enum! {
    /// BACnet program state (Clause 12.22).
    pub struct ProgramState(u32);

    const IDLE = 0;
    const LOADING = 1;
    const RUNNING = 2;
    const WAITING = 3;
    const HALTED = 4;
    const UNLOADING = 5;
}

bacnet_enum! {
    /// BACnet program request (Clause 12.22).
    pub struct ProgramChange(u32);

    const READY = 0;
    const LOAD = 1;
    const RUN = 2;
    const HALT = 3;
    const RESTART = 4;
    const UNLOAD = 5;
}

bacnet_enum! {
    /// BACnet action (Clause 12.17).
    pub struct Action(u32);

    const DIRECT = 0;
    const REVERSE = 1;
}

bacnet_enum! {
    /// BACnet event type (Clause 12.12.6).
    pub struct EventType(u32);

    const CHANGE_OF_BITSTRING = 0;
    const CHANGE_OF_STATE = 1;
    const CHANGE_OF_VALUE = 2;
    const COMMAND_FAILURE = 3;
    const FLOATING_LIMIT = 4;
    const OUT_OF_RANGE = 5;
    // 6-7: reserved
    const CHANGE_OF_LIFE_SAFETY = 8;
    const EXTENDED = 9;
    const BUFFER_READY = 10;
    const UNSIGNED_RANGE = 11;
    // 12: reserved
    const ACCESS_EVENT = 13;
    const DOUBLE_OUT_OF_RANGE = 14;
    const SIGNED_OUT_OF_RANGE = 15;
    const UNSIGNED_OUT_OF_RANGE = 16;
    const CHANGE_OF_CHARACTERSTRING = 17;
    const CHANGE_OF_STATUS_FLAGS = 18;
    const CHANGE_OF_RELIABILITY = 19;
    const NONE = 20;
    const CHANGE_OF_DISCRETE_VALUE = 21;
    const CHANGE_OF_TIMER = 22;
}

bacnet_enum! {
    /// BACnet notify type (Clause 12.21).
    pub struct NotifyType(u32);

    const ALARM = 0;
    const EVENT = 1;
    const ACK_NOTIFICATION = 2;
}

bacnet_enum! {
    /// BACnet backup and restore state (Clause 19.1).
    pub struct BackupAndRestoreState(u32);

    const IDLE = 0;
    const PREPARING_FOR_BACKUP = 1;
    const PREPARING_FOR_RESTORE = 2;
    const PERFORMING_A_BACKUP = 3;
    const PERFORMING_A_RESTORE = 4;
}

bacnet_enum! {
    /// BACnet logging type (Clause 12.25.14).
    pub struct LoggingType(u32);

    const POLLED = 0;
    const COV = 1;
    const TRIGGERED = 2;
}

// ===========================================================================
// Network port enums (Clause 12.56)
// ===========================================================================

bacnet_enum! {
    /// BACnet data link/network type (Clause 12.56.44).
    pub struct NetworkType(u32);

    const ETHERNET = 0;
    const ARCNET = 1;
    const MSTP = 2;
    const PTP = 3;
    const LONTALK = 4;
    const IPV4 = 5;
    const ZIGBEE = 6;
    const VIRTUAL = 7;
    /// Removed in protocol revision 16.
    const NON_BACNET = 8;
    const IPV6 = 9;
    const SERIAL = 10;
}

bacnet_enum! {
    /// IP addressing mode for a NetworkPort (Clause 12.56).
    pub struct IPMode(u32);

    const NORMAL = 0;
    const FOREIGN = 1;
    const BBMD = 2;
}

bacnet_enum! {
    /// Commands for a NetworkPort object (Clause 12.56.40).
    pub struct NetworkPortCommand(u32);

    const IDLE = 0;
    const DISCARD_CHANGES = 1;
    const RENEW_FD_REGISTRATION = 2;
    const RESTART_SLAVE_DISCOVERY = 3;
    const RENEW_DHCP = 4;
    const RESTART_AUTONEG = 5;
    const DISCONNECT = 6;
    const RESTART_PORT = 7;
}

bacnet_enum! {
    /// Quality of a NetworkPort's network number (Clause 12.56.42).
    pub struct NetworkNumberQuality(u32);

    const UNKNOWN = 0;
    const LEARNED = 1;
    const LEARNED_CONFIGURED = 2;
    const CONFIGURED = 3;
}

bacnet_enum! {
    /// Network reachability status (Clause 6.6.1).
    pub struct NetworkReachability(u32);

    const REACHABLE = 0;
    const BUSY = 1;
    const UNREACHABLE = 2;
}

bacnet_enum! {
    /// Protocol level of a NetworkPort (Clause 12.56).
    pub struct ProtocolLevel(u32);

    const PHYSICAL = 0;
    const PROTOCOL = 1;
    const BACNET_APPLICATION = 2;
    const NON_BACNET_APPLICATION = 3;
}

bacnet_enum! {
    /// Status of the last Channel write operation (Clause 12.53).
    pub struct WriteStatus(u32);

    const IDLE = 0;
    const IN_PROGRESS = 1;
    const SUCCESSFUL = 2;
    const FAILED = 3;
}

// ===========================================================================
// Life safety enums (Clause 12.15, 12.16)
// ===========================================================================

bacnet_enum! {
    /// Life safety point/zone sensor state (Clause 12.15/12.16).
    pub struct LifeSafetyState(u32);

    const QUIET = 0;
    const PRE_ALARM = 1;
    const ALARM = 2;
    const FAULT = 3;
    const FAULT_PRE_ALARM = 4;
    const FAULT_ALARM = 5;
    const NOT_READY = 6;
    const ACTIVE = 7;
    const TAMPER = 8;
    const TEST_ALARM = 9;
    const TEST_ACTIVE = 10;
    const TEST_FAULT = 11;
    const TEST_FAULT_ALARM = 12;
    const HOLDUP = 13;
    const DURESS = 14;
    const TAMPER_ALARM = 15;
    const ABNORMAL = 16;
    const EMERGENCY_POWER = 17;
    const DELAYED = 18;
    const BLOCKED = 19;
    const LOCAL_ALARM = 20;
    const GENERAL_ALARM = 21;
    const SUPERVISORY = 22;
    const TEST_SUPERVISORY = 23;
}

bacnet_enum! {
    /// Life safety operating mode (Clause 12.15.12).
    pub struct LifeSafetyMode(u32);

    const OFF = 0;
    const ON = 1;
    const TEST = 2;
    const MANNED = 3;
    const UNMANNED = 4;
    const ARMED = 5;
    const DISARMED = 6;
    const PRE_ARMED = 7;
    const SLOW = 8;
    const FAST = 9;
    const DISCONNECTED = 10;
    const ENABLED = 11;
    const DISABLED = 12;
    const AUTOMATIC_RELEASE_DISABLED = 13;
    const DEFAULT = 14;
}

bacnet_enum! {
    /// Life safety commanded operation (Clause 12.15.13).
    pub struct LifeSafetyOperation(u32);

    const NONE = 0;
    const SILENCE = 1;
    const SILENCE_AUDIBLE = 2;
    const SILENCE_VISUAL = 3;
    const SILENCE_ALL = 4;
    const UNSILENCE = 5;
    const UNSILENCE_AUDIBLE = 6;
    const UNSILENCE_VISUAL = 7;
    const UNSILENCE_ALL = 8;
    const RESET = 9;
    const RESET_ALARM = 10;
    const RESET_FAULT = 11;
}

bacnet_enum! {
    /// Silenced state for a life safety point/zone (Clause 12.15.14).
    pub struct SilencedState(u32);

    const UNSILENCED = 0;
    const AUDIBLE_SILENCED = 1;
    const VISIBLE_SILENCED = 2;
    const ALL_SILENCED = 3;
}

// ===========================================================================
// Timer enums (Clause 12.31)
// ===========================================================================

bacnet_enum! {
    /// BACnet timer state (Clause 12.31, new in 135-2020).
    pub struct TimerState(u32);

    const IDLE = 0;
    const RUNNING = 1;
    const EXPIRED = 2;
}

bacnet_enum! {
    /// BACnet timer state transition (Clause 12.31, new in 135-2020).
    pub struct TimerTransition(u32);

    const NONE = 0;
    const IDLE_TO_RUNNING = 1;
    const RUNNING_TO_IDLE = 2;
    const RUNNING_TO_RUNNING = 3;
    const RUNNING_TO_EXPIRED = 4;
    const FORCED_TO_EXPIRED = 5;
    const EXPIRED_TO_IDLE = 6;
    const EXPIRED_TO_RUNNING = 7;
}

// ===========================================================================
// Door / access control enums (Clause 12.26, 12.33)
// ===========================================================================

bacnet_enum! {
    /// BACnet door alarm state (Clause 12.26).
    pub struct DoorAlarmState(u32);

    const NORMAL = 0;
    const ALARM = 1;
    const DOOR_OPEN_TOO_LONG = 2;
    const FORCED_OPEN = 3;
    const TAMPER = 4;
    const DOOR_FAULT = 5;
    const LOCK_FAULT = 6;
    const FREE_ACCESS = 7;
    const EGRESS_OPEN = 8;
}

bacnet_enum! {
    /// BACnet door status (Clause 12.26).
    pub struct DoorStatus(u32);

    const CLOSED = 0;
    const OPENED = 1;
    const UNKNOWN = 2;
}

bacnet_enum! {
    /// BACnet lock status (Clause 12.26).
    pub struct LockStatus(u32);

    const LOCKED = 0;
    const UNLOCKED = 1;
    const LOCK_FAULT = 2;
    const UNUSED = 3;
    const UNKNOWN = 4;
}

bacnet_enum! {
    /// BACnet secured status for Access Door (Clause 12.26).
    pub struct DoorSecuredStatus(u32);

    const SECURED = 0;
    const UNSECURED = 1;
    const UNKNOWN = 2;
}

bacnet_enum! {
    /// BACnet access event (Clause 12.33).
    pub struct AccessEvent(u32);

    const NONE = 0;
    const GRANTED = 1;
    const MUSTER = 2;
    const PASSBACK_DETECTED = 3;
    const DURESS = 4;
    const TRACE = 5;
    const LOCKOUT_MAX_ATTEMPTS = 6;
    const LOCKOUT_OTHER = 7;
    const LOCKOUT_RELINQUISHED = 8;
    const LOCKED_BY_HIGHER_PRIORITY = 9;
    const OUT_OF_SERVICE = 10;
    const OUT_OF_SERVICE_RELINQUISHED = 11;
    const ACCOMPANIMENT_BY = 12;
    const AUTHENTICATION_FACTOR_READ = 13;
    const AUTHORIZATION_DELAYED = 14;
    const VERIFICATION_REQUIRED = 15;
    const NO_ENTRY_AFTER_GRANTED = 16;
    // Denied events (128+)
    const DENIED_DENY_ALL = 128;
    const DENIED_UNKNOWN_CREDENTIAL = 129;
    const DENIED_AUTHENTICATION_UNAVAILABLE = 130;
    const DENIED_AUTHENTICATION_FACTOR_TIMEOUT = 131;
    const DENIED_INCORRECT_AUTHENTICATION_FACTOR = 132;
    const DENIED_ZONE_NO_ACCESS_RIGHTS = 133;
    const DENIED_POINT_NO_ACCESS_RIGHTS = 134;
    const DENIED_NO_ACCESS_RIGHTS = 135;
    const DENIED_OUT_OF_TIME_RANGE = 136;
    const DENIED_THREAT_LEVEL = 137;
    const DENIED_PASSBACK = 138;
    const DENIED_UNEXPECTED_LOCATION_USAGE = 139;
    const DENIED_MAX_ATTEMPTS = 140;
    const DENIED_LOWER_OCCUPANCY_LIMIT = 141;
    const DENIED_UPPER_OCCUPANCY_LIMIT = 142;
    const DENIED_AUTHENTICATION_FACTOR_LOST = 143;
    const DENIED_AUTHENTICATION_FACTOR_STOLEN = 144;
    const DENIED_AUTHENTICATION_FACTOR_DAMAGED = 145;
    const DENIED_AUTHENTICATION_FACTOR_DESTROYED = 146;
    const DENIED_AUTHENTICATION_FACTOR_DISABLED = 147;
    const DENIED_AUTHENTICATION_FACTOR_ERROR = 148;
    const DENIED_CREDENTIAL_UNASSIGNED = 149;
    const DENIED_CREDENTIAL_NOT_PROVISIONED = 150;
    const DENIED_CREDENTIAL_NOT_YET_ACTIVE = 151;
    const DENIED_CREDENTIAL_EXPIRED = 152;
    const DENIED_CREDENTIAL_MANUAL_DISABLE = 153;
    const DENIED_CREDENTIAL_LOCKOUT = 154;
    const DENIED_CREDENTIAL_MAX_DAYS = 155;
    const DENIED_CREDENTIAL_MAX_USES = 156;
    const DENIED_CREDENTIAL_INACTIVITY = 157;
    const DENIED_CREDENTIAL_DISABLED = 158;
    const DENIED_NO_ACCOMPANIMENT = 159;
    const DENIED_INCORRECT_ACCOMPANIMENT = 160;
    const DENIED_LOCKOUT = 161;
    const DENIED_VERIFICATION_FAILED = 162;
    const DENIED_VERIFICATION_TIMEOUT = 163;
    const DENIED_OTHER = 164;
}

bacnet_enum! {
    /// BACnet access credential disable (Clause 21).
    pub struct AccessCredentialDisable(u32);

    const NONE = 0;
    const DISABLE = 1;
    const DISABLE_MANUAL = 2;
    const DISABLE_LOCKOUT = 3;
}

bacnet_enum! {
    /// BACnet access credential disable reason (Clause 21).
    pub struct AccessCredentialDisableReason(u32);

    const DISABLED = 0;
    const DISABLED_NEEDS_PROVISIONING = 1;
    const DISABLED_UNASSIGNED = 2;
    const DISABLED_NOT_YET_ACTIVE = 3;
    const DISABLED_EXPIRED = 4;
    const DISABLED_LOCKOUT = 5;
    const DISABLED_MAX_DAYS = 6;
    const DISABLED_MAX_USES = 7;
    const DISABLED_INACTIVITY = 8;
    const DISABLED_MANUAL = 9;
}

bacnet_enum! {
    /// BACnet access user type (Clause 12.35).
    pub struct AccessUserType(u32);

    const ASSET = 0;
    const GROUP = 1;
    const PERSON = 2;
}

bacnet_enum! {
    /// BACnet authorization mode (Clause 12.31).
    pub struct AuthorizationMode(u32);

    const AUTHORIZE = 0;
    const GRANT_ACTIVE = 1;
    const DENY_ALL = 2;
    const VERIFICATION_REQUIRED = 3;
    const AUTHORIZATION_DELAYED = 4;
    const NONE = 5;
}

bacnet_enum! {
    /// BACnet access passback mode (Clause 12.32).
    pub struct AccessPassbackMode(u32);

    const PASSBACK_OFF = 0;
    const HARD_PASSBACK = 1;
    const SOFT_PASSBACK = 2;
}

// ===========================================================================
// Lighting enums (Clause 12.54)
// ===========================================================================

bacnet_enum! {
    /// BACnet lighting operation (Clause 12.54).
    pub struct LightingOperation(u32);

    const NONE = 0;
    const FADE_TO = 1;
    const RAMP_TO = 2;
    const STEP_UP = 3;
    const STEP_DOWN = 4;
    const STEP_ON = 5;
    const STEP_OFF = 6;
    const WARN = 7;
    const WARN_OFF = 8;
    const WARN_RELINQUISH = 9;
    const STOP = 10;
}

bacnet_enum! {
    /// BACnet lighting in-progress state (Clause 12.54).
    pub struct LightingInProgress(u32);

    const IDLE = 0;
    const FADE_ACTIVE = 1;
    const RAMP_ACTIVE = 2;
    const NOT_CONTROLLED = 3;
    const OTHER = 4;
}

// ===========================================================================
// Lift / escalator enums (Clause 12.58-12.60)
// ===========================================================================

bacnet_enum! {
    /// BACnet escalator operating mode (Clause 12.60).
    pub struct EscalatorMode(u32);

    const UNKNOWN = 0;
    const STOP = 1;
    const UP = 2;
    const DOWN = 3;
    const INSPECTION = 4;
    const OUT_OF_SERVICE = 5;
}

bacnet_enum! {
    /// BACnet escalator fault signals (Clause 12.60).
    pub struct EscalatorFault(u32);

    const CONTROLLER_FAULT = 0;
    const DRIVE_AND_MOTOR_FAULT = 1;
    const MECHANICAL_COMPONENT_FAULT = 2;
    const OVERSPEED_FAULT = 3;
    const POWER_SUPPLY_FAULT = 4;
    const SAFETY_DEVICE_FAULT = 5;
    const CONTROLLER_SUPPLY_FAULT = 6;
    const DRIVE_TEMPERATURE_EXCEEDED = 7;
    const COMB_PLATE_FAULT = 8;
}

bacnet_enum! {
    /// BACnet lift car travel direction (Clause 12.59).
    pub struct LiftCarDirection(u32);

    const UNKNOWN = 0;
    const NONE = 1;
    const STOPPED = 2;
    const UP = 3;
    const DOWN = 4;
    const UP_AND_DOWN = 5;
}

bacnet_enum! {
    /// BACnet lift group operating mode (Clause 12.58).
    pub struct LiftGroupMode(u32);

    const UNKNOWN = 0;
    const NORMAL = 1;
    const DOWN_PEAK = 2;
    const TWO_WAY = 3;
    const FOUR_WAY = 4;
    const EMERGENCY_POWER = 5;
    const UP_PEAK = 6;
}

bacnet_enum! {
    /// BACnet lift car door status (Clause 12.59).
    pub struct LiftCarDoorStatus(u32);

    const UNKNOWN = 0;
    const NONE = 1;
    const CLOSING = 2;
    const CLOSED = 3;
    const OPENING = 4;
    const OPENED = 5;
    const SAFETY_LOCKED = 6;
    const LIMITED_OPENED = 7;
}

bacnet_enum! {
    /// BACnet lift car door command (Clause 21).
    pub struct LiftCarDoorCommand(u32);

    const NONE = 0;
    const OPEN = 1;
    const CLOSE = 2;
}

bacnet_enum! {
    /// BACnet lift car drive status (Clause 21).
    pub struct LiftCarDriveStatus(u32);

    const UNKNOWN = 0;
    const STATIONARY = 1;
    const BRAKING = 2;
    const ACCELERATE = 3;
    const DECELERATE = 4;
    const RATED_SPEED = 5;
    const SINGLE_FLOOR_JUMP = 6;
    const TWO_FLOOR_JUMP = 7;
    const THREE_FLOOR_JUMP = 8;
    const MULTI_FLOOR_JUMP = 9;
}

bacnet_enum! {
    /// BACnet lift car operating mode (Clause 21).
    pub struct LiftCarMode(u32);

    const UNKNOWN = 0;
    const NORMAL = 1;
    const VIP = 2;
    const HOMING = 3;
    const PARKING = 4;
    const ATTENDANT_CONTROL = 5;
    const FIREFIGHTER_CONTROL = 6;
    const EMERGENCY_POWER = 7;
    const INSPECTION = 8;
    const CABINET_RECALL = 9;
    const EARTHQUAKE_OPERATION = 10;
    const FIRE_OPERATION = 11;
    const OUT_OF_SERVICE = 12;
    const OCCUPANT_EVACUATION = 13;
}

bacnet_enum! {
    /// BACnet lift fault signals (Clause 21).
    pub struct LiftFault(u32);

    const CONTROLLER_FAULT = 0;
    const DRIVE_AND_MOTOR_FAULT = 1;
    const GOVERNOR_AND_SAFETY_GEAR_FAULT = 2;
    const LIFT_SHAFT_DEVICE_FAULT = 3;
    const POWER_SUPPLY_FAULT = 4;
    const SAFETY_INTERLOCK_FAULT = 5;
    const DOOR_CLOSING_FAULT = 6;
    const DOOR_OPENING_FAULT = 7;
    const CAR_STOPPED_OUTSIDE_LANDING_ZONE = 8;
    const CALL_BUTTON_STUCK = 9;
    const START_FAILURE = 10;
    const CONTROLLER_SUPPLY_FAULT = 11;
    const SELF_TEST_FAILURE = 12;
    const RUNTIME_LIMIT_EXCEEDED = 13;
    const POSITION_LOST = 14;
    const DRIVE_TEMPERATURE_EXCEEDED = 15;
    const LOAD_MEASUREMENT_FAULT = 16;
}

// ===========================================================================
// Miscellaneous enums
// ===========================================================================

bacnet_enum! {
    /// BACnet load control shed state (Clause 12.28).
    pub struct ShedState(u32);

    const SHED_INACTIVE = 0;
    const SHED_REQUEST_PENDING = 1;
    const SHED_COMPLIANT = 2;
    const SHED_NON_COMPLIANT = 3;
}

bacnet_enum! {
    /// BACnet node type for Structured View (Clause 12.29).
    pub struct NodeType(u32);

    const UNKNOWN = 0;
    const SYSTEM = 1;
    const NETWORK = 2;
    const DEVICE = 3;
    const ORGANIZATIONAL = 4;
    const AREA = 5;
    const EQUIPMENT = 6;
    const POINT = 7;
    const COLLECTION = 8;
    const PROPERTY = 9;
    const FUNCTIONAL = 10;
    const OTHER = 11;
    const SUBSYSTEM = 12;
    const BUILDING = 13;
    const FLOOR = 14;
    const SECTION = 15;
    const MODULE = 16;
    const TREE = 17;
    const MEMBER = 18;
    const PROTOCOL = 19;
    const ROOM = 20;
    const ZONE = 21;
}

bacnet_enum! {
    /// BACnet acknowledgment filter for GetEnrollmentSummary (Clause 13.7.1).
    pub struct AcknowledgmentFilter(u32);

    const ALL = 0;
    const ACKED = 1;
    const NOT_ACKED = 2;
}

bacnet_enum! {
    /// Event transition bit positions (Clause 12.11).
    pub struct EventTransitionBits(u8);

    const TO_OFFNORMAL = 0;
    const TO_FAULT = 1;
    const TO_NORMAL = 2;
}

bacnet_enum! {
    /// BACnet message priority for TextMessage services (Clause 16.5).
    pub struct MessagePriority(u32);

    const NORMAL = 0;
    const URGENT = 1;
}

bacnet_enum! {
    /// BACnet virtual terminal class (Clause 17.1).
    pub struct VTClass(u32);

    const DEFAULT_TERMINAL = 0;
    const ANSI_X3_64 = 1;
    const DEC_VT52 = 2;
    const DEC_VT100 = 3;
    const DEC_VT220 = 4;
    const HP_700_94 = 5;
    const IBM_3130 = 6;
}

// ===========================================================================
// Staging / Audit enums (new in 135-2020)
// ===========================================================================

bacnet_enum! {
    /// BACnet staging state (Clause 12.62, new in 135-2020).
    pub struct StagingState(u32);

    const NOT_STAGED = 0;
    const STAGING = 1;
    const STAGED = 2;
    const COMMITTING = 3;
    const COMMITTED = 4;
    const ABANDONING = 5;
    const ABANDONED = 6;
}

bacnet_enum! {
    /// BACnet audit level (Clause 19.6, new in 135-2020).
    pub struct AuditLevel(u32);

    const NONE = 0;
    const AUDIT_ALL = 1;
    const AUDIT_CONFIG = 2;
    const DEFAULT = 3;
}

bacnet_enum! {
    /// BACnet audit operation (Clause 19.6, new in 135-2020).
    pub struct AuditOperation(u32);

    const READ = 0;
    const WRITE = 1;
    const CREATE = 2;
    const DELETE = 3;
    const LIFE_SAFETY = 4;
    const ACKNOWLEDGE_ALARM = 5;
    const DEVICE_DISABLE_COMM = 6;
    const DEVICE_ENABLE_COMM = 7;
    const DEVICE_RESET = 8;
    const DEVICE_BACKUP = 9;
    const DEVICE_RESTORE = 10;
    const SUBSCRIPTION = 11;
    const NOTIFICATION = 12;
    const AUDITING_FAILURE = 13;
    const NETWORK_CHANGES = 14;
    const GENERAL = 15;
}

bacnet_enum! {
    /// BACnet success filter for audit log queries (Clause 13.19, new in 135-2020).
    pub struct BACnetSuccessFilter(u32);

    const ALL = 0;
    const SUCCESSES_ONLY = 1;
    const FAILURES_ONLY = 2;
}

// ===========================================================================
// EngineeringUnits (Clause 21) — large enum, grouped by category
// ===========================================================================

bacnet_enum! {
    /// BACnet engineering units (Clause 21).
    ///
    /// Values 0-255 and 47808-49999 are reserved for ASHRAE;
    /// 256-47807 and 50000-65535 may be used by vendors (Clause 23).
    pub struct EngineeringUnits(u32);

    // Acceleration
    const METERS_PER_SECOND_PER_SECOND = 166;
    // Area
    const SQUARE_METERS = 0;
    const SQUARE_CENTIMETERS = 116;
    const SQUARE_FEET = 1;
    const SQUARE_INCHES = 115;
    // Currency
    const CURRENCY1 = 105;
    const CURRENCY2 = 106;
    const CURRENCY3 = 107;
    const CURRENCY4 = 108;
    const CURRENCY5 = 109;
    const CURRENCY6 = 110;
    const CURRENCY7 = 111;
    const CURRENCY8 = 112;
    const CURRENCY9 = 113;
    const CURRENCY10 = 114;
    // Electrical
    const MILLIAMPERES = 2;
    const AMPERES = 3;
    const AMPERES_PER_METER = 167;
    const AMPERES_PER_SQUARE_METER = 168;
    const AMPERE_SQUARE_METERS = 169;
    const DECIBELS = 199;
    const DECIBELS_MILLIVOLT = 200;
    const DECIBELS_VOLT = 201;
    const FARADS = 170;
    const HENRYS = 171;
    const OHMS = 4;
    const OHM_METER_SQUARED_PER_METER = 237;
    const OHM_METERS = 172;
    const MILLIOHMS = 145;
    const KILOHMS = 122;
    const MEGOHMS = 123;
    const MICROSIEMENS = 190;
    const MILLISIEMENS = 202;
    const SIEMENS = 173;
    const SIEMENS_PER_METER = 174;
    const TESLAS = 175;
    const VOLTS = 5;
    const MILLIVOLTS = 124;
    const KILOVOLTS = 6;
    const MEGAVOLTS = 7;
    const VOLT_AMPERES = 8;
    const KILOVOLT_AMPERES = 9;
    const MEGAVOLT_AMPERES = 10;
    const VOLT_AMPERES_REACTIVE = 11;
    const KILOVOLT_AMPERES_REACTIVE = 12;
    const MEGAVOLT_AMPERES_REACTIVE = 13;
    const VOLTS_PER_DEGREE_KELVIN = 176;
    const VOLTS_PER_METER = 177;
    const DEGREES_PHASE = 14;
    const POWER_FACTOR = 15;
    const WEBERS = 178;
    // Energy
    const AMPERE_SECONDS = 238;
    const VOLT_AMPERE_HOURS = 239;
    const KILOVOLT_AMPERE_HOURS = 240;
    const MEGAVOLT_AMPERE_HOURS = 241;
    const VOLT_AMPERE_HOURS_REACTIVE = 242;
    const KILOVOLT_AMPERE_HOURS_REACTIVE = 243;
    const MEGAVOLT_AMPERE_HOURS_REACTIVE = 244;
    const VOLT_SQUARE_HOURS = 245;
    const AMPERE_SQUARE_HOURS = 246;
    const JOULES = 16;
    const KILOJOULES = 17;
    const KILOJOULES_PER_KILOGRAM = 125;
    const MEGAJOULES = 126;
    const WATT_HOURS = 18;
    const KILOWATT_HOURS = 19;
    const MEGAWATT_HOURS = 146;
    const WATT_HOURS_REACTIVE = 203;
    const KILOWATT_HOURS_REACTIVE = 204;
    const MEGAWATT_HOURS_REACTIVE = 205;
    const BTUS = 20;
    const KILO_BTUS = 147;
    const MEGA_BTUS = 148;
    const THERMS = 21;
    const TON_HOURS = 22;
    // Enthalpy
    const JOULES_PER_KILOGRAM_DRY_AIR = 23;
    const KILOJOULES_PER_KILOGRAM_DRY_AIR = 149;
    const MEGAJOULES_PER_KILOGRAM_DRY_AIR = 150;
    const BTUS_PER_POUND_DRY_AIR = 24;
    const BTUS_PER_POUND = 117;
    // Entropy
    const JOULES_PER_DEGREE_KELVIN = 127;
    const KILOJOULES_PER_DEGREE_KELVIN = 151;
    const MEGAJOULES_PER_DEGREE_KELVIN = 152;
    const JOULES_PER_KILOGRAM_DEGREE_KELVIN = 128;
    // Force
    const NEWTON = 153;
    // Frequency
    const CYCLES_PER_HOUR = 25;
    const CYCLES_PER_MINUTE = 26;
    const HERTZ = 27;
    const KILOHERTZ = 129;
    const MEGAHERTZ = 130;
    const PER_HOUR = 131;
    // Humidity
    const GRAMS_OF_WATER_PER_KILOGRAM_DRY_AIR = 28;
    const PERCENT_RELATIVE_HUMIDITY = 29;
    // Length
    const MICROMETERS = 194;
    const MILLIMETERS = 30;
    const CENTIMETERS = 118;
    const KILOMETERS = 193;
    const METERS = 31;
    const INCHES = 32;
    const FEET = 33;
    // Light
    const CANDELAS = 179;
    const CANDELAS_PER_SQUARE_METER = 180;
    const WATTS_PER_SQUARE_FOOT = 34;
    const WATTS_PER_SQUARE_METER = 35;
    const LUMENS = 36;
    const LUXES = 37;
    const FOOT_CANDLES = 38;
    // Mass
    const MILLIGRAMS = 196;
    const GRAMS = 195;
    const KILOGRAMS = 39;
    const POUNDS_MASS = 40;
    const TONS = 41;
    // Mass flow
    const GRAMS_PER_SECOND = 154;
    const GRAMS_PER_MINUTE = 155;
    const KILOGRAMS_PER_SECOND = 42;
    const KILOGRAMS_PER_MINUTE = 43;
    const KILOGRAMS_PER_HOUR = 44;
    const POUNDS_MASS_PER_SECOND = 119;
    const POUNDS_MASS_PER_MINUTE = 45;
    const POUNDS_MASS_PER_HOUR = 46;
    const TONS_PER_HOUR = 156;
    // Power
    const MILLIWATTS = 132;
    const WATTS = 47;
    const KILOWATTS = 48;
    const MEGAWATTS = 49;
    const BTUS_PER_HOUR = 50;
    const KILO_BTUS_PER_HOUR = 157;
    const JOULE_PER_HOURS = 247;
    const HORSEPOWER = 51;
    const TONS_REFRIGERATION = 52;
    // Pressure
    const PASCALS = 53;
    const HECTOPASCALS = 133;
    const KILOPASCALS = 54;
    const MILLIBARS = 134;
    const BARS = 55;
    const POUNDS_FORCE_PER_SQUARE_INCH = 56;
    const MILLIMETERS_OF_WATER = 206;
    const CENTIMETERS_OF_WATER = 57;
    const INCHES_OF_WATER = 58;
    const MILLIMETERS_OF_MERCURY = 59;
    const CENTIMETERS_OF_MERCURY = 60;
    const INCHES_OF_MERCURY = 61;
    // Temperature
    const DEGREES_CELSIUS = 62;
    const DEGREES_KELVIN = 63;
    const DEGREES_KELVIN_PER_HOUR = 181;
    const DEGREES_KELVIN_PER_MINUTE = 182;
    const DEGREES_FAHRENHEIT = 64;
    const DEGREE_DAYS_CELSIUS = 65;
    const DEGREE_DAYS_FAHRENHEIT = 66;
    const DELTA_DEGREES_FAHRENHEIT = 120;
    const DELTA_DEGREES_KELVIN = 121;
    // Time
    const YEARS = 67;
    const MONTHS = 68;
    const WEEKS = 69;
    const DAYS = 70;
    const HOURS = 71;
    const MINUTES = 72;
    const SECONDS = 73;
    const HUNDREDTHS_SECONDS = 158;
    const MILLISECONDS = 159;
    // Torque
    const NEWTON_METERS = 160;
    // Velocity
    const MILLIMETERS_PER_SECOND = 161;
    const MILLIMETERS_PER_MINUTE = 162;
    const METERS_PER_SECOND = 74;
    const METERS_PER_MINUTE = 163;
    const METERS_PER_HOUR = 164;
    const KILOMETERS_PER_HOUR = 75;
    const FEET_PER_SECOND = 76;
    const FEET_PER_MINUTE = 77;
    const MILES_PER_HOUR = 78;
    // Volume
    const CUBIC_FEET = 79;
    const CUBIC_METERS = 80;
    const IMPERIAL_GALLONS = 81;
    const MILLILITERS = 197;
    const LITERS = 82;
    const US_GALLONS = 83;
    // Volumetric flow
    const CUBIC_FEET_PER_SECOND = 142;
    const CUBIC_FEET_PER_MINUTE = 84;
    const MILLION_STANDARD_CUBIC_FEET_PER_MINUTE = 254;
    const CUBIC_FEET_PER_HOUR = 191;
    const CUBIC_FEET_PER_DAY = 248;
    const STANDARD_CUBIC_FEET_PER_DAY = 47808;
    const MILLION_STANDARD_CUBIC_FEET_PER_DAY = 47809;
    const THOUSAND_CUBIC_FEET_PER_DAY = 47810;
    const THOUSAND_STANDARD_CUBIC_FEET_PER_DAY = 47811;
    const POUNDS_MASS_PER_DAY = 47812;
    const CUBIC_METERS_PER_SECOND = 85;
    const CUBIC_METERS_PER_MINUTE = 165;
    const CUBIC_METERS_PER_HOUR = 135;
    const CUBIC_METERS_PER_DAY = 249;
    const IMPERIAL_GALLONS_PER_MINUTE = 86;
    const MILLILITERS_PER_SECOND = 198;
    const LITERS_PER_SECOND = 87;
    const LITERS_PER_MINUTE = 88;
    const LITERS_PER_HOUR = 136;
    const US_GALLONS_PER_MINUTE = 89;
    const US_GALLONS_PER_HOUR = 192;
    // Other
    const DEGREES_ANGULAR = 90;
    const DEGREES_CELSIUS_PER_HOUR = 91;
    const DEGREES_CELSIUS_PER_MINUTE = 92;
    const DEGREES_FAHRENHEIT_PER_HOUR = 93;
    const DEGREES_FAHRENHEIT_PER_MINUTE = 94;
    const JOULE_SECONDS = 183;
    const KILOGRAMS_PER_CUBIC_METER = 186;
    const KILOWATT_HOURS_PER_SQUARE_METER = 137;
    const KILOWATT_HOURS_PER_SQUARE_FOOT = 138;
    const WATT_HOURS_PER_CUBIC_METER = 250;
    const JOULES_PER_CUBIC_METER = 251;
    const MEGAJOULES_PER_SQUARE_METER = 139;
    const MEGAJOULES_PER_SQUARE_FOOT = 140;
    const MOLE_PERCENT = 252;
    const NO_UNITS = 95;
    const NEWTON_SECONDS = 187;
    const NEWTONS_PER_METER = 188;
    const PARTS_PER_MILLION = 96;
    const PARTS_PER_BILLION = 97;
    const PASCAL_SECONDS = 253;
    const PERCENT = 98;
    const PERCENT_OBSCURATION_PER_FOOT = 143;
    const PERCENT_OBSCURATION_PER_METER = 144;
    const PERCENT_PER_SECOND = 99;
    const PER_MINUTE = 100;
    const PER_SECOND = 101;
    const PSI_PER_DEGREE_FAHRENHEIT = 102;
    const RADIANS = 103;
    const RADIANS_PER_SECOND = 184;
    const REVOLUTIONS_PER_MINUTE = 104;
    const SQUARE_METERS_PER_NEWTON = 185;
    const WATTS_PER_METER_PER_DEGREE_KELVIN = 189;
    const WATTS_PER_SQUARE_METER_DEGREE_KELVIN = 141;
    const PER_MILLE = 207;
    const GRAMS_PER_GRAM = 208;
    const KILOGRAMS_PER_KILOGRAM = 209;
    const GRAMS_PER_KILOGRAM = 210;
    const MILLIGRAMS_PER_GRAM = 211;
    const MILLIGRAMS_PER_KILOGRAM = 212;
    const GRAMS_PER_MILLILITER = 213;
    const GRAMS_PER_LITER = 214;
    const MILLIGRAMS_PER_LITER = 215;
    const MICROGRAMS_PER_LITER = 216;
    const GRAMS_PER_CUBIC_METER = 217;
    const MILLIGRAMS_PER_CUBIC_METER = 218;
    const MICROGRAMS_PER_CUBIC_METER = 219;
    const NANOGRAMS_PER_CUBIC_METER = 220;
    const GRAMS_PER_CUBIC_CENTIMETER = 221;
    const BECQUERELS = 222;
    const KILOBECQUERELS = 223;
    const MEGABECQUERELS = 224;
    const GRAY = 225;
    const MILLIGRAY = 226;
    const MICROGRAY = 227;
    const SIEVERTS = 228;
    const MILLISIEVERTS = 229;
    const MICROSIEVERTS = 230;
    const MICROSIEVERTS_PER_HOUR = 231;
    const MILLIREMS = 47814;
    const MILLIREMS_PER_HOUR = 47815;
    const DECIBELS_A = 232;
    const NEPHELOMETRIC_TURBIDITY_UNIT = 233;
    const PH = 234;
    const GRAMS_PER_SQUARE_METER = 235;
    const MINUTES_PER_DEGREE_KELVIN = 236;
    const DEGREES_LOVIBOND = 47816;
    const ALCOHOL_BY_VOLUME = 47817;
    const INTERNATIONAL_BITTERING_UNITS = 47818;
    const EUROPEAN_BITTERNESS_UNITS = 47819;
    const DEGREES_PLATO = 47820;
    const SPECIFIC_GRAVITY = 47821;
    const EUROPEAN_BREWING_CONVENTION = 47822;
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_type_round_trip() {
        assert_eq!(ObjectType::DEVICE.to_raw(), 8);
        assert_eq!(ObjectType::from_raw(8), ObjectType::DEVICE);
    }

    #[test]
    fn object_type_vendor_proprietary() {
        let vendor = ObjectType::from_raw(128);
        assert_eq!(vendor.to_raw(), 128);
        assert_eq!(format!("{}", vendor), "128");
        assert_eq!(format!("{:?}", vendor), "ObjectType(128)");
    }

    #[test]
    fn object_type_display_known() {
        assert_eq!(format!("{}", ObjectType::ANALOG_INPUT), "ANALOG_INPUT");
        assert_eq!(format!("{:?}", ObjectType::DEVICE), "ObjectType::DEVICE");
    }

    #[test]
    fn property_identifier_round_trip() {
        assert_eq!(PropertyIdentifier::PRESENT_VALUE.to_raw(), 85);
        assert_eq!(
            PropertyIdentifier::from_raw(85),
            PropertyIdentifier::PRESENT_VALUE
        );
    }

    #[test]
    fn property_identifier_vendor() {
        let vendor = PropertyIdentifier::from_raw(512);
        assert_eq!(vendor.to_raw(), 512);
    }

    #[test]
    fn pdu_type_values() {
        assert_eq!(PduType::CONFIRMED_REQUEST.to_raw(), 0);
        assert_eq!(PduType::ABORT.to_raw(), 7);
    }

    #[test]
    fn confirmed_service_choice_values() {
        assert_eq!(ConfirmedServiceChoice::READ_PROPERTY.to_raw(), 12);
        assert_eq!(ConfirmedServiceChoice::WRITE_PROPERTY.to_raw(), 15);
    }

    #[test]
    fn unconfirmed_service_choice_values() {
        assert_eq!(UnconfirmedServiceChoice::WHO_IS.to_raw(), 8);
        assert_eq!(UnconfirmedServiceChoice::I_AM.to_raw(), 0);
    }

    #[test]
    fn bvlc_function_values() {
        assert_eq!(BvlcFunction::ORIGINAL_UNICAST_NPDU.to_raw(), 0x0A);
        assert_eq!(BvlcFunction::ORIGINAL_BROADCAST_NPDU.to_raw(), 0x0B);
    }

    #[test]
    fn engineering_units_round_trip() {
        assert_eq!(EngineeringUnits::DEGREES_CELSIUS.to_raw(), 62);
        assert_eq!(
            EngineeringUnits::from_raw(62),
            EngineeringUnits::DEGREES_CELSIUS
        );
    }

    #[test]
    fn engineering_units_ashrae_extended() {
        assert_eq!(
            EngineeringUnits::STANDARD_CUBIC_FEET_PER_DAY.to_raw(),
            47808
        );
    }

    #[test]
    fn segmentation_values() {
        assert_eq!(Segmentation::BOTH.to_raw(), 0);
        assert_eq!(Segmentation::NONE.to_raw(), 3);
    }

    #[test]
    fn network_message_type_values() {
        assert_eq!(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK.to_raw(), 0x00);
        assert_eq!(NetworkMessageType::NETWORK_NUMBER_IS.to_raw(), 0x13);
    }

    #[test]
    fn event_state_values() {
        assert_eq!(EventState::NORMAL.to_raw(), 0);
        assert_eq!(EventState::LIFE_SAFETY_ALARM.to_raw(), 5);
    }

    #[test]
    fn reliability_gap_at_11() {
        // Value 11 is intentionally missing from the standard
        assert_eq!(Reliability::CONFIGURATION_ERROR.to_raw(), 10);
        assert_eq!(Reliability::COMMUNICATION_FAILURE.to_raw(), 12);
    }
}
