//! File (type 10) object per ASHRAE 135-2020 Clause 12.13.
//!
//! Backs AtomicReadFile and AtomicWriteFile services. Supports both
//! stream-access and record-access modes.

use bacnet_types::enums::{
    ErrorClass, ErrorCode, FileAccessMethod, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;
use std::sync::Arc;

use crate::clock::ClockReader;
use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// File storage (the Clause 14 service data behind a File object)
// ---------------------------------------------------------------------------

/// Default growth cap, in octets, for network writes to one File object.
///
/// Clause 14.2 requires a write whose 'File Start Position' exceeds the
/// file size to extend the file to that size, and the position is a signed
/// 32-bit INTEGER, so an unbounded implementation would zero-fill up to
/// 2 GiB from one small request. Clause 18 defines FILE_FULL for exactly
/// this bound: "when a File Object becomes filled to a designed limit, as
/// opposed to a No Space Available / Out of Memory situation".
pub const DEFAULT_MAX_FILE_SIZE: u64 = 1_048_576;

/// Default growth cap, in records, for network writes to one record-access
/// File object.
///
/// Bounds the record vector independently of its payload octets, which
/// [`DEFAULT_MAX_FILE_SIZE`] bounds: extending to a far record index costs
/// a `Vec` header per intervening empty record even when no octet is
/// stored. The value is at most `bacnet_services::common::MAX_DECODED_ITEMS`
/// — today exactly that ceiling — the largest SEQUENCE OF the workspace
/// decoders accept in one ACK, so a record file read back at the cap still
/// decodes on the client side.
pub const DEFAULT_MAX_RECORD_COUNT: u64 = 10_000;

/// Ceiling for [`FileObject::set_max_record_count`]; see
/// [`DEFAULT_MAX_RECORD_COUNT`] for why it is the decoder's limit.
const MAX_RECORD_CAP: u64 = DEFAULT_MAX_RECORD_COUNT;

/// One stream-access read window, as AtomicReadFile returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStreamRead {
    /// The octets read: never more than requested, fewer when the file
    /// ends first (Clause 14.1 Service Procedure).
    pub data: Vec<u8>,
    /// Clause 14.1.3.1 'End Of File'. The built-in empty file reports EOF at
    /// its only valid start; an empty window in a non-empty file does not.
    pub end_of_file: bool,
}

/// One record-access read window, as AtomicReadFile returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRecordRead {
    /// The records read. Its length is the ACK's 'Returned Record Count'
    /// (Clause 14.1.3.3), which may be less than the requested count.
    pub records: Vec<Vec<u8>>,
    /// 'End Of File' per the Clause 14.1 Service Procedure. The built-in
    /// empty record file reports EOF at its only valid start.
    pub end_of_file: bool,
}

/// Where an AtomicWriteFile write starts.
///
/// Clauses 14.2.2.2 and 14.2.2.3 give 'File Start Position' and 'File
/// Start Record' the special value -1 for "an append to file operation";
/// every other value is an offset from the beginning of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileWriteStart {
    /// Write at this octet or record offset, extending the file first if
    /// the offset is past the current end (Clause 14.2 Service Procedure).
    At(u64),
    /// Write at the current end of the file.
    Append,
}

