//! Recipient_List writes, the cap on how many destinations the list holds
//! (#1098), and the same cap on the lists routing accepts (#1124).

use crate::prelude::*;
use bacnet_encoding::constructed::decode_destination;
use bacnet_types::constructed::{BACnetAddress, BACnetDestination, BACnetRecipient};
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use crate::common;

/// The most destinations a Notification Class Recipient_List holds (#1098).
///
/// A site usually routes a class to a few front ends, operator workstations
/// or gateways, and the AE-CRL-B building block (Annex K.2.25) asks for room
/// for at least 8, so 32 leaves ample headroom. The bound also keeps the list
/// small on the wire. A destination naming a device encodes in at most 27
/// octets, and one naming an address with a MAC of up to 6 octets in at most
/// 35, so a full list of either kind takes at most 1,120 octets and a
/// ReadProperty of it fits B/IP's 1476-octet APDU unsegmented. An address
/// MAC is at most [`BACnetAddress::MAX_MAC_LEN`] octets (#1124), so no
/// destination takes more than 47 octets and no full list more than 1,504.
///
/// A write that would leave more destinations fails with RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY, naming the first destination past the cap;
/// AddListElement reports that as NO_SPACE_TO_ADD_LIST_ELEMENT at the request
/// element that brought it (Clause 15.1.1.3). `add_destination` refuses the
/// same way, without the element.
///
/// Routing holds every Notification Class to the same bound, so each event
/// transition fans out to at most this many notifications per class (#1124).
/// Only a custom NOTIFICATION_CLASS object can serve a longer list, and the
/// transition then goes to none of it: see
/// [`RecipientLookupOutcome::RecipientListTooLong`](super::RecipientLookupOutcome::RecipientListTooLong).
pub const MAX_RECIPIENT_LIST_DESTINATIONS: usize = 32;

/// The refusal of a destination past [`MAX_RECIPIENT_LIST_DESTINATIONS`].
pub(crate) fn no_space_error() -> Error {
    common::protocol_error(ErrorClass::RESOURCES, ErrorCode::NO_SPACE_TO_WRITE_PROPERTY)
}

/// The refusal of a written list's destination at `index` (from 0), the first
/// past the cap, named so ChangeList-Error can report it (#1048).
fn past_the_cap(index: usize) -> Error {
    common::at_list_element(no_space_error(), index)
}

/// Why a framed Recipient_List was not taken.
pub(super) enum CappedListError {
    /// A destination, or what follows the last one, does not decode.
    Malformed,
    /// The list holds more than [`MAX_RECIPIENT_LIST_DESTINATIONS`]; the
    /// field is the index (from 0) of the first destination past the cap.
    PastTheCap(usize),
}

/// Decode a framed Recipient_List (a BACnetLIST of BACnetDestination, Clause
/// 12.21) of at most [`MAX_RECIPIENT_LIST_DESTINATIONS`] destinations, all or
/// nothing. Destinations are checked in order, so the first one that fails to
/// decode, or the first past the cap, decides the error. A destination past
/// the cap is refused before it is decoded, as Calendar's Date_List does, so
/// no list costs more than one destination past the cap to refuse.
///
/// The write path and routing share this decoder, so the object never stores
/// a list routing would refuse.
pub(super) fn decode_capped(bytes: &[u8]) -> Result<Vec<BACnetDestination>, CappedListError> {
    let mut list = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if list.len() == MAX_RECIPIENT_LIST_DESTINATIONS {
            return Err(CappedListError::PastTheCap(list.len()));
        }
        let (destination, next) =
            decode_destination(bytes, offset).map_err(|_| CappedListError::Malformed)?;
        list.push(destination);
        offset = next;
    }
    Ok(list)
}

/// Decode a whole-list Recipient_List write. Only the framed wire form
/// (`ApplicationData`) is a Recipient_List value (#1125); anything else, a
/// malformed destination included, fails with PROPERTY / INVALID_DATA_TYPE.
pub(crate) fn decode_write(value: PropertyValue) -> Result<Vec<BACnetDestination>, Error> {
    let PropertyValue::ApplicationData(bytes) = value else {
        return Err(common::invalid_data_type_error());
    };
    decode_capped(&bytes).map_err(|error| match error {
        CappedListError::Malformed => common::invalid_data_type_error(),
        CappedListError::PastTheCap(index) => past_the_cap(index),
    })
}

/// Refuse a destination given through `add_destination` that a framed write
/// could not carry: an address whose MAC is longer than
/// [`BACnetAddress::MAX_MAC_LEN`] octets fails, as the write would, with
/// PROPERTY / INVALID_DATA_TYPE (#1124). Stored, it would make the class's
/// own list undecodable and routing would drop every destination with it.
pub(crate) fn check_added(destination: &BACnetDestination) -> Result<(), Error> {
    match &destination.recipient {
        BACnetRecipient::Address(address)
            if address.mac_address.len() > BACnetAddress::MAX_MAC_LEN =>
        {
            Err(common::invalid_data_type_error())
        }
        _ => Ok(()),
    }
}
