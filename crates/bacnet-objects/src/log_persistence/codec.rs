//! The octets of one log snapshot slot, and the checks every snapshot passes
//! before it is written or after it is read.
//!
//! A slot is a 26-octet header (magic tag, schema version, object
//! identifier, generation, payload length), the payload, and a CRC-32 of
//! everything before it. The payload holds, in order: Buffer_Size, a flags
//! octet (Enable, Stop_When_Full), Total_Record_Count, Last_Notify_Record,
//! the Records_Since_Notification base, Notification_Threshold, Start_Time
//! and Stop_Time as four
//! octets of date and four of time each, the acquisition settings behind a
//! presence octet, then the references and the records, each list a count
//! followed by length-prefixed Clause 21 encodings. Every integer is
//! big-endian.

use bacnet_encoding::constructed::{
    decode_device_object_property_reference, decode_event_log_record, decode_log_multiple_record,
    decode_log_record, encode_device_object_property_reference,
};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetEventLogRecord, BACnetLogMultipleRecord,
    BACnetLogRecord,
};
use bacnet_types::enums::{LoggingType, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use super::{LogAcquisitionSettings, LogSnapshot};
use crate::durable::crc32;
use crate::log_buffer::ResidentLogRecord;
use crate::log_window::LogWindow;
use crate::trend::MAX_LOG_DEVICE_OBJECT_PROPERTIES;

const MAGIC: &[u8; 8] = b"RBLOGS01";
pub(super) const SCHEMA_VERSION: u16 = 1;
pub(super) const HEADER_LEN: usize = 8 + 2 + 4 + 8 + 4;
pub(super) const TRAILER_LEN: usize = 4;
/// The most octets one record or reference may take in a slot.
const MAX_ELEMENT_BYTES: usize = 1024 * 1024;
/// The most octets a slot may take.
pub(super) const MAX_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;

/// A record family a snapshot can hold: the record of one log object type.
pub(crate) trait SnapshotRecord:
    ResidentLogRecord + PartialEq + Send + Sync + 'static
{
    /// The log object type whose records these are.
    const OBJECT_TYPE: ObjectType;
    /// The most octets one record the stack itself makes can take: what a
    /// slot is sized for (see [`check_buffer_size`]).
    const WORST_RECORD_OCTETS: usize;
    /// Decode one record at `offset`, returning it and the offset past it.
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error>;
}

impl SnapshotRecord for BACnetLogRecord {
    const OBJECT_TYPE: ObjectType = ObjectType::TREND_LOG;
    /// A timestamp, an any-value at the pollers' 256-octet cap with its
    /// framing, and Status_Flags.
    const WORST_RECORD_OCTETS: usize = 320;
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_log_record(data, offset)
    }
}

impl SnapshotRecord for BACnetEventLogRecord {
    const OBJECT_TYPE: ObjectType = ObjectType::EVENT_LOG;
    /// A timestamp and a notification no larger than the 1476-octet APDU
    /// it could arrive in.
    const WORST_RECORD_OCTETS: usize = 1_500;
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_event_log_record(data, offset)
    }
}

impl SnapshotRecord for BACnetLogMultipleRecord {
    const OBJECT_TYPE: ObjectType = ObjectType::TREND_LOG_MULTIPLE;
    /// A timestamp and one 256-octet any-value, framed, for each of the 64
    /// references a log may hold.
    const WORST_RECORD_OCTETS: usize = 32 + MAX_LOG_DEVICE_OBJECT_PROPERTIES * 264;
    fn decode_at(data: &[u8], offset: usize) -> Result<(Self, usize), Error> {
        decode_log_multiple_record(data, offset)
    }
}

/// Room in a slot for everything but the records: header, trailer, the
/// fixed fields and 64 references.
const FIXED_ALLOWANCE: usize = 64 * 1024;

/// The largest Buffer_Size an `R` log can keep in a slot: enough of the
/// family's worst-case records, each with its length prefix, to fill the
/// slot cap.
pub(crate) fn max_buffer_size<R: SnapshotRecord>() -> u32 {
    let records = (MAX_SNAPSHOT_BYTES - FIXED_ALLOWANCE) / (R::WORST_RECORD_OCTETS + 4);
    u32::try_from(records).unwrap_or(u32::MAX)
}

/// Refuse a Buffer_Size whose full buffer of worst-case records could
/// outgrow a slot, so a durable log never grows into a state it can't save.
pub(crate) fn check_buffer_size<R: SnapshotRecord>(buffer_size: u32) -> Result<(), Error> {
    let max = max_buffer_size::<R>();
    if buffer_size > max {
        return Err(Error::OutOfRange(format!(
            "a durable {:?} keeps at most {max} records, not Buffer_Size {buffer_size}",
            R::OBJECT_TYPE
        )));
    }
    Ok(())
}

