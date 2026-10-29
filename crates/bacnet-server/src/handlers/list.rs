use super::*;
use bacnet_encoding::constructed::{
    decode_calendar_entry, decode_calendar_entry_list, decode_destination, decode_destination_list,
    decode_device_object_property_reference, decode_event_notification_subscription,
    encode_calendar_entry, encode_calendar_entry_list, encode_destination, encode_destination_list,
    encode_device_object_property_reference, encode_event_notification_subscription_list,
    encode_recipient,
};
use bacnet_encoding::primitives::decode_application_value;
use bacnet_services::list_manipulation::ListElementRequest;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDestination, BACnetDeviceObjectPropertyReference,
    BACnetEventNotificationSubscription,
};
use bacnet_types::error::RemoteError;
use bytes::Bytes;
use std::collections::hash_map::{Entry, HashMap};
use std::mem::{Discriminant, discriminant};

fn protocol_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::protocol(class, code)
}

/// A refusal of the element at `position` (from 1) of the request's List of
/// Elements, sent as a ChangeList-Error with that First Failed Element Number.
/// Every other refusal of these services goes out with element number 0.
fn element_error(class: ErrorClass, code: ErrorCode, position: u32) -> Error {
    Error::with_detail(
        class,
        code,
        Some(ErrorDetail::FirstFailedElementNumber(position)),
    )
}

/// A stored list the server cannot decode: a fault of the target, not of any
/// element of the request.
fn invalid_data_type() -> Error {
    protocol_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE)
}

/// The position, counted from 1, of the element at `index`.
fn position(index: usize) -> u32 {
    u32::try_from(index).map_or(u32::MAX, |index| index.saturating_add(1))
}

/// Handle an AddListElement request.
///
/// Adds every element not already in the list. An element already present is
/// the same whole element, so it is left as it is, except in a Notification
/// Forwarder's Subscribed_Recipients, where an element naming an entry already
/// present renews that entry. Nothing changes when any element is refused. A
/// refusal of one element is `Error::Structured` naming its position; a
/// refusal of the request or its target is `Error::protocol`.
pub fn handle_add_list_element(db: &mut ObjectDatabase, service_data: &[u8]) -> Result<(), Error> {
    handle_list_element_observed(db, service_data, false, |_, _, _| {}).map(|_| ())
}

/// Handle a RemoveListElement request.
///
/// Removes every listed element. Nothing is removed when any element is
/// refused, including one that is not in the list (LIST_ELEMENT_NOT_FOUND).
/// Errors take the same two forms as [`handle_add_list_element`].
pub fn handle_remove_list_element(
    db: &mut ObjectDatabase,
    service_data: &[u8],
) -> Result<(), Error> {
    handle_list_element_observed(db, service_data, true, |_, _, _| {}).map(|_| ())
}

/// How a list's elements travel in the request and how the object holds them.
#[derive(Clone, Copy)]
enum ElementCodec {
    /// Consecutive application-tagged values, held as `PropertyValue::List`.
    Values,
    /// BACnetDestination entries, held as the framed list in `ApplicationData`.
    Destinations,
    /// BACnetCalendarEntry entries under their Clause 21 CHOICE tags, held as
    /// a `PropertyValue::List` with one encoded entry per element (#996).
    CalendarEntries,
    /// BACnetDeviceObjectPropertyReference members, held as their encodings
    /// back to back in `ApplicationData` (#1121).
    References,
    /// BACnetEventNotificationSubscription entries, held as their encodings
    /// back to back in `ApplicationData` (#1049).
    Subscriptions,
}

