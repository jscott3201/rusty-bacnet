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
    /// Bit positions within a `BACnetServicesSupported` bit string (Clause 21).
    pub struct ServiceSupported(u8);

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
    // 13: readPropertyConditional (removed)
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
    // 24: authenticate (removed), 25: requestKey (removed)
    const I_AM = 26;
    const I_HAVE = 27;
    const UNCONFIRMED_COV_NOTIFICATION = 28;
    const UNCONFIRMED_EVENT_NOTIFICATION = 29;
    const UNCONFIRMED_PRIVATE_TRANSFER = 30;
    const UNCONFIRMED_TEXT_MESSAGE = 31;
    const TIME_SYNCHRONIZATION = 32;
    const WHO_HAS = 33;
    const WHO_IS = 34;
    const READ_RANGE = 35;
    const UTC_TIME_SYNCHRONIZATION = 36;
    const LIFE_SAFETY_OPERATION = 37;
    const SUBSCRIBE_COV_PROPERTY = 38;
    const GET_EVENT_INFORMATION = 39;
    const WRITE_GROUP = 40;
    const SUBSCRIBE_COV_PROPERTY_MULTIPLE = 41;
    const CONFIRMED_COV_NOTIFICATION_MULTIPLE = 42;
    const UNCONFIRMED_COV_NOTIFICATION_MULTIPLE = 43;
    const CONFIRMED_AUDIT_NOTIFICATION = 44;
    const AUDIT_LOG_QUERY = 45;
    const UNCONFIRMED_AUDIT_NOTIFICATION = 46;
    const WHO_AM_I = 47;
    const YOU_ARE = 48;
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
