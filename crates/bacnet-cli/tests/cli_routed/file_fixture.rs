//! FileObject-backed observations of each service window, including a later failure.
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::file::{
    FileConfiguration, FileObject, FileRecordRead, FileStorage, FileStreamRead, FileWriteStart,
};
use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, FileAccessMethod, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

pub(super) const STREAM: &[u8] = &[0x01, 0x7f, 0x80, 0xff, 0, 0x31, 0xa5];
pub(super) fn records() -> Vec<Vec<u8>> {
    vec![
        vec![0, 0xff, 0x80],
        vec![],
        vec![0x31],
        vec![0x7f, 0],
        vec![0xa5; 4],
    ]
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Window {
    pub instance: u32,
    pub start: u64,
    pub requested: u64,
    /// Returned octets or records, followed by EOF. None means a protocol error.
    pub returned: Option<(usize, bool)>,
}
pub(super) type Observations = Arc<Mutex<Vec<Window>>>;

pub(super) fn database() -> (ObjectDatabase, Observations) {
    let mut db = super::read_services::database();
    let observations = Arc::new(Mutex::new(Vec::new()));
    for instance in 10..=14 {
        let mut file =
            FileObject::new(instance, format!("Routed file {instance}"), "binary").unwrap();
        if matches!(instance, 11 | 14) {
            file.set_access_method(FileAccessMethod::RECORD_ACCESS);
            file.set_record_data(records()).unwrap();
        } else {
            file.set_stream_data(STREAM.to_vec()).unwrap();
        }
        file.set_read_only(instance == 13);
        db.add(Box::new(ObservedFile {
            inner: file,
            observations: Arc::clone(&observations),
            fail_at: matches!(instance, 12 | 14).then_some(2),
        }))
        .unwrap();
    }
    (db, observations)
}

pub(super) fn oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::FILE, instance).unwrap()
}

struct ObservedFile {
    inner: FileObject,
    observations: Observations,
    fail_at: Option<u64>,
}

impl ObservedFile {
    fn observe(&self, start: u64, requested: u64, returned: Option<(usize, bool)>) {
        self.observations.lock().unwrap().push(Window {
            instance: self.inner.object_identifier().instance_number(),
            start,
            requested,
            returned,
        });
    }
    fn refuse(&self, start: u64, count: u64) -> Result<(), Error> {
        if self.fail_at == Some(start) {
            self.observe(start, count, None);
            return Err(Error::protocol(
                ErrorClass::SERVICES,
                ErrorCode::FILE_ACCESS_DENIED,
            ));
        }
        Ok(())
    }
}

impl FileStorage for ObservedFile {
    fn read_stream(&self, start: u64, count: u64) -> Result<FileStreamRead, Error> {
        self.refuse(start, count)?;
        let result = self.inner.read_stream(start, count)?;
        self.observe(start, count, Some((result.data.len(), result.end_of_file)));
        Ok(result)
    }
    fn read_records(&self, start: u64, count: u64) -> Result<FileRecordRead, Error> {
        self.refuse(start, count)?;
        let result = self.inner.read_records(start, count)?;
        self.observe(
            start,
            count,
            Some((result.records.len(), result.end_of_file)),
        );
        Ok(result)
    }
    fn write_stream(&mut self, start: FileWriteStart, data: &[u8]) -> Result<u64, Error> {
        self.inner.write_stream(start, data)
    }
    fn write_records(&mut self, start: FileWriteStart, records: &[Vec<u8>]) -> Result<u64, Error> {
        self.inner.write_records(start, records)
    }
}

impl BACnetObject for ObservedFile {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.inner.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(property, index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        self.inner.property_metadata()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        Some(self)
    }
    fn file_storage_internal_mut(&mut self) -> Option<&mut dyn FileStorage> {
        Some(self)
    }
}
