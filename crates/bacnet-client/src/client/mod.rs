//! BACnetClient: high-level and low-level request APIs.
//!
//! The client owns a NetworkLayer, spawns an APDU dispatch task, and provides
//! methods for sending confirmed and unconfirmed BACnet requests.

use std::collections::HashMap;
use std::net::Ipv4Addr;
#[cfg(feature = "ipv6")]
use std::net::Ipv6Addr;
use std::sync::Arc;
#[cfg(test)]
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
#[cfg(test)]
use tokio::time::timeout;
use tokio::time::Duration;
use tracing::{debug, warn};

use bacnet_encoding::apdu::{
    self, encode_apdu, validate_max_apdu_length, validate_max_segments, AbortPdu, Apdu,
    ConfirmedRequest as ConfirmedRequestPdu, RejectPdu, SegmentAck as SegmentAckPdu, SimpleAck,
};
use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_endpoint_core::coordinator::{CanonicalPeer, OutboundTransactionCoordinator};
use bacnet_network::layer::NetworkLayer;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_transport::bip::BipTransport;
#[cfg(feature = "ipv6")]
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{
    ConfirmedServiceChoice, NetworkPriority, RejectReason, UnconfirmedServiceChoice,
};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use crate::discovery::{DeviceTable, DeviceUpsertResult, DiscoveredDevice, RoutedDeviceConfig};
use crate::segmentation::{
    duplicate_in_window, max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType,
};
use crate::tsm::{
    RequestTimerExpiration, SegmentAckPhase, SegmentTimerExpiration, SegmentedResponseAdmission,
    TransactionOwner, TransactionProgress, Tsm, TsmConfig, TsmResponse,
};
#[cfg(test)]
use transaction_cleanup::{SegmentedCleanupHook, SegmentedPostWaitCleanupHook};
use transaction_cleanup::{TransactionCleanup, TransactionGuard};

/// Default COV notification broadcast channel capacity.
pub const DEFAULT_COV_CHANNEL_CAPACITY: usize = 64;

/// Maximum COV notification broadcast channel capacity accepted at startup.
pub const MAX_COV_CHANNEL_CAPACITY: usize = 65_536;

/// Device discovery event broadcast channel capacity.
pub const DEVICE_EVENT_CHANNEL_CAPACITY: usize = 64;

const VALID_MAX_APDU_LENGTHS: [u16; 6] = [50, 128, 206, 480, 1024, 1476];

/// Type of change observed in the device discovery table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEventKind {
    /// A new device instance was inserted into the discovery table.
    Discovered,
    /// An existing device instance was refreshed by a later I-Am.
    Updated,
    /// A previously discovered device was purged as stale.
    Lost,
}

/// Notification emitted when the client observes a device discovery change.
#[derive(Debug, Clone)]
pub struct DeviceEvent {
    /// The kind of discovery-table change.
    pub kind: DeviceEventKind,
    /// Snapshot of the device entry after discovery/update or before removal.
    pub device: DiscoveredDevice,
}

/// Client configuration.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Local interface to bind.
    pub interface: Ipv4Addr,
    /// UDP port (0 for ephemeral).
    pub port: u16,
    /// Directed broadcast address.
    pub broadcast_address: Ipv4Addr,
    /// APDU timeout in milliseconds.
    pub apdu_timeout_ms: u64,
    /// Number of APDU retries.
    pub apdu_retries: u8,
    /// Maximum APDU length this client accepts.
    pub max_apdu_length: u16,
    /// Maximum segments this client accepts (`None` = unspecified).
    ///
    /// Values 0 and 1 are invalid. Finite values between the BACnet wire
    /// encodings are conservatively rounded down (for example, 5 advertises 4).
    pub max_segments: Option<u8>,
    /// Whether this client accepts segmented responses.
    pub segmented_response_accepted: bool,
    /// Proposed window size for segmented transfers (1-127, default 1).
    pub proposed_window_size: u8,
}

