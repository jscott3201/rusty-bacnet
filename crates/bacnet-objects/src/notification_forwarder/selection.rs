//! Which event notifications each forwarder takes, and the destinations it
//! sends them to (Clause 12.51). See [`forwarding_targets`].

use crate::prelude::*;
use bacnet_encoding::constructed::{
    decode_event_notification_subscription, decode_port_permission,
};
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::{EventState, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};

use crate::database::ObjectDatabase;
use crate::event::EventTransition;
use crate::notification_class::filter_recipient_list;
use crate::subscribed_recipients::MAX_SUBSCRIBED_RECIPIENTS;
use crate::traits::BACnetObject;

/// One event notification offered to this device's forwarders.
#[derive(Debug, Clone, Copy)]
pub struct ForwardingInput<'a> {
    /// The process identifier the notification carries.
    pub process_identifier: u32,
    /// The event state after the transition. Its transition (to-offnormal,
    /// to-fault or to-normal) is what Recipient_List entries filter on.
    pub to_state: EventState,
    /// Whether one of this device's own objects generated the notification.
    pub locally_initiated: bool,
    /// The port the notification arrived through, by its Clause 6 port ID,
    /// or `None` when this device handed it to its forwarders itself.
    pub receiving_port: Option<u8>,
    /// The current local day, for Recipient_List day filters.
    pub today: DaysOfWeek,
    /// The current local time, for Recipient_List time windows.
    pub current_time: &'a Time,
}

/// Where this device's forwarders send one notification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForwardingTargets {
    /// The forwarders that took the notification.
    pub forwarders: Vec<ObjectIdentifier>,
    /// Each destination once, as recipient, process identifier and whether
    /// to confirm, in forwarder then list order.
    pub recipients: Vec<(BACnetRecipient, u32, bool)>,
}

/// Select the forwarders that take `input` and their destinations, leaving
/// out the forwarders in `skip` (those that already took this notification
/// earlier in a chain within this device).
///
/// Every NOTIFICATION_FORWARDER object in the database is read through its
/// properties, so an application's own forwarder type takes part as the
/// bundled one does. A forwarder takes a notification when:
///
/// - it is in service;
/// - its Process_Identifier_Filter is NULL or equals the notification's
///   process identifier;
/// - its Local_Forwarding_Only is FALSE, or the notification came from one of
///   this device's own objects;
/// - for a notification that arrived through a network port, its Port_Filter
///   enables that port. A forwarder without Port_Filter takes from the one
///   port a device that does not route has.
///
/// A forwarder that cannot serve one of these properties as the clause types
/// it takes nothing. Its destinations are the Recipient_List entries whose
/// days, times and transitions admit the notification, filtered as a
/// Notification Class filters its own, and every live Subscribed_Recipients
/// entry. A list that does not decode, or runs past its cap, contributes
/// nothing rather than a part of itself.
///
/// The network rules that keep forwarded copies from looping (no copies by
/// global broadcast, none back onto the network a broadcast came from) need
/// each destination's route, so the server applies them as it sends.
pub fn forwarding_targets(
    db: &ObjectDatabase,
    input: &ForwardingInput<'_>,
    skip: &[ObjectIdentifier],
) -> ForwardingTargets {
    let mut selected = ForwardingTargets::default();
    for oid in db.find_by_type(ObjectType::NOTIFICATION_FORWARDER) {
        if skip.contains(&oid) {
            continue;
        }
        let Some(forwarder) = db.get(&oid) else {
            continue;
        };
        if !takes(forwarder, input) {
            continue;
        }
        selected.forwarders.push(oid);
        for destination in destinations(forwarder, input) {
            if !selected.recipients.contains(&destination) {
                selected.recipients.push(destination);
            }
        }
    }
    selected
}

fn takes(forwarder: &dyn BACnetObject, input: &ForwardingInput<'_>) -> bool {
    let read = |property| forwarder.read_property(property, None);
    if matches!(
        read(PropertyIdentifier::OUT_OF_SERVICE),
        Ok(PropertyValue::Boolean(true))
    ) {
        return false;
    }
    let process_matches = match read(PropertyIdentifier::PROCESS_IDENTIFIER_FILTER) {
        Ok(PropertyValue::Null) => true,
        Ok(PropertyValue::Unsigned(filter)) => filter == u64::from(input.process_identifier),
        _ => false,
    };
    let origin_allowed = match read(PropertyIdentifier::LOCAL_FORWARDING_ONLY) {
        Ok(PropertyValue::Boolean(local_only)) => !local_only || input.locally_initiated,
        _ => false,
    };
    let port_enabled = match input.receiving_port {
        None => true,
        Some(port) => match read(PropertyIdentifier::PORT_FILTER) {
            Ok(value) => port_enabled(&value, port),
            Err(_) => true,
        },
    };
    process_matches && origin_allowed && port_enabled
}

/// Whether a served Port_Filter value enables `port`. A port the array does
/// not name, or an array that does not decode, is not enabled.
fn port_enabled(value: &PropertyValue, port: u8) -> bool {
    let elements: &[PropertyValue] = match value {
        PropertyValue::List(elements) => elements,
        single => core::slice::from_ref(single),
    };
    elements.iter().any(|element| {
        let PropertyValue::ApplicationData(bytes) = element else {
            return false;
        };
        matches!(
            decode_port_permission(bytes, 0),
            Ok((permission, end)) if end == bytes.len()
                && permission.port_id == port
                && permission.enabled
        )
    })
}

fn destinations(
    forwarder: &dyn BACnetObject,
    input: &ForwardingInput<'_>,
) -> Vec<(BACnetRecipient, u32, bool)> {
    let mut destinations = match forwarder.read_property(PropertyIdentifier::RECIPIENT_LIST, None) {
        Ok(value) => filter_recipient_list(
            &value,
            EventTransition::for_target_state(input.to_state),
            input.today,
            input.current_time,
        ),
        Err(_) => Vec::new(),
    };
    if let Ok(PropertyValue::ApplicationData(bytes)) =
        forwarder.read_property(PropertyIdentifier::SUBSCRIBED_RECIPIENTS, None)
    {
        destinations.extend(subscribed(&bytes));
    }
    destinations
}

/// The live entries of a served Subscribed_Recipients, or none when the list
/// does not decode or holds more than [`MAX_SUBSCRIBED_RECIPIENTS`] entries.
fn subscribed(bytes: &[u8]) -> Vec<(BACnetRecipient, u32, bool)> {
    let mut entries = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if entries.len() == MAX_SUBSCRIBED_RECIPIENTS {
            return Vec::new();
        }
        let Ok((entry, next)) = decode_event_notification_subscription(bytes, offset) else {
            return Vec::new();
        };
        offset = next;
        if entry.time_remaining > 0 {
            entries.push((
                entry.recipient,
                entry.process_identifier,
                entry.issue_confirmed_notifications,
            ));
        }
    }
    entries
}
