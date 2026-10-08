//! BACnetClient: high-level and low-level request APIs.
//!
//! The client owns a NetworkLayer, spawns an APDU dispatch task, and provides
//! methods for sending confirmed and unconfirmed BACnet requests.
//! Replies to inbound accepted-direct confirmed requests use the original sealed
//! response capability before any prompt channel, without address fallback on
//! retirement or invalid authority. This does not change outgoing transaction
//! correlation, retries or their segmented controls.

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
use bacnet_transport::bip::{BipTransport, ForeignDeviceConfig};
#[cfg(feature = "ipv6")]
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::port::{TransportPort, TransportProvenance};
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

/// Notification emitted when two endpoints claim the same Device instance.
#[derive(Debug, Clone)]
pub struct DeviceCollisionEvent {
    /// Snapshot of the authoritative discovery-table row that was retained.
    pub retained: DiscoveredDevice,
    /// Snapshot constructed from the conflicting incoming I-Am.
    pub incoming: DiscoveredDevice,
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
    /// B/IP only: bind the interface address itself, so transports on other
    /// addresses of this host can share the port (#1538). Off by default.
    /// Needs an explicit interface and a nonzero port; see
    /// `BipTransport::set_share_port_by_address` for what changes, including
    /// that broadcasts and unicast are then received in no fixed order. Only
    /// the B/IP builder reads it; a transport passed to `start` has its own.
    pub share_port_by_address: bool,
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
    /// Least time, in milliseconds, from the latest confirmed request sent
    /// to a destination finishing (reply, error or cancellation) to the next
    /// being sent; while it is still outstanding, the next waits that long
    /// after it was sent. A waiting request checks again when it wakes;
    /// requests waiting together go one at a time, in no promised order. A
    /// request given up before it went leaves no trace. 0 (the default)
    /// sends at once. A slow
    /// device then serves its other clients between this client's requests,
    /// paging a log or polling alike (#1535). Behind a router, whose path
    /// lease every device on that network shares, the pause after a reply
    /// holds for requests made one after another only. At most
    /// [`MAX_MIN_REQUEST_INTERVAL_MS`], an hour; more fails the build. The
    /// endpoint client takes the same setting from its session (#1542).
    pub min_request_interval_ms: u64,
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
    /// Capacity of the independent event notification broadcast channel (default 64).
    /// Slow receivers observe lag; valid confirmed notifications are still acknowledged.
    pub event_channel_capacity: usize,
    confirmed_cov_notification_ack_policy: ConfirmedCOVNotificationAckPolicy,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            interface: Ipv4Addr::UNSPECIFIED,
            port: 0xBAC0,
            broadcast_address: Ipv4Addr::BROADCAST,
            share_port_by_address: false,
            apdu_timeout_ms: 6000,
            apdu_retries: 3,
            max_apdu_length: 1476,
            max_segments: None,
            segmented_response_accepted: true,
            proposed_window_size: 1,
            min_request_interval_ms: 0,
        }
    }
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            cov_channel_capacity: DEFAULT_COV_CHANNEL_CAPACITY,
            event_channel_capacity: DEFAULT_EVENT_CHANNEL_CAPACITY,
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
        TsmResponse::Error {
            class,
            code,
            detail,
        } => Err(Error::protocol(class, code, detail)),
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
        if !(1..=MAX_EVENT_CHANNEL_CAPACITY).contains(&self.event_channel_capacity) {
            return Err(Error::Encoding(format!(
                "invalid event-channel-capacity {}; expected 1..={}",
                self.event_channel_capacity, MAX_EVENT_CHANNEL_CAPACITY
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
    foreign_device: Option<ForeignDeviceConfig>,
}

impl BipClientBuilder {
    /// Enable automatic foreign-device registration and DBTN broadcasts to
    /// this BBMD. `build()` starts locally; it does not wait for BBMD acceptance.
    /// Poll `client.transport().bvlc_client_snapshot()` for matched outcomes.
    /// Invalid TTL/renewal settings fail before transport I/O.
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::bip::ForeignDeviceConfig;
    /// use std::{net::Ipv4Addr, time::Duration};
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let mut client = BACnetClient::bip_builder()
    ///     .port(0)
    ///     .foreign_device(ForeignDeviceConfig {
    ///         bbmd_ip: Ipv4Addr::new(192, 0, 2, 10), bbmd_port: 47808,
    ///         ttl: 60, renewal_interval: Some(Duration::from_secs(20)),
    ///     })
    ///     .build().await?;
    /// let observed = client.transport().bvlc_client_snapshot();
    /// // A successful local start alone does not establish registration.
    /// let last_attempt = observed.foreign_registration.last_outcome;
    /// client.stop().await?;
    /// # Ok(()) }
    /// ```
    pub fn foreign_device(mut self, config: ForeignDeviceConfig) -> Self {
        self.foreign_device = Some(config);
        self
    }

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

    /// Bind the interface address itself, so transports on other addresses of
    /// this host can share the port, each getting only its own unicast
    /// (#1538). Off by default, which binds `0.0.0.0`. Needs an explicit
    /// [`interface`](Self::interface) and a nonzero port, or `build` fails.
    /// Broadcasts and unicast are then received in no fixed order; see
    /// [`BipTransport::set_share_port_by_address`].
    pub fn share_port_by_address(mut self, enabled: bool) -> Self {
        self.config.share_port_by_address = enabled;
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
        let config = &self.config;
        let mut transport =
            BipTransport::new(config.interface, config.port, config.broadcast_address);
        transport.set_share_port_by_address(config.share_port_by_address);
        if let Some(foreign) = self.foreign_device {
            transport.register_as_foreign_device(foreign);
        }
        BACnetClient::start_with_options(self.config, transport, self.options).await
    }
}

// ---------------------------------------------------------------------------
// Multi-device batch operation types
// ---------------------------------------------------------------------------

/// Default concurrency limit for multi-device batch operations.
const DEFAULT_BATCH_CONCURRENCY: usize = 32;

mod batch_debug;

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
pub struct DeviceReadResult {
    /// Zero-based position of this occurrence in the original request vector.
    pub request_index: usize,
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
    pub specs: Vec<bacnet_types::constructed::ReadAccessSpecification>,
}

/// Result of an RPM to a single device within a batch.
pub struct DeviceRpmResult {
    /// Zero-based position of this occurrence in the original request vector.
    pub request_index: usize,
    /// The device instance this result corresponds to.
    pub device_instance: u32,
    /// The RPM result.
    pub result: Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error>,
}

/// A request to write a single property on a discovered device.
#[derive(Clone)]
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
    /// Zero-based position of this occurrence in the original request vector.
    pub request_index: usize,
    /// The device instance this result corresponds to.
    pub device_instance: u32,
    /// The write result (Ok = success, Err = protocol/timeout error).
    pub result: Result<(), Error>,
}

/// The receive-side promises this client puts in every confirmed request.
///
/// Clause 20.1.2.3 and Clause 20.1.2.4 let the responder choose a response
/// form the requester can receive, so the same values must bound what
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
    /// Provenance snapshot at session open (RB-07). Compared by value on
    /// every later segment; a conflicting context fails closed (abort).
    /// Expires with the session; SC disconnect drops the transport queue so
    /// no snapshot outlives its connection.
    provenance: TransportProvenance,
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

/// Key for tracking in-progress segmented receives:
/// (correlation_mac, invoke_id, provenance).
///
/// Including the immutable provenance snapshot gives cross-peer isolation:
/// the same MAC via different trust contexts never shares a reassembly
/// session. A conflicting provenance for an otherwise identical key is a
/// fail-closed abort, not a merge (RB-07 compat mode: no policy change).
type SegKey = (MacAddr, u8, TransportProvenance);

/// Key for routing inbound SegmentACKs to in-flight segmented sends:
/// (correlation_mac, invoke_id).
///
/// Compat mode: provenance is threaded to the dispatch point but does not
/// gate SegmentACK delivery (RB-09 consumes it later).
type SegAckKey = (MacAddr, u8);

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
    event_tx: broadcast::Sender<ReceivedEventNotification>,
    device_tx: broadcast::Sender<DeviceEvent>,
    device_collision_tx: broadcast::Sender<DeviceCollisionEvent>,
    dispatch_task: Option<JoinHandle<()>>,
    network_number_task: Option<JoinHandle<()>>,
    /// Owner-qualified channels feeding SegmentACKs to in-flight segmented sends.
    ///
    /// Dispatch may hold [`Self::tsm`] while acquiring this lock so phase
    /// validation and delivery cannot race terminal completion. No path may
    /// acquire the locks in the opposite order.
    seg_ack_senders: Arc<Mutex<HashMap<SegAckKey, SegmentAckRoute>>>,
    cleanup_tx: mpsc::UnboundedSender<TransactionCleanup>,
    #[cfg(test)]
    segmented_post_wait_cleanup: Arc<SegmentedPostWaitCleanupHook>,
    #[cfg(test)]
    segmented_cleanup: Arc<SegmentedCleanupHook>,
    local_mac: MacAddr,
    routed_path_limits: Arc<RoutedPathLimits>,
    /// See [`Self::group_source_request_drops`].
    group_source_request_drops: Arc<std::sync::atomic::AtomicU64>,
    pacer: Arc<pacing::RequestPacer>,
}

impl BACnetClient<BipTransport> {
    /// Create a BIP-specific builder with interface/port/broadcast fields.
    pub fn bip_builder() -> BipClientBuilder {
        BipClientBuilder {
            config: ClientConfig::default(),
            options: ClientOptions::default(),
            foreign_device: None,
        }
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
    tls_config: Option<bacnet_transport::sc_tls::ScNodeTlsConfig>,
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

    /// Set the validated local node TLS policy, shared across initial and
    /// reconnect attempts (including normal TLS resumption).
    ///
    /// ```
    /// use bacnet_client::client::{BACnetClient, ScClientBuilder};
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    /// fn configured(tls: ScNodeTlsConfig) -> ScClientBuilder {
    ///     BACnetClient::sc_builder().tls_config(tls)
    /// }
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use bacnet_client::client::BACnetClient;
    /// fn raw(config: std::sync::Arc<tokio_rustls::rustls::ClientConfig>) {
    ///     let _ = BACnetClient::sc_builder().tls_config(config);
    /// }
    /// ```
    pub fn tls_config(mut self, config: bacnet_transport::sc_tls::ScNodeTlsConfig) -> Self {
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
        if let Some(config) = &self.reconnect {
            config.validate()?;
        }
        self.validate_identity()?;
        self.options.validate()?;
        validate_max_segments(self.config.max_segments)?;
        pacing::validate_interval_ms(self.config.min_request_interval_ms)?;
        let transport = self.sc_transport(ws);
        BACnetClient::start_with_options(self.config, transport, self.options).await
    }

    /// Connect to the hub and start the client.
    ///
    /// Reconnect configuration is validated before TLS lookup or dialing, and
    /// again when the transport starts. An error still consumes this builder
    /// and drops its inputs; this does not promise generic endpoint rollback.
    ///
    /// The client's [`transport()`](BACnetClient::transport) lends the hub
    /// connection-state watch and the NPDU drop counts:
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::sc::ScConnectionState;
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    ///
    /// async fn watch_hub(tls: ScNodeTlsConfig) -> Result<(), bacnet_types::error::Error> {
    ///     let client = BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .tls_config(tls)
    ///         .vmac([0x02, 0, 0, 0, 0, 0x01])
    ///         .device_uuid([0x42; 16])
    ///         .build()
    ///         .await?;
    ///     // An owned receiver: it can live in its own task.
    ///     let mut state = client.transport().connection_state_changes();
    ///     tokio::spawn(async move {
    ///         while state.changed().await.is_ok() {
    ///             let _connected = *state.borrow_and_update() == ScConnectionState::Connected;
    ///         }
    ///     });
    ///     // A snapshot: poll it as often as needed.
    ///     let _full_queue_drops = client.transport().npdu_drop_counts().full_drops;
    ///     Ok(())
    /// }
    /// ```
    pub async fn build(
        self,
    ) -> Result<
        BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        if let Some(config) = &self.reconnect {
            config.validate()?;
        }
        self.validate_identity()?;
        self.options.validate()?;
        validate_max_segments(self.config.max_segments)?;
        pacing::validate_interval_ms(self.config.min_request_interval_ms)?;
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
            transport = transport
                .with_connector(move || {
                    let hub_url = hub_url.clone();
                    let tls_config = tls_config.clone();
                    async move {
                        bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config).await
                    }
                })
                .with_reconnect(rc);
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
    bacnet_encoding::apdu::max_apdu_header_at_or_below(u32::from(limit)).ok()
}

fn cap_max_apdu_to_transport(configured: u16, transport_limit: u16) -> Result<u16, Error> {
    let limit = configured.min(transport_limit);
    max_apdu_bucket_at_or_below(limit).ok_or_else(|| {
        Error::Encoding(format!(
            "transport max-APDU-length {transport_limit} leaves no valid BACnet APDU bucket"
        ))
    })
}

mod audit;
mod builder_options;
mod cov;
mod cov_notifications;
mod cov_renewal;
mod device_events;
mod device_mgmt;
mod discovery;
mod dispatch;
mod dispatch_context;
mod inbound_replies;
use inbound_replies::InboundReply;
mod event_notifications;
mod file_list;
mod lifecycle;
mod object_mgmt;
pub(crate) mod pacing;
mod property;
mod read_range;
mod requests;
mod response_admission;
mod routed_path_limits;
mod segmentation;
mod segmentation_abort;
mod segmentation_context;
mod segmented_request;
mod transaction_cleanup;
mod transaction_peer;
mod transport_access;
mod write_group;
pub(crate) use routed_path_limits::check_routed_unicast;
use routed_path_limits::{
    forwarded_npci_len, routed_path_quarantine_horizon, RoutedPathLease, RoutedPathLimits,
};
pub(crate) use transaction_peer::TransactionPeer;

pub use cov::CovPropertySubscription;
pub use cov_notifications::{
    COVNotificationDelivery, ConfirmedCOVNotificationAckPolicy, ConfirmedCOVNotificationResponse,
    ReceivedCOVNotification,
};
pub use cov_renewal::{
    ManagedCOVSubscription, ManagedCOVSubscriptionEvent, ManagedCOVSubscriptionOptions,
};
pub use event_notifications::{
    EventNotificationDelivery, ReceivedEventNotification, DEFAULT_EVENT_CHANNEL_CAPACITY,
    MAX_EVENT_CHANNEL_CAPACITY,
};
pub use pacing::MAX_MIN_REQUEST_INTERVAL_MS;
pub use write_group::WriteGroupDestination;

#[cfg(test)]
mod acknowledge_alarm_tests;
#[cfg(test)]
mod audit_tests;
#[cfg(test)]
mod batch_tests;
#[cfg(test)]
mod broadcast_mac_tests;
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
mod event_notification_tests;
#[cfg(test)]
pub(crate) mod fake_device;
#[cfg(test)]
mod group_source_request_tests;
#[cfg(test)]
mod list_error_tests;
#[cfg(test)]
mod list_validation_tests;
#[cfg(test)]
mod pacing_tests;
#[cfg(test)]
mod peer_max_apdu_tests;
#[cfg(test)]
mod peer_segmentation_tests;
#[cfg(test)]
mod rb07_provenance_tests;
#[cfg(test)]
mod read_range_tests;
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
mod structured_error_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_access_tests;
#[cfg(test)]
mod wpm_validation_tests;
#[cfg(test)]
mod write_priority_tests;

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

#[cfg(test)]
mod number_tests;

mod network_number;