/// Additional client startup options.
#[derive(Clone)]
pub struct ClientOptions {
    /// Capacity of the COV notification broadcast channel.
    ///
    /// Slow receivers lag once more than this many notifications arrive before
    /// they call `recv()`. The default preserves the historical fixed capacity
    /// of 64.
    pub cov_channel_capacity: usize,
    confirmed_cov_notification_ack_policy: ConfirmedCOVNotificationAckPolicy,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            interface: Ipv4Addr::UNSPECIFIED,
            port: 0xBAC0,
            broadcast_address: Ipv4Addr::BROADCAST,
            apdu_timeout_ms: 6000,
            apdu_retries: 3,
            max_apdu_length: 1476,
            max_segments: None,
            segmented_response_accepted: true,
            proposed_window_size: 1,
        }
    }
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            cov_channel_capacity: DEFAULT_COV_CHANNEL_CAPACITY,
            confirmed_cov_notification_ack_policy:
                cov_notifications::default_confirmed_cov_notification_ack_policy(),
        }
    }
}

pub(crate) fn new_coordinated_tsm(
    config: &ClientConfig,
    coordinator: Arc<OutboundTransactionCoordinator>,
) -> Tsm {
    Tsm::new_coordinated(
        TsmConfig {
            apdu_timeout_ms: config.apdu_timeout_ms,
            apdu_segment_timeout_ms: config.apdu_timeout_ms,
            apdu_retries: config.apdu_retries,
        },
        coordinator,
    )
}

pub(crate) fn confirmed_response_result(response: TsmResponse) -> Result<Bytes, Error> {
    match response {
        TsmResponse::SimpleAck => Ok(Bytes::new()),
        TsmResponse::ComplexAck { service_data } => Ok(service_data),
        TsmResponse::Error { class, code } => Err(Error::Protocol { class, code }),
        TsmResponse::Reject { reason } => Err(Error::Reject { reason }),
        TsmResponse::Abort { reason } => Err(Error::Abort { reason }),
        TsmResponse::NetworkPathTooLong { dnet } => Err(Error::RoutedPathTooLong { dnet }),
    }
}

impl ClientOptions {
    /// Set the COV notification broadcast channel capacity.
    pub fn with_cov_channel_capacity(mut self, capacity: usize) -> Self {
        self.cov_channel_capacity = capacity;
        self
    }

    fn validate(&self) -> Result<(), Error> {
        if !(1..=MAX_COV_CHANNEL_CAPACITY).contains(&self.cov_channel_capacity) {
            return Err(Error::Encoding(format!(
                "invalid cov-channel-capacity {}; expected 1..={}",
                self.cov_channel_capacity, MAX_COV_CHANNEL_CAPACITY
            )));
        }
        Ok(())
    }
}

/// Generic builder for BACnetClient with a pre-built transport.
pub struct ClientBuilder<T: TransportPort> {
    config: ClientConfig,
    options: ClientOptions,
    transport: Option<T>,
}

impl<T: TransportPort + 'static> ClientBuilder<T> {
    /// Set the pre-built transport.
    pub fn transport(mut self, transport: T) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Set APDU timeout in milliseconds.
    pub fn apdu_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_timeout_ms = ms;
        self
    }

    /// Set the maximum APDU length this client accepts.
    pub fn max_apdu_length(mut self, len: u16) -> Self {
        self.config.max_apdu_length = len;
        self
    }

    /// Set the COV notification broadcast channel capacity.
    pub fn cov_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.cov_channel_capacity = capacity;
        self
    }

    /// Build and start the client.
    pub async fn build(self) -> Result<BACnetClient<T>, Error> {
        let transport = self
            .transport
            .ok_or_else(|| Error::Encoding("transport not set on ClientBuilder".into()))?;
        BACnetClient::start_with_options(self.config, transport, self.options).await
    }
}

/// BIP-specific builder that constructs `BipTransport` from interface/port/broadcast fields.
pub struct BipClientBuilder {
    config: ClientConfig,
    options: ClientOptions,
}

impl BipClientBuilder {
    /// Set the local interface IP.
    pub fn interface(mut self, ip: Ipv4Addr) -> Self {
        self.config.interface = ip;
        self
    }

