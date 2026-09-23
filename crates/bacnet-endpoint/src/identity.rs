//! Single Device identity source (RB-16, proven narrow scope).
//!
//! # Truth direction
//!
//! [`DeviceIdentity`] is the single source for one device:
//!
//! ```text
//! DeviceIdentity
//!   -> ObjectDatabase (Device + Network-Port objects + DEVICE_UUID)
//!   -> ServerConfig / ClientConfig / transport limits
//!   -> I-Am contents + Device ReadProperty + role behavior
//! ```
//!
//! I-Am (`object_identifier`, `max_apdu`, `segmentation`, `vendor`) +
//! Device `ReadProperty` + client/server role limits ALL derive from the
//! same identity. The session I-Am path ([`EndpointSession::broadcast_i_am`](crate::session::EndpointSession::broadcast_i_am))
//! encodes from the composed identity; the database is built by
//! [`DeviceIdentity::build_database`] so readback agrees; role limits use
//! [`DeviceIdentity::client_max_apdu`] / [`DeviceIdentity::server_max_apdu`].
//!
//! `SessionConfig::max_apdu_length` (480) stays the default for standalone
//! sessions without an identity; [`EndpointSession::with_identity`](crate::session::EndpointSession::with_identity)
//! overrides it when an identity is composed.
//!
//! # Evidence level (honest)
//!
//! I-Am identical to Device `ReadProperty` for the composed identity, with
//! role traffic both directions, is corroborated on **real B/IP loopback UDP
//! and the real constrained-TLS SC hub** (RB-16 proofs). Port/UUID/capability
//! corners beyond that matrix — Network-Port population details, SC UUID
//! sync into `DEVICE_UUID`, service-profile alignment — are **Loopback-only**
//! (deterministic `LoopbackTransport` / `LoopbackWebSocket` coverage), not
//! on-wire claims.
//!
//! # Services profile (no superset flags)
//!
//! Default = the endpoint server-executed set (`READ_PROPERTY` only). That is
//! the narrow composition reality in this crate: the client role initiates
//! `ReadProperty`, the server role executes `ReadProperty` (+ `Reject`/`Abort`
//! + segmentation-`Abort`). It deliberately differs from the full
//! `bacnet-server` dispatch surface (`EXECUTED_SERVICES`): advertising the
//! full set here would be a superset flag the endpoint roles cannot honor.
//! Explicit [`EndpointSession::with_device_writes`](crate::session::EndpointSession::with_device_writes)
//! adds `WRITE_PROPERTY` for authorized local Device.Description writes and
//! sets the Device and identity to that actual two-service profile at startup.
//! This opt-in rejects any other configured identity service bits before start.
//! Deployments needing the full server surface override via
//! [`DeviceIdentity::with_services`], but must then compose the full server —
//! not the narrow endpoint responder — or the I-Am vs ReadProperty vs behavior
//! matrix fails.
//!
//! # Network-Port population rule
//!
//! One port entry per bound transport, with the actual bound IP/port +
//! network number observed at bind:
//!
//! - B/IP: `Network_Type = IPV4 (5)`, `MAC` = 6-byte B/IP MAC
//!   (bound IP octets + bound UDP port big-endian), `IP_Address` = bound IP,
//!   `BACnet_IP_UDP_Port` = bound port, `Network_Number` = caller-configured.
//! - SC: `Network_Type = VIRTUAL (7)`, `MAC` = 6-byte VMAC, IP fields stay
//!   zero (SC has no B/IP socket), `Network_Number` = caller-configured.
//! - Loopback/determinism: `Network_Type = VIRTUAL (7)` with the loopback MAC.
//!
//! Instance numbering is caller-chosen but must be stable per device; the
//! convention used in proofs is B/IP = 1, SC = 2, loopback = 1 when solo.
//! [`DeviceIdentity::with_bip_port`] / [`with_sc_port`](DeviceIdentity::with_sc_port)
//! encode this rule; [`DeviceIdentity::sync_bip_bind`] refreshes the B/IP
//! entry with the actual bound socket address after `start()`.
//!
//! # SC UUID rule (durable-caller-owned)
//!
//! Mirrors `sc_builder` docs: the builder neither generates nor stores the
//! device UUID. The caller provisions 16 bytes durably (Annex AB.1.5.3) and
//! reuses them for the device lifetime; [`DeviceIdentity::with_device_uuid`]
//! only stores the supplied bytes, and [`DeviceIdentity::build_database`]
//! syncs them into the Device-object `DEVICE_UUID` instead of leaving zeros.
//! Startup rejects all-zero only where a real SC identity is required
//! (SC transport start, SC endpoint hub-dial); B/IP-only identities may keep
//! zeros and still build.
//!
//! # Transport-dependent behavior
//!
//! - B/IP real loopback UDP (`127.0.0.1`, ephemeral port): one socket per
//!   endpoint; broadcast reaches the bound socket via `INADDR_ANY`; I-Am via
//!   local broadcast; routed NPDU via unicast to the router MAC.
//! - SC constrained-TLS hub (`ScHub` + `TlsWebSocket`): hub relay preserves
//!   `source_network` + `provenance` (verified-relayed-origin); VMAC+UUID
//!   survive reconnect via caller-owned reconnect config; direct dial is out
//!   of scope for the proof.
//! - In-memory `LoopbackTransport` / `LoopbackWebSocket` appear alongside for
//!   determinism, never INSTEAD of the real-loopback proofs.

