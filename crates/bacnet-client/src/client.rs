//! BACnetClient: high-level and low-level request APIs.
//!
//! The client owns a NetworkLayer, spawns an APDU dispatch task, and provides
//! methods for sending confirmed and unconfirmed BACnet requests.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};
use tracing::{debug, warn};

use bacnet_encoding::apdu::{
    self, encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu, SegmentAck as SegmentAckPdu,
    SimpleAck,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_network::layer::NetworkLayer;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_transport::port::TransportPort;
use bacnet_types::enums::{ConfirmedServiceChoice, NetworkPriority, UnconfirmedServiceChoice};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;

use crate::discovery::{DeviceTable, DiscoveredDevice};
use crate::segmentation::{max_segment_payload, split_payload, SegmentReceiver, SegmentedPduType};
use crate::tsm::{Tsm, TsmConfig, TsmResponse};

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
    /// Maximum segments this client accepts (None = unspecified).
    pub max_segments: Option<u8>,
    /// Whether this client accepts segmented responses.
    pub segmented_response_accepted: bool,
    /// Proposed window size for segmented transfers (1-127, default 1).
    pub proposed_window_size: u8,
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

/// Generic builder for BACnetClient with a pre-built transport.
pub struct ClientBuilder<T: TransportPort> {
    config: ClientConfig,
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

    /// Build and start the client.
    pub async fn build(self) -> Result<BACnetClient<T>, Error> {
        let transport = self
            .transport
            .ok_or_else(|| Error::Encoding("transport not set on ClientBuilder".into()))?;
        BACnetClient::start(self.config, transport).await
    }
}

/// BIP-specific builder that constructs `BipTransport` from interface/port/broadcast fields.
pub struct BipClientBuilder {
    config: ClientConfig,
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

    /// Build and start the client, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetClient<BipTransport>, Error> {
        let transport = BipTransport::new(
            self.config.interface,
            self.config.port,
            self.config.broadcast_address,
        );
        BACnetClient::start(self.config, transport).await
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

/// In-progress segmented receive state.
struct SegmentedReceiveState {
    receiver: SegmentReceiver,
    /// Next expected sequence number (for gap detection).
    expected_next_seq: u8,
    /// Timestamp of last received segment (for reaping stale sessions).
    last_activity: Instant,
}

/// Timeout for idle segmented reassembly sessions.
const SEG_RECEIVER_TIMEOUT: Duration = Duration::from_secs(4);

/// Key for tracking in-progress segmented receives: (source_mac, invoke_id).
type SegKey = (MacAddr, u8);

/// BACnet client with low-level and high-level request APIs.
pub struct BACnetClient<T: TransportPort> {
    config: ClientConfig,
    network: Arc<NetworkLayer<T>>,
    tsm: Arc<Mutex<Tsm>>,
    device_table: Arc<Mutex<DeviceTable>>,
    cov_tx: broadcast::Sender<COVNotificationRequest>,
    dispatch_task: Option<JoinHandle<()>>,
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
    local_mac: MacAddr,
}

impl BACnetClient<BipTransport> {
    /// Create a BIP-specific builder with interface/port/broadcast fields.
    pub fn bip_builder() -> BipClientBuilder {
        BipClientBuilder {
            config: ClientConfig::default(),
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

impl BACnetClient<Bip6Transport> {
    /// Create a BIP6-specific builder for BACnet/IPv6 transport.
    pub fn bip6_builder() -> Bip6ClientBuilder {
        Bip6ClientBuilder {
            config: ClientConfig::default(),
            interface: Ipv6Addr::UNSPECIFIED,
            device_instance: None,
        }
    }
}

/// BIP6-specific builder that constructs `Bip6Transport` from IPv6 interface/port/device-instance.
pub struct Bip6ClientBuilder {
    config: ClientConfig,
    interface: Ipv6Addr,
    device_instance: Option<u32>,
}

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

    /// Build and start the client, constructing a Bip6Transport from the config.
    pub async fn build(self) -> Result<BACnetClient<Bip6Transport>, Error> {
        let transport = Bip6Transport::new(self.interface, self.config.port, self.device_instance);
        BACnetClient::start(self.config, transport).await
    }
}

#[cfg(feature = "sc-tls")]
impl BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>> {
    /// Create an SC-specific builder that connects to a BACnet/SC hub.
    pub fn sc_builder() -> ScClientBuilder {
        ScClientBuilder {
            config: ClientConfig::default(),
            hub_url: String::new(),
            tls_config: None,
            vmac: [0; 6],
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
    hub_url: String,
    tls_config: Option<std::sync::Arc<tokio_rustls::rustls::ClientConfig>>,
    vmac: bacnet_transport::sc_frame::Vmac,
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

    /// Set the APDU timeout in milliseconds.
    pub fn apdu_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_timeout_ms = ms;
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

    /// Connect to the hub and start the client.
    pub async fn build(
        self,
    ) -> Result<
        BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        let tls_config = self
            .tls_config
            .ok_or_else(|| Error::Encoding("SC client builder: tls_config is required".into()))?;

        let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&self.hub_url, tls_config).await?;

        let mut transport = bacnet_transport::sc::ScTransport::new(ws, self.vmac)
            .with_heartbeat_interval_ms(self.heartbeat_interval_ms)
            .with_heartbeat_timeout_ms(self.heartbeat_timeout_ms);
        if let Some(rc) = self.reconnect {
            #[allow(deprecated)]
            {
                transport = transport.with_reconnect(rc);
            }
        }

        BACnetClient::start(self.config, transport).await
    }
}

/// Routing target for confirmed requests.
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
    /// The MAC used for TSM transaction matching.
    fn tsm_mac(&self) -> &[u8] {
        match self {
            Self::Local { mac } => mac,
            Self::Routed { router_mac, .. } => router_mac,
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Create a generic builder that accepts a pre-built transport.
    pub fn generic_builder() -> ClientBuilder<T> {
        ClientBuilder {
            config: ClientConfig::default(),
            transport: None,
        }
    }

    /// Start the client: bind transport, start network layer, spawn dispatch.
    pub async fn start(mut config: ClientConfig, transport: T) -> Result<Self, Error> {
        let transport_max = transport.max_apdu_length();
        config.max_apdu_length = config.max_apdu_length.min(transport_max);

        let mut network = NetworkLayer::new(transport);
        let mut apdu_rx = network.start().await?;
        let local_mac = MacAddr::from_slice(network.local_mac());

        let network = Arc::new(network);

        let tsm_config = TsmConfig {
            apdu_timeout_ms: config.apdu_timeout_ms,
            apdu_segment_timeout_ms: config.apdu_timeout_ms,
            apdu_retries: config.apdu_retries,
        };
        let tsm = Arc::new(Mutex::new(Tsm::new(tsm_config)));
        let tsm_dispatch = Arc::clone(&tsm);
        let device_table = Arc::new(Mutex::new(DeviceTable::new()));
        let device_table_dispatch = Arc::clone(&device_table);
        let network_dispatch = Arc::clone(&network);
        let (cov_tx, _) = broadcast::channel::<COVNotificationRequest>(64);
        let cov_tx_dispatch = cov_tx.clone();
        let seg_ack_senders: Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let seg_ack_senders_dispatch = Arc::clone(&seg_ack_senders);

        let dispatch_task = tokio::spawn(async move {
            let mut seg_state: HashMap<SegKey, SegmentedReceiveState> = HashMap::new();

            while let Some(received) = apdu_rx.recv().await {
                let now = Instant::now();
                seg_state.retain(|_key, state| {
                    now.duration_since(state.last_activity) < SEG_RECEIVER_TIMEOUT
                });

                match apdu::decode_apdu(received.apdu.clone()) {
                    Ok(decoded) => {
                        Self::dispatch_apdu(
                            &tsm_dispatch,
                            &device_table_dispatch,
                            &network_dispatch,
                            &cov_tx_dispatch,
                            &mut seg_state,
                            &seg_ack_senders_dispatch,
                            &received.source_mac,
                            &received.source_network,
                            decoded,
                        )
                        .await;
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to decode received APDU");
                    }
                }
            }
        });

        Ok(Self {
            config,
            network,
            tsm,
            device_table,
            cov_tx,
            dispatch_task: Some(dispatch_task),
            seg_ack_senders,
            local_mac,
        })
    }

    /// Dispatch a received APDU to the appropriate handler.
    #[allow(clippy::too_many_arguments)]
    async fn dispatch_apdu(
        tsm: &Arc<Mutex<Tsm>>,
        device_table: &Arc<Mutex<DeviceTable>>,
        network: &Arc<NetworkLayer<T>>,
        cov_tx: &broadcast::Sender<COVNotificationRequest>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        seg_ack_senders: &Arc<Mutex<HashMap<SegKey, mpsc::Sender<SegmentAckPdu>>>>,
        source_mac: &[u8],
        source_network: &Option<NpduAddress>,
        apdu: Apdu,
    ) {
        match apdu {
            Apdu::SimpleAck(ack) => {
                debug!(invoke_id = ack.invoke_id, "Received SimpleAck");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(source_mac, ack.invoke_id, TsmResponse::SimpleAck);
            }
            Apdu::ComplexAck(ack) => {
                if ack.segmented {
                    Self::handle_segmented_complex_ack(tsm, network, seg_state, source_mac, ack)
                        .await;
                } else {
                    debug!(invoke_id = ack.invoke_id, "Received ComplexAck");
                    let mut tsm = tsm.lock().await;
                    tsm.complete_transaction(
                        source_mac,
                        ack.invoke_id,
                        TsmResponse::ComplexAck {
                            service_data: ack.service_ack,
                        },
                    );
                }
            }
            Apdu::Error(err) => {
                debug!(invoke_id = err.invoke_id, "Received Error PDU");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(
                    source_mac,
                    err.invoke_id,
                    TsmResponse::Error {
                        class: err.error_class.to_raw() as u32,
                        code: err.error_code.to_raw() as u32,
                    },
                );
            }
            Apdu::Reject(rej) => {
                debug!(invoke_id = rej.invoke_id, "Received Reject PDU");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(
                    source_mac,
                    rej.invoke_id,
                    TsmResponse::Reject {
                        reason: rej.reject_reason.to_raw(),
                    },
                );
            }
            Apdu::Abort(abt) => {
                debug!(invoke_id = abt.invoke_id, "Received Abort PDU");
                let mut tsm = tsm.lock().await;
                tsm.complete_transaction(
                    source_mac,
                    abt.invoke_id,
                    TsmResponse::Abort {
                        reason: abt.abort_reason.to_raw(),
                    },
                );
            }
            Apdu::ConfirmedRequest(req) => {
                if req.service_choice == ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION {
                    match COVNotificationRequest::decode(&req.service_request) {
                        Ok(notification) => {
                            debug!(
                                object = ?notification.monitored_object_identifier,
                                "Received ConfirmedCOVNotification"
                            );
                            let _ = cov_tx.send(notification);

                            let ack = Apdu::SimpleAck(SimpleAck {
                                invoke_id: req.invoke_id,
                                service_choice: req.service_choice,
                            });
                            let mut buf = BytesMut::with_capacity(4);
                            encode_apdu(&mut buf, &ack);
                            if let Err(e) = network
                                .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                                .await
                            {
                                warn!(error = %e, "Failed to send SimpleAck for COV notification");
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode ConfirmedCOVNotification");
                        }
                    }
                } else {
                    debug!(
                        service = req.service_choice.to_raw(),
                        "Ignoring ConfirmedRequest (client mode)"
                    );
                }
            }
            Apdu::UnconfirmedRequest(req) => {
                if req.service_choice == UnconfirmedServiceChoice::I_AM {
                    match bacnet_services::who_is::IAmRequest::decode(&req.service_request) {
                        Ok(i_am) => {
                            debug!(
                                device = i_am.object_identifier.instance_number(),
                                vendor = i_am.vendor_id,
                                "Received IAm"
                            );
                            let (src_net, src_addr) = match source_network {
                                Some(npdu_addr) if !npdu_addr.mac_address.is_empty() => {
                                    (Some(npdu_addr.network), Some(npdu_addr.mac_address.clone()))
                                }
                                _ => (None, None),
                            };
                            let device = DiscoveredDevice {
                                object_identifier: i_am.object_identifier,
                                mac_address: MacAddr::from_slice(source_mac),
                                max_apdu_length: i_am.max_apdu_length,
                                segmentation_supported: i_am.segmentation_supported,
                                max_segments_accepted: None,
                                vendor_id: i_am.vendor_id,
                                last_seen: std::time::Instant::now(),
                                source_network: src_net,
                                source_address: src_addr,
                            };
                            device_table.lock().await.upsert(device);
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode IAm");
                        }
                    }
                } else if req.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
                {
                    match COVNotificationRequest::decode(&req.service_request) {
                        Ok(notification) => {
                            debug!(
                                object = ?notification.monitored_object_identifier,
                                "Received UnconfirmedCOVNotification"
                            );
                            let _ = cov_tx.send(notification);
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to decode UnconfirmedCOVNotification");
                        }
                    }
                } else {
                    debug!(
                        service = req.service_choice.to_raw(),
                        "Ignoring unconfirmed service in client dispatch"
                    );
                }
            }
            Apdu::SegmentAck(sa) => {
                let key = (MacAddr::from_slice(source_mac), sa.invoke_id);
                let senders = seg_ack_senders.lock().await;
                if let Some(tx) = senders.get(&key) {
                    let _ = tx.try_send(sa);
                } else {
                    debug!(
                        invoke_id = sa.invoke_id,
                        "Ignoring SegmentAck for unknown transaction"
                    );
                }
            }
        }
    }

    /// Handle a segmented ComplexAck: accumulate segments, send SegmentAcks,
    /// and reassemble when all segments are received.
    async fn handle_segmented_complex_ack(
        tsm: &Arc<Mutex<Tsm>>,
        network: &Arc<NetworkLayer<T>>,
        seg_state: &mut HashMap<SegKey, SegmentedReceiveState>,
        source_mac: &[u8],
        ack: bacnet_encoding::apdu::ComplexAck,
    ) {
        let seq = ack.sequence_number.unwrap_or(0);
        let key = (MacAddr::from_slice(source_mac), ack.invoke_id);

        debug!(
            invoke_id = ack.invoke_id,
            seq = seq,
            more = ack.more_follows,
            "Received segmented ComplexAck"
        );

        const MAX_CONCURRENT_SEG_SESSIONS: usize = 64;
        if !seg_state.contains_key(&key) && seg_state.len() >= MAX_CONCURRENT_SEG_SESSIONS {
            warn!(
                invoke_id = ack.invoke_id,
                sessions = seg_state.len(),
                "Max concurrent segmented sessions reached, dropping segment"
            );
            return;
        }

        let state = seg_state
            .entry(key.clone())
            .or_insert_with(|| SegmentedReceiveState {
                receiver: SegmentReceiver::new(),
                expected_next_seq: 0,
                last_activity: Instant::now(),
            });

        state.last_activity = Instant::now();

        if seq != state.expected_next_seq {
            warn!(
                invoke_id = ack.invoke_id,
                expected = state.expected_next_seq,
                received = seq,
                "Segment gap detected, sending negative SegmentAck"
            );
            let neg_ack = Apdu::SegmentAck(SegmentAckPdu {
                negative_ack: true,
                sent_by_server: false,
                invoke_id: ack.invoke_id,
                sequence_number: state.expected_next_seq,
                actual_window_size: ack.proposed_window_size.unwrap_or(1),
            });
            let mut buf = BytesMut::with_capacity(4);
            encode_apdu(&mut buf, &neg_ack);
            if let Err(e) = network
                .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
                .await
            {
                warn!(error = %e, "Failed to send negative SegmentAck");
            }
            return;
        }

        if let Err(e) = state.receiver.receive(seq, ack.service_ack) {
            warn!(error = %e, "Rejecting oversized segment");
            return;
        }
        state.expected_next_seq = seq.wrapping_add(1);

        let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
            negative_ack: false,
            sent_by_server: false,
            invoke_id: ack.invoke_id,
            sequence_number: seq,
            actual_window_size: ack.proposed_window_size.unwrap_or(1),
        });
        let mut buf = BytesMut::with_capacity(4);
        encode_apdu(&mut buf, &seg_ack);
        if let Err(e) = network
            .send_apdu(&buf, source_mac, false, NetworkPriority::NORMAL)
            .await
        {
            warn!(error = %e, "Failed to send SegmentAck");
        }

        if !ack.more_follows {
            let state = seg_state.remove(&key).unwrap();
            let total = state.receiver.received_count();
            match state.receiver.reassemble(total) {
                Ok(service_data) => {
                    debug!(
                        invoke_id = ack.invoke_id,
                        segments = total,
                        bytes = service_data.len(),
                        "Reassembled segmented ComplexAck"
                    );
                    let mut tsm = tsm.lock().await;
                    tsm.complete_transaction(
                        source_mac,
                        ack.invoke_id,
                        TsmResponse::ComplexAck {
                            service_data: Bytes::from(service_data),
                        },
                    );
                }
                Err(e) => {
                    warn!(error = %e, "Failed to reassemble segmented ComplexAck");
                }
            }
        }
    }

    /// Get the client's local MAC address.
    pub fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    /// Send a confirmed request and wait for the response.
    ///
    /// Returns the service response data (empty for SimpleAck). Automatically
    /// uses segmented transfer when the payload exceeds the remote device's
    /// max APDU length.
    pub async fn confirmed_request(
        &self,
        destination_mac: &[u8],
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        self.confirmed_request_inner(
            ConfirmedTarget::Local {
                mac: destination_mac,
            },
            service_choice,
            service_data,
        )
        .await
    }

    /// Send a confirmed request routed through a BACnet router.
    ///
    /// The NPDU is sent as a unicast to `router_mac` with DNET/DADR set so
    /// the router forwards it to `dest_network`/`dest_mac`.
    pub async fn confirmed_request_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        self.confirmed_request_inner(
            ConfirmedTarget::Routed {
                router_mac,
                dest_network,
                dest_mac,
            },
            service_choice,
            service_data,
        )
        .await
    }

    async fn confirmed_request_inner(
        &self,
        target: ConfirmedTarget<'_>,
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<Bytes, Error> {
        let tsm_mac = target.tsm_mac();

        if let ConfirmedTarget::Local { mac } = &target {
            let unsegmented_apdu_size = 4 + service_data.len();
            let (remote_max_apdu, remote_max_segments) = {
                let dt = self.device_table.lock().await;
                let device = dt.get_by_mac(mac);
                let max_apdu = device
                    .map(|d| d.max_apdu_length as u16)
                    .unwrap_or(self.config.max_apdu_length);
                let max_seg = device.and_then(|d| d.max_segments_accepted);
                (max_apdu, max_seg)
            };
            if unsegmented_apdu_size > remote_max_apdu as usize {
                return self
                    .segmented_confirmed_request(
                        mac,
                        service_choice,
                        service_data,
                        remote_max_apdu,
                        remote_max_segments,
                    )
                    .await;
            }
        }

        let (invoke_id, rx) = {
            let mut tsm = self.tsm.lock().await;
            let invoke_id = tsm.allocate_invoke_id(tsm_mac).ok_or_else(|| {
                Error::Encoding("all invoke IDs exhausted for destination".into())
            })?;
            let rx = tsm.register_transaction(MacAddr::from_slice(tsm_mac), invoke_id);
            (invoke_id, rx)
        };

        // Guard cleans up invoke ID if this task is cancelled/aborted
        let mut guard = crate::tsm::TsmGuard::new(
            std::sync::Arc::clone(&self.tsm),
            MacAddr::from_slice(tsm_mac),
            invoke_id,
        );

        let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: self.config.segmented_response_accepted,
            max_segments: self.config.max_segments,
            max_apdu_length: self.config.max_apdu_length,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(6 + service_data.len());
        encode_apdu(&mut buf, &pdu);

        let timeout_duration = Duration::from_millis(self.config.apdu_timeout_ms);
        let max_retries = self.config.apdu_retries;
        let mut attempts: u8 = 0;
        let mut rx = rx;

        loop {
            let send_result = match &target {
                ConfirmedTarget::Local { mac } => {
                    self.network
                        .send_apdu(&buf, mac, true, NetworkPriority::NORMAL)
                        .await
                }
                ConfirmedTarget::Routed {
                    router_mac,
                    dest_network,
                    dest_mac,
                } => {
                    self.network
                        .send_apdu_routed(
                            &buf,
                            *dest_network,
                            dest_mac,
                            router_mac,
                            true,
                            NetworkPriority::NORMAL,
                        )
                        .await
                }
            };
            if let Err(e) = send_result {
                guard.mark_completed();
                let mut tsm = self.tsm.lock().await;
                tsm.cancel_transaction(tsm_mac, invoke_id);
                return Err(e);
            }

            match timeout(timeout_duration, &mut rx).await {
                Ok(Ok(response)) => {
                    guard.mark_completed();
                    return match response {
                        TsmResponse::SimpleAck => Ok(Bytes::new()),
                        TsmResponse::ComplexAck { service_data } => Ok(service_data),
                        TsmResponse::Error { class, code } => Err(Error::Protocol { class, code }),
                        TsmResponse::Reject { reason } => Err(Error::Reject { reason }),
                        TsmResponse::Abort { reason } => Err(Error::Abort { reason }),
                    };
                }
                Ok(Err(_)) => {
                    guard.mark_completed();
                    return Err(Error::Encoding("TSM response channel closed".into()));
                }
                Err(_timeout) => {
                    attempts += 1;
                    if attempts > max_retries {
                        guard.mark_completed();
                        let mut tsm = self.tsm.lock().await;
                        tsm.cancel_transaction(tsm_mac, invoke_id);
                        return Err(Error::Timeout(timeout_duration));
                    }
                    debug!(
                        invoke_id,
                        attempt = attempts,
                        max_retries,
                        "APDU timeout, retrying confirmed request"
                    );
                }
            }
        }
    }

    /// Send a confirmed request using segmented transfer with windowed flow control.
    async fn segmented_confirmed_request(
        &self,
        destination_mac: &[u8],
        service_choice: ConfirmedServiceChoice,
        service_data: &[u8],
        remote_max_apdu: u16,
        remote_max_segments: Option<u32>,
    ) -> Result<Bytes, Error> {
        let max_seg_size = max_segment_payload(remote_max_apdu, SegmentedPduType::ConfirmedRequest);
        let segments = split_payload(service_data, max_seg_size);
        let total_segments = segments.len();

        if total_segments > 256 {
            return Err(Error::Segmentation(format!(
                "payload requires {} segments, maximum is 256",
                total_segments
            )));
        }

        if let Some(max_seg) = remote_max_segments {
            if total_segments > max_seg as usize {
                return Err(Error::Segmentation(format!(
                    "request requires {} segments but remote accepts at most {}",
                    total_segments, max_seg
                )));
            }
        }

        debug!(
            total_segments,
            max_seg_size,
            service_data_len = service_data.len(),
            "Starting segmented confirmed request"
        );

        let (invoke_id, rx) = {
            let mut tsm = self.tsm.lock().await;
            let invoke_id = tsm.allocate_invoke_id(destination_mac).ok_or_else(|| {
                Error::Encoding("all invoke IDs exhausted for destination".into())
            })?;
            let rx = tsm.register_transaction(MacAddr::from_slice(destination_mac), invoke_id);
            (invoke_id, rx)
        };

        let (seg_ack_tx, mut seg_ack_rx) = mpsc::channel(16);
        {
            let key = (MacAddr::from_slice(destination_mac), invoke_id);
            self.seg_ack_senders.lock().await.insert(key, seg_ack_tx);
        }

        let timeout_duration = Duration::from_millis(self.config.apdu_timeout_ms);
        let max_ack_retries = self.config.apdu_retries;
        let mut window_size = self.config.proposed_window_size.max(1) as usize;
        let mut next_seq: usize = 0;
        let mut neg_ack_retries: u32 = 0;
        const MAX_NEG_ACK_RETRIES: u32 = 10;

        let result = async {
            while next_seq < total_segments {
                let window_end = (next_seq + window_size).min(total_segments);

                for (seq, segment_data) in segments[next_seq..window_end]
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (next_seq + i, s))
                {
                    let is_last = seq == total_segments - 1;
                    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                        segmented: true,
                        more_follows: !is_last,
                        segmented_response_accepted: self.config.segmented_response_accepted,
                        max_segments: self.config.max_segments,
                        max_apdu_length: self.config.max_apdu_length,
                        invoke_id,
                        sequence_number: Some(seq as u8),
                        proposed_window_size: Some(self.config.proposed_window_size.max(1)),
                        service_choice,
                        service_request: segment_data.clone(),
                    });

                    let mut buf = BytesMut::with_capacity(remote_max_apdu as usize);
                    encode_apdu(&mut buf, &pdu);

                    self.network
                        .send_apdu(&buf, destination_mac, true, NetworkPriority::NORMAL)
                        .await?;

                    debug!(seq, is_last, "Sent segment");
                }

                let ack = {
                    let mut ack_retries: u8 = 0;
                    loop {
                        match timeout(timeout_duration, seg_ack_rx.recv()).await {
                            Ok(Some(ack)) => break ack,
                            Ok(None) => {
                                return Err(Error::Encoding("SegmentAck channel closed".into()));
                            }
                            Err(_timeout) => {
                                ack_retries += 1;
                                if ack_retries > max_ack_retries {
                                    return Err(Error::Timeout(timeout_duration));
                                }
                                warn!(
                                    attempt = ack_retries,
                                    "Retransmitting segmented request window"
                                );
                                for (seq, segment_data) in segments[next_seq..window_end]
                                    .iter()
                                    .enumerate()
                                    .map(|(i, s)| (next_seq + i, s))
                                {
                                    let is_last = seq == total_segments - 1;
                                    let pdu = Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                                        segmented: true,
                                        more_follows: !is_last,
                                        segmented_response_accepted: self
                                            .config
                                            .segmented_response_accepted,
                                        max_segments: self.config.max_segments,
                                        max_apdu_length: self.config.max_apdu_length,
                                        invoke_id,
                                        sequence_number: Some(seq as u8),
                                        proposed_window_size: Some(
                                            self.config.proposed_window_size.max(1),
                                        ),
                                        service_choice,
                                        service_request: segment_data.clone(),
                                    });

                                    let mut buf = BytesMut::with_capacity(remote_max_apdu as usize);
                                    encode_apdu(&mut buf, &pdu);

                                    self.network
                                        .send_apdu(
                                            &buf,
                                            destination_mac,
                                            true,
                                            NetworkPriority::NORMAL,
                                        )
                                        .await?;
                                }
                            }
                        }
                    }
                };

                debug!(
                    seq = ack.sequence_number,
                    negative = ack.negative_ack,
                    window = ack.actual_window_size,
                    "Received SegmentAck"
                );

                window_size = ack.actual_window_size.max(1) as usize;

                let ack_seq = ack.sequence_number as usize;
                if ack_seq >= total_segments {
                    return Err(Error::Segmentation(format!(
                        "SegmentAck sequence {} out of range (total {})",
                        ack_seq, total_segments
                    )));
                }

                if ack.negative_ack {
                    neg_ack_retries += 1;
                    if neg_ack_retries > MAX_NEG_ACK_RETRIES {
                        return Err(Error::Segmentation(
                            "too many negative SegmentAck retransmissions".into(),
                        ));
                    }
                    next_seq = ack_seq;
                } else {
                    neg_ack_retries = 0;
                    next_seq = ack_seq + 1;
                }
            }

            timeout(timeout_duration, rx)
                .await
                .map_err(|_| Error::Timeout(timeout_duration))?
                .map_err(|_| Error::Encoding("TSM response channel closed".into()))
        }
        .await;

        {
            let key = (MacAddr::from_slice(destination_mac), invoke_id);
            self.seg_ack_senders.lock().await.remove(&key);
        }

        let response = match result {
            Ok(response) => response,
            Err(e) => {
                let mut tsm = self.tsm.lock().await;
                tsm.cancel_transaction(destination_mac, invoke_id);
                return Err(e);
            }
        };

        match response {
            TsmResponse::SimpleAck => Ok(Bytes::new()),
            TsmResponse::ComplexAck { service_data } => Ok(service_data),
            TsmResponse::Error { class, code } => Err(Error::Protocol { class, code }),
            TsmResponse::Reject { reason } => Err(Error::Reject { reason }),
            TsmResponse::Abort { reason } => Err(Error::Abort { reason }),
        }
    }

    /// Send an unconfirmed request (fire-and-forget) to a specific destination.
    pub async fn unconfirmed_request(
        &self,
        destination_mac: &[u8],
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu);

        self.network
            .send_apdu(&buf, destination_mac, false, NetworkPriority::NORMAL)
            .await
    }

    /// Broadcast an unconfirmed request on the local network.
    pub async fn broadcast_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu);

        self.network
            .broadcast_apdu(&buf, false, NetworkPriority::NORMAL)
            .await
    }

    /// Broadcast an unconfirmed request globally (DNET=0xFFFF).
    pub async fn broadcast_global_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu);

        self.network
            .broadcast_global_apdu(&buf, false, NetworkPriority::NORMAL)
            .await
    }

    /// Broadcast an unconfirmed request to a specific remote network.
    pub async fn broadcast_network_unconfirmed(
        &self,
        service_choice: UnconfirmedServiceChoice,
        service_data: &[u8],
        dest_network: u16,
    ) -> Result<(), Error> {
        let pdu = Apdu::UnconfirmedRequest(bacnet_encoding::apdu::UnconfirmedRequest {
            service_choice,
            service_request: Bytes::copy_from_slice(service_data),
        });

        let mut buf = BytesMut::with_capacity(2 + service_data.len());
        encode_apdu(&mut buf, &pdu);

        self.network
            .broadcast_to_network(&buf, dest_network, false, NetworkPriority::NORMAL)
            .await
    }

    /// Read a property from a remote device.
    pub async fn read_property(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        use bacnet_services::read_property::ReadPropertyRequest;

        let request = ReadPropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let response_data = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::READ_PROPERTY, &buf)
            .await?;

        bacnet_services::read_property::ReadPropertyACK::decode(&response_data)
    }

    /// Read a property from a discovered device, auto-routing if needed.
    pub async fn read_property_from_device(
        &self,
        device_instance: u32,
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        let (mac, routing) = {
            let dt = self.device_table.lock().await;
            let device = dt.get(device_instance).ok_or_else(|| {
                Error::Encoding(format!("device {device_instance} not in device table"))
            })?;
            let routing = match (&device.source_network, &device.source_address) {
                (Some(snet), Some(sadr)) => Some((*snet, sadr.to_vec())),
                _ => None,
            };
            (device.mac_address.to_vec(), routing)
        };

        if let Some((dnet, dadr)) = routing {
            self.read_property_routed(
                &mac,
                dnet,
                &dadr,
                object_identifier,
                property_identifier,
                property_array_index,
            )
            .await
        } else {
            self.read_property(
                &mac,
                object_identifier,
                property_identifier,
                property_array_index,
            )
            .await
        }
    }

    /// Read a property from a device on a remote BACnet network via a router.
    pub async fn read_property_routed(
        &self,
        router_mac: &[u8],
        dest_network: u16,
        dest_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
    ) -> Result<bacnet_services::read_property::ReadPropertyACK, Error> {
        use bacnet_services::read_property::ReadPropertyRequest;

        let request = ReadPropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let response_data = self
            .confirmed_request_routed(
                router_mac,
                dest_network,
                dest_mac,
                ConfirmedServiceChoice::READ_PROPERTY,
                &buf,
            )
            .await?;

        bacnet_services::read_property::ReadPropertyACK::decode(&response_data)
    }

    /// Write a property on a remote device.
    pub async fn write_property(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        property_value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        use bacnet_services::write_property::WritePropertyRequest;

        let request = WritePropertyRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            property_value,
            priority,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Read multiple properties from one or more objects on a remote device.
    pub async fn read_property_multiple(
        &self,
        destination_mac: &[u8],
        specs: Vec<bacnet_services::rpm::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};

        let request = ReadPropertyMultipleRequest {
            list_of_read_access_specs: specs,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let response_data = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                &buf,
            )
            .await?;

        ReadPropertyMultipleACK::decode(&response_data)
    }

    /// Write multiple properties on one or more objects on a remote device.
    pub async fn write_property_multiple(
        &self,
        destination_mac: &[u8],
        specs: Vec<bacnet_services::wpm::WriteAccessSpecification>,
    ) -> Result<(), Error> {
        use bacnet_services::wpm::WritePropertyMultipleRequest;

        let request = WritePropertyMultipleRequest {
            list_of_write_access_specs: specs,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                &buf,
            )
            .await?;

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Auto-routing _from_device variants (RPM, WP, WPM)
    // -----------------------------------------------------------------------

    /// Read multiple properties from a discovered device, auto-routing if needed.
    pub async fn read_property_multiple_from_device(
        &self,
        device_instance: u32,
        specs: Vec<bacnet_services::rpm::ReadAccessSpecification>,
    ) -> Result<bacnet_services::rpm::ReadPropertyMultipleACK, Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;

        if let Some((dnet, dadr)) = routing {
            use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};

            let request = ReadPropertyMultipleRequest {
                list_of_read_access_specs: specs,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf);

            let response_data = self
                .confirmed_request_routed(
                    &mac,
                    dnet,
                    &dadr,
                    ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                    &buf,
                )
                .await?;

            ReadPropertyMultipleACK::decode(&response_data)
        } else {
            self.read_property_multiple(&mac, specs).await
        }
    }

    /// Write a property on a discovered device, auto-routing if needed.
    pub async fn write_property_to_device(
        &self,
        device_instance: u32,
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        property_value: Vec<u8>,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;

        if let Some((dnet, dadr)) = routing {
            use bacnet_services::write_property::WritePropertyRequest;

            let request = WritePropertyRequest {
                object_identifier,
                property_identifier,
                property_array_index,
                property_value,
                priority,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf);

            let _ = self
                .confirmed_request_routed(
                    &mac,
                    dnet,
                    &dadr,
                    ConfirmedServiceChoice::WRITE_PROPERTY,
                    &buf,
                )
                .await?;
            Ok(())
        } else {
            self.write_property(
                &mac,
                object_identifier,
                property_identifier,
                property_array_index,
                property_value,
                priority,
            )
            .await
        }
    }

    /// Write multiple properties on a discovered device, auto-routing if needed.
    pub async fn write_property_multiple_to_device(
        &self,
        device_instance: u32,
        specs: Vec<bacnet_services::wpm::WriteAccessSpecification>,
    ) -> Result<(), Error> {
        let (mac, routing) = self.resolve_device(device_instance).await?;

        if let Some((dnet, dadr)) = routing {
            use bacnet_services::wpm::WritePropertyMultipleRequest;

            let request = WritePropertyMultipleRequest {
                list_of_write_access_specs: specs,
            };
            let mut buf = BytesMut::new();
            request.encode(&mut buf);

            let _ = self
                .confirmed_request_routed(
                    &mac,
                    dnet,
                    &dadr,
                    ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
                    &buf,
                )
                .await?;
            Ok(())
        } else {
            self.write_property_multiple(&mac, specs).await
        }
    }

    /// Resolve a device instance to its MAC address and optional routing info.
    async fn resolve_device(
        &self,
        device_instance: u32,
    ) -> Result<(Vec<u8>, Option<(u16, Vec<u8>)>), Error> {
        let dt = self.device_table.lock().await;
        let device = dt.get(device_instance).ok_or_else(|| {
            Error::Encoding(format!("device {device_instance} not in device table"))
        })?;
        let routing = match (&device.source_network, &device.source_address) {
            (Some(snet), Some(sadr)) => Some((*snet, sadr.to_vec())),
            _ => None,
        };
        Ok((device.mac_address.to_vec(), routing))
    }

    // -----------------------------------------------------------------------
    // Multi-device batch operations
    // -----------------------------------------------------------------------

    /// Read a property from multiple discovered devices concurrently.
    ///
    /// All requests are dispatched concurrently (up to `max_concurrent`,
    /// default 32) and results are returned in completion order. Each device
    /// is resolved from the device table and auto-routed if behind a router.
    pub async fn read_property_from_devices(
        &self,
        requests: Vec<DeviceReadRequest>,
        max_concurrent: Option<usize>,
    ) -> Vec<DeviceReadResult> {
        use futures_util::stream::{self, StreamExt};

        let concurrency = max_concurrent.unwrap_or(DEFAULT_BATCH_CONCURRENCY);

        stream::iter(requests)
            .map(|req| async move {
                let result = self
                    .read_property_from_device(
                        req.device_instance,
                        req.object_identifier,
                        req.property_identifier,
                        req.property_array_index,
                    )
                    .await;
                DeviceReadResult {
                    device_instance: req.device_instance,
                    result,
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }

    /// Read multiple properties from multiple devices concurrently (RPM batch).
    ///
    /// Sends an RPM to each device concurrently. This is the most efficient
    /// way to poll many properties across many devices — RPM batches within
    /// a single device, and this method batches across devices.
    pub async fn read_property_multiple_from_devices(
        &self,
        requests: Vec<DeviceRpmRequest>,
        max_concurrent: Option<usize>,
    ) -> Vec<DeviceRpmResult> {
        use futures_util::stream::{self, StreamExt};

        let concurrency = max_concurrent.unwrap_or(DEFAULT_BATCH_CONCURRENCY);

        stream::iter(requests)
            .map(|req| async move {
                let result = self
                    .read_property_multiple_from_device(req.device_instance, req.specs)
                    .await;
                DeviceRpmResult {
                    device_instance: req.device_instance,
                    result,
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }

    /// Write a property on multiple devices concurrently.
    ///
    /// All writes are dispatched concurrently (up to `max_concurrent`,
    /// default 32). Results are returned in completion order.
    pub async fn write_property_to_devices(
        &self,
        requests: Vec<DeviceWriteRequest>,
        max_concurrent: Option<usize>,
    ) -> Vec<DeviceWriteResult> {
        use futures_util::stream::{self, StreamExt};

        let concurrency = max_concurrent.unwrap_or(DEFAULT_BATCH_CONCURRENCY);

        stream::iter(requests)
            .map(|req| async move {
                let result = self
                    .write_property_to_device(
                        req.device_instance,
                        req.object_identifier,
                        req.property_identifier,
                        req.property_array_index,
                        req.property_value,
                        req.priority,
                    )
                    .await;
                DeviceWriteResult {
                    device_instance: req.device_instance,
                    result,
                }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await
    }

    /// Send a WhoIs broadcast to discover devices.
    pub async fn who_is(
        &self,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_is::WhoIsRequest;

        let request = WhoIsRequest {
            low_limit,
            high_limit,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.broadcast_global_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf)
            .await
    }

    /// Send a directed (unicast) WhoIs to a specific device.
    pub async fn who_is_directed(
        &self,
        destination_mac: &[u8],
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_is::WhoIsRequest;

        let request = WhoIsRequest {
            low_limit,
            high_limit,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.unconfirmed_request(destination_mac, UnconfirmedServiceChoice::WHO_IS, &buf)
            .await
    }

    /// Send a WhoIs broadcast to a specific remote network.
    pub async fn who_is_network(
        &self,
        dest_network: u16,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_is::WhoIsRequest;

        let request = WhoIsRequest {
            low_limit,
            high_limit,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.broadcast_network_unconfirmed(UnconfirmedServiceChoice::WHO_IS, &buf, dest_network)
            .await
    }

    /// Send a WhoHas broadcast to find an object by identifier or name.
    pub async fn who_has(
        &self,
        object: bacnet_services::who_has::WhoHasObject,
        low_limit: Option<u32>,
        high_limit: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::who_has::WhoHasRequest;

        let request = WhoHasRequest {
            low_limit,
            high_limit,
            object,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        self.broadcast_unconfirmed(UnconfirmedServiceChoice::WHO_HAS, &buf)
            .await
    }

    /// Subscribe to COV notifications for an object on a remote device.
    pub async fn subscribe_cov(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: bacnet_types::primitives::ObjectIdentifier,
        confirmed: bool,
        lifetime: Option<u32>,
    ) -> Result<(), Error> {
        use bacnet_services::cov::SubscribeCOVRequest;

        let request = SubscribeCOVRequest {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications: Some(confirmed),
            lifetime,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::SUBSCRIBE_COV, &buf)
            .await?;

        Ok(())
    }

    /// Cancel a COV subscription on a remote device.
    pub async fn unsubscribe_cov(
        &self,
        destination_mac: &[u8],
        subscriber_process_identifier: u32,
        monitored_object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        use bacnet_services::cov::SubscribeCOVRequest;

        let request = SubscribeCOVRequest {
            subscriber_process_identifier,
            monitored_object_identifier,
            issue_confirmed_notifications: None,
            lifetime: None,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::SUBSCRIBE_COV, &buf)
            .await?;

        Ok(())
    }

    /// Delete an object on a remote device.
    pub async fn delete_object(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
    ) -> Result<(), Error> {
        use bacnet_services::object_mgmt::DeleteObjectRequest;

        let request = DeleteObjectRequest { object_identifier };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::DELETE_OBJECT, &buf)
            .await?;

        Ok(())
    }

    /// Create an object on a remote device.
    pub async fn create_object(
        &self,
        destination_mac: &[u8],
        object_specifier: bacnet_services::object_mgmt::ObjectSpecifier,
        initial_values: Vec<bacnet_services::common::BACnetPropertyValue>,
    ) -> Result<Bytes, Error> {
        use bacnet_services::object_mgmt::CreateObjectRequest;

        let request = CreateObjectRequest {
            object_specifier,
            list_of_initial_values: initial_values,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(destination_mac, ConfirmedServiceChoice::CREATE_OBJECT, &buf)
            .await
    }

    /// Send DeviceCommunicationControl to a remote device.
    pub async fn device_communication_control(
        &self,
        destination_mac: &[u8],
        enable_disable: bacnet_types::enums::EnableDisable,
        time_duration: Option<u16>,
        password: Option<String>,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;

        let request = DeviceCommunicationControlRequest {
            time_duration,
            enable_disable,
            password,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Send ReinitializeDevice to a remote device.
    pub async fn reinitialize_device(
        &self,
        destination_mac: &[u8],
        reinitialized_state: bacnet_types::enums::ReinitializedState,
        password: Option<String>,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::ReinitializeDeviceRequest;

        let request = ReinitializeDeviceRequest {
            reinitialized_state,
            password,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::REINITIALIZE_DEVICE,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Get event information from a remote device.
    pub async fn get_event_information(
        &self,
        destination_mac: &[u8],
        last_received_object_identifier: Option<bacnet_types::primitives::ObjectIdentifier>,
    ) -> Result<Bytes, Error> {
        use bacnet_services::alarm_event::GetEventInformationRequest;

        let request = GetEventInformationRequest {
            last_received_object_identifier,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(
            destination_mac,
            ConfirmedServiceChoice::GET_EVENT_INFORMATION,
            &buf,
        )
        .await
    }

    /// Acknowledge an alarm on a remote device.
    pub async fn acknowledge_alarm(
        &self,
        destination_mac: &[u8],
        acknowledging_process_identifier: u32,
        event_object_identifier: bacnet_types::primitives::ObjectIdentifier,
        event_state_acknowledged: u32,
        acknowledgment_source: &str,
    ) -> Result<(), Error> {
        use bacnet_services::alarm_event::AcknowledgeAlarmRequest;

        let request = AcknowledgeAlarmRequest {
            acknowledging_process_identifier,
            event_object_identifier,
            event_state_acknowledged,
            timestamp: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0),
            acknowledgment_source: acknowledgment_source.to_string(),
            time_of_acknowledgment: bacnet_types::primitives::BACnetTimeStamp::SequenceNumber(0),
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf)?;

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::ACKNOWLEDGE_ALARM,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Read a range of items from a list or log-buffer property.
    pub async fn read_range(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        range: Option<bacnet_services::read_range::RangeSpec>,
    ) -> Result<bacnet_services::read_range::ReadRangeAck, Error> {
        use bacnet_services::read_range::{ReadRangeAck, ReadRangeRequest};

        let request = ReadRangeRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            range,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let response_data = self
            .confirmed_request(destination_mac, ConfirmedServiceChoice::READ_RANGE, &buf)
            .await?;

        ReadRangeAck::decode(&response_data)
    }

    /// Read file data from a remote device (stream or record access).
    pub async fn atomic_read_file(
        &self,
        destination_mac: &[u8],
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileAccessMethod,
    ) -> Result<Bytes, Error> {
        use bacnet_services::file::AtomicReadFileRequest;

        let request = AtomicReadFileRequest {
            file_identifier,
            access,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(
            destination_mac,
            ConfirmedServiceChoice::ATOMIC_READ_FILE,
            &buf,
        )
        .await
    }

    /// Write file data to a remote device (stream or record access).
    pub async fn atomic_write_file(
        &self,
        destination_mac: &[u8],
        file_identifier: bacnet_types::primitives::ObjectIdentifier,
        access: bacnet_services::file::FileWriteAccessMethod,
    ) -> Result<Bytes, Error> {
        use bacnet_services::file::AtomicWriteFileRequest;

        let request = AtomicWriteFileRequest {
            file_identifier,
            access,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.confirmed_request(
            destination_mac,
            ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
            &buf,
        )
        .await
    }

    /// Add elements to a list property on a remote device.
    pub async fn add_list_element(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), Error> {
        use bacnet_services::list_manipulation::ListElementRequest;

        let request = ListElementRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::ADD_LIST_ELEMENT,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Remove elements from a list property on a remote device.
    pub async fn remove_list_element(
        &self,
        destination_mac: &[u8],
        object_identifier: bacnet_types::primitives::ObjectIdentifier,
        property_identifier: bacnet_types::enums::PropertyIdentifier,
        property_array_index: Option<u32>,
        list_of_elements: Vec<u8>,
    ) -> Result<(), Error> {
        use bacnet_services::list_manipulation::ListElementRequest;

        let request = ListElementRequest {
            object_identifier,
            property_identifier,
            property_array_index,
            list_of_elements,
        };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        let _ = self
            .confirmed_request(
                destination_mac,
                ConfirmedServiceChoice::REMOVE_LIST_ELEMENT,
                &buf,
            )
            .await?;

        Ok(())
    }

    /// Send a TimeSynchronization request (unconfirmed, no response expected).
    pub async fn time_synchronization(
        &self,
        destination_mac: &[u8],
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::TimeSynchronizationRequest;

        let request = TimeSynchronizationRequest { date, time };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.unconfirmed_request(
            destination_mac,
            UnconfirmedServiceChoice::TIME_SYNCHRONIZATION,
            &buf,
        )
        .await
    }

    /// Send a UTCTimeSynchronization request (unconfirmed, no response expected).
    pub async fn utc_time_synchronization(
        &self,
        destination_mac: &[u8],
        date: bacnet_types::primitives::Date,
        time: bacnet_types::primitives::Time,
    ) -> Result<(), Error> {
        use bacnet_services::device_mgmt::TimeSynchronizationRequest;

        let request = TimeSynchronizationRequest { date, time };
        let mut buf = BytesMut::new();
        request.encode(&mut buf);

        self.unconfirmed_request(
            destination_mac,
            UnconfirmedServiceChoice::UTC_TIME_SYNCHRONIZATION,
            &buf,
        )
        .await
    }

    /// Get a receiver for incoming COV notifications. Each call returns a new
    /// independent receiver.
    pub fn cov_notifications(&self) -> broadcast::Receiver<COVNotificationRequest> {
        self.cov_tx.subscribe()
    }

    /// Get a snapshot of all discovered devices.
    pub async fn discovered_devices(&self) -> Vec<DiscoveredDevice> {
        self.device_table.lock().await.all()
    }

    /// Look up a discovered device by instance number.
    pub async fn get_device(&self, instance: u32) -> Option<DiscoveredDevice> {
        self.device_table.lock().await.get(instance).cloned()
    }

    /// Clear the discovered devices table.
    pub async fn clear_devices(&self) {
        self.device_table.lock().await.clear();
    }

    /// Manually register a device in the device table.
    ///
    /// Useful for adding known devices without requiring WhoIs/IAm exchange.
    /// Sets default values for max_apdu_length (1476), segmentation (NONE),
    /// and vendor_id (0) since these are unknown without IAm.
    pub async fn add_device(&self, instance: u32, mac: &[u8]) -> Result<(), Error> {
        let oid = bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::DEVICE,
            instance,
        )?;
        let device = DiscoveredDevice {
            object_identifier: oid,
            mac_address: MacAddr::from_slice(mac),
            max_apdu_length: 1476,
            segmentation_supported: bacnet_types::enums::Segmentation::NONE,
            max_segments_accepted: None,
            vendor_id: 0,
            last_seen: std::time::Instant::now(),
            source_network: None,
            source_address: None,
        };
        self.device_table.lock().await.upsert(device);
        Ok(())
    }

    /// Stop the client, aborting the dispatch task.
    pub async fn stop(&mut self) -> Result<(), Error> {
        if let Some(task) = self.dispatch_task.take() {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_encoding::apdu::{ComplexAck, SimpleAck};
    use std::net::Ipv4Addr;
    use tokio::time::Duration;

    async fn make_client() -> BACnetClient<BipTransport> {
        BACnetClient::builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .apdu_timeout_ms(2000)
            .build()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn client_start_stop() {
        let mut client = make_client().await;
        assert!(!client.local_mac().is_empty());
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn confirmed_request_simple_ack() {
        let mut client_a = make_client().await;

        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut net_b = NetworkLayer::new(transport_b);
        let mut rx_b = net_b.start().await.unwrap();
        let b_mac = net_b.local_mac().to_vec();

        let b_handle = tokio::spawn(async move {
            let received = timeout(Duration::from_secs(2), rx_b.recv())
                .await
                .expect("B timed out")
                .expect("B channel closed");

            let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
            if let Apdu::ConfirmedRequest(req) = decoded {
                let ack = Apdu::SimpleAck(SimpleAck {
                    invoke_id: req.invoke_id,
                    service_choice: req.service_choice,
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &ack);
                net_b
                    .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                    .await
                    .unwrap();
            }
            net_b.stop().await.unwrap();
        });

        let result = client_a
            .confirmed_request(
                &b_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x01, 0x02],
            )
            .await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.is_empty());

        b_handle.await.unwrap();
        client_a.stop().await.unwrap();
    }

    #[tokio::test]
    async fn confirmed_request_complex_ack() {
        let mut client_a = make_client().await;

        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut net_b = NetworkLayer::new(transport_b);
        let mut rx_b = net_b.start().await.unwrap();
        let b_mac = net_b.local_mac().to_vec();

        let b_handle = tokio::spawn(async move {
            let received = timeout(Duration::from_secs(2), rx_b.recv())
                .await
                .unwrap()
                .unwrap();

            let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
            if let Apdu::ConfirmedRequest(req) = decoded {
                let ack = Apdu::ComplexAck(ComplexAck {
                    segmented: false,
                    more_follows: false,
                    invoke_id: req.invoke_id,
                    sequence_number: None,
                    proposed_window_size: None,
                    service_choice: req.service_choice,
                    service_ack: Bytes::from_static(&[0xDE, 0xAD, 0xBE, 0xEF]),
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &ack);
                net_b
                    .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                    .await
                    .unwrap();
            }
            net_b.stop().await.unwrap();
        });

        let result = client_a
            .confirmed_request(&b_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![0xDE, 0xAD, 0xBE, 0xEF]);

        b_handle.await.unwrap();
        client_a.stop().await.unwrap();
    }

    #[tokio::test]
    async fn confirmed_request_timeout() {
        let mut client = make_client().await;
        let fake_mac = vec![10, 99, 99, 99, 0xBA, 0xC0];
        let result = client
            .confirmed_request(&fake_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
            .await;
        assert!(result.is_err());
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn segmented_complex_ack_reassembly() {
        let mut client = make_client().await;

        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut net_b = NetworkLayer::new(transport_b);
        let mut rx_b = net_b.start().await.unwrap();
        let b_mac = net_b.local_mac().to_vec();

        let b_handle = tokio::spawn(async move {
            let received = timeout(Duration::from_secs(2), rx_b.recv())
                .await
                .unwrap()
                .unwrap();

            let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
            let invoke_id = if let Apdu::ConfirmedRequest(req) = decoded {
                req.invoke_id
            } else {
                panic!("Expected ConfirmedRequest");
            };

            let service_choice = ConfirmedServiceChoice::READ_PROPERTY;
            let segments: Vec<Bytes> = vec![
                Bytes::from_static(&[0x01, 0x02, 0x03]),
                Bytes::from_static(&[0x04, 0x05, 0x06]),
                Bytes::from_static(&[0x07, 0x08]),
            ];

            for (i, seg) in segments.iter().enumerate() {
                let is_last = i == segments.len() - 1;
                let ack = Apdu::ComplexAck(ComplexAck {
                    segmented: true,
                    more_follows: !is_last,
                    invoke_id,
                    sequence_number: Some(i as u8),
                    proposed_window_size: Some(1),
                    service_choice,
                    service_ack: seg.clone(),
                });
                let mut buf = BytesMut::new();
                encode_apdu(&mut buf, &ack);
                net_b
                    .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                    .await
                    .unwrap();

                let seg_ack_msg = timeout(Duration::from_secs(2), rx_b.recv())
                    .await
                    .unwrap()
                    .unwrap();
                let decoded = apdu::decode_apdu(seg_ack_msg.apdu.clone()).unwrap();
                if let Apdu::SegmentAck(sa) = decoded {
                    assert_eq!(sa.invoke_id, invoke_id);
                    assert_eq!(sa.sequence_number, i as u8);
                } else {
                    panic!("Expected SegmentAck, got {:?}", decoded);
                }
            }

            net_b.stop().await.unwrap();
        });

        let result = client
            .confirmed_request(&b_mac, ConfirmedServiceChoice::READ_PROPERTY, &[0x01])
            .await;

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );

        b_handle.await.unwrap();
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn segmented_confirmed_request_sends_segments() {
        let mut client = BACnetClient::builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .apdu_timeout_ms(5000)
            .max_apdu_length(50)
            .build()
            .await
            .unwrap();

        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut net_b = NetworkLayer::new(transport_b);
        let mut rx_b = net_b.start().await.unwrap();
        let b_mac = net_b.local_mac().to_vec();

        let service_data: Vec<u8> = (0u8..100).collect();
        let expected_data = service_data.clone();

        let b_handle = tokio::spawn(async move {
            let mut all_service_data = Vec::new();
            let mut client_mac;
            let mut invoke_id;

            loop {
                let received = timeout(Duration::from_secs(3), rx_b.recv())
                    .await
                    .expect("server timed out waiting for segment")
                    .expect("server channel closed");

                let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
                if let Apdu::ConfirmedRequest(req) = decoded {
                    assert!(req.segmented, "expected segmented request");
                    invoke_id = req.invoke_id;
                    client_mac = received.source_mac.clone();
                    let seq = req.sequence_number.unwrap();
                    all_service_data.extend_from_slice(&req.service_request);

                    let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
                        negative_ack: false,
                        sent_by_server: true,
                        invoke_id,
                        sequence_number: seq,
                        actual_window_size: 1,
                    });
                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &seg_ack);
                    net_b
                        .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                        .await
                        .unwrap();

                    if !req.more_follows {
                        break;
                    }
                } else {
                    panic!("Expected ConfirmedRequest, got {:?}", decoded);
                }
            }

            let ack = Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &ack);
            net_b
                .send_apdu(&buf, &client_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();

            net_b.stop().await.unwrap();
            all_service_data
        });

        let result = client
            .confirmed_request(
                &b_mac,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());

        let received_data = b_handle.await.unwrap();
        assert_eq!(received_data, expected_data);

        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn segmented_request_with_complex_ack_response() {
        let mut client = BACnetClient::builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .apdu_timeout_ms(5000)
            .max_apdu_length(50)
            .build()
            .await
            .unwrap();

        let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
        let mut net_b = NetworkLayer::new(transport_b);
        let mut rx_b = net_b.start().await.unwrap();
        let b_mac = net_b.local_mac().to_vec();

        let service_data: Vec<u8> = (0u8..60).collect();

        let b_handle = tokio::spawn(async move {
            let mut client_mac;
            let mut invoke_id;

            loop {
                let received = timeout(Duration::from_secs(3), rx_b.recv())
                    .await
                    .unwrap()
                    .unwrap();

                let decoded = apdu::decode_apdu(received.apdu.clone()).unwrap();
                if let Apdu::ConfirmedRequest(req) = decoded {
                    invoke_id = req.invoke_id;
                    client_mac = received.source_mac.clone();
                    let seq = req.sequence_number.unwrap();

                    let seg_ack = Apdu::SegmentAck(SegmentAckPdu {
                        negative_ack: false,
                        sent_by_server: true,
                        invoke_id,
                        sequence_number: seq,
                        actual_window_size: 1,
                    });
                    let mut buf = BytesMut::new();
                    encode_apdu(&mut buf, &seg_ack);
                    net_b
                        .send_apdu(&buf, &received.source_mac, false, NetworkPriority::NORMAL)
                        .await
                        .unwrap();

                    if !req.more_follows {
                        break;
                    }
                }
            }

            let ack = Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_ack: Bytes::from_static(&[0xCA, 0xFE]),
            });
            let mut buf = BytesMut::new();
            encode_apdu(&mut buf, &ack);
            net_b
                .send_apdu(&buf, &client_mac, false, NetworkPriority::NORMAL)
                .await
                .unwrap();

            net_b.stop().await.unwrap();
        });

        let result = client
            .confirmed_request(&b_mac, ConfirmedServiceChoice::READ_PROPERTY, &service_data)
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vec![0xCA, 0xFE]);

        b_handle.await.unwrap();
        client.stop().await.unwrap();
    }

    #[tokio::test]
    async fn segment_overflow_guard() {
        let mut client = BACnetClient::builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .apdu_timeout_ms(2000)
            .max_apdu_length(50)
            .build()
            .await
            .unwrap();

        let huge_payload = vec![0u8; 257 * 44];
        let fake_mac = vec![10, 99, 99, 99, 0xBA, 0xC0];

        let result = client
            .confirmed_request(
                &fake_mac,
                ConfirmedServiceChoice::READ_PROPERTY,
                &huge_payload,
            )
            .await;

        assert!(
            result.is_err(),
            "expected error for oversized payload, got success"
        );

        client.stop().await.unwrap();
    }

    #[test]
    fn seg_receiver_timeout_is_4s() {
        assert_eq!(SEG_RECEIVER_TIMEOUT, Duration::from_secs(4));
    }
}
