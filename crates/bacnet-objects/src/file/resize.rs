//! Private property-resize authority.

use super::{common, file_full, FileObject};
use bacnet_types::enums::{FileAccessMethod, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

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
