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
