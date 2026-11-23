//! A ReadPropertyMultiple the client refuses before sending it, because it
//! needs segments the client or the device can't use (#2617), falls back as
//! a request too long does.
use super::*;

/// A ReadPropertyMultiple that needs segments its client may not send (an
/// endpoint whose Device doesn't transmit them, #2617): refused before it
/// went out.
pub(super) fn rpm_needs_segments_it_may_not_send(
    inner: bacnet_transport::loopback::LoopbackTransport,
) -> TestTransport {
    failing(inner, |service, _| {
        matches!(service, Service::Rpm).then_some(Error::Segmentation(
            bacnet_types::error::SegmentationError::LocalCannotTransmit,
        ))
    })
}

/// A ReadPropertyMultiple that needs more segments than the device takes.
pub(super) fn rpm_needs_more_segments_than_it_takes(
    inner: bacnet_transport::loopback::LoopbackTransport,
) -> TestTransport {
    failing(inner, |service, _| {
        matches!(service, Service::Rpm).then_some(Error::Segmentation(
            bacnet_types::error::SegmentationError::PeerSegmentLimit {
                segments: 9,
                max: 4,
            },
        ))
    })
}

/// Each refusal of a request that needs segments is a request too long,
/// for a ReadPropertyMultiple as for the link's own refusal (#2617).
#[test]
fn a_request_refused_for_segments_is_too_large() {
    use crate::client::read::{RpmFailure, rpm_failure};
    use bacnet_types::enums::Segmentation;
    use bacnet_types::error::SegmentationError;
    for refusal in [
        SegmentationError::LocalCannotTransmit,
        SegmentationError::PeerCannotReceive {
            capability: Segmentation::SEGMENTED_TRANSMIT,
        },
        SegmentationError::PeerSegmentLimit {
            segments: 9,
            max: 4,
        },
        SegmentationError::TooManySegments { segments: 300 },
    ] {
        assert_eq!(
            rpm_failure(&Error::Segmentation(refusal)),
            RpmFailure::TooLarge,
            "{refusal:?}"
        );
    }
    // A transfer that failed partway is no such refusal.
    assert_eq!(
        rpm_failure(&Error::Segmentation(
            SegmentationError::NegativeAckRetriesExhausted
        )),
        RpmFailure::Other
    );
}