    /// Set the UDP port (0 for ephemeral).
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the directed broadcast address.
    pub fn broadcast_address(mut self, addr: Ipv4Addr) -> Self {
        self.config.broadcast_address = addr;
        self
    }

    /// Set APDU timeout in milliseconds.
    pub fn apdu_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_timeout_ms = ms;
        self
    }

    /// Set the maximum APDU length this client accepts.
    pub fn max_apdu_length(mut self, len: u16) -> Self {
        self.config.max_apdu_length = len;
        self
    }

    /// Set the COV notification broadcast channel capacity.
    pub fn cov_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.cov_channel_capacity = capacity;
        self
    }

    /// Build and start the client, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetClient<BipTransport>, Error> {
        let transport = BipTransport::new(
            self.config.interface,
            self.config.port,
            self.config.broadcast_address,
        );
        BACnetClient::start_with_options(self.config, transport, self.options).await
    }
}

// ---------------------------------------------------------------------------
// Multi-device batch operation types
// ---------------------------------------------------------------------------

/// Default concurrency limit for multi-device batch operations.
const DEFAULT_BATCH_CONCURRENCY: usize = 32;

/// A request to read a single property from a discovered device.
#[derive(Debug, Clone)]
pub struct DeviceReadRequest {
    /// Device instance number (must be in the device table).
    pub device_instance: u32,
    /// Object to read from.
    pub object_identifier: bacnet_types::primitives::ObjectIdentifier,
    /// Property to read.
    pub property_identifier: bacnet_types::enums::PropertyIdentifier,
    /// Optional array index.
    pub property_array_index: Option<u32>,
}

/// Result of a single-property read from a device within a batch.
#[derive(Debug)]
pub struct DeviceReadResult {
    /// The device instance this result corresponds to.
    pub device_instance: u32,
    /// The read result (Ok = decoded ACK, Err = protocol/timeout error).
    pub result: Result<bacnet_services::read_property::ReadPropertyACK, Error>,
}

/// A request to read multiple properties from a discovered device (RPM).
#[derive(Debug, Clone)]
pub struct DeviceRpmRequest {
    /// Device instance number (must be in the device table).
    pub device_instance: u32,
    /// ReadAccessSpecifications to send in a single RPM.
    pub specs: Vec<bacnet_services::rpm::ReadAccessSpecification>,
}

/// Result of an RPM to a single device within a batch.
#[derive(Debug)]
pub struct DeviceRpmResult {
    /// The device instance this result corresponds to.
    pub device_instance: u32,
    /// The RPM result.
    pub result: Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error>,
}

/// A request to write a single property on a discovered device.
#[derive(Debug, Clone)]
pub struct DeviceWriteRequest {
    /// Device instance number (must be in the device table).
    pub device_instance: u32,
    /// Object to write to.
    pub object_identifier: bacnet_types::primitives::ObjectIdentifier,
    /// Property to write.
    pub property_identifier: bacnet_types::enums::PropertyIdentifier,
    /// Optional array index.
    pub property_array_index: Option<u32>,
    /// Encoded property value bytes.
    pub property_value: Vec<u8>,
    /// Optional write priority (1-16).
    pub priority: Option<u8>,
}

/// Result of a single-property write to a device within a batch.
#[derive(Debug)]
pub struct DeviceWriteResult {
    /// The device instance this result corresponds to.
    pub device_instance: u32,
    /// The write result (Ok = success, Err = protocol/timeout error).
    pub result: Result<(), Error>,
}

/// The receive-side promises this client puts in every confirmed request.
///
/// Clause 20.1.2.3 and Clause 20.1.2.4 exist "so that the responding device may
/// determine how to convey its response", so the same values must bound what
/// the dispatch loop is willing to take back. Keeping the pair together stops
/// the two halves from drifting as they are threaded through dispatch.
#[derive(Debug, Clone, Copy)]
struct ResponseLimits {
    /// Clause 20.1.2.3 'segmented-response-accepted'.
    segmented_response_accepted: bool,
    /// Most segments this client will hold for one reassembly.
    max_reassembly_segments: usize,
}

