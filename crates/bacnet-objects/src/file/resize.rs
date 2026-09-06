//! Private property-resize authority and compatibility snapshot state.
//!
//! Service 16 no longer invokes the retained object-local snapshot hooks.

use super::{common, file_full, FileObject};
use crate::traits::WritePropertyRollback;
use bacnet_types::enums::{FileAccessMethod, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, PropertyValue, Time};

enum FileResizeRollback {
    Stream {
        data: Vec<u8>,
        file_size: u64,
        record_count: Option<u64>,
        modification_date: (Date, Time),
        archive: bool,
    },
    Record {
        records: Vec<Vec<u8>>,
        file_size: u64,
        record_count: Option<u64>,
        modification_date: (Date, Time),
        archive: bool,
    },
}

pub(super) fn write_stream(file: &mut FileObject, value: PropertyValue) -> Result<(), Error> {
    if file.file_access_method != FileAccessMethod::STREAM_ACCESS.to_raw() || file.read_only {
        return Err(common::write_access_denied_error());
    }
    let PropertyValue::Unsigned(target) = value else {
        return Err(common::invalid_data_type_error());
    };
    let current = file.data.len() as u64;
    if target == current {
        return Ok(());
    }
    if target > file.max_file_size.max(current) {
        return Err(file_full());
    }
    let target = usize::try_from(target).map_err(|_| file_full())?;
    let mut updated = file.data.clone();
    updated.resize(target, 0);
    file.set_data(updated);
    Ok(())
}

pub(super) fn write_records(file: &mut FileObject, value: PropertyValue) -> Result<(), Error> {
    if file.file_access_method != FileAccessMethod::RECORD_ACCESS.to_raw() || file.read_only {
        return Err(common::write_access_denied_error());
    }
    let PropertyValue::Unsigned(target) = value else {
        return Err(common::invalid_data_type_error());
    };
    let current = file.records.len() as u64;
    if target == current {
        return Ok(());
    }
    if target > file.max_record_count.max(current) {
        return Err(file_full());
    }
    let target = usize::try_from(target).map_err(|_| file_full())?;
    let mut updated = file.records.clone();
    updated.resize(target, Vec::new());
    file.set_records(updated);
    Ok(())
}

pub(super) fn is_writable(file: &FileObject, property: PropertyIdentifier) -> bool {
    matches!(
        property,
        PropertyIdentifier::DESCRIPTION
            | PropertyIdentifier::OUT_OF_SERVICE
            | PropertyIdentifier::ARCHIVE
            | PropertyIdentifier::FILE_TYPE
    ) || (property == PropertyIdentifier::FILE_SIZE
        && file.file_access_method == FileAccessMethod::STREAM_ACCESS.to_raw()
        && !file.read_only)
        || (property == PropertyIdentifier::RECORD_COUNT
            && file.file_access_method == FileAccessMethod::RECORD_ACCESS.to_raw()
            && !file.read_only)
}

pub(super) fn capture(
    file: &FileObject,
    property: PropertyIdentifier,
    value: &PropertyValue,
) -> Option<WritePropertyRollback> {
    match (property, value) {
        (PropertyIdentifier::FILE_SIZE, PropertyValue::Unsigned(target))
            if file.file_access_method == FileAccessMethod::STREAM_ACCESS.to_raw()
                && !file.read_only
                && *target != file.data.len() as u64 =>
        {
            Some(WritePropertyRollback::new(FileResizeRollback::Stream {
                data: file.data.clone(),
                file_size: file.file_size,
                record_count: file.record_count,
                modification_date: file.modification_date,
                archive: file.archive,
            }))
        }
        (PropertyIdentifier::RECORD_COUNT, PropertyValue::Unsigned(target))
            if file.file_access_method == FileAccessMethod::RECORD_ACCESS.to_raw()
                && !file.read_only
                && *target != file.records.len() as u64 =>
        {
            Some(WritePropertyRollback::new(FileResizeRollback::Record {
                records: file.records.clone(),
                file_size: file.file_size,
                record_count: file.record_count,
                modification_date: file.modification_date,
                archive: file.archive,
            }))
        }
        _ => None,
    }
}

pub(super) fn restore(file: &mut FileObject, rollback: WritePropertyRollback) -> Result<(), Error> {
    match rollback.downcast::<FileResizeRollback>()? {
        FileResizeRollback::Stream {
            data,
            file_size,
            record_count,
            modification_date,
            archive,
        } => {
            file.data = data;
            file.file_size = file_size;
            file.record_count = record_count;
            file.modification_date = modification_date;
            file.archive = archive;
        }
        FileResizeRollback::Record {
            records,
            file_size,
            record_count,
            modification_date,
            archive,
        } => {
            file.records = records;
            file.file_size = file_size;
            file.record_count = record_count;
            file.modification_date = modification_date;
            file.archive = archive;
        }
    }
    Ok(())
}