impl ElementCodec {
    /// The codec the property's datatype implies, known even when the object
    /// is missing. Only a BACnetLIST of BACnetDestination, the Recipient_List
    /// of Notification Class and Notification Forwarder (Tables 12-24 and
    /// 12-58), takes the destination codec, only Calendar's Date_List, a
    /// BACnetLIST of BACnetCalendarEntry (Table 12-11), the calendar codec,
    /// only the Schedule's List_Of_Object_Property_References, a BACnetLIST
    /// of BACnetDeviceObjectPropertyReference (Table 12-28), the reference
    /// codec, and only the Notification Forwarder's Subscribed_Recipients, a
    /// BACnetLIST of BACnetEventNotificationSubscription (Table 12-58), the
    /// subscription codec.
    fn for_datatype(object_type: ObjectType, property: PropertyIdentifier) -> Self {
        match property {
            PropertyIdentifier::RECIPIENT_LIST
                if matches!(
                    object_type,
                    ObjectType::NOTIFICATION_CLASS | ObjectType::NOTIFICATION_FORWARDER
                ) =>
            {
                Self::Destinations
            }
            PropertyIdentifier::DATE_LIST if object_type == ObjectType::CALENDAR => {
                Self::CalendarEntries
            }
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES
                if object_type == ObjectType::SCHEDULE =>
            {
                Self::References
            }
            PropertyIdentifier::SUBSCRIBED_RECIPIENTS
                if object_type == ObjectType::NOTIFICATION_FORWARDER =>
            {
                Self::Subscriptions
            }
            _ => Self::Values,
        }
    }

    /// Whether the object holds the list in the form this codec edits.
    fn holds(self, stored: &PropertyValue) -> bool {
        match self {
            Self::Values | Self::CalendarEntries => matches!(stored, PropertyValue::List(_)),
            Self::Destinations | Self::References | Self::Subscriptions => {
                matches!(stored, PropertyValue::ApplicationData(_))
            }
        }
    }

    /// Decode the request's elements one at a time. A failure is the position
    /// of the first element that does not decode.
    fn decode(self, elements: &[u8]) -> Result<Elements, u32> {
        match self {
            Self::Values => decode_each(elements, decode_application_value).map(Elements::Values),
            Self::Destinations => {
                decode_each(elements, decode_destination).map(Elements::Destinations)
            }
            Self::CalendarEntries => {
                decode_each(elements, decode_calendar_entry).map(Elements::CalendarEntries)
            }
            Self::References => decode_each(elements, decode_device_object_property_reference)
                .map(Elements::References),
            Self::Subscriptions => decode_each(elements, decode_event_notification_subscription)
                .map(Elements::Subscriptions),
        }
    }

    /// The stored list, in the form [`Self::holds`] admitted. A malformed
    /// stored entry is refused, never dropped, so a removal cannot shorten the
    /// list by more than the request names.
    fn stored(self, current: &PropertyValue) -> Result<Elements, Error> {
        match (self, current) {
            (Self::Values, PropertyValue::List(items)) => Ok(Elements::Values(items.clone())),
            (Self::Destinations, PropertyValue::ApplicationData(bytes)) => {
                decode_destination_list(bytes)
                    .map(Elements::Destinations)
                    .map_err(|_| invalid_data_type())
            }
            (Self::CalendarEntries, PropertyValue::List(items)) => {
                let mut entries = Vec::with_capacity(items.len());
                for item in items {
                    let PropertyValue::ApplicationData(bytes) = item else {
                        return Err(invalid_data_type());
                    };
                    entries.extend(
                        decode_calendar_entry_list(bytes).map_err(|_| invalid_data_type())?,
                    );
                }
                Ok(Elements::CalendarEntries(entries))
            }
            (Self::References, PropertyValue::ApplicationData(bytes)) => {
                decode_each(bytes, decode_device_object_property_reference)
                    .map(Elements::References)
                    .map_err(|_| invalid_data_type())
            }
            (Self::Subscriptions, PropertyValue::ApplicationData(bytes)) => {
                decode_each(bytes, decode_event_notification_subscription)
                    .map(Elements::Subscriptions)
                    .map_err(|_| invalid_data_type())
            }
            _ => unreachable!("list_target admits only the form the codec edits"),
        }
    }
}

/// Decode consecutive elements, failing with the position of the first one
/// that does not decode.
fn decode_each<T>(
    elements: &[u8],
    decode: impl Fn(&[u8], usize) -> Result<(T, usize), Error>,
) -> Result<Vec<T>, u32> {
    let mut decoded = Vec::new();
    let mut offset = 0;
    while offset < elements.len() {
        let (element, next) = decode(elements, offset).map_err(|_| position(decoded.len()))?;
        decoded.push(element);
        offset = next;
    }
    Ok(decoded)
}

/// One list's elements, decoded with one codec.
enum Elements {
    Values(Vec<PropertyValue>),
    Destinations(Vec<BACnetDestination>),
    CalendarEntries(Vec<BACnetCalendarEntry>),
    References(Vec<BACnetDeviceObjectPropertyReference>),
    Subscriptions(Vec<BACnetEventNotificationSubscription>),
}

