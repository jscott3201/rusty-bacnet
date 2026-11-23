//! Which acknowledgement each confirmed service takes (#2721), checked
//! where every final acknowledgement completes its transaction.
//!
//! A service whose result carries data is acknowledged with a Complex-ACK;
//! Clause 21's BACnet-Confirmed-Service-ACK names them. Every other
//! service 135-2020 defines is acknowledged with a Simple-ACK. The state
//! machine ends the transaction on either (Clause 5.4.4.3); the wrong one
//! for the service fails the request instead of completing it as a
//! success. Some take either:
//!
//! - ConfirmedPrivateTransfer (18): vendor devices answer it both ways, so
//!   the raw request takes either, as it always did; the typed call, which
//!   needs the acknowledgement's vendor and service numbers, reports a
//!   Simple-ACK itself. Every other Complex-ACK production has members the
//!   standard requires, so no other service has an answer that may be
//!   left out.
//! - A removed service (13, 24, 25) and a number the standard doesn't
//!   assign.
//!
//! A client with
//! [`ClientConfig::accept_complex_ack_for_simple_services`](crate::client::ClientConfig::accept_complex_ack_for_simple_services)
//! takes a Complex-ACK for a Simple-ACK service too, for a device in the
//! field that answers so; a Simple-ACK where data is due stays an error.
//!
//! A segmented Complex-ACK of the wrong kind is caught once it is
//! reassembled, when it completes the transaction: no service acknowledged
//! with a Simple-ACK has a result to segment, so a device sending one is
//! rare enough that its segments are acknowledged first.

use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::error::Error;

use super::TsmResponse;

const fn bit(service: ConfirmedServiceChoice) -> u64 {
    1 << service.to_raw()
}

/// Services acknowledged with a Complex-ACK.
const COMPLEX: u64 = bit(ConfirmedServiceChoice::GET_ALARM_SUMMARY)
    | bit(ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY)
    | bit(ConfirmedServiceChoice::GET_EVENT_INFORMATION)
    | bit(ConfirmedServiceChoice::ATOMIC_READ_FILE)
    | bit(ConfirmedServiceChoice::ATOMIC_WRITE_FILE)
    | bit(ConfirmedServiceChoice::CREATE_OBJECT)
    | bit(ConfirmedServiceChoice::READ_PROPERTY)
    | bit(ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE)
    | bit(ConfirmedServiceChoice::READ_RANGE)
    | bit(ConfirmedServiceChoice::AUDIT_LOG_QUERY)
    | bit(ConfirmedServiceChoice::VT_OPEN)
    | bit(ConfirmedServiceChoice::VT_DATA);

/// Services acknowledged with a Simple-ACK.
const SIMPLE: u64 = bit(ConfirmedServiceChoice::ACKNOWLEDGE_ALARM)
    | bit(ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION)
    | bit(ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE)
    | bit(ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION)
    | bit(ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION)
    | bit(ConfirmedServiceChoice::SUBSCRIBE_COV)
    | bit(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY)
    | bit(ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE)
    | bit(ConfirmedServiceChoice::LIFE_SAFETY_OPERATION)
    | bit(ConfirmedServiceChoice::ADD_LIST_ELEMENT)
    | bit(ConfirmedServiceChoice::REMOVE_LIST_ELEMENT)
    | bit(ConfirmedServiceChoice::DELETE_OBJECT)
    | bit(ConfirmedServiceChoice::WRITE_PROPERTY)
    | bit(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE)
    | bit(ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL)
    | bit(ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE)
    | bit(ConfirmedServiceChoice::REINITIALIZE_DEVICE)
    | bit(ConfirmedServiceChoice::VT_CLOSE);

