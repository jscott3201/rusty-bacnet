//! Shared Device identity and endpoint-specific configuration adapters.
//!
//! The object model owns declarations and stock object construction. These
//! functions adapt that data to client/server settings and I-Am encoding.
use bacnet_encoding::apdu::{encode_apdu, Apdu, UnconfirmedRequest};
pub use bacnet_objects::device_identity::{
    build_database_with_extra, DeviceIdentity, NetworkPortEntry,
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

/// Apply receive capacity without changing client timers or retry policy.
pub fn apply_to_client_config(
    identity: &DeviceIdentity,
    config: &mut bacnet_client::client::ClientConfig,
) {
    config.max_apdu_length =
        bacnet_encoding::apdu::max_apdu_header_at_or_below(u32::from(identity.max_apdu_length()))
            .expect("validated identity exceeds the minimum message size");
}

/// Full-server settings using the same Device declarations.
pub fn server_config(identity: &DeviceIdentity) -> bacnet_server::server::ServerConfig {
    bacnet_server::server::ServerConfig {
        device: identity.clone(),
        ..Default::default()
    }
}
