//! Shared Device declarations and stock object construction.
mod network_port;
use crate::prelude::*;
use crate::{
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
    network_port::{Bip6PortConfig, BipPortConfig},
    traits::BACnetObject,
};
use bacnet_types::{
    enums::{NetworkType, ObjectType, PropertyIdentifier as P, Segmentation, ServiceSupported},
    error::Error,
    primitives::{ObjectIdentifier, PropertyValue},
};
use core::net::{Ipv4Addr, Ipv6Addr};
pub use network_port::NetworkPortEntry;

/// Single Device identity source.
///
/// Owns the Device declarations shared by a server and an endpoint.
/// Resolve the requested receive ceiling against the owned transport before
/// generating a stock Device or validating a supplied one. Services and
/// Network Ports are configuration data; each executor controls its served
/// service profile.
///
/// ```
/// use bacnet_objects::device_identity::DeviceIdentity;
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
    apdu_segment_timeout_ms: u64,
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
    /// [`Error::Encoding`] for an
    /// out-of-range Device instance.
    pub fn new(instance: u32, vendor_id: u16) -> Result<Self, Error> {
        ObjectIdentifier::new(ObjectType::DEVICE, instance)?;
        Ok(Self {
            instance,
            vendor_id,
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::NONE,
            apdu_segment_timeout_ms: 5000,
            services: vec![ServiceSupported::READ_PROPERTY],
            network_ports: Vec::new(),
            device_uuid: [0; 16],
            name: format!("device-{instance}"),
        })
    }

    /// Sets the raw requested receive ceiling (at least 50 bytes).
    ///
    /// Discovery and Device reads preserve this raw value, including 1474.
    /// Confirmed-Request header encoders floor it separately. Startup resolves
    /// it against the owned transport's stable local receive capacity.
    pub fn with_max_apdu(mut self, max_apdu: u16) -> Result<Self, Error> {
        if max_apdu < 50 {
            return Err(Error::Encoding(
                "Device max-APDU must be at least 50".into(),
            ));
        }
        self.max_apdu_length = max_apdu;
        Ok(self)
    }

    /// Overrides segmentation support for a matching full server.
    ///
    /// Narrow EndpointSession roles require NONE and reject any other identity
    /// before ingress starts. A full server uses this declaration with the
    /// segment timeout supplied by this identity.
    pub fn with_segmentation(mut self, segmentation: Segmentation) -> Self {
        self.segmentation_supported = segmentation;
        self
    }

    /// Sets the Device and full-server segment timeout. EndpointSession roles
    /// require segmentation NONE; client timers remain caller-owned.
    pub fn with_apdu_segment_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.apdu_segment_timeout_ms = timeout_ms;
        self
    }

    /// Overrides the services profile (must equal what roles can do; no superset).
    ///
    /// Default `[READ_PROPERTY]` matches the narrow endpoint responder.
    /// `EndpointSession::with_device_writes` adds `WRITE_PROPERTY` and
    /// `EndpointSession::with_reinitialize` adds `REINITIALIZE_DEVICE`; other
    /// service bits are rejected. Server-role startup also rejects missing
    /// ReadProperty or unsupported bits with neither opt-in, before ingress starts.
    /// ClientOnly has no responder and retains this as a local declaration.
    /// Broader executing profiles require a matching full server.
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

    /// Adds one validated configured Network-Port entry.
    ///
    /// Rejects duplicate instances and out-of-range Network-Port instances
    /// and invalid profile configuration.
    /// Prefer [`with_bip_port`](Self::with_bip_port),
    /// [`with_bip6_port`](Self::with_bip6_port) or
    /// [`with_sc_port`](Self::with_sc_port), which encode the population rule.
    pub fn with_network_port(mut self, entry: NetworkPortEntry) -> Result<Self, Error> {
        entry.build_object()?;
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

    /// Adds a B/IP port entry with configured IP/UDP values, including UDP zero.
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

    /// Adds a B/IPv6 port entry with the configured IPv6 address and UDP
    /// port, including UDP zero. See [`NetworkPortEntry::bip6`].
    pub fn with_bip6_port(
        self,
        instance: u32,
        network_number: u32,
        ip: Ipv6Addr,
        udp_port: u16,
    ) -> Result<Self, Error> {
        self.with_network_port(NetworkPortEntry::bip6(
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

    /// The identity's entry for the selected port, checked as a
    /// declaration.
    fn registered_entry(&self, oid: ObjectIdentifier) -> Result<&NetworkPortEntry, Error> {
        let entry = self
            .network_ports
            .iter()
            .find(|entry| entry.instance == oid.instance_number())
            .ok_or_else(|| Error::Encoding("selected Network Port missing from identity".into()))?;
        entry.build_object()?;
        Ok(entry)
    }

    #[doc(hidden)]
    pub fn validate_registered_bip_internal(
        &self,
        oid: ObjectIdentifier,
        config: &BipPortConfig,
    ) -> Result<(), Error> {
        let entry = self.registered_entry(oid)?;
        if entry.network_type != NetworkType::IPV4.to_raw()
            || entry.ip != config.ip_address
            || entry.udp_port != config.udp_port
            || entry.network_number != u32::from(config.network_number)
        {
            return Err(Error::Encoding(
                "selected identity Network Port differs from database".into(),
            ));
        }
        Ok(())
    }

    /// Check the selected B/IPv6 entry against the database's configured
    /// port: address, UDP port and network number. The VMAC is not compared:
    /// both are placeholders that publication replaces with the VMAC the
    /// transport uses, as a B/IP check leaves the derived MAC out.
    #[doc(hidden)]
    pub fn validate_registered_bip6_internal(
        &self,
        oid: ObjectIdentifier,
        config: &Bip6PortConfig,
    ) -> Result<(), Error> {
        let entry = self.registered_entry(oid)?;
        if entry.network_type != NetworkType::IPV6.to_raw()
            || entry.ipv6 != config.ip_address
            || entry.udp_port != config.udp_port
            || entry.network_number != u32::from(config.network_number)
        {
            return Err(Error::Encoding(
                "selected identity Network Port differs from database".into(),
            ));
        }
        Ok(())
    }

    /// Record the started B/IPv6 port's bound address and the VMAC it uses
    /// in the selected entry.
    #[doc(hidden)]
    pub fn publish_registered_bip6_internal(
        &mut self,
        oid: ObjectIdentifier,
        address: core::net::SocketAddrV6,
        vmac: [u8; 3],
    ) {
        let entry = self
            .network_ports
            .iter_mut()
            .find(|entry| entry.instance == oid.instance_number())
            .expect("validated identity registration");
        *entry = NetworkPortEntry::bip6_with_vmac(
            entry.instance,
            entry.network_number,
            *address.ip(),
            address.port(),
            vmac,
        );
    }

    #[doc(hidden)]
    pub fn publish_registered_bip_internal(
        &mut self,
        oid: ObjectIdentifier,
        address: core::net::SocketAddrV4,
    ) {
        let entry = self
            .network_ports
            .iter_mut()
            .find(|entry| entry.instance == oid.instance_number())
            .expect("validated identity registration");
        *entry = NetworkPortEntry::bip(
            entry.instance,
            entry.network_number,
            *address.ip(),
            address.port(),
        );
    }

    /// Device instance number.
    pub fn instance(&self) -> u32 {
        self.instance
    }

    /// Vendor identifier (I-Am + Device VENDOR_IDENTIFIER).
    pub fn vendor_id(&self) -> u16 {
        self.vendor_id
    }

    /// Raw requested or resolved local receive ceiling.
    pub fn max_apdu_length(&self) -> u16 {
        self.max_apdu_length
    }

    /// Device object name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Segment timeout in milliseconds. The shared default is 5000 ms.
    pub fn apdu_segment_timeout_ms(&self) -> u64 {
        self.apdu_segment_timeout_ms
    }

    /// Resolve the requested receive ceiling before any transport I/O.
    pub fn resolve(&self, local_receive_capacity: u16) -> Result<Self, Error> {
        let mut identity = self.clone();
        identity.max_apdu_length = identity.max_apdu_length.min(local_receive_capacity);
        identity.validate()?;
        Ok(identity)
    }

    /// Check the pure Device declaration. Executors also validate deadlines.
    pub fn validate(&self) -> Result<(), Error> {
        ObjectIdentifier::new(ObjectType::DEVICE, self.instance)?;
        if self.max_apdu_length < 50 {
            return Err(Error::Encoding(
                "Device max-APDU must be at least 50".into(),
            ));
        }
        if self.segmentation_supported.to_raw() > Segmentation::NONE.to_raw() {
            return Err(Error::Encoding(
                "invalid Device segmentation support".into(),
            ));
        }
        if self.segmentation_supported != Segmentation::NONE && self.apdu_segment_timeout_ms == 0 {
            return Err(Error::Encoding(
                "APDU_Segment_Timeout must be positive when segmentation is supported".into(),
            ));
        }
        Ok(())
    }

    /// Validate a supplied Device without rebuilding it or changing its state.
    pub fn validate_device(&self, device: &dyn BACnetObject) -> Result<(), Error> {
        self.validate()?;
        if device.object_identifier() != self.device_oid() {
            return Err(Error::Encoding(
                "selected Device identifier must match identity".into(),
            ));
        }
        for (property, expected) in [
            (
                P::OBJECT_IDENTIFIER,
                PropertyValue::ObjectIdentifier(self.device_oid()),
            ),
            (
                P::VENDOR_IDENTIFIER,
                PropertyValue::Unsigned(u64::from(self.vendor_id)),
            ),
            (
                P::MAX_APDU_LENGTH_ACCEPTED,
                PropertyValue::Unsigned(u64::from(self.max_apdu_length)),
            ),
            (
                P::SEGMENTATION_SUPPORTED,
                PropertyValue::Enumerated(u32::from(self.segmentation_supported.to_raw())),
            ),
        ] {
            if device.read_property(property, None)? != expected {
                return Err(Error::Encoding(format!(
                    "selected Device {property:?} must match identity"
                )));
            }
        }
        if self.segmentation_supported != Segmentation::NONE
            && device.read_property(P::APDU_SEGMENT_TIMEOUT, None)?
                != PropertyValue::Unsigned(self.apdu_segment_timeout_ms)
        {
            return Err(Error::Encoding(
                "selected Device APDU_Segment_Timeout must match identity".into(),
            ));
        }
        Ok(())
    }

    /// Segmentation support (I-Am + Device SEGMENTATION_SUPPORTED).
    pub fn segmentation(&self) -> Segmentation {
        self.segmentation_supported
    }

    /// Services profile (Device PROTOCOL_SERVICES_SUPPORTED; no superset).
    pub fn services(&self) -> &[ServiceSupported] {
        &self.services
    }

    /// Configured Network-Port entries, independent of live transport binding.
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

    /// Constructs the ObjectDatabase (Device + Network-Port + DEVICE_UUID).
    ///
    /// Truth direction: identity -> database. For extra application objects
    /// use [`build_database_with_extra`]: it seeds `Object_List` with
    /// Device + ports + extras upfront so no post-add mutation is needed.
    pub fn build_database(&self) -> Result<ObjectDatabase, Error> {
        let mut device = DeviceObject::new(DeviceConfig {
            identity: self.clone(),
            firmware_revision: env!("CARGO_PKG_VERSION").into(),
            application_software_version: env!("CARGO_PKG_VERSION").into(),
            ..DeviceConfig::default()
        })?;
        device.set_services_supported(&self.services);
        device.set_device_uuid(self.device_uuid);
        let device_oid = device.object_identifier();

        let mut oids = vec![device_oid];
        let mut ports = Vec::new();
        for entry in &self.network_ports {
            let port = entry.build_object()?;
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
}

/// Builds a database from an identity plus extra application objects.
///
/// Truth direction stays identity-first: the Device `Object_List` is seeded
/// with Device + ports + every extra OID upfront, so no post-add mutation
/// (and no device/network-port fork) is needed. Extra objects are boxed
/// `BACnetObject`s (e.g. Analog Input for proofs).
///
/// ```
/// use bacnet_objects::device_identity::{build_database_with_extra, DeviceIdentity};
/// use bacnet_objects::analog::AnalogInputObject;
/// use bacnet_types::enums::EngineeringUnits;
///
/// let identity = DeviceIdentity::new(1001, 42)?;
/// let mut point = AnalogInputObject::new(1, "ai-1", EngineeringUnits::NO_UNITS)?;
/// point.set_present_value(21.5);
/// let db = build_database_with_extra(&identity, vec![Box::new(point)])?;
/// assert!(db.get(&identity.device_oid()).is_some());
/// # Ok::<(), bacnet_types::error::Error>(())
/// ```
pub fn build_database_with_extra(
    identity: &DeviceIdentity,
    extra: Vec<Box<dyn crate::traits::BACnetObject>>,
) -> Result<ObjectDatabase, Error> {
    let mut device = DeviceObject::new(DeviceConfig {
        identity: identity.clone(),
        firmware_revision: env!("CARGO_PKG_VERSION").into(),
        application_software_version: env!("CARGO_PKG_VERSION").into(),
        ..DeviceConfig::default()
    })?;
    device.set_services_supported(&identity.services);
    device.set_device_uuid(identity.device_uuid);
    let device_oid = device.object_identifier();

    let mut oids = vec![device_oid];
    let mut ports = Vec::new();
    for entry in &identity.network_ports {
        let port = entry.build_object()?;
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

impl Default for DeviceIdentity {
    fn default() -> Self {
        Self::new(1, 0)
            .expect("default Device instance is valid")
            .with_name("BACnet Device")
    }
}