/// Which acknowledgement is the wrong kind for each service number, one
/// octet per number: bit 0 set when a Simple-ACK is, bit 1 when a
/// Complex-ACK is. The check on every answer is then one load and one bit
/// test, with no bounds check and no 64-bit mask to build.
static WRONG: [u8; 256] = {
    let mut table = [0; 256];
    let mut raw = 0;
    while raw < 64 {
        if (COMPLEX >> raw) & 1 == 1 {
            table[raw] |= 1 << SIMPLE_ACK;
        }
        if (SIMPLE >> raw) & 1 == 1 {
            table[raw] |= 1 << COMPLEX_ACK;
        }
        raw += 1;
    }
    table
};

/// [`slot`] of a Simple-ACK.
const SIMPLE_ACK: u8 = 0;
/// [`slot`] of a Complex-ACK.
const COMPLEX_ACK: u8 = 1;

/// Which of [`WRONG`]'s bits `response` reads: [`SIMPLE_ACK`],
/// [`COMPLEX_ACK`], or a bit no service sets for any other response. The
/// match is the one the completion makes anyway, so it costs nothing more.
#[inline(always)]
pub(super) fn slot(response: &TsmResponse) -> u8 {
    match response {
        TsmResponse::SimpleAck => SIMPLE_ACK,
        TsmResponse::ComplexAck { .. } => COMPLEX_ACK,
        TsmResponse::Remote { .. } => 2,
        TsmResponse::Failed(_) => 3,
    }
}

/// Whether `slot` is an acknowledgement's.
#[inline(always)]
pub(super) const fn is_ack(slot: u8) -> bool {
    slot <= COMPLEX_ACK
}

/// Whether the answer in `slot` is an acknowledgement of the wrong kind
/// for `service`: `false` for the right kind, for a service that takes
/// either and for any other answer. [`refuse`] does the rest.
#[inline(always)]
pub(super) fn wrong(service: ConfirmedServiceChoice, slot: u8) -> bool {
    (WRONG[usize::from(service.to_raw())] >> slot) & 1 == 1
}

/// Make `response`, an acknowledgement in `slot` of the wrong kind for
/// `service`, the request's failure, unless it is a Complex-ACK and the
/// client takes those (`accept_complex`, read only here).
#[cold]
#[inline(never)]
pub(super) fn refuse(
    service: ConfirmedServiceChoice,
    slot: u8,
    accept_complex: bool,
    response: &mut TsmResponse,
) {
    let complex = slot == COMPLEX_ACK;
    if !(complex && accept_complex) {
        *response = TsmResponse::Failed(Box::new(Error::UnexpectedAck { service, complex }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether `raw` is in `set`.
    fn listed(set: u64, raw: u8) -> bool {
        raw < 64 && (set >> raw) & 1 == 1
    }

    /// Every service 135-2020 defines takes exactly one kind, and the
    /// removed and unassigned numbers take either; the table the check
    /// reads says the same.
    #[test]
    fn each_defined_service_takes_one_acknowledgement() {
        assert_eq!(COMPLEX & SIMPLE, 0);
        for raw in 0..=u8::MAX {
            let service = ConfirmedServiceChoice::from_raw(raw);
            let complex = listed(COMPLEX, raw);
            let simple = listed(SIMPLE, raw);
            let either = matches!(raw, 13 | 18 | 24 | 25);
            assert_eq!(
                complex || simple,
                service.name().is_some() && !either,
                "service {raw}"
            );
            assert_eq!(wrong(service, SIMPLE_ACK), complex, "service {raw}");
            assert_eq!(wrong(service, COMPLEX_ACK), simple, "service {raw}");
            assert!(!wrong(service, 2) && !wrong(service, 3), "service {raw}");
        }
        assert_eq!(COMPLEX.count_ones(), 12);
        assert_eq!(SIMPLE.count_ones(), 18);
    }

    /// Every answer moves through the response channel and the request
    /// future by value, so a completion stays as small as the
    /// acknowledgements and the device's errors make it: the rare endings
    /// are boxed (#2724, #2721).
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_completion_stays_small() {
        assert!(core::mem::size_of::<TsmResponse>() <= 40);
        assert!(core::mem::size_of::<Result<TsmResponse, bacnet_types::error::Error>>() <= 64);
    }
}
