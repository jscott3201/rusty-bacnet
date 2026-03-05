//! File (type 10) object per ASHRAE 135-2020 Clause 12.11.
//!
//! Backs AtomicReadFile and AtomicWriteFile services. Supports both
//! stream-access and record-access modes.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// FileObject (type 10)
// ---------------------------------------------------------------------------

/// BACnet File object.
///
/// Represents a file accessible via AtomicReadFile / AtomicWriteFile services.
/// The `file_access_method` determines whether the file is accessed as a
/// byte stream (0) or as a sequence of fixed-length records (1).
pub struct FileObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    file_type: String,
    file_size: u64,
    modification_date: (Date, Time),
    archive: bool,
    read_only: bool,
    /// 0 = stream access, 1 = record access.
    file_access_method: u32,
    /// Record count (only meaningful for record-access files).
    record_count: Option<u64>,
    /// Stream data (used when file_access_method == 0).
    data: Vec<u8>,
    /// Record data (used when file_access_method == 1).
    records: Vec<Vec<u8>>,
    status_flags: StatusFlags,
    out_of_service: bool,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
}

impl FileObject {
    /// Create a new File object.
    ///
    /// Defaults to stream access (file_access_method = 0), empty data,
    /// not read-only, archive = false.
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
                    year: 0xFF,
                    month: 0xFF,
                    day: 0xFF,
                    day_of_week: 0xFF,
                },
                Time {
                    hour: 0xFF,
                    minute: 0xFF,
                    second: 0xFF,
                    hundredths: 0xFF,
                },
            ),
            archive: false,
            read_only: false,
            file_access_method: 0,
            record_count: None,
            data: Vec::new(),
            records: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
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

    /// Set stream data and update file_size accordingly.
    pub fn set_data(&mut self, data: Vec<u8>) {
        self.file_size = data.len() as u64;
        self.data = data;
    }

    /// Get a reference to the stream data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Set the file access method (0 = stream, 1 = record).
    pub fn set_file_access_method(&mut self, method: u32) {
        self.file_access_method = method;
        if method == 1 {
            self.record_count = Some(self.records.len() as u64);
        } else {
            self.record_count = None;
        }
    }

    /// Set the records (for record-access files) and update record_count.
    pub fn set_records(&mut self, records: Vec<Vec<u8>>) {
        let total_size: u64 = records.iter().map(|r| r.len() as u64).sum();
        self.file_size = total_size;
        self.record_count = Some(records.len() as u64);
        self.records = records;
    }

    /// Get a reference to the records.
    pub fn records(&self) -> &[Vec<u8>] {
        &self.records
    }

    /// Set the modification date.
    pub fn set_modification_date(&mut self, date: Date, time: Time) {
        self.modification_date = (date, time);
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
        // Try common properties first.
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
        // DESCRIPTION
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }

        // OUT_OF_SERVICE
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::enums::ErrorCode;

    #[test]
    fn file_object_creation() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        assert_eq!(file.object_name(), "FILE-1");
        assert_eq!(file.object_identifier().instance_number(), 1);
    }

    #[test]
    fn file_read_object_type() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(ObjectType::FILE.to_raw()));
    }

    #[test]
    fn file_read_object_identifier() {
        let file = FileObject::new(42, "FILE-42", "application/octet-stream").unwrap();
        let val = file
            .read_property(PropertyIdentifier::OBJECT_IDENTIFIER, None)
            .unwrap();
        if let PropertyValue::ObjectIdentifier(oid) = val {
            assert_eq!(oid.instance_number(), 42);
        } else {
            panic!("expected ObjectIdentifier");
        }
    }

    #[test]
    fn file_read_object_name() {
        let file = FileObject::new(1, "MY-FILE", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::OBJECT_NAME, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("MY-FILE".into()));
    }

    #[test]
    fn file_read_file_type() {
        let file = FileObject::new(1, "FILE-1", "text/csv").unwrap();
        let val = file
            .read_property(PropertyIdentifier::FILE_TYPE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("text/csv".into()));
    }

    #[test]
    fn file_read_file_size_default_zero() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::FILE_SIZE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(0));
    }

    #[test]
    fn file_set_data_updates_file_size() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.set_data(vec![0x48, 0x65, 0x6C, 0x6C, 0x6F]); // "Hello"
        let val = file
            .read_property(PropertyIdentifier::FILE_SIZE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(5));
        assert_eq!(file.data(), &[0x48, 0x65, 0x6C, 0x6C, 0x6F]);
    }

    #[test]
    fn file_read_archive_default_false() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::ARCHIVE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn file_set_and_read_archive() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.set_archive(true);
        assert!(file.archive());
        let val = file
            .read_property(PropertyIdentifier::ARCHIVE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn file_read_read_only_default_false() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::READ_ONLY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn file_set_and_read_read_only() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.set_read_only(true);
        assert!(file.read_only());
        let val = file
            .read_property(PropertyIdentifier::READ_ONLY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn file_read_modification_date_default_unspecified() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::MODIFICATION_DATE, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 2);
            let unspec_date = Date {
                year: 0xFF,
                month: 0xFF,
                day: 0xFF,
                day_of_week: 0xFF,
            };
            let unspec_time = Time {
                hour: 0xFF,
                minute: 0xFF,
                second: 0xFF,
                hundredths: 0xFF,
            };
            assert_eq!(items[0], PropertyValue::Date(unspec_date));
            assert_eq!(items[1], PropertyValue::Time(unspec_time));
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn file_set_and_read_modification_date() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let d = Date {
            year: 126,
            month: 3,
            day: 1,
            day_of_week: 7,
        };
        let t = Time {
            hour: 14,
            minute: 30,
            second: 0,
            hundredths: 0,
        };
        file.set_modification_date(d, t);
        let val = file
            .read_property(PropertyIdentifier::MODIFICATION_DATE, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items[0], PropertyValue::Date(d));
            assert_eq!(items[1], PropertyValue::Time(t));
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn file_read_file_access_method_default_stream() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::FILE_ACCESS_METHOD, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0));
    }

    #[test]
    fn file_record_count_unavailable_for_stream() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let result = file.read_property(PropertyIdentifier::RECORD_COUNT, None);
        assert!(result.is_err());
    }

    #[test]
    fn file_set_records_updates_record_count_and_size() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.set_file_access_method(1);
        file.set_records(vec![vec![0x01, 0x02], vec![0x03, 0x04, 0x05]]);
        let count = file
            .read_property(PropertyIdentifier::RECORD_COUNT, None)
            .unwrap();
        assert_eq!(count, PropertyValue::Unsigned(2));
        let size = file
            .read_property(PropertyIdentifier::FILE_SIZE, None)
            .unwrap();
        assert_eq!(size, PropertyValue::Unsigned(5)); // 2 + 3 bytes
        assert_eq!(file.records().len(), 2);
    }

    #[test]
    fn file_read_status_flags_default() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::STATUS_FLAGS, None)
            .unwrap();
        if let PropertyValue::BitString { unused_bits, data } = val {
            assert_eq!(unused_bits, 4);
            assert_eq!(data, vec![0x00]);
        } else {
            panic!("expected BitString");
        }
    }

    #[test]
    fn file_read_out_of_service_default_false() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn file_read_reliability_default() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::RELIABILITY, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Enumerated(0));
    }

    #[test]
    fn file_read_description_default_empty() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let val = file
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString(String::new()));
    }

    #[test]
    fn file_write_description() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("A test file".into()),
            None,
        )
        .unwrap();
        let val = file
            .read_property(PropertyIdentifier::DESCRIPTION, None)
            .unwrap();
        assert_eq!(val, PropertyValue::CharacterString("A test file".into()));
    }

    #[test]
    fn file_write_archive() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.write_property(
            PropertyIdentifier::ARCHIVE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let val = file
            .read_property(PropertyIdentifier::ARCHIVE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn file_write_archive_invalid_type() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let result = file.write_property(
            PropertyIdentifier::ARCHIVE,
            None,
            PropertyValue::Unsigned(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn file_write_file_type() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.write_property(
            PropertyIdentifier::FILE_TYPE,
            None,
            PropertyValue::CharacterString("application/json".into()),
            None,
        )
        .unwrap();
        let val = file
            .read_property(PropertyIdentifier::FILE_TYPE, None)
            .unwrap();
        assert_eq!(
            val,
            PropertyValue::CharacterString("application/json".into())
        );
    }

    #[test]
    fn file_write_out_of_service() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
        let val = file
            .read_property(PropertyIdentifier::OUT_OF_SERVICE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn file_write_read_only_denied() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let result = file.write_property(
            PropertyIdentifier::READ_ONLY,
            None,
            PropertyValue::Boolean(true),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn file_write_file_size_denied() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let result = file.write_property(
            PropertyIdentifier::FILE_SIZE,
            None,
            PropertyValue::Unsigned(100),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn file_property_list_stream() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let props = file.property_list();
        assert!(props.contains(&PropertyIdentifier::OBJECT_IDENTIFIER));
        assert!(props.contains(&PropertyIdentifier::OBJECT_NAME));
        assert!(props.contains(&PropertyIdentifier::OBJECT_TYPE));
        assert!(props.contains(&PropertyIdentifier::FILE_TYPE));
        assert!(props.contains(&PropertyIdentifier::FILE_SIZE));
        assert!(props.contains(&PropertyIdentifier::MODIFICATION_DATE));
        assert!(props.contains(&PropertyIdentifier::ARCHIVE));
        assert!(props.contains(&PropertyIdentifier::READ_ONLY));
        assert!(props.contains(&PropertyIdentifier::FILE_ACCESS_METHOD));
        assert!(props.contains(&PropertyIdentifier::STATUS_FLAGS));
        assert!(props.contains(&PropertyIdentifier::OUT_OF_SERVICE));
        assert!(props.contains(&PropertyIdentifier::RELIABILITY));
        // RECORD_COUNT should NOT be in property list for stream-access files
        assert!(!props.contains(&PropertyIdentifier::RECORD_COUNT));
    }

    #[test]
    fn file_property_list_record_access() {
        let mut file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        file.set_file_access_method(1);
        let props = file.property_list();
        assert!(props.contains(&PropertyIdentifier::RECORD_COUNT));
    }

    #[test]
    fn file_unknown_property_error() {
        let file = FileObject::new(1, "FILE-1", "text/plain").unwrap();
        let result = file.read_property(PropertyIdentifier::PRESENT_VALUE, None);
        assert!(result.is_err());
        if let Err(Error::Protocol { code, .. }) = result {
            assert_eq!(code, ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32);
        } else {
            panic!("expected Protocol error");
        }
    }
}