use bacnet_encoding::apdu::{encode_apdu, validate_max_apdu_length, Apdu, UnconfirmedRequest};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::network_port::NetworkPortObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::{NetworkType, ObjectType, Segmentation, ServiceSupported};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;
use bytes::BytesMut;
use std::net::Ipv4Addr;

/// One Network-Port entry per bound transport (see module docs for the rule).
///
/// Provenance: population-rule coverage is Loopback-only except the B/IP +
/// SC entries exercised in the RB-16 real-transport proofs. Field layout
/// follows Clause 12.56; see the module docs for per-transport semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkPortEntry {
    /// Network-Port object instance (caller-stable; B/IP=1, SC=2 by convention).
    pub instance: u32,
    /// Clause 12.56 Network_Type (IPV4=5 for B/IP, VIRTUAL=7 for SC/loopback).
    pub network_type: u32,
    /// BACnet network number for this port.
    pub network_number: u32,
    /// Port MAC (B/IP 6-byte IP+port, SC VMAC, loopback MAC).
    pub mac: MacAddr,
    /// IPv4 octets for B/IP; zeros for SC/loopback.
    pub ip: [u8; 4],
    /// UDP port for B/IP; 0 for SC/loopback.
    pub udp_port: u16,
}

impl NetworkPortEntry {
    /// B/IP entry with the actual bound IP/port.
    ///
    /// Derives the 6-byte B/IP MAC (IP octets + big-endian UDP port) from the
    /// bound socket address. Ephemeral-port tests refresh via
    /// [`DeviceIdentity::sync_bip_bind`] after `start()`.
    ///
    /// ```
    /// use std::net::Ipv4Addr;
    /// use bacnet_endpoint::identity::NetworkPortEntry;
    ///
    /// let entry = NetworkPortEntry::bip(1, 0, Ipv4Addr::LOCALHOST, 47808);
    /// assert_eq!(entry.instance, 1);
    /// assert_eq!(entry.udp_port, 47808);
    /// ```
    pub fn bip(instance: u32, network_number: u32, ip: Ipv4Addr, udp_port: u16) -> Self {
        let o = ip.octets();
        let mut mac = MacAddr::new();
        mac.extend_from_slice(&o);
        mac.extend_from_slice(&udp_port.to_be_bytes());
        Self {
            instance,
            network_type: NetworkType::IPV4.to_raw(),
            network_number,
            mac,
            ip: o,
            udp_port,
        }
    }

