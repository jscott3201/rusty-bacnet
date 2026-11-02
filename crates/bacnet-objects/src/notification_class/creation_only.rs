//! What a CreateObject initial value may set on a new Notification Class
//! that WriteProperty can't change afterwards (#2208).
//!
//! Object_Name, Priority and Ack_Required are read-only over the network,
//! yet a client that creates a class names it and gives the other two for
//! its notifications to mean anything, so all three take an initial value.
//! Notification_Class and Recipient_List are writable and go the write
//! route.

use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::NotificationClass;
use crate::common;

/// The properties [`initialize`] sets.
pub(super) const CREATION_ONLY: &[PropertyIdentifier] = &[
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::PRIORITY,
    PropertyIdentifier::ACK_REQUIRED,
];

/// Apply a whole Object_Name, Priority or Ack_Required initial value to
/// `class`.
///
/// Object_Name is a non-empty CharacterString, as a rename elsewhere is.
/// Priority is the three priorities, for TO_OFFNORMAL, TO_FAULT and
/// TO_NORMAL in that order, each an Unsigned up to 255: any other count or a
/// larger value is VALUE_OUT_OF_RANGE, and any other datatype
/// INVALID_DATA_TYPE. Ack_Required is the three-bit transition set, held to
/// its one encoding as an Event_Enable write is. On a refusal the class is
/// unchanged.
pub(super) fn initialize(
    class: &mut NotificationClass,
    property: PropertyIdentifier,
    value: PropertyValue,
) -> Result<(), Error> {
    match property {
        PropertyIdentifier::OBJECT_NAME => common::initialize_object_name(&mut class.name, &value),
        PropertyIdentifier::PRIORITY => {
            class.priority = priorities(value)?;
            Ok(())
        }
        PropertyIdentifier::ACK_REQUIRED => {
            let PropertyValue::BitString { unused_bits, data } = value else {
                return Err(common::invalid_data_type_error());
            };
            let byte = common::check_fixed_width_bit_string(unused_bits, &data, 3)?;
            class.ack_required = EventTransitionBits::from_bacnet(&[byte]);
            Ok(())
        }
        _ => Err(common::write_access_denied_error()),
    }
}

fn priorities(value: PropertyValue) -> Result<[u8; 3], Error> {
    let PropertyValue::List(elements) = value else {
        // One Unsigned alone is the wrong count; anything else the wrong
        // datatype.
        return Err(match value {
            PropertyValue::Unsigned(_) => common::value_out_of_range_error(),
            _ => common::invalid_data_type_error(),
        });
    };
    let mut priorities = [0u8; 3];
    if elements.len() != priorities.len() {
        return Err(common::value_out_of_range_error());
    }
    for (slot, element) in priorities.iter_mut().zip(elements) {
        let PropertyValue::Unsigned(priority) = element else {
            return Err(common::invalid_data_type_error());
        };
        *slot = u8::try_from(priority).map_err(|_| common::value_out_of_range_error())?;
    }
    Ok(priorities)
}