/// Where the elements an AddListElement adds sit in the edited list: after
/// the `kept` elements already stored, in request order, each with its
/// position in the request. Empty for a removal, which adds nothing.
///
/// `renewed` records the Subscribed_Recipients entries an element replaced in
/// place, by index in the edited list, with the request position of the
/// element whose members the entry now holds; a later renewal of an index
/// supersedes an earlier one.
#[derive(Default)]
struct Added {
    kept: usize,
    positions: Vec<u32>,
    renewed: Vec<(usize, u32)>,
}

impl Added {
    /// The request position of the element at `named` (from 1) in the edited
    /// list. None for an element the list already held and no request
    /// element renewed, or a position past its end: no element of the
    /// request.
    fn request_position(&self, named: u32) -> Option<u32> {
        let index = usize::try_from(named).ok()?.checked_sub(1)?;
        if let Some(&(_, position)) = self.renewed.iter().rev().find(|(at, _)| *at == index) {
            return Some(position);
        }
        index
            .checked_sub(self.kept)
            .and_then(|index| self.positions.get(index).copied())
    }

    /// The first request element the edit added or renewed an entry with.
    fn first_position(&self) -> Option<u32> {
        let renewed = self.renewed.iter().map(|&(_, position)| position).min();
        match (self.positions.first().copied(), renewed) {
            (Some(added), Some(renewed)) => Some(added.min(renewed)),
            (added, renewed) => added.or(renewed),
        }
    }
}

/// The list to write back, and where its added elements came from.
struct Edited {
    value: PropertyValue,
    added: Added,
}

impl Elements {
    /// Apply `edits`, the request's elements, to the stored list in `self`.
    fn apply(self, edits: Elements, remove: bool) -> Result<Edited, Error> {
        match (self, edits) {
            (Self::Values(stored), Self::Values(edits)) => {
                // A list's elements share the property's datatype, so the
                // stored elements show it. An empty list shows nothing; there,
                // any element is simply not found.
                let kinds: HashSet<Discriminant<PropertyValue>> =
                    stored.iter().map(discriminant).collect();
                let (list, added) = edit(stored, edits, remove, encode_property_value, |value| {
                    kinds.is_empty() || kinds.contains(&discriminant(value))
                })?;
                Ok(Edited {
                    value: PropertyValue::List(list),
                    added,
                })
            }
            (Self::Destinations(stored), Self::Destinations(edits)) => {
                let (list, added) = edit(stored, edits, remove, encode_destination, |_| true)?;
                let mut bytes = BytesMut::new();
                encode_destination_list(&mut bytes, &list)?;
                Ok(Edited {
                    value: PropertyValue::ApplicationData(bytes.to_vec()),
                    added,
                })
            }
            (Self::CalendarEntries(stored), Self::CalendarEntries(edits)) => {
                // The Calendar refuses an entry out of its Clause 21 range and
                // names it (#1029, #1048). A removal has no range error
                // (Clause 15.2.1.3.1): no such entry is ever stored, so it is
                // not found.
                let (list, added) = edit(
                    stored,
                    edits,
                    remove,
                    |buf, entry| {
                        encode_calendar_entry(buf, entry);
                        Ok(())
                    },
                    |_| true,
                )?;
                // One pre-encoded list, which the Calendar validates.
                let mut bytes = BytesMut::new();
                encode_calendar_entry_list(&mut bytes, &list);
                Ok(Edited {
                    value: PropertyValue::ApplicationData(bytes.to_vec()),
                    added,
                })
            }
            (Self::References(stored), Self::References(edits)) => {
                // The Schedule takes the result through its whole-list write,
                // which refuses a member in another device or past its cap
                // and names it (#1121). Such a member is never stored, so a
                // removal doesn't find it.
                let (list, added) = edit(
                    stored,
                    edits,
                    remove,
                    |buf, member| {
                        encode_device_object_property_reference(buf, member);
                        Ok(())
                    },
                    |_| true,
                )?;
                let mut bytes = BytesMut::new();
                for member in &list {
                    encode_device_object_property_reference(&mut bytes, member);
                }
                Ok(Edited {
                    value: PropertyValue::ApplicationData(bytes.to_vec()),
                    added,
                })
            }
            (Self::Subscriptions(stored), Self::Subscriptions(edits)) => {
                // The forwarder takes the result through its whole-list
                // write, which refuses a Time Remaining out of its range or
                // an entry past its cap and names it.
                let (list, added) = edit_subscriptions(stored, edits, remove)?;
                let mut bytes = BytesMut::new();
                encode_event_notification_subscription_list(&mut bytes, &list)?;
                Ok(Edited {
                    value: PropertyValue::ApplicationData(bytes.to_vec()),
                    added,
                })
            }
            _ => unreachable!("one codec decodes both lists"),
        }
    }
}

