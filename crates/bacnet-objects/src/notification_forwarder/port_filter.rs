//! Port_Filter reads and writes (Clause 12.51.11): a BACnetARRAY of
//! BACnetPortPermission whose size and Port_IDs are fixed, so a write changes
//! only the Enabled members.

use crate::prelude::*;
use bacnet_encoding::constructed::{decode_port_permission, encode_port_permission};
use bacnet_types::constructed::BACnetPortPermission;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

use crate::common;

fn encoded(permission: &BACnetPortPermission) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_port_permission(&mut buf, permission);
    PropertyValue::ApplicationData(buf.to_vec())
}

pub(super) fn read(
    ports: &[BACnetPortPermission],
    array_index: Option<u32>,
) -> Result<PropertyValue, Error> {
    common::read_array(ports.iter().map(encoded).collect(), array_index)
}

/// One written element: exactly one framed BACnetPortPermission.
fn decode(value: &PropertyValue) -> Result<BACnetPortPermission, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    match decode_port_permission(bytes, 0) {
        Ok((permission, end)) if end == bytes.len() => Ok(permission),
        _ => Err(common::invalid_data_type_error()),
    }
}

/// Take the Enabled member of `written` for the element holding `held`'s
/// port. A written element naming another port is refused with PROPERTY /
/// VALUE_OUT_OF_RANGE, since the Port_IDs are not writable.
fn take(held: &mut BACnetPortPermission, written: BACnetPortPermission) -> Result<(), Error> {
    if written.port_id != held.port_id {
        return Err(common::value_out_of_range_error());
    }
    held.enabled = written.enabled;
    Ok(())
}

/// Apply a write, all or nothing. A whole-array write must give one element
/// per port, in the same order (PROPERTY / VALUE_OUT_OF_RANGE otherwise), and
/// the array size, index 0, is not writable (WRITE_ACCESS_DENIED).
pub(super) fn write(
    ports: &mut [BACnetPortPermission],
    array_index: Option<u32>,
    value: PropertyValue,
) -> Result<(), Error> {
    match array_index {
        None => {
            let written = match &value {
                PropertyValue::List(elements) => {
                    elements.iter().map(decode).collect::<Result<Vec<_>, _>>()?
                }
                single @ PropertyValue::ApplicationData(_) => vec![decode(single)?],
                _ => return Err(common::invalid_data_type_error()),
            };
            if written.len() != ports.len() {
                return Err(common::value_out_of_range_error());
            }
            let mut next = ports.to_vec();
            for (held, written) in next.iter_mut().zip(written) {
                take(held, written)?;
            }
            ports.copy_from_slice(&next);
            Ok(())
        }
        Some(0) => Err(common::write_access_denied_error()),
        Some(index) => {
            let Some(held) = usize::try_from(index - 1)
                .ok()
                .and_then(|slot| ports.get_mut(slot))
            else {
                return Err(common::invalid_array_index_error());
            };
            take(held, decode(&value)?)
        }
    }
}
