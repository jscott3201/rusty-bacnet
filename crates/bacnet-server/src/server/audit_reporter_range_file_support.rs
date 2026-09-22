use super::*;
use bacnet_objects::file::{
    FileObject, FileRecordRead, FileStorage, FileStreamRead, FileWriteStart,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Range,
    Stream,
    Record,
}

impl Kind {
    pub(super) fn service(self) -> ConfirmedServiceChoice {
        if self == Self::Range {
            ConfirmedServiceChoice::READ_RANGE
        } else {
            ConfirmedServiceChoice::ATOMIC_READ_FILE
        }
    }

    pub(super) fn target(self) -> ObjectIdentifier {
        oid(
            if self == Self::Range {
                ObjectType::ANALOG_VALUE
            } else {
                ObjectType::FILE
            },
            9,
        )
    }

    pub(super) fn request(self, start: i32, count: u32) -> Bytes {
        let mut out = BytesMut::new();
        if self == Self::Range {
            range_request(
                self.target(),
                PropertyIdentifier::LOG_BUFFER,
                None,
                Some(RangeSpec::ByPosition {
                    reference_index: start as u32,
                    count: count as i32,
                }),
            )
            .encode(&mut out);
        } else {
            AtomicReadFileRequest {
                file_identifier: self.target(),
                access: if self == Self::Stream {
                    FileAccessMethod::Stream {
                        file_start_position: start,
                        requested_octet_count: count,
                    }
                } else {
                    FileAccessMethod::Record {
                        file_start_record: start,
                        requested_record_count: count,
                    }
                },
            }
            .encode(&mut out);
        }
        out.freeze()
    }

    pub(super) fn expected(
        self,
        invoke: u8,
        result: Option<(ErrorClass, ErrorCode)>,
    ) -> BACnetAuditNotification {
        let mut record = expected(
            self.target(),
            PropertyIdentifier::LOG_BUFFER,
            None,
            invoke,
            0,
            result,
        );
        if self != Self::Range {
            record.target_property = None;
        }
        record
    }
}

pub(super) fn range_request(
    target: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    range: Option<RangeSpec>,
) -> ReadRangeRequest {
    ReadRangeRequest {
        object_identifier: target,
        property_identifier: property,
        property_array_index: index,
        range,
    }
}

struct Probe {
    kind: Kind,
    file: FileObject,
    reads: Arc<AtomicUsize>,
    failure: Option<fn() -> Error>,
    large: bool,
}

impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.kind.target()
    }
    fn object_name(&self) -> &str {
        "read target"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn is_array_property(&self, property: PropertyIdentifier) -> bool {
        property == PropertyIdentifier::WEEKLY_SCHEDULE
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if self.kind != Kind::Range {
            assert_eq!(
                property,
                PropertyIdentifier::FILE_ACCESS_METHOD,
                "no extra file metadata reads"
            );
            return self.file.read_property(property, index);
        }
        self.reads.fetch_add(1, Ordering::AcqRel);
        if let Some(failure) = self.failure {
            return Err(failure());
        }
        match property {
            PropertyIdentifier::LOG_BUFFER
            | PropertyIdentifier::WEEKLY_SCHEDULE
            | PropertyIdentifier::PRESENT_VALUE => Ok(PropertyValue::List(if self.large {
                vec![PropertyValue::OctetString(vec![42; 2000])]
            } else {
                vec![PropertyValue::Unsigned(11), PropertyValue::Unsigned(22)]
            })),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString("not a list".into()))
            }
            _ => Err(Error::Protocol { class: 2, code: 32 }),
        }
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("read-only fixture")
    }
    fn file_storage_internal(&self) -> Option<&dyn FileStorage> {
        Some(self)
    }
}

impl FileStorage for Probe {
    fn read_stream(&self, start: u64, count: u64) -> Result<FileStreamRead, Error> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        if let Some(failure) = self.failure {
            return Err(failure());
        }
        self.file.read_stream(start, count)
    }
    fn read_records(&self, start: u64, count: u64) -> Result<FileRecordRead, Error> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        if let Some(failure) = self.failure {
            return Err(failure());
        }
        self.file.read_records(start, count)
    }
    fn write_stream(&mut self, _: FileWriteStart, _: &[u8]) -> Result<u64, Error> {
        panic!("no writes")
    }
    fn write_records(&mut self, _: FileWriteStart, _: &[Vec<u8>]) -> Result<u64, Error> {
        panic!("no writes")
    }
}

pub(super) async fn add_target(
    fixture: &Fixture,
    kind: Kind,
    failure: Option<fn() -> Error>,
    large: bool,
) -> Arc<AtomicUsize> {
    let mut file = FileObject::new(9, "read file", "binary").unwrap();
    if kind == Kind::Record {
        file.set_file_access_method(bacnet_types::enums::FileAccessMethod::RECORD_ACCESS.to_raw());
        file.set_records(vec![vec![], vec![1, 2, 3, 4, 5]]);
    } else {
        file.set_data(if large {
            vec![42; 2000]
        } else {
            vec![1, 2, 3, 4, 5]
        });
    }
    let reads = Arc::new(AtomicUsize::new(0));
    fixture
        .server
        .db
        .write()
        .await
        .add(Box::new(Probe {
            kind,
            file,
            reads: reads.clone(),
            failure,
            large,
        }))
        .unwrap();
    reads
}

pub(super) fn assert_idle(fixture: &Fixture) {
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    assert_eq!(fixture.server.notification_transactions.active_count(), 0);
}
