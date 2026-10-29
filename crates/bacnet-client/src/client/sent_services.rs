//! The services the client's own methods send.
use bacnet_types::enums::ServiceSupported;

/// Every service a [`BACnetClient`](super::BACnetClient) method sends, as
/// `BACnetServicesSupported` bit positions (Clause 21), confirmed services
/// first, each group in bit order.
///
/// A service is here when a dedicated method builds and sends its request:
/// [`read_property`](super::BACnetClient::read_property),
/// [`who_is`](super::BACnetClient::who_is) and so on. The generic
/// [`confirmed_request`](super::BACnetClient::confirmed_request) and
/// [`unconfirmed_request`](super::BACnetClient::unconfirmed_request) can send
/// any other service from octets the application encodes; those aren't listed.
/// Network-layer messages such as Who-Is-Router-To-Network aren't services, so
/// they aren't listed either.
///
/// The list is a source for documentation and capability tables. Tests keep it
/// honest: one calls every sending method and compares the services that reach
/// the wire with this list, and another fails if the client's source names a
/// service this list leaves out.
pub const SENT_SERVICES: &[ServiceSupported] = &[
    // Confirmed.
    ServiceSupported::ACKNOWLEDGE_ALARM,
    ServiceSupported::GET_ALARM_SUMMARY,
    ServiceSupported::GET_ENROLLMENT_SUMMARY,
    ServiceSupported::SUBSCRIBE_COV,
    ServiceSupported::ATOMIC_READ_FILE,
    ServiceSupported::ATOMIC_WRITE_FILE,
    ServiceSupported::ADD_LIST_ELEMENT,
    ServiceSupported::REMOVE_LIST_ELEMENT,
    ServiceSupported::CREATE_OBJECT,
    ServiceSupported::DELETE_OBJECT,
    ServiceSupported::READ_PROPERTY,
    ServiceSupported::READ_PROPERTY_MULTIPLE,
    ServiceSupported::WRITE_PROPERTY,
    ServiceSupported::WRITE_PROPERTY_MULTIPLE,
    ServiceSupported::DEVICE_COMMUNICATION_CONTROL,
    ServiceSupported::REINITIALIZE_DEVICE,
    ServiceSupported::READ_RANGE,
    ServiceSupported::SUBSCRIBE_COV_PROPERTY,
    ServiceSupported::GET_EVENT_INFORMATION,
    ServiceSupported::CONFIRMED_AUDIT_NOTIFICATION,
    ServiceSupported::AUDIT_LOG_QUERY,
    // Unconfirmed.
    ServiceSupported::TIME_SYNCHRONIZATION,
    ServiceSupported::WHO_HAS,
    ServiceSupported::WHO_IS,
    ServiceSupported::UTC_TIME_SYNCHRONIZATION,
    ServiceSupported::WRITE_GROUP,
    ServiceSupported::UNCONFIRMED_AUDIT_NOTIFICATION,
];
