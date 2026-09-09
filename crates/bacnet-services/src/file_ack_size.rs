//! Bounded sizing for AtomicReadFile ACKs only; never copies storage payloads.
use super::*;

fn read_ack_octet_string_len(len: usize) -> Option<usize> {
    let wire_len = u32::try_from(len).ok()?;
    let prefix = match wire_len {
        0..=4 => 1,
        5..=253 => 2,
        254..=65535 => 4,
        _ => 6,
    };
    len.checked_add(prefix)
}

impl AtomicReadFileAck {
    /// Exact logical service encoding length if it fits `limit` and wire lengths.
    ///
    /// Borrows payloads without encoding or copying them and stops summing as
    /// soon as the cap is exceeded. APDU/NPDU headers are not included.
    pub fn encoded_len_bounded(&self, limit: usize) -> Option<usize> {
        fn add(total: &mut usize, amount: usize, limit: usize) -> Option<()> {
            *total = total.checked_add(amount)?;
            (*total <= limit).then_some(())
        }
        // Application Boolean plus opening and closing CHOICE tags.
        let mut total = 0;
        add(&mut total, 3, limit)?;
        match &self.access {
            FileReadAckMethod::Stream {
                file_start_position,
                file_data,
            } => {
                add(
                    &mut total,
                    1 + primitives::signed_len(*file_start_position) as usize,
                    limit,
                )?;
                add(
                    &mut total,
                    read_ack_octet_string_len(file_data.len())?,
                    limit,
                )?;
            }
            FileReadAckMethod::Record {
                file_start_record,
                returned_record_count,
                file_record_data,
            } => {
                u32::try_from(file_record_data.len()).ok()?;
                add(
                    &mut total,
                    1 + primitives::signed_len(*file_start_record) as usize,
                    limit,
                )?;
                add(
                    &mut total,
                    1 + primitives::unsigned_len(u64::from(*returned_record_count)) as usize,
                    limit,
                )?;
                for record in file_record_data {
                    add(&mut total, read_ack_octet_string_len(record.len())?, limit)?;
                }
            }
        }
        Some(total)
    }
}

#[cfg(test)]
#[path = "file_ack_size_tests.rs"]
mod tests;
