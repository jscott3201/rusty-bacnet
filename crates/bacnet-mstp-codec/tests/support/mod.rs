//! What a receiver's CRC registers hold after good input, shared by the
//! CRC tests. Each register is the complement of what the matching `crc`
//! function returns. Each test binary uses a subset.

#![allow(dead_code)]

/// The header CRC register after a good header and its CRC octet.
pub const HEADER_CRC_RESIDUAL: u8 = 0x55;
/// The data CRC register after good data and its two CRC octets.
pub const DATA_CRC_RESIDUAL: u16 = 0xF0B8;
/// The CRC-32K register after a good extended frame's Encoded Data and the
/// four CRC octets decoded from its Encoded CRC-32K (Clause 9.6.3).
pub const CRC32K_RESIDUAL: u32 = 0x0843_323B;
