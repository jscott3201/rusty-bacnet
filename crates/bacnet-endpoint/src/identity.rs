//! Shared Device identity and endpoint-specific configuration adapters.
//!
//! The object model owns declarations and stock object construction. These
//! functions adapt that data to client/server settings and I-Am encoding.
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
pub use bacnet_objects::device_identity::{
    DeviceIdentity, NetworkPortEntry, build_database_with_extra,
};
use bacnet_services::who_is::IAmRequest;
use bacnet_types::{enums::UnconfirmedServiceChoice, error::Error};
use bytes::BytesMut;

/// I-Am's four Device declarations, preserving the raw receive ceiling.
pub fn iam_request(identity: &DeviceIdentity) -> IAmRequest {
    IAmRequest {
        object_identifier: identity.device_oid(),
        max_apdu_length: u32::from(identity.max_apdu_length()),
        segmentation_supported: identity.segmentation(),
        vendor_id: identity.vendor_id(),
    }
}

/// Encode the shared identity as an I-Am APDU.
pub fn encode_iam_apdu(identity: &DeviceIdentity) -> Result<Vec<u8>, Error> {
    let mut service = BytesMut::new();
    iam_request(identity).encode(&mut service);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service.freeze(),
        }),
    )?;
    Ok(apdu.to_vec())
}

/// Standalone client settings. Its existing API takes a header-table value,
/// so this adapter floors the raw Device ceiling; Device and I-Am stay raw.
/// EndpointSession itself retains the raw ceiling and floors only its headers.
pub fn client_config(identity: &DeviceIdentity) -> bacnet_client::client::ClientConfig {
    let mut config = bacnet_client::client::ClientConfig::default();
    apply_to_client_config(identity, &mut config);
    config
}

/// Apply the Device's receive capacity and segmentation without changing
/// the client's APDU timeout or retries. The client then does what the
/// Device's segmentation says, as an endpoint session's client does
/// (#2617): see [`EndpointSession::client`](crate::session::EndpointSession::client).
pub fn apply_to_client_config(
    identity: &DeviceIdentity,
    config: &mut bacnet_client::client::ClientConfig,
) {
    config.max_apdu_length =
        bacnet_encoding::apdu::max_apdu_header_at_or_below(u32::from(identity.max_apdu_length()))
            .expect("validated identity exceeds the minimum message size");
    apply_segmentation(identity, config);
}

/// Have a client do what the Device's segmentation says (#2617):
///
/// - its [`segmentation`](bacnet_client::client::ClientConfig::segmentation)
///   is the Device's Segmentation_Supported, so it accepts segmented
///   answers and notifications only when the Device receives segments, and
///   sends a request too long for one APDU as segments only when the Device
///   transmits them;
/// - it states more than 64 segments in its requests (the Device's
///   Max_Segments_Accepted of 65, Clause 12.11.20) when the Device receives
///   segments;
/// - its segment timeout is the Device's APDU_Segment_Timeout when the
///   Device segments at all, and its APDU timeout otherwise.
pub(crate) fn apply_segmentation(
    identity: &DeviceIdentity,
    config: &mut bacnet_client::client::ClientConfig,
) {
    use bacnet_types::enums::Segmentation;
    let mode = identity.segmentation();
    config.segmentation = mode;
    let receives = matches!(
        mode,
        Segmentation::SEGMENTED_BOTH | Segmentation::SEGMENTED_RECEIVE
    );
    config.max_segments = receives.then_some(MORE_THAN_64_SEGMENTS);
    config.apdu_segment_timeout_ms =
        (mode != Segmentation::NO_SEGMENTATION).then(|| identity.apdu_segment_timeout_ms());
}

/// The client's max-segments setting that states "more than 64" in a
/// request header (B'111', Clause 20.1.2.4), the Device's
/// Max_Segments_Accepted of 65 for a device that receives segments.
const MORE_THAN_64_SEGMENTS: u8 = 65;

/// Full-server settings using the same Device declarations.
pub fn server_config(identity: &DeviceIdentity) -> bacnet_server::server::ServerConfig {
    bacnet_server::server::ServerConfig {
        device: identity.clone(),
        ..Default::default()
    }
}
