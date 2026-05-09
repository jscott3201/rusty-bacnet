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
