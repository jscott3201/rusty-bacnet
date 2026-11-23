//! Which acknowledgement each confirmed service takes (#2721), checked
//! where every final acknowledgement completes its transaction.
//!
//! A service whose result carries data is acknowledged with a Complex-ACK;
//! Clause 21's BACnet-Confirmed-Service-ACK names them. Every other
//! service 135-2020 defines is acknowledged with a Simple-ACK. The state
//! machine ends the transaction on either (Clause 5.4.4.3); the wrong one
//! for the service fails the request instead of completing it as a
//! success. A removed service (13, 24, 25) and a number the standard
//! doesn't assign take either, as before.

use bacnet_types::enums::ConfirmedServiceChoice;

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
    | bit(ConfirmedServiceChoice::CONFIRMED_PRIVATE_TRANSFER)
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

/// Whether `service` is in `set`.
const fn listed(set: u64, service: ConfirmedServiceChoice) -> bool {
    let raw = service.to_raw();
    raw < 64 && (set >> raw) & 1 == 1
}

/// The acknowledgement `response` would complete a request for `service`
/// with is the wrong kind for it: `Some(true)` for a Complex-ACK the
/// service doesn't take, `Some(false)` for a Simple-ACK. `None` for the
/// right kind, any other response, and a service that takes either.
#[inline]
pub(super) fn wrong_kind(service: ConfirmedServiceChoice, response: &TsmResponse) -> Option<bool> {
    match response {
        TsmResponse::SimpleAck if listed(COMPLEX, service) => Some(false),
        TsmResponse::ComplexAck { .. } if listed(SIMPLE, service) => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every service 135-2020 defines takes exactly one kind, and the
    /// removed and unassigned numbers take either.
    #[test]
    fn each_defined_service_takes_one_acknowledgement() {
        assert_eq!(COMPLEX & SIMPLE, 0);
        for raw in 0..=u8::MAX {
            let service = ConfirmedServiceChoice::from_raw(raw);
            let complex = listed(COMPLEX, service);
            let simple = listed(SIMPLE, service);
            let removed = matches!(raw, 13 | 24 | 25);
            assert_eq!(
                complex || simple,
                service.name().is_some() && !removed,
                "service {raw}"
            );
        }
        assert_eq!(COMPLEX.count_ones(), 13);
        assert_eq!(SIMPLE.count_ones(), 18);
    }
}