/// Apply the service procedures of Clauses 15.1.2 and 15.2.2 to one list, all
/// or nothing. Elements compare whole, since none of these properties narrows
/// the comparison in its description (Recipient_List, Date_List,
/// List_Of_Object_Property_References and the Values lists all compare
/// whole); Subscribed_Recipients, whose description does, has
/// [`edit_subscriptions`]. Two elements are the same exactly when their
/// canonical encodings are, which also keeps the work linear in the list
/// sizes.
///
/// AddListElement appends each element not yet present, a repeat within the
/// request once; an element already present is identical, so updating it in
/// place and ignoring it leave the same list. RemoveListElement checks the
/// elements in request order, each for the property's datatype
/// (`datatype_matches`) and then for presence, and refuses the request at the
/// first that fails.
fn edit<T>(
    mut stored: Vec<T>,
    edits: Vec<T>,
    remove: bool,
    encode: impl Fn(&mut BytesMut, &T) -> Result<(), Error>,
    datatype_matches: impl Fn(&T) -> bool,
) -> Result<(Vec<T>, Added), Error> {
    let key = |element: &T| {
        let mut buf = BytesMut::new();
        encode(&mut buf, element).map(|()| buf.freeze())
    };
    let keys = stored
        .iter()
        .map(key)
        .collect::<Result<Vec<Bytes>, Error>>()
        .map_err(|_| invalid_data_type())?;
    // A request element decoded from the wire always encodes again; if one
    // did not, it could not be the property's datatype.
    let request_key = |index: usize, element: &T| {
        key(element).map_err(|_| {
            element_error(
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
                position(index),
            )
        })
    };
    if remove {
        let present: HashSet<&Bytes> = keys.iter().collect();
        let mut removed = HashSet::with_capacity(edits.len());
        for (index, element) in edits.iter().enumerate() {
            if !datatype_matches(element) {
                return Err(element_error(
                    ErrorClass::PROPERTY,
                    ErrorCode::INVALID_DATA_TYPE,
                    position(index),
                ));
            }
            let element_key = request_key(index, element)?;
            if !present.contains(&element_key) {
                return Err(element_error(
                    ErrorClass::SERVICES,
                    ErrorCode::LIST_ELEMENT_NOT_FOUND,
                    position(index),
                ));
            }
            removed.insert(element_key);
        }
        let list = stored
            .into_iter()
            .zip(&keys)
            .filter(|(_, key)| !removed.contains(*key))
            .map(|(element, _)| element)
            .collect();
        return Ok((list, Added::default()));
    }
    let mut added = Added {
        kept: stored.len(),
        ..Added::default()
    };
    let mut present: HashSet<Bytes> = keys.into_iter().collect();
    for (index, element) in edits.into_iter().enumerate() {
        if present.insert(request_key(index, &element)?) {
            added.positions.push(position(index));
            stored.push(element);
        }
    }
    Ok((stored, added))
}