    /// SC entry keyed by VMAC (no B/IP socket fields).
    ///
    /// IP octets stay zero and the UDP port stays 0: SC has no B/IP socket.
    /// The VMAC must equal the SC builder's VMAC at hub-dial build time.
    pub fn sc(instance: u32, network_number: u32, vmac: [u8; 6]) -> Self {
        Self {
            instance,
            network_type: NetworkType::VIRTUAL.to_raw(),
            network_number,
            mac: MacAddr::from_slice(&vmac),
            ip: [0; 4],
            udp_port: 0,
        }
    }
}

/// Single Device identity source.
///
/// Owns instance, vendor, max-APDU, segmentation, services profile,
/// Network-Port set, and SC UUID. Constructs the database AND derives
/// role/transport limits. I-Am identical to Device ReadProperty is proven on
/// real B/IP + SC; finer port/UUID/capability corners are Loopback-only (see
/// module docs).
///
/// ```
/// use bacnet_endpoint::identity::DeviceIdentity;
///
/// let identity = DeviceIdentity::new(1001, 42)?;
/// assert_eq!(identity.instance(), 1001);
/// assert_eq!(identity.vendor_id(), 42);
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
#[derive(Clone, Debug)]
pub struct DeviceIdentity {
    instance: u32,
    vendor_id: u16,
    max_apdu_length: u16,
    segmentation_supported: Segmentation,
    services: Vec<ServiceSupported>,
    network_ports: Vec<NetworkPortEntry>,
    device_uuid: [u8; 16],
    name: String,
}

impl DeviceIdentity {
    /// Creates an identity with endpoint-composed defaults.
    ///
    /// Defaults: `max_apdu = 1476`, `segmentation = NONE`,
    /// `services = [READ_PROPERTY]` (narrow endpoint reality, no superset),
    /// no ports, zero UUID (B/IP-only may keep zeros; SC dial requires
    /// [`with_device_uuid`](Self::with_device_uuid)). Returns
    /// [`Error::Encoding`](bacnet_types::error::Error::Encoding) for an
    /// out-of-range Device instance.
    pub fn new(instance: u32, vendor_id: u16) -> Result<Self, Error> {
        ObjectIdentifier::new(ObjectType::DEVICE, instance)?;
        Ok(Self {
            instance,
            vendor_id,
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            services: vec![ServiceSupported::READ_PROPERTY],
            network_ports: Vec::new(),
            device_uuid: [0; 16],
            name: format!("device-{instance}"),
        })
    }

    /// Overrides the max-APDU accepted/advertised (must be a wire-legal value).
    ///
    /// Validated by the codec's APDU-length table; illegal values return
    /// [`Error::Encoding`](bacnet_types::error::Error::Encoding). Note the
    /// MS/TP builder additionally rejects values above its 480 transport
    /// bound at build time.
    pub fn with_max_apdu(mut self, max_apdu: u16) -> Result<Self, Error> {
        validate_max_apdu_length(max_apdu)?;
        self.max_apdu_length = max_apdu;
        Ok(self)
    }

    /// Overrides segmentation support (endpoint proofs use NONE).
    ///
    /// The endpoint roles answer segmentation-`Abort`; advertising support
    /// beyond what the composed roles honor breaks the I-Am vs behavior
    /// matrix.
    pub fn with_segmentation(mut self, segmentation: Segmentation) -> Self {
        self.segmentation_supported = segmentation;
        self
    }

    /// Overrides the services profile (must equal what roles can do; no superset).
    ///
    /// Default `[READ_PROPERTY]` matches the narrow endpoint responder.
    /// `EndpointSession::with_device_writes` adds `WRITE_PROPERTY` and rejects
    /// other service bits. Broader profiles require a matching full server.
    pub fn with_services(mut self, services: &[ServiceSupported]) -> Self {
        self.services = services.to_vec();
        self
    }

