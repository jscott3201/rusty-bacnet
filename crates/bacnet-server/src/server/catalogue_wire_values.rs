//! The check each value an RPM ALL reads over the wire gets (#2175): it
//! decodes, encodes back, has its row's datatype and equals the object's
//! own read.

use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::rpm::ReadResultElement;
use bacnet_types::datatype::Datatype;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use bytes::BytesMut;

/// Whether `value` is an application value of the primitive `datatype`.
/// `None` for a datatype that isn't a primitive, whose elements this check
/// doesn't type.
fn of_datatype(datatype: Datatype, value: &PropertyValue) -> Option<bool> {
    use PropertyValue as V;
    Some(match datatype {
        Datatype::Null => matches!(value, V::Null),
        Datatype::Boolean => matches!(value, V::Boolean(_)),
        Datatype::Unsigned => matches!(value, V::Unsigned(_)),
        Datatype::Integer => matches!(value, V::Signed(_)),
        Datatype::Real => matches!(value, V::Real(_)),
        Datatype::Double => matches!(value, V::Double(_)),
        Datatype::OctetString => matches!(value, V::OctetString(_)),
        Datatype::CharacterString => matches!(value, V::CharacterString(_)),
        Datatype::BitString => matches!(value, V::BitString { .. }),
        Datatype::Enumerated => matches!(value, V::Enumerated(_)),
        Datatype::Date => matches!(value, V::Date(_)),
        Datatype::Time => matches!(value, V::Time(_)),
        Datatype::ObjectIdentifier => matches!(value, V::ObjectIdentifier(_)),
        _ => return None,
    })
}

/// Whether `result` is the refusal a log's Log_Buffer read gives: only
/// ReadRange (and AuditLogQuery) serve it, so a read answers PROPERTY /
/// READ_ACCESS_DENIED. The logs are the objects that lend the server their
/// records for those services, Trend and Event Logs through `log_buffer`
/// and the Audit Log through `audit_log`; the list of types that do is a
/// bacnet-objects test's, out of reach here.
pub(super) fn range_only_refusal(object: &dyn BACnetObject, result: &ReadResultElement) -> bool {
    result.property_identifier == PropertyIdentifier::LOG_BUFFER
        && (object.log_buffer().is_some() || object.audit_log().is_some())
        && result.property_value.is_none()
        && result.error == Some((ErrorClass::PROPERTY, ErrorCode::READ_ACCESS_DENIED))
}

/// Check one result of an RPM ALL on `object`, whose row is `row`.
pub(super) fn check(
    object: &dyn BACnetObject,
    row: &PropertyMetadata,
    result: &ReadResultElement,
) -> Result<(), String> {
    let object_type = object.object_identifier().object_type();
    let property = row.property_identifier;
    if range_only_refusal(object, result) {
        return Ok(());
    }
    if let Some(error) = result.error {
        return Err(format!("read failed: {error:?}"));
    }
    let octets = result
        .property_value
        .as_deref()
        .ok_or("neither a value nor an error")?;

    // Decode into elements, each application-tagged or one whole
    // context-tagged element, until the octets run out.
    let mut elements = Vec::new();
    let mut offset = 0;
    while offset < octets.len() {
        let (element, next) = decode_application_value(octets, offset)
            .map_err(|error| format!("{octets:02X?} at {offset}: {error:?}"))?;
        elements.push(element);
        offset = next;
    }
    let mut again = BytesMut::new();
    for element in &elements {
        encode_property_value(&mut again, element).map_err(|error| format!("{error:?}"))?;
    }
    if again[..] != octets[..] {
        return Err(format!("{octets:02X?} encodes back as {:02X?}", &again[..]));
    }

    if let Some(value_type) = row.effective_value_type(object_type) {
        let single = !value_type.is_array() && !value_type.is_list();
        if single
            && elements.len() != 1
            && of_datatype(value_type.datatype, &PropertyValue::Null).is_some()
        {
            return Err(format!("one value expected, decoded {elements:?}"));
        }
        for element in &elements {
            let admitted =
                matches!(element, PropertyValue::Null) && value_type.datatype.admits_null();
            if of_datatype(value_type.datatype, element) == Some(false) && !admitted {
                return Err(format!("{element:?} is no {:?}", value_type.datatype));
            }
        }
    }

    // The object's own read, encoded, is what the server sent.
    let direct = object
        .read_property(property, None)
        .map_err(|error| format!("the object's own read failed: {error:?}"))?;
    let mut expected = BytesMut::new();
    encode_property_value(&mut expected, &direct).map_err(|error| format!("{error:?}"))?;
    if expected[..] != octets[..] {
        return Err(format!("served {octets:02X?}, the object reads {direct:?}"));
    }
    Ok(())
}