/// In-progress segmented receive state.
struct SegmentedReceiveState {
    receiver: SegmentReceiver,
    owner: TransactionOwner,
    /// Immediate MAC used to send SegmentAck/Abort PDUs.
    reply_mac: MacAddr,
    /// The peer's SNET/SADR when the segments arrive through a router; the
    /// reply's DNET/DADR, or the router takes the reply for itself instead
    /// of forwarding it (Clause 6.5.2.1).
    reply_network: Option<NpduAddress>,
    /// Next expected sequence number (for gap detection).
    expected_next_seq: u8,
    /// Last sequence number in the previously completed receive window.
    initial_sequence_number: u8,
    /// Last segment accepted in order.
    last_sequence_number: u8,
    /// Duplicates silently discarded in the current receive window.
    duplicate_count: u8,
    /// Window position counter for per-window SegmentAck (Clause 5.2.2).
    window_position: u8,
    /// Window size accepted for this receive session.
    actual_window_size: u8,
    /// How many distinct segments have been stored.
    ///
    /// Monotonic, and deliberately not derived from `expected_next_seq`, which
    /// is a `u8` and wraps: Clause 20.1.5.4 makes sequence numbers modulo 256,
    /// so after 256 segments the counter returns to a slot already occupied.
    /// This is the only value that can tell 257 segments from 1.
    accepted_segments: usize,
}

/// Key for tracking in-progress segmented receives: (correlation_mac, invoke_id).
type SegKey = (MacAddr, u8);

struct SegmentAckRoute {
    owner: TransactionOwner,
    sender: mpsc::Sender<SegmentAckPdu>,
}

/// BACnet client with low-level and high-level request APIs.
pub struct BACnetClient<T: TransportPort> {
    config: ClientConfig,
    network: Arc<NetworkLayer<T>>,
    tsm: Arc<Mutex<Tsm>>,
    device_table: Arc<Mutex<DeviceTable>>,
    cov_tx: broadcast::Sender<ReceivedCOVNotification>,
    device_tx: broadcast::Sender<DeviceEvent>,
    dispatch_task: Option<JoinHandle<()>>,
    /// Owner-qualified channels feeding SegmentACKs to in-flight segmented sends.
    ///
    /// Dispatch may hold [`Self::tsm`] while acquiring this lock so phase
    /// validation and delivery cannot race terminal completion. No path may
    /// acquire the locks in the opposite order.
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, SegmentAckRoute>>>,
    cleanup_tx: mpsc::UnboundedSender<TransactionCleanup>,
    #[cfg(test)]
    segmented_post_wait_cleanup: Arc<SegmentedPostWaitCleanupHook>,
    #[cfg(test)]
    segmented_cleanup: Arc<SegmentedCleanupHook>,
    local_mac: MacAddr,
    routed_path_limits: Arc<RoutedPathLimits>,
}

impl BACnetClient<BipTransport> {
    /// Create a BIP-specific builder with interface/port/broadcast fields.
    pub fn bip_builder() -> BipClientBuilder {
        BipClientBuilder {
            config: ClientConfig::default(),
            options: ClientOptions::default(),
        }
    }

    pub fn builder() -> BipClientBuilder {
        Self::bip_builder()
    }

    /// Read the Broadcast Distribution Table from a BBMD.
    pub async fn read_bdt(
        &self,
        target: &[u8],
    ) -> Result<Vec<bacnet_transport::bbmd::BdtEntry>, Error> {
        self.network.transport().read_bdt(target).await
    }

    /// Write the Broadcast Distribution Table to a BBMD.
    pub async fn write_bdt(
        &self,
        target: &[u8],
        entries: &[bacnet_transport::bbmd::BdtEntry],
    ) -> Result<bacnet_types::enums::BvlcResultCode, Error> {
        self.network.transport().write_bdt(target, entries).await
    }