    /// Stores the durable caller-owned SC device UUID (no generation/storage).
    ///
    /// The caller provisions 16 bytes durably and reuses them for the device
    /// lifetime. SC hub-dial rejects all-zero; B/IP-only identities may keep
    /// zeros.
    pub fn with_device_uuid(mut self, uuid: [u8; 16]) -> Self {
        self.device_uuid = uuid;
        self
    }

    /// Overrides the Device object name (defaults to `device-{instance}`).
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Adds one Network-Port entry (one per bound transport).
    ///
    /// Rejects duplicate instances and out-of-range Network-Port instances
    /// with [`Error::Encoding`](bacnet_types::error::Error::Encoding).
    /// Prefer [`with_bip_port`](Self::with_bip_port) /
    /// [`with_sc_port`](Self::with_sc_port), which encode the population rule.
    pub fn with_network_port(mut self, entry: NetworkPortEntry) -> Result<Self, Error> {
        ObjectIdentifier::new(ObjectType::NETWORK_PORT, entry.instance)?;
        if self
            .network_ports
            .iter()
            .any(|e| e.instance == entry.instance)
        {
            return Err(Error::Encoding(format!(
                "duplicate Network-Port instance {}",
                entry.instance
            )));
        }
        self.network_ports.push(entry);
        Ok(self)
    }

    /// Adds a B/IP port entry with the actual bound IP/port.
    pub fn with_bip_port(
        self,
        instance: u32,
        network_number: u32,
        ip: Ipv4Addr,
        udp_port: u16,
    ) -> Result<Self, Error> {
        self.with_network_port(NetworkPortEntry::bip(
            instance,
            network_number,
            ip,
            udp_port,
        ))
    }

    /// Adds an SC port entry keyed by VMAC.
    pub fn with_sc_port(
        self,
        instance: u32,
        network_number: u32,
        vmac: [u8; 6],
    ) -> Result<Self, Error> {
        self.with_network_port(NetworkPortEntry::sc(instance, network_number, vmac))
    }

    /// Refreshes the B/IP entry with the actual bound socket address.
    ///
    /// Call after `start()` when the port was 0 (ephemeral): replaces the
    /// matching instance entry, or pushes instance 1 when no B/IP entry
    /// exists yet. SC/loopback entries are untouched.
    pub fn sync_bip_bind(&mut self, instance: u32, network_number: u32, ip: Ipv4Addr, port: u16) {
        let entry = NetworkPortEntry::bip(instance, network_number, ip, port);
        if let Some(slot) = self
            .network_ports
            .iter_mut()
            .find(|e| e.instance == instance)
        {
            *slot = entry;
        } else {
            self.network_ports.push(entry);
        }
    }

    /// Device instance number.
    pub fn instance(&self) -> u32 {
        self.instance
    }

    /// Vendor identifier (I-Am + Device VENDOR_IDENTIFIER).
    pub fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    /// Composed max-APDU (I-Am + Device + client/server limits).
    pub fn max_apdu_length(&self) -> u16 {
        self.max_apdu_length
    }

    /// Client role max-APDU (same composed value; overrides SessionConfig 480).
    pub fn client_max_apdu(&self) -> u16 {
        self.max_apdu_length
    }

    /// Server role max-APDU clamp (same composed value).
    pub fn server_max_apdu(&self) -> u16 {
        self.max_apdu_length
    }

    /// Segmentation support (I-Am + Device SEGMENTATION_SUPPORTED).
    pub fn segmentation(&self) -> Segmentation {
        self.segmentation_supported
    }

    /// Services profile (Device PROTOCOL_SERVICES_SUPPORTED; no superset).
    pub fn services(&self) -> &[ServiceSupported] {
        &self.services
    }

    /// Network-Port entries (one per bound transport).
    pub fn network_ports(&self) -> &[NetworkPortEntry] {
        &self.network_ports
    }

