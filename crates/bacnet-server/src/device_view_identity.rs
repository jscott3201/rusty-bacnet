//! The small immutable part of DeviceIdentity needed by a property read.
use super::*;
use bacnet_types::enums::Segmentation;

#[derive(Clone, Copy)]
pub(super) struct IdentityView {
    pub(super) oid: ObjectIdentifier,
    vendor: u16,
    capacity: u16,
    segmentation: Segmentation,
    timeout: u64,
}

pub(super) fn identity_property(property: P) -> bool {
    matches!(
        property,
        P::OBJECT_IDENTIFIER
            | P::VENDOR_IDENTIFIER
            | P::MAX_APDU_LENGTH_ACCEPTED
            | P::SEGMENTATION_SUPPORTED
            | P::APDU_SEGMENT_TIMEOUT
            | P::MAX_SEGMENTS_ACCEPTED
    )
}

impl From<&bacnet_objects::DeviceIdentity> for IdentityView {
    fn from(identity: &bacnet_objects::DeviceIdentity) -> Self {
        Self {
            oid: identity.device_oid(),
            vendor: identity.vendor_id(),
            capacity: identity.max_apdu_length(),
            segmentation: identity.segmentation(),
            timeout: identity.apdu_segment_timeout_ms(),
        }
    }
}

impl IdentityView {
    pub(super) fn value(&self, property: P) -> Result<PropertyValue, Error> {
        Ok(match property {
            P::OBJECT_IDENTIFIER => PropertyValue::ObjectIdentifier(self.oid),
            P::VENDOR_IDENTIFIER => PropertyValue::Unsigned(u64::from(self.vendor)),
            P::MAX_APDU_LENGTH_ACCEPTED => PropertyValue::Unsigned(u64::from(self.capacity)),
            P::SEGMENTATION_SUPPORTED => {
                PropertyValue::Enumerated(u32::from(self.segmentation.to_raw()))
            }
            P::APDU_SEGMENT_TIMEOUT if self.segmentation != Segmentation::NONE => {
                PropertyValue::Unsigned(self.timeout)
            }
            P::MAX_SEGMENTS_ACCEPTED if self.segmentation != Segmentation::NONE => {
                PropertyValue::Unsigned(if self.segmentation == Segmentation::TRANSMIT {
                    1
                } else {
                    65
                })
            }
            _ => return Err(property_error(ErrorCode::UNKNOWN_PROPERTY)),
        })
    }

    pub(super) fn properties(&self) -> impl Iterator<Item = P> + '_ {
        [
            P::OBJECT_IDENTIFIER,
            P::VENDOR_IDENTIFIER,
            P::MAX_APDU_LENGTH_ACCEPTED,
            P::SEGMENTATION_SUPPORTED,
            P::APDU_SEGMENT_TIMEOUT,
            P::MAX_SEGMENTS_ACCEPTED,
        ]
        .into_iter()
        .filter(|property| self.value(*property).is_ok())
    }
}