    /// Read the Foreign Device Table from a BBMD.
    pub async fn read_fdt(
        &self,
        target: &[u8],
    ) -> Result<Vec<bacnet_transport::bbmd::FdtEntryWire>, Error> {
        self.network.transport().read_fdt(target).await
    }

    /// Delete a Foreign Device Table entry on a BBMD.
    pub async fn delete_fdt_entry(
        &self,
        target: &[u8],
        ip: [u8; 4],
        port: u16,
    ) -> Result<bacnet_types::enums::BvlcResultCode, Error> {
        self.network
            .transport()
            .delete_fdt_entry(target, ip, port)
            .await
    }

    /// Register as a foreign device with a BBMD and return the result code.
    pub async fn register_foreign_device_bvlc(
        &self,
        target: &[u8],
        ttl: u16,
    ) -> Result<bacnet_types::enums::BvlcResultCode, Error> {
        self.network
            .transport()
            .register_foreign_device_bvlc(target, ttl)
            .await
    }
}

#[cfg(feature = "ipv6")]
impl BACnetClient<Bip6Transport> {
    /// Create a BIP6-specific builder for BACnet/IPv6 transport.
    pub fn bip6_builder() -> Bip6ClientBuilder {
        Bip6ClientBuilder {
            config: ClientConfig::default(),
            options: ClientOptions::default(),
            interface: Ipv6Addr::UNSPECIFIED,
            device_instance: None,
        }
    }
}

/// BIP6-specific builder that constructs `Bip6Transport` from IPv6 interface/port/device-instance.
#[cfg(feature = "ipv6")]
pub struct Bip6ClientBuilder {
    config: ClientConfig,
    options: ClientOptions,
    interface: Ipv6Addr,
    device_instance: Option<u32>,
}

#[cfg(feature = "ipv6")]
impl Bip6ClientBuilder {
    /// Set the local IPv6 interface address.
    pub fn interface(mut self, ip: Ipv6Addr) -> Self {
        self.interface = ip;
        self
    }

    /// Set the UDP port (0 for ephemeral).
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Set the device instance for VMAC derivation (Annex U.5).
    pub fn device_instance(mut self, instance: u32) -> Self {
        self.device_instance = Some(instance);
        self
    }

    /// Set APDU timeout in milliseconds.
    pub fn apdu_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_timeout_ms = ms;
        self
    }

    /// Set the maximum APDU length this client accepts.
    pub fn max_apdu_length(mut self, len: u16) -> Self {
        self.config.max_apdu_length = len;
        self
    }

    /// Set the COV notification broadcast channel capacity.
    pub fn cov_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.cov_channel_capacity = capacity;
        self
    }

    /// Build and start the client, constructing a Bip6Transport from the config.
    pub async fn build(self) -> Result<BACnetClient<Bip6Transport>, Error> {
        let transport = Bip6Transport::new(self.interface, self.config.port, self.device_instance);
        BACnetClient::start_with_options(self.config, transport, self.options).await
    }
}

#[cfg(feature = "sc-tls")]
impl BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>> {
    /// Create an SC-specific builder that connects to a BACnet/SC hub.
    pub fn sc_builder() -> ScClientBuilder {
        ScClientBuilder {
            config: ClientConfig::default(),
            options: ClientOptions::default(),
            hub_url: String::new(),
            tls_config: None,
            vmac: [0; 6],
            device_uuid: [0; 16],
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            reconnect: None,
        }
    }
}

/// SC-specific client builder.
///
/// Created by [`BACnetClient::sc_builder()`].  Requires the `sc-tls` feature.
#[cfg(feature = "sc-tls")]
pub struct ScClientBuilder {
    config: ClientConfig,
    options: ClientOptions,
    hub_url: String,
    tls_config: Option<std::sync::Arc<tokio_rustls::rustls::ClientConfig>>,
    vmac: bacnet_transport::sc_frame::Vmac,
    device_uuid: [u8; 16],
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    reconnect: Option<bacnet_transport::sc::ScReconnectConfig>,
}

