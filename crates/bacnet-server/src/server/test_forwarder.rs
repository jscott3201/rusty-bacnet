//! A Notification Forwarder reduced to its identity and Subscribed_Recipients,
//! for the server's tests (#1049).
//!
//! It stands for an application's own forwarder type, which keeps the list in
//! a [`SubscribedRecipients`] and routes the property and the monotonic clock
//! hooks to it, as this one does, so the list services, reads and the
//! operation task can be driven against it. Having no filter rows, it takes no
//! notifications to forward.

use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

use bacnet_objects::subscribed_recipients::SubscribedRecipients;
use bacnet_objects::traits::{BACnetObject, MonotonicClock};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

const PROPERTIES: &[PropertyIdentifier] = &[
    PropertyIdentifier::OBJECT_IDENTIFIER,
    PropertyIdentifier::OBJECT_NAME,
    PropertyIdentifier::OBJECT_TYPE,
    PropertyIdentifier::SUBSCRIBED_RECIPIENTS,
];

pub(crate) struct TestForwarder {
    oid: ObjectIdentifier,
    pub(crate) subscribed_recipients: SubscribedRecipients,
}

impl TestForwarder {
    pub(crate) fn new(instance: u32) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::NOTIFICATION_FORWARDER, instance).unwrap(),
            subscribed_recipients: SubscribedRecipients::new(),
        }
    }
}

fn refusal(class: ErrorClass, code: ErrorCode) -> Error {
    Error::Protocol { class, code }
}

impl BACnetObject for TestForwarder {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "Forwarder"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::NOTIFICATION_FORWARDER.to_raw(),
            )),
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS => Ok(self.subscribed_recipients.read()),
            _ => Err(refusal(ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        match property {
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS if array_index.is_none() => {
                self.subscribed_recipients.write(value)
            }
            _ => Err(refusal(
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED,
            )),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(PROPERTIES)
    }

    fn advance_time_internal(&mut self, elapsed: Duration) -> bool {
        self.subscribed_recipients.advance_by(elapsed)
    }

    fn bind_monotonic_clock_internal(&mut self, clock: Option<Arc<MonotonicClock>>) {
        self.subscribed_recipients.bind_monotonic_clock(clock);
    }

    fn advance_monotonic_time_internal(&mut self, now: Duration) -> bool {
        self.subscribed_recipients.advance_to(now)
    }

    fn next_monotonic_deadline_internal(&self) -> Option<Duration> {
        self.subscribed_recipients.next_deadline()
    }
}
