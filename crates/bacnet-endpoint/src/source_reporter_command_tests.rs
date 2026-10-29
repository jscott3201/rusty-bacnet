//! A writable decorator must preserve source authority and its own write gate.
use super::*;
use bacnet_objects::command_source::CommandOrigin;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;
use std::sync::Mutex;

struct RecordingObject(Arc<Mutex<Vec<(PropertyIdentifier, CommandOrigin)>>>);
impl BACnetObject for RecordingObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
    }
    fn object_name(&self) -> &str {
        "source-forwarding"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
    fn read_property(&self, _: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Null)
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("the source-aware override must be forwarded without falling back")
    }
    fn write_property_from(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
        origin: &CommandOrigin,
    ) -> Result<(), Error> {
        assert_eq!(i, Some(2));
        assert_eq!(priority, Some(8));
        assert_eq!(v, PropertyValue::Unsigned(99));
        self.0.lock().unwrap().push((p, origin.clone()));
        Ok(())
    }
}
#[test]
fn command_source_decorator_forwards_exact_origin_and_preserves_active_gate() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut object: Box<dyn BACnetObject> = Box::new(RecordingObject(seen.clone()));
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 5).unwrap();
    let reporter = ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap();
    let owner = AuditOwnership::for_source(device, reporter);
    install(&mut object, &owner).unwrap();
    let origin = CommandOrigin::Local {
        owner_device: device,
        initiating_object: Some(object.object_identifier()),
    };
    object
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            Some(2),
            PropertyValue::Unsigned(99),
            Some(8),
            &origin,
        )
        .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [(PropertyIdentifier::PRESENT_VALUE, origin.clone())]
    );
    for sourced in [false, true] {
        let result = if sourced {
            object.write_property_from(
                PropertyIdentifier::AUDIT_SOURCE_REPORTER,
                Some(2),
                PropertyValue::Unsigned(99),
                Some(8),
                &origin,
            )
        } else {
            object.write_property(
                PropertyIdentifier::AUDIT_SOURCE_REPORTER,
                Some(2),
                PropertyValue::Unsigned(99),
                Some(8),
            )
        };
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::WRITE_ACCESS_DENIED)
        );
    }
    assert_eq!(seen.lock().unwrap().len(), 1);
    drop(owner);
    object
        .write_property_from(
            PropertyIdentifier::AUDIT_SOURCE_REPORTER,
            Some(2),
            PropertyValue::Unsigned(99),
            Some(8),
            &origin,
        )
        .unwrap();
    assert_eq!(seen.lock().unwrap().len(), 2);
}
