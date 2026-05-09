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