#[cfg(feature = "sc-tls")]
impl ScClientBuilder {
    /// Set the hub WebSocket URL (e.g. `wss://hub.example.com/bacnet`).
    pub fn hub_url(mut self, url: &str) -> Self {
        self.hub_url = url.to_string();
        self
    }

    /// Set the TLS client configuration.
    pub fn tls_config(
        mut self,
        config: std::sync::Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Self {
        self.tls_config = Some(config);
        self
    }

    /// Set the local VMAC address.
    pub fn vmac(mut self, vmac: [u8; 6]) -> Self {
        self.vmac = vmac;
        self
    }

    /// Set the persistent BACnet/SC device UUID.
    pub fn device_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.device_uuid = uuid;
        self
    }

    /// Set the APDU timeout in milliseconds.
    pub fn apdu_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_timeout_ms = ms;
        self
    }

    /// Set the COV notification broadcast channel capacity.
    pub fn cov_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.cov_channel_capacity = capacity;
        self
    }

    /// Set the heartbeat interval in milliseconds (default 30 000).
    pub fn heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.heartbeat_interval_ms = ms;
        self
    }

    /// Set the heartbeat timeout in milliseconds (default 60 000).
    pub fn heartbeat_timeout_ms(mut self, ms: u64) -> Self {
        self.heartbeat_timeout_ms = ms;
        self
    }

    /// Enable automatic reconnection with the given configuration.
    pub fn reconnect(mut self, config: bacnet_transport::sc::ScReconnectConfig) -> Self {
        self.reconnect = Some(config);
        self
    }

    fn validate_identity(&self) -> Result<(), Error> {
        match self.vmac {
            bacnet_transport::sc_frame::UNKNOWN_VMAC => Err(Error::Encoding(
                "SC client builder: vmac must not be the reserved unknown VMAC".into(),
            )),
            bacnet_transport::sc_frame::BROADCAST_VMAC => Err(Error::Encoding(
                "SC client builder: vmac must not be the reserved broadcast VMAC".into(),
            )),
            _ if self.device_uuid == [0; 16] => Err(Error::Encoding(
                "SC client builder: device_uuid is required and must not be all zero".into(),
            )),
            _ => Ok(()),
        }
    }

    fn sc_transport<W: bacnet_transport::sc::WebSocketPort>(
        &self,
        ws: W,
    ) -> bacnet_transport::sc::ScTransport<W> {
        bacnet_transport::sc::ScTransport::new(ws, self.vmac)
            .with_device_uuid(self.device_uuid)
            .with_heartbeat_interval_ms(self.heartbeat_interval_ms)
            .with_heartbeat_timeout_ms(self.heartbeat_timeout_ms)
    }

    #[cfg(test)]
    pub(crate) async fn build_with_websocket_for_test<
        W: bacnet_transport::sc::WebSocketPort + 'static,
    >(
        self,
        ws: W,
    ) -> Result<BACnetClient<bacnet_transport::sc::ScTransport<W>>, Error> {
        self.validate_identity()?;
        self.options.validate()?;
        validate_max_segments(self.config.max_segments)?;
        let transport = self.sc_transport(ws);
        BACnetClient::start_with_options(self.config, transport, self.options).await
    }

    /// Connect to the hub and start the client.
    pub async fn build(
        self,
    ) -> Result<
        BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        self.validate_identity()?;
        self.options.validate()?;
        validate_max_segments(self.config.max_segments)?;
        let tls_config =
            self.tls_config.as_ref().cloned().ok_or_else(|| {
                Error::Encoding("SC client builder: tls_config is required".into())
            })?;

        let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&self.hub_url, tls_config.clone())
            .await?;

        let mut transport = self.sc_transport(ws);
        if let Some(rc) = self.reconnect {
            let hub_url = self.hub_url.clone();
            let tls_config = tls_config.clone();
            #[allow(deprecated)]
            {
                transport = transport
                    .with_connector(move || {
                        let hub_url = hub_url.clone();
                        let tls_config = tls_config.clone();
                        async move {
                            bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config)
                                .await
                        }
                    })
                    .with_reconnect(rc);
            }
        }

        BACnetClient::start_with_options(self.config, transport, self.options).await
    }
}

