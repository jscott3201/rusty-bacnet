//! [`CovSupport`]: what is known of a device's COV subscriptions, learned
//! from its Protocol_Services_Supported and from the hub's own requests,
//! and remembered by the [`Client`] per address, as its RPM refusals are.

use bacnet_transport::port::TransportPort;
use bacnet_types::bitstring::ServicesSupported;
use bacnet_types::enums::{ErrorClass, ErrorCode, RejectReason, ServiceSupported};
use bacnet_types::error::{Error, RemoteError};

use super::Watched;
use crate::client::{Client, lock};
use crate::device::DeviceAddress;

/// What is known of COV for a device or one of its points.
///
/// For a device ([`Device::cov_support`](crate::Device::cov_support)) it
/// is about SubscribeCOV, which an object watch sends; for a watched point
/// ([`Update::cov`](super::Update::cov)), about the subscription that
/// point's watch sends. What the device did outranks what it declared:
/// its Protocol_Services_Supported is noted when
/// [`Device::describe_device`](crate::Device::describe_device) reads it.
///
/// Serialized as its name in kebab case: `"unknown"`, `"not-offered"`,
/// `"offered"`, `"accepted"`, `"refused"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum CovSupport {
    /// Nothing yet: the device's Protocol_Services_Supported wasn't read
    /// and no subscription was asked for.
    #[default]
    Unknown,
    /// Its Protocol_Services_Supported leaves the service out. The hub
    /// still asks once: a device's list can be wrong.
    NotOffered,
    /// Its Protocol_Services_Supported lists the service.
    Offered,
    /// It took a subscription.
    Accepted,
    /// It refused one. For a device: it refused the service itself (an
    /// unknown service, or a services-class denial), so the hub polls its
    /// later points without asking. For a point: it refused that point's
    /// subscription, for any reason.
    Refused,
}

/// The most addresses whose COV support a client remembers; past it, one
/// goes to make room, and is learned again.
const REMEMBERED: usize = 4096;

/// What a client knows of one address's two COV services.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CovMemory {
    /// SubscribeCOV, an object watch's.
    object: Known,
    /// SubscribeCOVProperty, a property watch's.
    property: Known,
}

#[derive(Debug, Clone, Copy, Default)]
struct Known {
    /// Whether Protocol_Services_Supported lists the service.
    declared: Option<bool>,
    /// Whether the device took a subscription (true) or refused the
    /// service itself (false), whichever came last.
    observed: Option<bool>,
}

impl Known {
    fn support(self) -> CovSupport {
        match (self.observed, self.declared) {
            (Some(true), _) => CovSupport::Accepted,
            (Some(false), _) => CovSupport::Refused,
            (None, Some(true)) => CovSupport::Offered,
            (None, Some(false)) => CovSupport::NotOffered,
            (None, None) => CovSupport::Unknown,
        }
    }
}

impl CovMemory {
    fn of(&mut self, watched: &Watched) -> &mut Known {
        match watched {
            Watched::Object => &mut self.object,
            Watched::Property { .. } => &mut self.property,
        }
    }
}

/// Whether `error`, a device's answer to a subscription, refuses the
/// service itself rather than this object or property: a reject for an
/// unknown service, or a services-class Error PDU denying the request.
/// Anything else (an unknown object, a property without COV, a full
/// subscription table) is about the one point.
pub(super) fn refuses_service(error: &Error) -> bool {
    match error.remote() {
        Some(RemoteError::Reject { reason }) => reason == RejectReason::UNRECOGNIZED_SERVICE,
        Some(RemoteError::Error { class, code }) => {
            class == ErrorClass::SERVICES && code == ErrorCode::SERVICE_REQUEST_DENIED
        }
        _ => false,
    }
}

/// Whether `error`, a device's answer to a subscription, says its
/// subscription table is full (a resources-class Error PDU): not a
/// refusal, since a full table frees up; the point is polled meanwhile and
/// subscribed again after a backoff.
pub(super) fn fills_table(error: &Error) -> bool {
    matches!(
        error.remote(),
        Some(RemoteError::Error { class, .. }) if class == ErrorClass::RESOURCES
    )
}