    /// Durable caller-owned SC device UUID.
    pub fn device_uuid(&self) -> [u8; 16] {
        self.device_uuid
    }

    /// Device object identifier derived from the instance.
    pub fn device_oid(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::DEVICE, self.instance)
            .expect("identity instance was validated at construction")
    }

    /// I-Am request derived from this identity (endpoint + server alignment).
    ///
    /// Field-for-field identical to `broadcast_i_am_from`'s construction from
    /// a [`ServerConfig`](bacnet_server::server::ServerConfig) derived via
    /// [`Self::apply_to_server_config`]: object id + max-apdu + segmentation
    /// + vendor. Assert on wire bytes where practical (see proof tests).
    pub fn iam_request(&self) -> IAmRequest {
        IAmRequest {
            object_identifier: self.device_oid(),
            max_apdu_length: u32::from(self.max_apdu_length),
            segmentation_supported: self.segmentation_supported,
            vendor_id: self.vendor_id,
        }
    }

    /// Encodes the I-Am Unconfirmed-Request APDU bytes for this identity.
    pub fn encode_iam_apdu(&self) -> Result<Vec<u8>, Error> {
        let mut service = BytesMut::new();
        self.iam_request().encode(&mut service);
        let mut buf = BytesMut::new();
        encode_apdu(
            &mut buf,
            &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice: bacnet_types::enums::UnconfirmedServiceChoice::I_AM,
                service_request: service.freeze(),
            }),
        )?;
        Ok(buf.to_vec())
    }

    /// Constructs the ObjectDatabase (Device + Network-Port + DEVICE_UUID).
    ///
    /// Truth direction: identity -> database. For extra application objects
    /// use [`build_database_with_extra`]: it seeds `Object_List` with
    /// Device + ports + extras upfront so no post-add mutation is needed.
    pub fn build_database(&self) -> Result<ObjectDatabase, Error> {
        let mut device = DeviceObject::new(DeviceConfig {
            instance: self.instance,
            name: self.name.clone(),
            vendor_name: "Rusty BACnet".into(),
            vendor_id: self.vendor_id,
            model_name: "rusty-bacnet-endpoint".into(),
            firmware_revision: env!("CARGO_PKG_VERSION").into(),
            application_software_version: env!("CARGO_PKG_VERSION").into(),
            max_apdu_length: u32::from(self.max_apdu_length),
            segmentation_supported: self.segmentation_supported,
            apdu_timeout: 6000,
            apdu_retries: 3,
        })?;
        device.set_services_supported(&self.services);
        device.set_device_uuid(self.device_uuid);
        let device_oid = device.object_identifier();

        let mut oids = vec![device_oid];
        let mut ports = Vec::new();
        for entry in &self.network_ports {
            let mut port = NetworkPortObject::new(
                entry.instance,
                format!("port-{}", entry.instance),
                entry.network_type,
            )?;
            port.set_network_number(entry.network_number);
            port.set_mac_address(entry.mac.clone());
            port.set_max_apdu_length_accepted(u32::from(self.max_apdu_length));
            if entry.network_type == NetworkType::IPV4.to_raw() {
                port.set_ip_address(entry.ip.to_vec());
                port.set_udp_port(entry.udp_port);
            }
            oids.push(port.object_identifier());
            ports.push(port);
        }
        device.set_object_list(oids);

        let mut db = ObjectDatabase::new();
        db.add(Box::new(device))?;
        for port in ports {
            db.add(Box::new(port))?;
        }
        Ok(db)
    }

    /// Applies identity limits to a standalone SessionConfig (overrides 480).
    pub fn apply_to_session_config(&self, config: &mut crate::session::SessionConfig) {
        config.max_apdu_length = self.max_apdu_length;
    }

    /// Applies identity to a client config (max-APDU only; timers stay caller-owned).
    pub fn apply_to_client_config(&self, config: &mut bacnet_client::client::ClientConfig) {
        config.max_apdu_length = self.max_apdu_length;
    }

    /// Applies identity to a server config (max-apdu + segmentation + vendor).
    ///
    /// This is the discovery-alignment bridge: a `ServerConfig` derived here
    /// makes `broadcast_i_am_from` emit bytes identical to
    /// [`Self::encode_iam_apdu`].
    pub fn apply_to_server_config(&self, config: &mut bacnet_server::server::ServerConfig) {
        config.max_apdu_length = u32::from(self.max_apdu_length);
        config.segmentation_supported = self.segmentation_supported;
        config.vendor_id = self.vendor_id;
    }

    /// Derives a server config from this identity (discovery-alignment helper).
    pub fn server_config(&self) -> bacnet_server::server::ServerConfig {
        let mut config = bacnet_server::server::ServerConfig::default();
        self.apply_to_server_config(&mut config);
        config
    }

    /// Derives a client config from this identity (timers stay default).
    pub fn client_config(&self) -> bacnet_client::client::ClientConfig {
        let mut config = bacnet_client::client::ClientConfig::default();
        self.apply_to_client_config(&mut config);
        config
    }
}