/// Routing target for confirmed requests.
#[derive(Clone, Copy)]
enum ConfirmedTarget<'a> {
    Local {
        mac: &'a [u8],
    },
    Routed {
        router_mac: &'a [u8],
        dest_network: u16,
        dest_mac: &'a [u8],
    },
}

impl<'a> ConfirmedTarget<'a> {
    fn additional_npdu_header_len(&self) -> u16 {
        match self {
            Self::Local { .. } => 0,
            Self::Routed { dest_mac, .. } => u16::try_from(dest_mac.len())
                .unwrap_or(u16::MAX)
                .saturating_add(4),
        }
    }
}

fn max_apdu_bucket_at_or_below(limit: u16) -> Option<u16> {
    VALID_MAX_APDU_LENGTHS
        .iter()
        .rev()
        .copied()
        .find(|bucket| *bucket <= limit)
}

fn cap_max_apdu_to_transport(configured: u16, transport_limit: u16) -> Result<u16, Error> {
    let limit = configured.min(transport_limit);
    max_apdu_bucket_at_or_below(limit).ok_or_else(|| {
        Error::Encoding(format!(
            "transport max-APDU-length {transport_limit} leaves no valid BACnet APDU bucket"
        ))
    })
}

mod builder_options;
mod cov;
mod cov_notifications;
mod cov_renewal;
mod device_events;
mod device_mgmt;
mod discovery;
mod dispatch;
mod file_list;
mod lifecycle;
mod object_mgmt;
mod property;
mod requests;
mod response_admission;
mod routed_path_limits;
mod segmentation;
mod segmented_request;
mod transaction_cleanup;
mod transaction_peer;
use routed_path_limits::{routed_path_quarantine_horizon, RoutedPathLease, RoutedPathLimits};
use transaction_peer::response_transaction_peer;

pub use cov_notifications::{
    COVNotificationDelivery, ConfirmedCOVNotificationAckPolicy, ConfirmedCOVNotificationResponse,
    ReceivedCOVNotification,
};
pub use cov_renewal::{
    ManagedCOVSubscription, ManagedCOVSubscriptionEvent, ManagedCOVSubscriptionOptions,
};

#[cfg(test)]
mod builder_options_tests;
#[cfg(test)]
mod confirmed_request_dispatch_tests;
#[cfg(test)]
mod coordinator_tests;
#[cfg(test)]
mod cov_notification_tests;
#[cfg(test)]
mod cov_renewal_tests;
#[cfg(test)]
mod cov_tests;
#[cfg(test)]
mod device_events_tests;
#[cfg(test)]
mod peer_max_apdu_tests;
#[cfg(test)]
mod peer_segmentation_tests;
#[cfg(test)]
mod request_timer_tests;
#[cfg(test)]
mod response_correlation_tests;
#[cfg(test)]
mod routed_max_apdu_tests;
#[cfg(test)]
mod routed_path_limit_tests;
#[cfg(test)]
mod routed_reply_tests;
#[cfg(all(test, feature = "sc-tls"))]
mod sc_builder_tests;
#[cfg(test)]
mod sc_max_apdu_tests;
#[cfg(test)]
mod segmentation_retransmit_tests;
#[cfg(test)]
mod segmented_receive_duplicate_tests;
#[cfg(test)]
mod segmented_receive_lifecycle_tests;
#[cfg(test)]
mod segmented_request_ordering_tests;
#[cfg(test)]
mod segmented_request_state_tests;
#[cfg(test)]
mod segmented_response_admission_tests;
#[cfg(test)]
mod segmented_response_capacity_tests;
#[cfg(test)]
mod segmented_timeout_tests;
#[cfg(test)]
mod tests;

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Create a generic builder that accepts a pre-built transport.
    pub fn generic_builder() -> ClientBuilder<T> {
        ClientBuilder {
            config: ClientConfig::default(),
            options: ClientOptions::default(),
            transport: None,
        }
    }
}