fn corrupt(reason: impl std::fmt::Display) -> Error {
    Error::Encoding(format!("log snapshot {reason}"))
}

/// Check what any snapshot of an `R` log must hold, whether it is about to
/// be written or was just read: the identity and generation, the record
/// count against Buffer_Size and Total_Record_Count (as `restore_log_buffer`
/// checks them), the window ends, and the settings and references the
/// object type has.
pub(crate) fn validate<R: SnapshotRecord>(snapshot: &LogSnapshot<R>) -> Result<(), Error> {
    let kind = snapshot.object_identifier.object_type();
    if kind != R::OBJECT_TYPE {
        return Err(corrupt(format_args!(
            "for a {kind:?} cannot hold {:?} records",
            R::OBJECT_TYPE
        )));
    }
    if snapshot.generation == 0 {
        return Err(corrupt("generation must be nonzero"));
    }
    let held = snapshot.records.len();
    if held > snapshot.buffer_size as usize {
        return Err(corrupt(format_args!(
            "holds {held} records, more than Buffer_Size {}",
            snapshot.buffer_size
        )));
    }
    if snapshot.total_record_count == 0 && held > 0 {
        return Err(corrupt("holds records with a Total_Record_Count of 0"));
    }
    if snapshot.log_enable
        && snapshot.stop_when_full
        && held > 0
        && held >= snapshot.buffer_size as usize
    {
        return Err(corrupt(
            "holds a full buffer while Stop_When_Full and Enable are both TRUE",
        ));
    }
    let mut window = LogWindow::default();
    window.set(P::START_TIME, snapshot.start_time)?;
    window.set(P::STOP_TIME, snapshot.stop_time)?;
    let references = snapshot.log_device_object_property.len();
    let (acquisition_allowed, max_references) = match R::OBJECT_TYPE {
        ObjectType::TREND_LOG => (
            &[
                LoggingType::POLLED,
                LoggingType::TRIGGERED,
                LoggingType::COV,
            ][..],
            1,
        ),
        ObjectType::TREND_LOG_MULTIPLE => (
            &[LoggingType::POLLED, LoggingType::TRIGGERED][..],
            MAX_LOG_DEVICE_OBJECT_PROPERTIES,
        ),
        _ => (&[][..], 0),
    };
    match (&snapshot.acquisition, acquisition_allowed.is_empty()) {
        (None, true) => {}
        (Some(acquisition), false) => {
            if !acquisition_allowed.contains(&acquisition.logging_type) {
                return Err(corrupt(format_args!(
                    "holds a Logging_Type of {:?} this log can't take",
                    acquisition.logging_type
                )));
            }
            if acquisition.logging_type != LoggingType::POLLED && acquisition.log_interval != 0 {
                return Err(corrupt(
                    "holds a nonzero Log_Interval for a log that isn't POLLED",
                ));
            }
        }
        (None, false) => return Err(corrupt("lacks the acquisition settings of a trend log")),
        (Some(_), true) => return Err(corrupt("holds acquisition settings an Event Log lacks")),
    }
    if references > max_references {
        return Err(corrupt(format_args!(
            "holds {references} Log_DeviceObjectProperty references, more than {max_references}"
        )));
    }
    for reference in &snapshot.log_device_object_property {
        crate::device_reference::check_device_member(reference.device_identifier)?;
    }
    Ok(())
}

