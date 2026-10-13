//! The CRCs of Clause 9.6: the header CRC-8, the data CRC-16, and the
//! CRC-32K of extended frames.
//!
//! The codec computes and checks them itself when it encodes and decodes;
//! they are public for a caller that builds or inspects frames by hand.
//! Each function returns the complemented register, the value that goes on
//! the wire. The register itself, which the worked examples of Annex G give
//! octet by octet, is its bitwise complement.

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
#[must_use]
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0xFF;
    for &byte in data {
        crc = CRC8_TABLE[(crc ^ byte) as usize];
    }
    !crc
}

/// Verify a header CRC whose final byte is the transmitted CRC.
///
/// Any non-empty input is checked, so a single octet is compared with the
/// CRC of nothing. Unlike [`crc16_valid`], this does not demand content: the
/// codec only calls it on a whole header, which always has its five fields.
#[must_use]
pub fn crc8_valid(data_with_crc: &[u8]) -> bool {
    !data_with_crc.is_empty()
        && crc8(&data_with_crc[..data_with_crc.len() - 1]) == data_with_crc[data_with_crc.len() - 1]
}

/// Compute the reflected BACnet data CRC-16.
#[must_use]
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFF;
    for &byte in data {
        crc = (crc >> 8) ^ CRC16_TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    !crc
}

/// Verify a data CRC whose final two bytes are little-endian on the wire.
///
/// Needs at least one data octet before the two CRC octets, and returns
/// `false` for anything shorter: a frame sends a data CRC only after a
/// non-empty data field, so two octets alone are never a valid data field.
#[must_use]
pub fn crc16_valid(data_with_crc: &[u8]) -> bool {
    if data_with_crc.len() < 3 {
        return false;
    }
    let split = data_with_crc.len() - 2;
    let stored = u16::from_le_bytes([data_with_crc[split], data_with_crc[split + 1]]);
    crc16(&data_with_crc[..split]) == stored
}

/// The CRC-32K (Koopman) generator of Clause 9.6.3, bit-reversed for a
/// register that shifts right, least significant bit first (Annex G.3).
const CRC32K_POLYNOMIAL: u32 = 0xEB31_D82E;

/// The register after `bits` shifts of `crc` with no new input.
const fn crc32k_shift(mut crc: u32, bits: u32) -> u32 {
    let mut bit = 0;
    while bit < bits {
        crc = if crc & 1 != 0 {
            (crc >> 1) ^ CRC32K_POLYNOMIAL
        } else {
            crc >> 1
        };
        bit += 1;
    }
    crc
}

/// One entry per octet value: 1 KiB, one lookup per octet. Unit tests build
/// both tables, whichever the features pick.
#[cfg(any(feature = "crc32k-table", test))]
const CRC32K_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        table[i] = crc32k_shift(i as u32, 8);
        i += 1;
    }
    table
};

/// One entry per nibble value: 64 octets, two lookups per octet.
#[cfg(any(not(feature = "crc32k-table"), test))]
const CRC32K_NIBBLES: [u32; 16] = {
    let mut table = [0u32; 16];
    let mut i = 0;
    while i < 16 {
        table[i] = crc32k_shift(i as u32, 4);
        i += 1;
    }
    table
};

/// Accumulate one octet into the register with the 256-entry table.
#[cfg(any(feature = "crc32k-table", test))]
fn crc32k_octet_by_table(crc: u32, byte: u8) -> u32 {
    (crc >> 8) ^ CRC32K_TABLE[((crc ^ u32::from(byte)) & 0xFF) as usize]
}

/// Accumulate one octet into the register with the 16-entry table.
#[cfg(any(not(feature = "crc32k-table"), test))]
fn crc32k_octet_by_nibbles(crc: u32, byte: u8) -> u32 {
    let crc = crc ^ u32::from(byte);
    let crc = (crc >> 4) ^ CRC32K_NIBBLES[(crc & 0x0F) as usize];
    (crc >> 4) ^ CRC32K_NIBBLES[(crc & 0x0F) as usize]
}

/// Accumulate `data` into the CRC-32K register `crc`.
pub(crate) fn crc32k_update(crc: u32, data: &[u8]) -> u32 {
    #[cfg(feature = "crc32k-table")]
    let octet = crc32k_octet_by_table;
    #[cfg(not(feature = "crc32k-table"))]
    let octet = crc32k_octet_by_nibbles;
    data.iter().fold(crc, |crc, &byte| octet(crc, byte))
}

/// Compute the CRC-32K of an extended frame's Encoded Data field: the
/// complement of the register, which goes out low octet first, COBS-encoded
/// as the Encoded CRC-32K field (Clauses 9.6.3, 9.10.2).
///
/// By default the codec works it out with a 16-entry table, two lookups per
/// octet. The `crc32k-table` feature uses a 256-entry table instead, one
/// lookup per octet, for about 1 KiB more flash. The results are the same.
#[must_use]
pub fn crc32k(data: &[u8]) -> u32 {
    !crc32k_update(u32::MAX, data)
}

#[cfg(test)]
mod tests {
    use super::{crc32k_octet_by_nibbles, crc32k_octet_by_table, crc32k_shift};

    /// Both tables against eight plain shifts, for every octet value from
    /// several register states, so the default test run covers the table the
    /// `crc32k-table` feature selects as well as the default one.
    #[test]
    fn both_crc32k_tables_match_the_bitwise_register() {
        let registers = [
            0,
            u32::MAX,
            1,
            0x8000_0000,
            0x0843_323B,
            0x1234_5678,
            0xEDCB_A987,
            0x5555_AAAA,
        ];
        for register in registers {
            for byte in 0..=u8::MAX {
                let bitwise = crc32k_shift(register ^ u32::from(byte), 8);
                assert_eq!(crc32k_octet_by_table(register, byte), bitwise);
                assert_eq!(crc32k_octet_by_nibbles(register, byte), bitwise);
            }
        }
    }
}
