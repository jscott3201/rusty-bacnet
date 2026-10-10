//! Clause 9.6 CRC implementations.

const CRC8_TABLE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u8;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x81
            } else {
                crc >> 1
            };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

const CRC16_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u16;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// Compute the reflected BACnet header CRC-8.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0xFF;
    for &byte in data {
        crc = CRC8_TABLE[(crc ^ byte) as usize];
    }
    !crc
}

/// Compute the running header CRC including its transmitted CRC octet.
pub fn crc8_accumulate_all(data: &[u8]) -> u8 {
    let mut crc = 0xFF;
    for &byte in data {
        crc = CRC8_TABLE[(crc ^ byte) as usize];
    }
    crc
}

/// Verify a header CRC whose final byte is the transmitted CRC.
pub fn crc8_valid(data_with_crc: &[u8]) -> bool {
    !data_with_crc.is_empty()
        && crc8(&data_with_crc[..data_with_crc.len() - 1]) == data_with_crc[data_with_crc.len() - 1]
}

/// Compute the reflected BACnet data CRC-16.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFF;
    for &byte in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    !crc
}

/// Compute the running data CRC including its transmitted CRC octets.
pub fn crc16_accumulate_all(data: &[u8]) -> u16 {
    let mut crc = 0xFFFF;
    for &byte in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    crc
}

/// Verify a data CRC whose final two bytes are little-endian on the wire.
pub fn crc16_valid(data_with_crc: &[u8]) -> bool {
    if data_with_crc.len() < 3 {
        return false;
    }
    let split = data_with_crc.len() - 2;
    let stored = u16::from_le_bytes([data_with_crc[split], data_with_crc[split + 1]]);
    crc16(&data_with_crc[..split]) == stored
}
