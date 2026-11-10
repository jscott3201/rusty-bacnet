// ===========================================================================
// Network port enums (Clause 12.56)
// ===========================================================================

bacnet_enum! {
    /// BACnet data link/network type (Clause 12.56 (Network_Type)).
    pub struct NetworkType(u32);

    const ETHERNET = 0;
    const ARCNET = 1;
    const MSTP = 2;
    const PTP = 3;
    const LONTALK = 4;
    const IPV4 = 5;
    const ZIGBEE = 6;
    const VIRTUAL = 7;
    /// Dropped from the standard in protocol revision 18 (Clause 21); named so
    /// an older device's value still decodes.
    const NON_BACNET = 8;
    const IPV6 = 9;
    const SERIAL = 10;
    /// BACnet/SC. Added after revision 22 (135-2020); ASHRAE's revision 24
    /// data types name it.
    const SECURE_CONNECT = 11;
}

bacnet_enum! {
    /// IP addressing mode for a NetworkPort (Clause 12.56).
    pub struct IPMode(u32);

    const NORMAL = 0;
    const FOREIGN = 1;
    const BBMD = 2;
}

bacnet_enum! {
    /// Commands for a NetworkPort object (Clause 12.56 (Command)).
    pub struct NetworkPortCommand(u32);

    const IDLE = 0;
    const DISCARD_CHANGES = 1;
    const RENEW_FD_REGISTRATION = 2;
    const RESTART_SLAVE_DISCOVERY = 3;
    const RENEW_DHCP = 4;
    const RESTART_AUTONEG = 5;
    const DISCONNECT = 6;
    const RESTART_PORT = 7;
    // 8-9: added after revision 22 (135-2020); ASHRAE's revision 24 data
    // types name them.
    const GENERATE_CSR_FILE = 8;
    const VALIDATE_CHANGES = 9;
}

bacnet_enum! {
    /// Quality of a NetworkPort's network number (Clause 12.56 (Network_Number_Quality)).
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
