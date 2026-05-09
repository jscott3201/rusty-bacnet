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
    /// New in 135-2020 Addendum bj (Color objects).
    const COLOR_COMMAND = 508;
    /// New in 135-2020 Addendum bj (Color Temperature objects).
    const DEFAULT_COLOR_TEMPERATURE = 509;
    /// New in 135-2020 Addendum bj (Color objects).
    const DEFAULT_COLOR = 510;
}