/// The octet- and record-addressed storage behind a File object.
///
/// This is the **internal** channel AtomicReadFile and AtomicWriteFile use
/// to reach file contents (Clauses 14.1 and 14.2). It is deliberately not a
/// property: Table 12-16 (Clause 12.13) defines no File Data property, so
/// file contents have no network route other than the File Access Services,
/// and nothing here appears in `Property_List`.
///
/// The server reaches an implementation through
/// [`BACnetObject::file_storage_internal`] and
/// [`BACnetObject::file_storage_internal_mut`]. Every method has a default
/// that refuses with SERVICES / FILE_ACCESS_DENIED — there is no
/// implementation behind it, so the file is "otherwise not accessible" in
/// Clause 18's words — and a stream-only implementation overrides only the
/// two stream methods.
///
/// Error contract, using the Clause 14 pairs:
///
/// - SERVICES / INVALID_FILE_START_POSITION when a read starts past the
///   current end (Clause 14.1 Service Procedure). Reading *at* the end is
///   legal and yields an empty window. The built-in empty file reports
///   `end_of_file` TRUE there; non-empty files report FALSE for such a window.
/// - OBJECT / FILE_FULL when a write would grow the file past the
///   implementation's designed limit (Clause 14.2.4.1; Clause 18).
/// - SERVICES / INVALID_FILE_ACCESS_METHOD when the method does not match
///   the object's `File_Access_Method`; only a genuine mismatch, never a
///   missing implementation, reports this.
/// - SERVICES / FILE_ACCESS_DENIED from the defaults above.
///
/// A write that returns `Err` must leave the storage unchanged: the service
/// fails "in its entirety" (Clause 14.2.4), and the server encodes no ACK on
/// the error path. Resolved write positions must fit the ACK's INTEGER, so
/// implementations keep their limit at or below `i32::MAX`.
pub trait FileStorage: Send + Sync {
    /// Read up to `count` octets starting `start` octets into the file.
    fn read_stream(&self, _start: u64, _count: u64) -> Result<FileStreamRead, Error> {
        Err(file_access_denied())
    }

    /// Write `data` at `start`, extending the file first when `start` is
    /// past the end; intervening octets are a local matter. Returns the
    /// octet offset the data was written at, which for
    /// [`FileWriteStart::Append`] is the previous file size.
    fn write_stream(&mut self, _start: FileWriteStart, _data: &[u8]) -> Result<u64, Error> {
        Err(file_access_denied())
    }

    /// Read up to `count` records starting at record `start`.
    fn read_records(&self, _start: u64, _count: u64) -> Result<FileRecordRead, Error> {
        Err(file_access_denied())
    }

    /// Write `records` starting at record `start`, replacing records in
    /// place and extending the file first when `start` is past the end.
    /// Returns the record index the data was written at.
    fn write_records(
        &mut self,
        _start: FileWriteStart,
        _records: &[Vec<u8>],
    ) -> Result<u64, Error> {
        Err(file_access_denied())
    }
}

fn invalid_file_access_method() -> Error {
    common::protocol_error(ErrorClass::SERVICES, ErrorCode::INVALID_FILE_ACCESS_METHOD)
}

fn file_access_denied() -> Error {
    common::protocol_error(ErrorClass::SERVICES, ErrorCode::FILE_ACCESS_DENIED)
}

fn invalid_file_start_position() -> Error {
    common::protocol_error(ErrorClass::SERVICES, ErrorCode::INVALID_FILE_START_POSITION)
}

fn file_full() -> Error {
    common::protocol_error(ErrorClass::OBJECT, ErrorCode::FILE_FULL)
}

/// Resolved positions travel in the ACK as a BACnet INTEGER; caps above
/// `i32::MAX` could produce one that does not fit.
const MAX_REPRESENTABLE: u64 = i32::MAX as u64;

fn octet_total(records: &[Vec<u8>]) -> u64 {
    records.iter().map(|r| r.len() as u64).sum()
}

// ---------------------------------------------------------------------------
// FileObject (type 10)
// ---------------------------------------------------------------------------

/// BACnet File object.
///
/// Represents a file accessible via AtomicReadFile / AtomicWriteFile services.
/// The `file_access_method` determines whether the file is accessed as a
/// byte stream ([`FileAccessMethod::STREAM_ACCESS`]) or as a sequence of
/// fixed-length records ([`FileAccessMethod::RECORD_ACCESS`]).
pub struct FileObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    file_type: String,
    file_size: u64,
    modification_date: (Date, Time),
    archive: bool,
    read_only: bool,
    /// Raw `BACnetFileAccessMethod` value; named in [`FileAccessMethod`]
    /// (record-access = 0, stream-access = 1 per the Clause 21 production).
    file_access_method: u32,
    /// Record count (only meaningful for record-access files).
    record_count: Option<u64>,
    /// Stream data (used when file_access_method == STREAM_ACCESS).
    data: Vec<u8>,
    /// Record data (used when file_access_method == RECORD_ACCESS).
    records: Vec<Vec<u8>>,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    /// Growth cap in octets for network writes; not a BACnet property.
    max_file_size: u64,
    /// Growth cap in records for network writes; not a BACnet property.
    max_record_count: u64,
    /// Database-owned coherent clock source used at successful mutations.
    clock: Option<Arc<dyn ClockReader>>,
}

