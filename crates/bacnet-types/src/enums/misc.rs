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