/// The slot octets for `snapshot`, after [`validate`].
pub(crate) fn encode<R: SnapshotRecord>(snapshot: &LogSnapshot<R>) -> Result<Vec<u8>, Error> {
    validate(snapshot)?;
    let mut payload = Vec::new();
    payload.extend_from_slice(&snapshot.buffer_size.to_be_bytes());
    payload.push(u8::from(snapshot.log_enable) | (u8::from(snapshot.stop_when_full) << 1));
    payload.extend_from_slice(&snapshot.total_record_count.to_be_bytes());
    payload.extend_from_slice(&snapshot.last_notify_record.to_be_bytes());
    payload.extend_from_slice(&snapshot.records_since_notification_from.to_be_bytes());
    payload.extend_from_slice(&snapshot.notification_threshold.to_be_bytes());
    for (date, time) in [snapshot.start_time, snapshot.stop_time] {
        payload.extend_from_slice(&date.encode());
        payload.extend_from_slice(&time.encode());
    }
    match &snapshot.acquisition {
        None => payload.push(0),
        Some(acquisition) => {
            payload.push(1);
            payload.extend_from_slice(&acquisition.logging_type.to_raw().to_be_bytes());
            payload.extend_from_slice(&acquisition.log_interval.to_be_bytes());
            payload.push(u8::from(acquisition.align_intervals));
            payload.extend_from_slice(&acquisition.interval_offset.to_be_bytes());
        }
    }
    let mut element = BytesMut::new();
    put_list(
        &mut payload,
        &snapshot.log_device_object_property,
        &mut element,
        |reference, buf| {
            encode_device_object_property_reference(buf, reference);
            Ok(())
        },
    )?;
    put_list(
        &mut payload,
        &snapshot.records,
        &mut element,
        |record, buf| record.encode(buf),
    )?;
    let total_len = HEADER_LEN + payload.len() + TRAILER_LEN;
    if total_len > MAX_SNAPSHOT_BYTES {
        return Err(Error::OutOfRange(format!(
            "log snapshot length {total_len} exceeds {MAX_SNAPSHOT_BYTES}"
        )));
    }
    let mut data = Vec::with_capacity(total_len);
    data.extend_from_slice(MAGIC);
    data.extend_from_slice(&SCHEMA_VERSION.to_be_bytes());
    data.extend_from_slice(&snapshot.object_identifier.encode());
    data.extend_from_slice(&snapshot.generation.to_be_bytes());
    data.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    data.extend_from_slice(&payload);
    let checksum = crc32(&data);
    data.extend_from_slice(&checksum.to_be_bytes());
    Ok(data)
}

fn put_list<T>(
    payload: &mut Vec<u8>,
    items: &[T],
    element: &mut BytesMut,
    encode: impl Fn(&T, &mut BytesMut) -> Result<(), Error>,
) -> Result<(), Error> {
    let count = u32::try_from(items.len()).map_err(|_| corrupt("list is too long"))?;
    payload.extend_from_slice(&count.to_be_bytes());
    for item in items {
        element.clear();
        encode(item, element)?;
        if element.len() > MAX_ELEMENT_BYTES {
            return Err(Error::OutOfRange(format!(
                "log snapshot element length {} exceeds {MAX_ELEMENT_BYTES}",
                element.len()
            )));
        }
        payload.extend_from_slice(&(element.len() as u32).to_be_bytes());
        payload.extend_from_slice(element);
        if payload.len() > MAX_SNAPSHOT_BYTES {
            return Err(Error::OutOfRange(format!(
                "log snapshot length exceeds {MAX_SNAPSHOT_BYTES}"
            )));
        }
    }
    Ok(())
}

/// What one slot's octets turned out to hold.
pub(super) enum Decoded<R> {
    /// A valid snapshot.
    Valid(LogSnapshot<R>),
    /// Damaged or not a snapshot at all: the other slot may still serve.
    Corrupt(Error),
    /// A sound slot this log must not take: another object's, or a schema
    /// this build can't read. Loading stops with this error rather than
    /// falling back to an older slot.
    Refused(Error),
}

/// Read one slot's octets as a snapshot for `expected`.
pub(super) fn decode<R: SnapshotRecord>(data: &[u8], expected: ObjectIdentifier) -> Decoded<R> {
    if let Err(error) = verify_integrity(data) {
        return Decoded::Corrupt(error);
    }
    let version = u16::from_be_bytes([data[8], data[9]]);
    if version != SCHEMA_VERSION {
        return Decoded::Refused(corrupt(format_args!(
            "schema version {version} is unsupported"
        )));
    }
    match ObjectIdentifier::decode(&data[10..14]) {
        Ok(oid) if oid == expected => {}
        Ok(_) => return Decoded::Refused(corrupt("belongs to another object")),
        Err(error) => return Decoded::Corrupt(error),
    }
    match decode_payload(data, expected) {
        Ok(snapshot) => Decoded::Valid(snapshot),
        Err(error) => Decoded::Corrupt(error),
    }
}

