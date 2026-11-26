//! Shared Device declarations and stock object construction.
mod network_port;
use crate::common::numbered_name;
use crate::prelude::*;
use crate::{
    database::ObjectDatabase,
    device::{DeviceConfig, DeviceObject},
    network_port::{Bip6PortConfig, BipPortConfig},
    traits::BACnetObject,
};
use bacnet_types::{
    enums::{NetworkType, ObjectType, PropertyIdentifier as P, Segmentation, ServiceSupported},
    error::{Error, Refusal},
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Defaults: `max_apdu = 1476`, `segmentation = SEGMENTED_BOTH` with a
    /// 5000 ms segment timeout (#2617), `services = [READ_PROPERTY]`, no
    /// ports, zero UUID (B/IP-only may keep zeros; SC dial requires
    /// [`with_device_uuid`](Self::with_device_uuid)). A device that segments
    /// neither way, such as one answered by `bacnet-device-core`'s
    /// `Responder`, says so with
    /// [`with_segmentation`](Self::with_segmentation). Returns
    /// [`Error::Range`] for an instance past 4194303, or for 4194303 itself,
    /// which no object may have (Clause 12.1.1, #2171).
    pub fn new(instance: u32, vendor_id: u16) -> Result<Self, Error> {
        ObjectIdentifier::new_addressable(ObjectType::DEVICE, instance)?;
        Ok(Self {
            instance,
            vendor_id,
            max_apdu_length: 1476,
            segmentation_supported: Segmentation::SEGMENTED_BOTH,
            apdu_segment_timeout_ms: 5000,
            services: vec![ServiceSupported::READ_PROPERTY],
            network_ports: Vec::new(),
            device_uuid: [0; 16],
            name: numbered_name("device-", instance),
        })
    }

    /// Sets the raw requested receive ceiling (at least 50 bytes).
    ///
    /// Discovery and Device reads preserve this raw value, including 1474.
    /// Confirmed-Request header encoders floor it separately. Startup resolves
    /// it against the owned transport's stable local receive capacity.
    pub fn with_max_apdu(mut self, max_apdu: u16) -> Result<Self, Error> {
        if max_apdu < 50 {
            return Err(Error::invalid_config("Device max-APDU must be at least 50"));
        }
        self.max_apdu_length = max_apdu;
        Ok(self)
    }

    /// Overrides segmentation support (default SEGMENTED_BOTH): the
    /// Device's Segmentation_Supported and the I-Am's. A full server, and
    /// both roles of an endpoint session, do what it says, with this
    /// identity's segment timeout. NO_SEGMENTATION suits a constrained link
    /// or a device core that can't segment.
    pub fn with_segmentation(mut self, segmentation: Segmentation) -> Self {
        self.segmentation_supported = segmentation;
        self
    }

    /// Sets the segment timeout (default 5000 ms): the Device's
    /// APDU_Segment_Timeout, which a full server and an endpoint session's
    /// client use while they segment. APDU timeouts and retries stay the
    /// caller's.
    pub fn with_apdu_segment_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.apdu_segment_timeout_ms = timeout_ms;
        self
    }

    /// Overrides the services profile: the Protocol_Services_Supported of a
    /// Device [`build_database`](Self::build_database) builds (default
    /// `[READ_PROPERTY]`). List only services the device executes.
    ///
    /// An endpoint session with a server role doesn't use it: its full
    /// server lists what the node executes (#2617). A client-only session
    /// keeps it as a local declaration.
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
            return Err(Error::InvalidConfig(
                Refusal::new("duplicate Network-Port instance {}")
                    .with_value(u64::from(entry.instance)),
            ));
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
            .ok_or_else(|| Error::invalid_config("selected Network Port missing from identity"))?;
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
            return Err(Error::invalid_config(
                "selected identity Network Port differs from database",
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
            return Err(Error::invalid_config(
                "selected identity Network Port differs from database",
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
        ObjectIdentifier::new_addressable(ObjectType::DEVICE, self.instance)?;
        if self.max_apdu_length < 50 {
            return Err(Error::invalid_config("Device max-APDU must be at least 50"));
        }
        if self.segmentation_supported.to_raw() > Segmentation::NO_SEGMENTATION.to_raw() {
            return Err(Error::invalid_config("invalid Device segmentation support"));
        }
        if self.segmentation_supported != Segmentation::NO_SEGMENTATION
            && self.apdu_segment_timeout_ms == 0
        {
            return Err(Error::invalid_config(
                "APDU_Segment_Timeout must be positive when segmentation is supported",
            ));
        }
        Ok(())
    }

    /// Validate a supplied Device without rebuilding it or changing its state.
    pub fn validate_device(&self, device: &dyn BACnetObject) -> Result<(), Error> {
        self.validate()?;
        if device.object_identifier() != self.device_oid() {
            return Err(Error::invalid_config(
                "selected Device identifier must match identity",
            ));
        }
        // One whole static message per property, which names it as its
        // `Debug` form does (#1833).
        for (property, expected, refusal) in [
            (
                P::OBJECT_IDENTIFIER,
                PropertyValue::ObjectIdentifier(self.device_oid()),
                "selected Device PropertyIdentifier::OBJECT_IDENTIFIER must match identity",
            ),
            (
                P::VENDOR_IDENTIFIER,
                PropertyValue::Unsigned(u64::from(self.vendor_id)),
                "selected Device PropertyIdentifier::VENDOR_IDENTIFIER must match identity",
            ),
            (
                P::MAX_APDU_LENGTH_ACCEPTED,
                PropertyValue::Unsigned(u64::from(self.max_apdu_length)),
                "selected Device PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED must match identity",
            ),
            (
                P::SEGMENTATION_SUPPORTED,
                PropertyValue::Enumerated(u32::from(self.segmentation_supported.to_raw())),
                "selected Device PropertyIdentifier::SEGMENTATION_SUPPORTED must match identity",
            ),
        ] {
            if device.read_property(property, None)? != expected {
                return Err(Error::invalid_config(refusal));
            }
        }
        if self.segmentation_supported != Segmentation::NO_SEGMENTATION
            && device.read_property(P::APDU_SEGMENT_TIMEOUT, None)?
                != PropertyValue::Unsigned(self.apdu_segment_timeout_ms)
        {
            return Err(Error::invalid_config(
                "selected Device APDU_Segment_Timeout must match identity",
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
        ObjectIdentifier::new_addressable(ObjectType::DEVICE, self.instance)
            .expect("identity instance was validated at construction")
    }

    /// Constructs the ObjectDatabase (Device + Network-Port + DEVICE_UUID).
    ///
    /// Truth direction: identity -> database. For extra application objects
    /// use [`build_database_with_extra`]. The Device's Object_List is the
    /// database's own ([`ObjectDatabase::object_list`]): the Device, then the
    /// ports.
    pub fn build_database(&self) -> Result<ObjectDatabase, Error> {
        let mut device = DeviceObject::new(stock_config(self))?;
        device.set_services_supported(&self.services);
        device.set_device_uuid(self.device_uuid);

        let mut ports = Vec::new();
        for entry in &self.network_ports {
            ports.push(entry.build_object()?);
        }

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
/// Truth direction stays identity-first. The Device's Object_List is the
/// database's own ([`ObjectDatabase::object_list`]), which follows every
/// later add and remove: the Device, the ports, then the extra objects in
/// order. Extra objects are boxed `BACnetObject`s (e.g. Analog Input for
/// proofs).
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
    let device = DeviceObject::new(stock_config(identity))?;
    assemble(identity, device, extra)
}

/// [`build_database_with_extra`], with the Device built on `persistence`
/// ([`DeviceObject::with_tags_persistence`]), so it keeps what peers write
/// to it across a restart (#2730): a written Description and, where the
/// server lets peers write the clock's zone, a written UTC_Offset and
/// Daylight_Savings_Status. The Device has no Tags row, as one built
/// without storage has none. Fails as `with_tags_persistence` does on
/// storage it can't load.
pub fn build_database_with_tags_persistence(
    identity: &DeviceIdentity,
    persistence: alloc::sync::Arc<dyn crate::object_profile::TagsPersistence>,
    extra: Vec<Box<dyn crate::traits::BACnetObject>>,
) -> Result<ObjectDatabase, Error> {
    let device = DeviceObject::with_tags_persistence(stock_config(identity), persistence)?;
    assemble(identity, device, extra)
}

/// The stock Device's configuration for `identity`.
fn stock_config(identity: &DeviceIdentity) -> DeviceConfig {
    DeviceConfig {
        identity: identity.clone(),
        firmware_revision: env!("CARGO_PKG_VERSION").into(),
        application_software_version: env!("CARGO_PKG_VERSION").into(),
        ..DeviceConfig::default()
    }
}

/// The database holding `device`, given `identity`'s services and UUID, the
/// identity's ports and `extra`, refusing a repeated identifier.
fn assemble(
    identity: &DeviceIdentity,
    mut device: DeviceObject,
    extra: Vec<Box<dyn crate::traits::BACnetObject>>,
) -> Result<ObjectDatabase, Error> {
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
    // The database replaces an object added under an identifier it already
    // holds, which would silently drop the Device, a declared port or an
    // earlier extra object. Refuse before anything is added (#2250).
    let mut seen = crate::collections::HashSet::with_capacity(oids.len());
    if !oids.iter().all(|oid| seen.insert(*oid)) {
        return Err(Error::protocol(
            bacnet_types::enums::ErrorClass::OBJECT,
            bacnet_types::enums::ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
        ));
    }

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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod identity_tests;

#[cfg(test)]
mod network_port_tests;