impl<T: TransportPort + 'static> Client<T> {
    /// What this client knows of the COV service `watched` sends at
    /// `device`. Sends nothing.
    pub(crate) fn cov_known(&self, device: &DeviceAddress, watched: &Watched) -> CovSupport {
        lock(&self.shared.cov)
            .get_mut(device)
            .map_or(CovSupport::Unknown, |memory| memory.of(watched).support())
    }

    /// Note what `device`'s Protocol_Services_Supported lists of the two
    /// COV services.
    pub(crate) fn cov_declared(&self, device: &DeviceAddress, services: ServicesSupported) {
        self.cov_remember(device, |memory| {
            memory.object.declared = Some(services.contains(ServiceSupported::SUBSCRIBE_COV));
            memory.property.declared =
                Some(services.contains(ServiceSupported::SUBSCRIBE_COV_PROPERTY));
        });
    }

    /// Note that `device` took the subscription `watched` sends
    /// (`accepted`), or refused that service itself.
    pub(super) fn cov_observed(&self, device: &DeviceAddress, watched: &Watched, accepted: bool) {
        self.cov_remember(device, |memory| {
            memory.of(watched).observed = Some(accepted)
        });
    }

    fn cov_remember(&self, device: &DeviceAddress, note: impl FnOnce(&mut CovMemory)) {
        let mut known = lock(&self.shared.cov);
        if known.len() >= REMEMBERED
            && !known.contains_key(device)
            && let Some(any) = known.keys().next().cloned()
        {
            known.remove(&any);
        }
        note(known.entry(device.clone()).or_default());
    }
}

/// The services a Protocol_Services_Supported value's octets list; `None`
/// for a value that isn't a bit string.
pub(crate) fn services(raw: &[u8]) -> Option<ServicesSupported> {
    use bacnet_types::primitives::PropertyValue;
    match bacnet_encoding::primitives::decode_application_value(raw, 0) {
        Ok((PropertyValue::BitString { data, .. }, _)) => {
            Some(ServicesSupported::from_bacnet(&data))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the device did outranks what it declared, the last thing it
    /// did wins, and the two services are kept apart.
    #[test]
    fn an_observation_outranks_a_declaration() {
        let mut memory = CovMemory::default();
        let object = Watched::Object;
        let property = Watched::Property {
            property: bacnet_types::enums::PropertyIdentifier::PRESENT_VALUE,
            index: None,
        };
        assert_eq!(memory.of(&object).support(), CovSupport::Unknown);
        memory.of(&object).declared = Some(false);
        assert_eq!(memory.of(&object).support(), CovSupport::NotOffered);
        memory.of(&object).observed = Some(true);
        assert_eq!(memory.of(&object).support(), CovSupport::Accepted);
        memory.of(&object).declared = Some(true);
        assert_eq!(memory.of(&object).support(), CovSupport::Accepted);
        memory.of(&object).observed = Some(false);
        assert_eq!(memory.of(&object).support(), CovSupport::Refused);
        assert_eq!(memory.of(&property).support(), CovSupport::Unknown);
    }

    /// Only an unknown service or a services-class denial refuses the
    /// service; an unknown object or a full table is the point's own.
    #[test]
    fn only_the_service_itself_is_remembered_as_refused() {
        let error = |class, code| Error::Remote(RemoteError::Error { class, code });
        assert!(refuses_service(&Error::reject(
            RejectReason::UNRECOGNIZED_SERVICE
        )));
        assert!(refuses_service(&error(
            ErrorClass::SERVICES,
            ErrorCode::SERVICE_REQUEST_DENIED
        )));
        assert!(!refuses_service(&error(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT
        )));
        assert!(!refuses_service(&error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT
        )));
        assert!(!refuses_service(&Error::reject(
            RejectReason::PARAMETER_OUT_OF_RANGE
        )));
        assert!(!refuses_service(&Error::Timeout(
            std::time::Duration::from_secs(1)
        )));
    }

    /// Protocol_Services_Supported's bits name the two COV services.
    #[test]
    fn the_services_bit_string_names_both_cov_services() {
        // Bits 5 (SubscribeCOV) and 38 (SubscribeCOVProperty) of 40.
        let mut data = [0u8; 5];
        data[0] = 0x80 >> 5;
        data[4] = 0x80 >> 6;
        let mut raw = vec![0x85, 6, 0];
        raw.extend(data);
        let services = services(&raw).expect("a bit string");
        assert!(services.contains(ServiceSupported::SUBSCRIBE_COV));
        assert!(services.contains(ServiceSupported::SUBSCRIBE_COV_PROPERTY));
        assert!(!services.contains(ServiceSupported::READ_PROPERTY_MULTIPLE));
        assert!(super::services(&[0x21, 1]).is_none(), "an unsigned");
    }
}
