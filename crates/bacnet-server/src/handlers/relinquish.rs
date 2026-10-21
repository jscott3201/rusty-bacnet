//! A NULL written to a property that isn't commandable and whose datatype
//! has no NULL in it (Clauses 15.9.2, 15.10.2 and 19.2.1).
//!
//! The standard treats such a write as a relinquish there is nothing to
//! relinquish from: if the request is otherwise sound, the property keeps
//! its value and the write counts as a success. WriteProperty,
//! WritePropertyMultiple, the server's local write path (`write_local`,
//! `write_local_encoded` and a Command's writes), CreateObject's initial
//! values and a Schedule's writes to its targets settle it here, for every
//! object, rather than each object deciding for itself. Clause 19.2.1 holds
//! local actions to the same prioritization rules as service requests, so
//! the local paths follow the services.
//!
//! The write still goes to the object first, so every check the object
//! makes before it looks at the value keeps its answer: an unknown property,
//! a property that isn't writable or isn't writable in the object's state,
//! an array index out of range or an array whose size is fixed. Only a
//! refusal of the NULL as a value of the wrong datatype, PROPERTY /
//! INVALID_DATA_TYPE, becomes a success, and then only when
//!
//! - the property isn't commandable on this object (where a NULL takes a
//!   command out of the Priority_Array), and
//! - no member of its datatype is an application NULL ([`null_in_datatype`]).
//!
//! A property the object takes a NULL for never reaches this rule: a
//! commandable Present_Value relinquishing, a Schedule_Default storing NULL.
//! A reference property is no exception: an unset one reads, and is cleared
//! by writing, a reference to the reserved instance 4194303 (#1417), so a
//! NULL there comes here like on any other property.
//!
//! The value counts as a NULL when its whole encoding is one application
//! NULL. That covers a raw-octet property such as Recipient_List or an
//! Access Rights rule array, which reaches the object as those octets, and a
//! BACnetLIST written whole, which reaches it as a list of one NULL.
//!
//! An array element written with a NULL is judged the same way, against the
//! element's datatype, once the object has checked the index; index 0, the
//! array's size, is an Unsigned.
//!
//! Nothing changed, so nothing follows the write as a change: no COV report,
//! event pass, Schedule re-evaluation, Command run or save. An Audit
//! Reporter records it as the successful write the requester was told it is
//! (Clause 19.6.1). A CreateObject goes on to its next initial value, and a
//! Schedule counts the target as one that took its write.

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

/// The one encoding the rule applies to: an application-tagged NULL.
const NULL_OCTETS: [u8; 1] = [0x00];

/// Whether a write's encoded value is one application NULL and nothing else.
pub(crate) fn is_null_octets(octets: &[u8]) -> bool {
    octets == NULL_OCTETS
}

/// Whether `value` encodes as one application NULL: `Null` itself, the raw
/// octets a raw-octet property receives for one, or a list of one NULL.
pub(crate) fn is_null_value(value: &PropertyValue) -> bool {
    match value {
        PropertyValue::Null => true,
        PropertyValue::ApplicationData(octets) => is_null_octets(octets),
        PropertyValue::List(values) => matches!(values.as_slice(), [only] if is_null_value(only)),
        _ => false,
    }
}

/// Whether `error`, the object's refusal of a NULL written to `property` (at
/// `array_index`), makes the write a success that leaves the property as it
/// is. The caller has checked that the written value is a NULL.
pub(crate) fn leaves_unchanged(
    object: &dyn BACnetObject,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    error: &Error,
) -> bool {
    refused_as_datatype(error)
        && !commandable(object, property)
        && !null_in_datatype(
            object.object_identifier().object_type(),
            property,
            array_index,
        )
}

/// PROPERTY / INVALID_DATA_TYPE, plain or naming the list element refused.
fn refused_as_datatype(error: &Error) -> bool {
    let (Error::Protocol { class, code } | Error::Structured { class, code, .. }) = error else {
        return false;
    };
    *class == ErrorClass::PROPERTY && *code == ErrorCode::INVALID_DATA_TYPE
}

/// Present_Value is the standard commandable property (Clause 19.2.1.1): on
/// an object that keeps a Priority_Array for it, and on a Channel, which
/// passes its commands on without one (Clause 19.2.1.6).
fn commandable(object: &dyn BACnetObject, property: PropertyIdentifier) -> bool {
    property == PropertyIdentifier::PRESENT_VALUE
        && (object.object_identifier().object_type() == ObjectType::CHANNEL
            || object
                .property_list()
                .contains(&PropertyIdentifier::PRIORITY_ARRAY))
}

/// Whether an application NULL is a value of `property` on an object of
/// `object_type`, or of one element of it when it is an array or list.
///
/// These are the standard properties whose Clause 21 datatype is a CHOICE
/// with an untagged NULL member (BACnetPriorityValue, BACnetChannelValue,
/// BACnetClientCOV, BACnetProcessIdSelection, BACnetTimerStateChangeValue,
/// BACnetObjectSelector and the BACnetOptional types), and the Schedule's
/// Present_Value and Schedule_Default, which take a value of any datatype
/// (Clause 12.24). A CHOICE whose NULL member is context-tagged, such as
/// BACnetValueSource or BACnetFaultParameter, has no application NULL.
/// Index 0 of an array is its size, an Unsigned.
///
/// Most entries match on the property alone, whatever the object type, on
/// purpose: a property this names keeps its NULL away from the rule, so the
/// object's own answer stands. Naming too many is the safe direction; naming
/// too few would turn a refusal of a NULL the datatype allows into a
/// success that discards it.
pub(crate) fn null_in_datatype(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
) -> bool {
    if array_index == Some(0) {
        return false;
    }
    match property {
        PropertyIdentifier::PRESENT_VALUE => {
            matches!(object_type, ObjectType::SCHEDULE | ObjectType::CHANNEL)
        }
        PropertyIdentifier::SCHEDULE_DEFAULT => object_type == ObjectType::SCHEDULE,
        PropertyIdentifier::ALARM_VALUES | PropertyIdentifier::FAULT_VALUES => {
            object_type == ObjectType::CHARACTERSTRING_VALUE
        }
        PropertyIdentifier::PRIORITY_ARRAY
        | PropertyIdentifier::AUDIT_PRIORITY_FILTER
        | PropertyIdentifier::CLIENT_COV_INCREMENT
        | PropertyIdentifier::CURRENT_COMMAND_PRIORITY
        | PropertyIdentifier::INTERFACE_VALUE
        | PropertyIdentifier::LOW_DIFF_LIMIT
        | PropertyIdentifier::MONITORED_OBJECTS
        | PropertyIdentifier::PROCESS_IDENTIFIER_FILTER
        | PropertyIdentifier::STATE_CHANGE_VALUES => true,
        _ => false,
    }
}