impl FileObject {
    /// Create a new File object.
    ///
    /// Defaults to stream access (file_access_method = STREAM_ACCESS), empty
    /// data, not read-only, archive = false.
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        file_type: impl Into<String>,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::FILE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            file_type: file_type.into(),
            file_size: 0,
            modification_date: (
                Date {
                    year: Date::UNSPECIFIED,
                    month: Date::UNSPECIFIED,
                    day: Date::UNSPECIFIED,
                    day_of_week: Date::UNSPECIFIED,
                },
                Time {
                    hour: Time::UNSPECIFIED,
                    minute: Time::UNSPECIFIED,
                    second: Time::UNSPECIFIED,
                    hundredths: Time::UNSPECIFIED,
                },
            ),
            archive: false,
            read_only: false,
            file_access_method: FileAccessMethod::STREAM_ACCESS.to_raw(),
            record_count: None,
            data: Vec::new(),
            records: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
            max_file_size: DEFAULT_MAX_FILE_SIZE,
            max_record_count: DEFAULT_MAX_RECORD_COUNT,
            clock: None,
        })
    }

    /// Set the description.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set the file type string.
    pub fn set_file_type(&mut self, ft: impl Into<String>) {
        self.file_type = ft.into();
    }

    /// Set stream data; File_Size follows it unless the object is
    /// RECORD_ACCESS (any other access-method value, recognised or not,
    /// is the stream channel, as in
    /// [`set_file_access_method`](Self::set_file_access_method)).
    pub fn set_data(&mut self, data: Vec<u8>) {
        self.data = data;
        if self.file_access_method != FileAccessMethod::RECORD_ACCESS.to_raw() {
            self.file_size = self.data.len() as u64;
        }
        self.mark_modified();
    }

    /// Get a reference to the stream data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Set the file access method. Accepts the raw `BACnetFileAccessMethod`
    /// value: [`FileAccessMethod::STREAM_ACCESS`] (1) or
    /// [`FileAccessMethod::RECORD_ACCESS`] (0).
    ///
    /// File_Size and Record_Count are recomputed from the channel the
    /// object switches to: Table 12-16 footnote 2 has Record_Count present
    /// only under RECORD_ACCESS, and File_Size counts that channel's octets.
    pub fn set_file_access_method(&mut self, method: u32) {
        let old_file_size = self.file_size;
        self.file_access_method = method;
        if method == FileAccessMethod::RECORD_ACCESS.to_raw() {
            self.record_count = Some(self.records.len() as u64);
            self.file_size = octet_total(&self.records);
        } else {
            self.record_count = None;
            self.file_size = self.data.len() as u64;
        }
        if self.file_size != old_file_size {
            self.mark_modified();
        }
    }

    /// Set the records; Record_Count and File_Size follow them while the
    /// object is RECORD_ACCESS (Table 12-16 footnote 2).
    pub fn set_records(&mut self, records: Vec<Vec<u8>>) {
        self.records = records;
        if self.file_access_method == FileAccessMethod::RECORD_ACCESS.to_raw() {
            self.record_count = Some(self.records.len() as u64);
            self.file_size = octet_total(&self.records);
        }
        self.mark_modified();
    }

    /// Get a reference to the records.
    pub fn records(&self) -> &[Vec<u8>] {
        &self.records
    }

    /// Set the modification date.
    pub fn set_modification_date(&mut self, date: Date, time: Time) {
        let modification_date = (date, time);
        if self.modification_date != modification_date {
            self.modification_date = modification_date;
            self.archive = false;
        }
    }

    /// Set the archive flag.
    pub fn set_archive(&mut self, archive: bool) {
        self.archive = archive;
    }

    /// Set the read-only flag.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Get the file size in bytes.
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Get the archive flag.
    pub fn archive(&self) -> bool {
        self.archive
    }

    /// Get the read-only flag.
    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Set the growth cap, in octets, for network writes to this file.
    ///
    /// The cap bounds what AtomicWriteFile can add; it never invalidates
    /// contents preloaded through [`set_data`](Self::set_data), which may
    /// exceed it and still be overwritten in place. Clamped to `i32::MAX`
    /// so every resolved write position fits the ACK.
    pub fn set_max_file_size(&mut self, max_octets: u64) {
        self.max_file_size = max_octets.min(MAX_REPRESENTABLE);
    }

    /// The growth cap, in octets, for network writes to this file.
    pub fn max_file_size(&self) -> u64 {
        self.max_file_size
    }

    /// Set the growth cap, in records, for network writes to this file.
    ///
    /// Same growth-only semantics as
    /// [`set_max_file_size`](Self::set_max_file_size): records preloaded
    /// through [`set_records`](Self::set_records) are never refused. Clamped
    /// to [`DEFAULT_MAX_RECORD_COUNT`], the decoder ceiling, so a file grown
    /// by network writes stays at or under what one AtomicReadFile-ACK
    /// carries; a larger preloaded file reads back in windows of at most
    /// that many records.
    pub fn set_max_record_count(&mut self, max_records: u64) {
        self.max_record_count = max_records.min(MAX_RECORD_CAP).min(MAX_REPRESENTABLE);
    }

    /// The growth cap, in records, for network writes to this file.
    pub fn max_record_count(&self) -> u64 {
        self.max_record_count
    }

    /// The handler gate is normative; this keeps `data` and `records` from
    /// both filling up when a non-handler caller reaches the storage.
    fn require_access(&self, method: FileAccessMethod) -> Result<(), Error> {
        if self.file_access_method == method.to_raw() {
            Ok(())
        } else {
            Err(invalid_file_access_method())
        }
    }

    fn modification_datetime(&self) -> (Date, Time) {
        let frame = self.clock.as_ref().and_then(|clock| clock.read_clock());
        match frame {
            Some(frame) if frame.is_valid_actual_datetime() => (frame.local_date, frame.local_time),
            _ => (
                Date {
                    year: Date::UNSPECIFIED,
                    month: Date::UNSPECIFIED,
                    day: Date::UNSPECIFIED,
                    day_of_week: Date::UNSPECIFIED,
                },
                Time {
                    hour: Time::UNSPECIFIED,
                    minute: Time::UNSPECIFIED,
                    second: Time::UNSPECIFIED,
                    hundredths: Time::UNSPECIFIED,
                },
            ),
        }
    }

    fn mark_modified(&mut self) {
        self.modification_date = self.modification_datetime();
        self.archive = false;
    }
}