/// Apply the service procedures to a Notification Forwarder's
/// Subscribed_Recipients, all or nothing. Clause 12.51.9 narrows the
/// comparison: an element names the entry with the same recipient and process
/// identifier, whatever its other members.
///
/// AddListElement renews a named entry in place with the element's members, a
/// later element naming the same entry winning, and appends the others.
/// RemoveListElement refuses the request at the first element naming no entry
/// (LIST_ELEMENT_NOT_FOUND) and otherwise removes every named entry. Entries
/// are keyed by the canonical encoding of the two identifying members, so the
/// work stays linear in the list sizes.
fn edit_subscriptions(
    mut stored: Vec<BACnetEventNotificationSubscription>,
    edits: Vec<BACnetEventNotificationSubscription>,
    remove: bool,
) -> Result<(Vec<BACnetEventNotificationSubscription>, Added), Error> {
    let key = |subscription: &BACnetEventNotificationSubscription| {
        let mut buf = BytesMut::new();
        // Stored entries and request elements both decode through the bounded
        // recipient decoder (#1156), so each one encodes.
        encode_recipient(&mut buf, &subscription.recipient)
            .expect("decoded recipients fit BACnetAddress::MAX_MAC_LEN");
        buf.extend_from_slice(&subscription.process_identifier.to_be_bytes());
        buf.freeze()
    };
    let mut named: HashMap<Bytes, usize> = stored
        .iter()
        .enumerate()
        .map(|(index, subscription)| (key(subscription), index))
        .collect();
    if remove {
        let mut removed = HashSet::with_capacity(edits.len());
        for (index, element) in edits.iter().enumerate() {
            let element_key = key(element);
            if !named.contains_key(&element_key) {
                return Err(element_error(
                    ErrorClass::SERVICES,
                    ErrorCode::LIST_ELEMENT_NOT_FOUND,
                    position(index),
                ));
            }
            removed.insert(element_key);
        }
        stored.retain(|subscription| !removed.contains(&key(subscription)));
        return Ok((stored, Added::default()));
    }
    let mut added = Added {
        kept: stored.len(),
        ..Added::default()
    };
    for (index, element) in edits.into_iter().enumerate() {
        match named.entry(key(&element)) {
            Entry::Occupied(entry) => {
                let at = *entry.get();
                stored[at] = element;
                added.renewed.push((at, position(index)));
            }
            Entry::Vacant(entry) => {
                entry.insert(stored.len());
                added.positions.push(position(index));
                stored.push(element);
            }
        }
    }
    Ok((stored, added))
}

/// Map the object's refusal of the edited list. AddListElement reports a lack
/// of space with its own code (Clause 15.1.1.3.1).
///
/// An object that names the element it refused (#1048) names a position in
/// the edited list; the error carries the request position of that element,
/// as Clause 15.1.1.3.2 counts it. An element the list already held came from
/// no request element, so then the number is 0, unless a request element
/// renewed it in place. A refusal naming no element keeps an estimate: for
/// the elements' datatype, encoding, range or space, the first element the
/// list would have gained or renewed an entry with, exact when the request
/// carries one; for anything else, such as WRITE_ACCESS_DENIED, 0, since it
/// concerns the target. A removal's refusals keep 0 too: the object judges
/// what remains, which holds no element of the request.
fn object_refusal(error: Error, remove: bool, added: &Added) -> Error {
    let (class, code, named) = match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code, None),
        Error::Structured {
            class,
            code,
            detail,
        } => match *detail {
            ErrorDetail::FirstFailedElementNumber(named) => (class, code, Some(named)),
            _ => (class, code, None),
        },
        other => return other,
    };
    let code = if !remove
        && class == ErrorClass::RESOURCES
        && code == ErrorCode::NO_SPACE_TO_WRITE_PROPERTY
    {
        ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT
    } else {
        code
    };
    let about_elements = [
        ErrorCode::INVALID_DATA_TYPE,
        ErrorCode::DATATYPE_NOT_SUPPORTED,
        ErrorCode::INVALID_DATA_ENCODING,
        ErrorCode::VALUE_OUT_OF_RANGE,
        ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
    ]
    .contains(&code);
    let element = match named {
        _ if remove => None,
        Some(named) => added.request_position(named),
        None => added.first_position().filter(|_| about_elements),
    };
    Error::with_detail(
        class,
        code,
        element.map(ErrorDetail::FirstFailedElementNumber),
    )
}

/// A request refused before any element is applied. `current` is the target
/// property's value when it could be read, kept for the observer's pre-image.
struct Refusal {
    error: Error,
    current: Option<PropertyValue>,
}

impl Refusal {
    fn unread(error: Error) -> Self {
        Self {
            error,
            current: None,
        }
    }

    fn read(class: ErrorClass, code: ErrorCode, current: PropertyValue) -> Self {
        Self {
            error: protocol_error(class, code),
            current: Some(current),
        }
    }
}