/// Builds a database from an identity plus extra application objects.
///
/// Truth direction stays identity-first: the Device `Object_List` is seeded
/// with Device + ports + every extra OID upfront, so no post-add mutation
/// (and no device/network-port fork) is needed. Extra objects are boxed
/// `BACnetObject`s (e.g. Analog Input for proofs).
///
/// ```
/// use bacnet_endpoint::identity::{build_database_with_extra, DeviceIdentity};
/// use bacnet_objects::analog::AnalogInputObject;
///
/// let identity = DeviceIdentity::new(1001, 42)?;
/// let mut point = AnalogInputObject::new(1, "ai-1", 0)?;
/// point.set_present_value(21.5);
/// let db = build_database_with_extra(&identity, vec![Box::new(point)])?;
/// assert!(db.get(&identity.device_oid()).is_some());
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
pub fn build_database_with_extra(
    identity: &DeviceIdentity,
    extra: Vec<Box<dyn bacnet_objects::traits::BACnetObject>>,
) -> Result<ObjectDatabase, Error> {
    use bacnet_types::enums::NetworkType;

    let mut device = DeviceObject::new(DeviceConfig {
        instance: identity.instance,
        name: identity.name.clone(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: identity.vendor_id,
        model_name: "rusty-bacnet-endpoint".into(),
        firmware_revision: env!("CARGO_PKG_VERSION").into(),
        application_software_version: env!("CARGO_PKG_VERSION").into(),
        max_apdu_length: u32::from(identity.max_apdu_length),
        segmentation_supported: identity.segmentation_supported,
        apdu_timeout: 6000,
        apdu_retries: 3,
    })?;
    device.set_services_supported(&identity.services);
    device.set_device_uuid(identity.device_uuid);
    let device_oid = device.object_identifier();

    let mut oids = vec![device_oid];
    let mut ports = Vec::new();
    for entry in &identity.network_ports {
        let mut port = NetworkPortObject::new(
            entry.instance,
            format!("port-{}", entry.instance),
            entry.network_type,
        )?;
        port.set_network_number(entry.network_number);
        port.set_mac_address(entry.mac.clone());
        port.set_max_apdu_length_accepted(u32::from(identity.max_apdu_length));
        if entry.network_type == NetworkType::IPV4.to_raw() {
            port.set_ip_address(entry.ip.to_vec());
            port.set_udp_port(entry.udp_port);
        }
        oids.push(port.object_identifier());
        ports.push(port);
    }
    for obj in &extra {
        oids.push(obj.object_identifier());
    }
    device.set_object_list(oids);

    let mut db = ObjectDatabase::new();
    db.add(Box::new(device))?;
    for port in ports {
        db.add(Box::new(port))?;
    }
    for obj in extra {
        db.add(obj)?;
    }
    Ok(db)
}