impl FileStorage for FileObject {
    fn read_stream(&self, start: u64, count: u64) -> Result<FileStreamRead, Error> {
        self.require_access(FileAccessMethod::STREAM_ACCESS)?;
        let len = self.data.len() as u64;
        if start > len {
            return Err(invalid_file_start_position());
        }
        let end = start.saturating_add(count).min(len);
        Ok(FileStreamRead {
            data: self.data[start as usize..end as usize].to_vec(),
            end_of_file: len == 0 || (end >= len && end > start),
        })
    }

    fn write_stream(&mut self, start: FileWriteStart, data: &[u8]) -> Result<u64, Error> {
        self.require_access(FileAccessMethod::STREAM_ACCESS)?;
        let len = self.data.len() as u64;
        let start = match start {
            FileWriteStart::At(offset) => offset,
            FileWriteStart::Append => len,
        };
        let end = start.checked_add(data.len() as u64).ok_or_else(file_full)?;
        if end > self.max_file_size.max(len) {
            return Err(file_full());
        }
        let (start_idx, end_idx) = (
            usize::try_from(start).map_err(|_| file_full())?,
            usize::try_from(end).map_err(|_| file_full())?,
        );
        let mut updated = std::mem::take(&mut self.data);
        if end_idx > updated.len() {
            updated.resize(end_idx, 0);
        }
        updated[start_idx..end_idx].copy_from_slice(data);
        self.set_data(updated);
        Ok(start)
    }

    fn read_records(&self, start: u64, count: u64) -> Result<FileRecordRead, Error> {
        self.require_access(FileAccessMethod::RECORD_ACCESS)?;
        let len = self.records.len() as u64;
        if start > len {
            return Err(invalid_file_start_position());
        }
        let end = start.saturating_add(count).min(len);
        Ok(FileRecordRead {
            records: self.records[start as usize..end as usize].to_vec(),
            end_of_file: len == 0 || (end >= len && end > start),
        })
    }

