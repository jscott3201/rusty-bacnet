//! The checksum of the two-slot formats, and sealing a slot with it: for
//! the corruption test and the fuzz crate, which damage a slot's payload
//! and then make its checksum right again (`restore.rs`).

/// The two-slot header the log and Audit Log formats share: magic tag,
/// schema version, object identifier, generation, payload length.
const SLOT_HEADER_LEN: usize = 8 + 2 + 4 + 8 + 4;
const SLOT_TRAILER_LEN: usize = 4;

/// The CRC-32 (IEEE 802.3, reflected) the slots end with, written here
/// from its definition: the table holds each octet's eight shift steps, so
/// the fuzz targets seal a slot with one lookup per octet (#2233).
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut crc = n as u32;
            let mut bit = 0;
            while bit < 8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
                bit += 1;
            }
            table[n] = crc;
            n += 1;
        }
        table
    };
    !data.iter().fold(!0u32, |crc, &byte| {
        TABLE[usize::from(crc as u8 ^ byte)] ^ (crc >> 8)
    })
}

/// Give a two-slot format's `slot` the length field and checksum of what
/// it holds, if it is long enough to have them, so damage past the header
/// reaches the payload decoders.
pub fn seal(slot: &mut [u8]) {
    if slot.len() < SLOT_HEADER_LEN + SLOT_TRAILER_LEN {
        return;
    }
    let payload = (slot.len() - SLOT_HEADER_LEN - SLOT_TRAILER_LEN) as u32;
    slot[22..26].copy_from_slice(&payload.to_be_bytes());
    let at = slot.len() - SLOT_TRAILER_LEN;
    let checksum = crc32(&slot[..at]);
    slot[at..].copy_from_slice(&checksum.to_be_bytes());
}
