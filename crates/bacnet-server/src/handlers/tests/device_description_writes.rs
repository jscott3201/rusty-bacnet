use super::*;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

#[test]
fn device_description_full_handler_null_is_noop_and_priority_range_is_typed() {
    let mut device = DeviceObject::new(DeviceConfig::default()).unwrap();
    device.set_description("retained");
    let oid = device.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device)).unwrap();
    for priority in [None, Some(1), Some(16), Some(0), Some(17)] {
        let mut bytes = BytesMut::new();
        WritePropertyRequest {
            object_identifier: oid,
            property_identifier: PropertyIdentifier::DESCRIPTION,
            property_array_index: None,
            property_value: vec![0],
            priority: None,
        }
        .encode(&mut bytes)
        .unwrap();
        // Independent inbound vector: malformed peers are not typed encoders.
        if let Some(priority) = priority {
            bacnet_encoding::primitives::encode_ctx_unsigned(&mut bytes, 4, priority);
        }
        let result = handle_write_property(&mut db, &bytes);
        if matches!(priority, Some(0 | 17)) {
            assert!(
                matches!(result, Err(Error::Protocol {class,code}) if class == ErrorClass::SERVICES.to_raw() as u32 && code == ErrorCode::PARAMETER_OUT_OF_RANGE.to_raw() as u32)
            );
        } else {
            assert_eq!(result.unwrap(), oid);
        }
        assert_eq!(
            db.get(&oid)
                .unwrap()
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap(),
            PropertyValue::CharacterString("retained".into())
        );
    }
}
