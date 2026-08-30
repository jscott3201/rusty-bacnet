use super::*;

/// Every confirmed service choice with an inbound execution arm in
/// `handle_confirmed_request`. Keep in lockstep with the dispatch match; the
/// PICS synchronization test maps this list through `ServiceSupported` and
/// compares it with `bacnet_objects::device::EXECUTED_SERVICES`.
pub(crate) const EXECUTED_CONFIRMED: &[ConfirmedServiceChoice] = &[
    ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
    ConfirmedServiceChoice::GET_ALARM_SUMMARY,
    ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
    ConfirmedServiceChoice::SUBSCRIBE_COV,
    ConfirmedServiceChoice::ATOMIC_READ_FILE,
    ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
    ConfirmedServiceChoice::ADD_LIST_ELEMENT,
    ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
    ConfirmedServiceChoice::CREATE_OBJECT,
    ConfirmedServiceChoice::DELETE_OBJECT,
    ConfirmedServiceChoice::READ_PROPERTY,
    ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::WRITE_PROPERTY,
    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
    ConfirmedServiceChoice::CONFIRMED_TEXT_MESSAGE,
    ConfirmedServiceChoice::REINITIALIZE_DEVICE,
    ConfirmedServiceChoice::READ_RANGE,
    ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
    ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY,
    ConfirmedServiceChoice::GET_EVENT_INFORMATION,
    ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
    ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
    ConfirmedServiceChoice::AUDIT_LOG_QUERY,
];
