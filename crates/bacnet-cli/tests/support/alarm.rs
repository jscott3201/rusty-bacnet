//! An event-initiating object with fixed event data, for the alarm
//! commands' tests (#2077).

use std::borrow::Cow;

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{
    ErrorClass, ErrorCode, EventState, NotifyType, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// An event-initiating object with a fixed event state, acknowledged
/// transitions and timestamps, so the output doesn't depend on the clock.
pub struct FixedAlarm {
    oid: ObjectIdentifier,
    name: String,
    state: EventState,
    /// Acked_Transitions, first bit first.
    acked: u8,
    notify: NotifyType,
    /// Its Notification_Class.
    pub class: u32,
}

impl FixedAlarm {
    /// AV `instance` in `state`, Notification_Class 5.
    pub fn new(instance: u32, state: EventState, acked: u8, notify: NotifyType) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap(),
            name: format!("Alarm {instance}"),
            state,
            acked,
            notify,
            class: 5,
        }
    }
}

/// `[0] Time 14:30:05` and the like, context tagged as Event_Time_Stamps
/// holds them: a date and time, a sequence number, and a time.
fn timestamps() -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::ApplicationData(vec![0x2E, 0xA4, 126, 10, 25, 7, 0xB4, 14, 30, 5, 0, 0x2F]),
        PropertyValue::ApplicationData(vec![0x19, 42]),
        PropertyValue::ApplicationData(vec![0x0C, 7, 0, 0, 0]),
    ])
}

impl BACnetObject for FixedAlarm {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Ok(match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(self.oid),
            PropertyIdentifier::OBJECT_NAME => PropertyValue::CharacterString(self.name.clone()),
            PropertyIdentifier::OBJECT_TYPE => {
                PropertyValue::Enumerated(ObjectType::ANALOG_VALUE.to_raw())
            }
            PropertyIdentifier::EVENT_STATE => PropertyValue::Enumerated(self.state.to_raw()),
            PropertyIdentifier::ACKED_TRANSITIONS => PropertyValue::BitString {
                unused_bits: 5,
                data: vec![self.acked << 5],
            },
            PropertyIdentifier::EVENT_TIME_STAMPS => timestamps(),
            PropertyIdentifier::NOTIFY_TYPE => PropertyValue::Enumerated(self.notify.to_raw()),
            PropertyIdentifier::EVENT_ENABLE => PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xE0],
            },
            PropertyIdentifier::NOTIFICATION_CLASS => {
                PropertyValue::Unsigned(u64::from(self.class))
            }
            _ => {
                return Err(Error::protocol(
                    ErrorClass::PROPERTY,
                    ErrorCode::UNKNOWN_PROPERTY,
                ));
            }
        })
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::ACKED_TRANSITIONS,
            PropertyIdentifier::EVENT_TIME_STAMPS,
            PropertyIdentifier::NOTIFY_TYPE,
            PropertyIdentifier::EVENT_ENABLE,
            PropertyIdentifier::NOTIFICATION_CLASS,
        ])
    }
}