fn verify_integrity(data: &[u8]) -> Result<(), Error> {
    if data.len() < HEADER_LEN + TRAILER_LEN {
        return Err(corrupt("is truncated"));
    }
    if &data[..8] != MAGIC {
        return Err(corrupt("magic is invalid"));
    }
    let payload_len = u32::from_be_bytes(data[22..26].try_into().unwrap()) as usize;
    if HEADER_LEN + payload_len + TRAILER_LEN != data.len() {
        return Err(corrupt("declared length does not match its length"));
    }
    let checksum_at = data.len() - TRAILER_LEN;
    let stored = u32::from_be_bytes(data[checksum_at..].try_into().unwrap());
    if crc32(&data[..checksum_at]) != stored {
        return Err(corrupt("checksum is invalid"));
    }
    Ok(())
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize, field: &str) -> Result<&'a [u8], Error> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| corrupt(format_args!("{field} is truncated")))?;
        let value = &self.data[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self, field: &str) -> Result<u8, Error> {
        Ok(self.take(1, field)?[0])
    }

    fn u32(&mut self, field: &str) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4, field)?.try_into().unwrap()))
    }

    fn flag(&mut self, field: &str) -> Result<bool, Error> {
        match self.u8(field)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(corrupt(format_args!("{field} is not a Boolean"))),
        }
    }

    fn date_time(&mut self, field: &str) -> Result<(Date, Time), Error> {
        let date = Date::decode(self.take(4, field)?)?;
        let time = Time::decode(self.take(4, field)?)?;
        Ok((date, time))
    }

    /// A count of at most `cap`, then that many length-prefixed elements,
    /// each decoded whole by `decode`. The count is checked against `cap`
    /// and what the payload can hold before anything is reserved for it, so
    /// a crafted count can't reserve more than a valid snapshot needs.
    fn list<T>(
        &mut self,
        field: &str,
        cap: usize,
        decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
    ) -> Result<Vec<T>, Error> {
        let count = self.u32(field)? as usize;
        if count > cap || count > (self.data.len() - self.offset) / 4 {
            return Err(corrupt(format_args!("{field} count is too large")));
        }
        // Reserve a modest start; a long list grows as its elements decode.
        let mut items = Vec::with_capacity(count.min(4096));
        for _ in 0..count {
            let length = self.u32(field)? as usize;
            if length > MAX_ELEMENT_BYTES {
                return Err(corrupt(format_args!("{field} element is too long")));
            }
            let element = self.take(length, field)?;
            let (item, end) = decode(element, 0)?;
            if end != element.len() {
                return Err(corrupt(format_args!("{field} element has trailing octets")));
            }
            items.push(item);
        }
        Ok(items)
    }
}

fn decode_payload<R: SnapshotRecord>(
    data: &[u8],
    object_identifier: ObjectIdentifier,
) -> Result<LogSnapshot<R>, Error> {
    let generation = u64::from_be_bytes(data[14..22].try_into().unwrap());
    let mut reader = Reader {
        data: &data[HEADER_LEN..data.len() - TRAILER_LEN],
        offset: 0,
    };
    let buffer_size = reader.u32("Buffer_Size")?;
    let flags = reader.u8("flags")?;
    if flags & !0b11 != 0 {
        return Err(corrupt("flags hold unknown bits"));
    }
    let total_record_count = reader.u32("Total_Record_Count")?;
    let last_notify_record = reader.u32("Last_Notify_Record")?;
    let records_since_notification_from = reader.u32("Records_Since_Notification base")?;
    let notification_threshold = reader.u32("Notification_Threshold")?;
    let start_time = reader.date_time("Start_Time")?;
    let stop_time = reader.date_time("Stop_Time")?;
    let acquisition = if reader.flag("acquisition presence")? {
        Some(LogAcquisitionSettings {
            logging_type: LoggingType::from_raw(reader.u32("Logging_Type")?),
            log_interval: reader.u32("Log_Interval")?,
            align_intervals: reader.flag("Align_Intervals")?,
            interval_offset: reader.u32("Interval_Offset")?,
        })
    } else {
        None
    };
    let log_device_object_property: Vec<BACnetDeviceObjectPropertyReference> = reader.list(
        "Log_DeviceObjectProperty",
        MAX_LOG_DEVICE_OBJECT_PROPERTIES,
        decode_device_object_property_reference,
    )?;
    let records = reader.list(
        "record",
        buffer_size.min(max_buffer_size::<R>()) as usize,
        R::decode_at,
    )?;
    if reader.offset != reader.data.len() {
        return Err(corrupt("payload has trailing octets"));
    }
    let snapshot = LogSnapshot {
        object_identifier,
        generation,
        buffer_size,
        log_enable: flags & 1 != 0,
        stop_when_full: flags & 2 != 0,
        total_record_count,
        records,
        last_notify_record,
        records_since_notification_from,
        notification_threshold,
        start_time,
        stop_time,
        acquisition,
        log_device_object_property,
    };
    validate(&snapshot)?;
    // A record that decodes but would not encode again could never be
    // served, so such a slot is as damaged as one that fails to decode.
    let mut scratch = BytesMut::new();
    for record in &snapshot.records {
        scratch.clear();
        record.encode(&mut scratch)?;
    }
    Ok(snapshot)
}