/// Resolve the target list without looking at the elements. The checks run in
/// the order the service procedures of Clauses 15.1 and 15.2 imply: the
/// object, the property, a supplied array index, and then whether the target
/// is a BACnetLIST at all. Element datatype errors come only after every one
/// of these has passed.
fn list_target(
    db: &ObjectDatabase,
    request: &ListElementRequest,
    codec: ElementCodec,
) -> Result<PropertyValue, Refusal> {
    let object = db.get(&request.object_identifier).ok_or_else(|| {
        Refusal::unread(protocol_error(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ))
    })?;
    let property = request.property_identifier;
    let index = request.property_array_index;
    let read = |index| {
        object
            .read_property(property, index)
            .map_err(Refusal::unread)
    };
    if index.is_some() && !object.is_array_property(property) {
        // Read the whole property first so an unknown one keeps its error.
        return Err(Refusal::read(
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
            read(None)?,
        ));
    }
    let current = read(index)?;
    // An indexed array element is never a list: 135-2020 defines no
    // BACnetARRAY of BACnetLIST property.
    if index.is_some() || !object.is_list_property(property) {
        return Err(Refusal::read(
            ErrorClass::SERVICES,
            ErrorCode::PROPERTY_IS_NOT_A_LIST,
            current,
        ));
    }
    if !codec.holds(&current) {
        // A list held in a form its codec doesn't edit, such as the framed
        // COV subscription lists of a standalone Device, cannot change
        // through these services.
        return Err(Refusal::read(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
            current,
        ));
    }
    Ok(current)
}

/// A reference naming this device stands for the local one, so it matches
/// the stored member and is added in that form (#1122).
fn localize_edits(db: &ObjectDatabase, edits: &mut Elements) {
    if let Elements::References(members) = edits {
        let local = db.local_device();
        for member in members {
            crate::local_references::localize_member(member, local);
        }
    }
}

/// The whole list an AddListElement (`remove` false) or RemoveListElement
/// request would write back, computed as [`handle_list_element_observed`]
/// computes it, or `None` when the request would be refused before the
/// write. The server stages that write when the object saves it (#1270).
pub(crate) fn edited_list_value(
    db: &ObjectDatabase,
    service_data: &[u8],
    remove: bool,
) -> Option<PropertyValue> {
    let request = ListElementRequest::decode(service_data).ok()?;
    let codec = ElementCodec::for_datatype(
        request.object_identifier.object_type(),
        request.property_identifier,
    );
    let current = list_target(db, &request, codec).ok()?;
    let mut edits = codec.decode(&request.list_of_elements).ok()?;
    localize_edits(db, &mut edits);
    let stored = codec.stored(&current).ok()?;
    stored.apply(edits, remove).ok().map(|edited| edited.value)
}

/// Decode once and observe only executable requests. The callback borrows the
/// pre-image already needed by execution; it never causes a second property read.
/// Target errors (lookup, read, array index, not a list) keep their precedence
/// over element errors, and go out with element number 0. Element errors
/// (decode, datatype, not found, and the object's refusal of the edited list)
/// name their element. Returns the object written.
pub(crate) fn handle_list_element_observed(
    db: &mut ObjectDatabase,
    service_data: &[u8],
    remove: bool,
    mut before: impl FnMut(&ObjectDatabase, &ListElementRequest, Option<&PropertyValue>),
) -> Result<ObjectIdentifier, Error> {
    let request = ListElementRequest::decode(service_data).map_err(Error::into_request_reject)?;
    let codec = ElementCodec::for_datatype(
        request.object_identifier.object_type(),
        request.property_identifier,
    );
    let target = list_target(db, &request, codec);
    // Every request decodes its elements once, with the codec its datatype
    // implies; a refused request reaches the observer only if they decode.
    let edits = codec.decode(&request.list_of_elements);
    let current = match target {
        Ok(current) => current,
        Err(Refusal { error, current }) => {
            if edits.is_ok() {
                before(db, &request, current.as_ref());
            }
            return Err(error);
        }
    };
    let mut edits = edits.map_err(|position| {
        element_error(ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE, position)
    })?;
    localize_edits(db, &mut edits);
    // Decode the stored list before observation or mutation.
    let stored = codec.stored(&current)?;
    before(db, &request, Some(&current));
    let Edited { value, added } = stored.apply(edits, remove)?;
    db.get_mut(&request.object_identifier)
        .expect("readable object")
        .write_property(
            request.property_identifier,
            request.property_array_index,
            value,
            None,
        )
        .map_err(|error| object_refusal(error, remove, &added))?;
    Ok(request.object_identifier)
}