    fn write_records(&mut self, start: FileWriteStart, records: &[Vec<u8>]) -> Result<u64, Error> {
        self.require_access(FileAccessMethod::RECORD_ACCESS)?;
        let len = self.records.len() as u64;
        let start = match start {
            FileWriteStart::At(index) => index,
            FileWriteStart::Append => len,
        };
        let end = start
            .checked_add(records.len() as u64)
            .ok_or_else(file_full)?;
        if end > self.max_record_count.max(len) {
            return Err(file_full());
        }
        let (start_idx, end_idx) = (
            usize::try_from(start).map_err(|_| file_full())?,
            usize::try_from(end).map_err(|_| file_full())?,
        );
        // Octet cap on the projected total: existing octets, minus those
        // in the records being replaced, plus the incoming payload.
        let existing = octet_total(&self.records);
        let replaced = octet_total(
            &self.records[start_idx.min(self.records.len())..end_idx.min(self.records.len())],
        );
        let projected = existing - replaced + octet_total(records);
        if projected > self.max_file_size.max(existing) {
            return Err(file_full());
        }
        let mut updated = std::mem::take(&mut self.records);
        if end_idx > updated.len() {
            updated.resize(end_idx, Vec::new());
        }
        updated[start_idx..end_idx].clone_from_slice(records);
        self.set_records(updated);
        Ok(start)
    }
}

impl BACnetObject for FileObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }

        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::FILE.to_raw()))
            }
            p if p == PropertyIdentifier::FILE_TYPE => {
                Ok(PropertyValue::CharacterString(self.file_type.clone()))
            }
            p if p == PropertyIdentifier::FILE_SIZE => Ok(PropertyValue::Unsigned(self.file_size)),
            p if p == PropertyIdentifier::MODIFICATION_DATE => Ok(PropertyValue::List(vec![
                PropertyValue::Date(self.modification_date.0),
                PropertyValue::Time(self.modification_date.1),
            ])),
            p if p == PropertyIdentifier::ARCHIVE => Ok(PropertyValue::Boolean(self.archive)),
            p if p == PropertyIdentifier::READ_ONLY => Ok(PropertyValue::Boolean(self.read_only)),
            p if p == PropertyIdentifier::FILE_ACCESS_METHOD => {
                Ok(PropertyValue::Enumerated(self.file_access_method))
            }
            p if p == PropertyIdentifier::RECORD_COUNT => match self.record_count {
                Some(count) => Ok(PropertyValue::Unsigned(count)),
                None => Err(common::unknown_property_error()),
            },
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }

        match property {
            p if p == PropertyIdentifier::ARCHIVE => {
                if let PropertyValue::Boolean(v) = value {
                    self.archive = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::FILE_TYPE => {
                if let PropertyValue::CharacterString(s) = value {
                    self.file_type = s;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::READ_ONLY => {
                // Read-only is typically not writable from BACnet, but the
                // application may need it. Deny remote writes.
                Err(common::write_access_denied_error())
            }
            p if p == PropertyIdentifier::FILE_SIZE => Err(common::write_access_denied_error()),
            p if p == PropertyIdentifier::FILE_ACCESS_METHOD => {
                Err(common::write_access_denied_error())
            }
            p if p == PropertyIdentifier::MODIFICATION_DATE => {
                Err(common::write_access_denied_error())
            }
            p if p == PropertyIdentifier::RECORD_COUNT => Err(common::write_access_denied_error()),
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let mut props = vec![
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::FILE_TYPE,
            PropertyIdentifier::FILE_SIZE,
            PropertyIdentifier::MODIFICATION_DATE,
            PropertyIdentifier::ARCHIVE,
            PropertyIdentifier::READ_ONLY,
            PropertyIdentifier::FILE_ACCESS_METHOD,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        if self.record_count.is_some() {
            props.push(PropertyIdentifier::RECORD_COUNT);
        }
        Cow::Owned(props)
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        Some(self)
    }

    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests;
