//! The endpoints' source Audit Reporter keywords (#2348, #2357):
//! `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take the same ones, each
//! binding address in its link's format. They are parsed once at
//! construction and kept for the rebuild at each start, as the object
//! keywords are (#1867).

use std::net::{Ipv4Addr, SocketAddrV4};

use bacnet_endpoint::bip::BipEndpointBuilder;
use bacnet_endpoint::identity::DeviceIdentity;
use bacnet_endpoint::session::EndpointSession;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::port::TransportPort;
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetRecipient;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;

use crate::errors::to_py_err;
use crate::types::PyObjectIdentifier;

/// The endpoint's link, with what its bindings are checked against.
#[derive(Clone, Copy, Debug)]
pub(super) enum AuditLink {
    /// IPv4 B/IP, with the configured broadcast address.
    Bip(Ipv4Addr),
    /// BACnet/SC, with this node's VMAC.
    Sc([u8; 6]),
    /// MS/TP, with this node's station.
    Mstp(u8),
}

impl AuditLink {
    /// A binding's address, in the link's format, as a recipient MAC: an
    /// IPv4 B/IP `"ipv4:port"`; an SC VMAC as `local_address()` gives it,
    /// `"xx:xx:xx:xx:xx:xx"`, or the same twelve hex digits with no
    /// separators; an MS/TP station in ASCII decimal digits. `None` when it
    /// doesn't parse.
    fn parse(self, address: &str) -> Option<Binding> {
        match self {
            Self::Bip(_) => address.parse().ok().map(Binding::Bip),
            Self::Sc(_) => {
                let digits: String = match address.len() {
                    12 => address.into(),
                    // A colon between every pair of digits, and nowhere else.
                    17 if address
                        .bytes()
                        .enumerate()
                        .all(|(index, byte)| (byte == b':') == (index % 3 == 2)) =>
                    {
                        address.split(':').collect()
                    }
                    _ => return None,
                };
                if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return None;
                }
                let octets: Vec<u8> = (0..6)
                    .map(|i| u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).ok())
                    .collect::<Option<_>>()?;
                Some(Binding::Mac(MacAddr::from_slice(&octets)))
            }
            // Digits only: `u8::from_str` would also take a sign.
            Self::Mstp(_) if !address.is_empty() && address.bytes().all(|b| b.is_ascii_digit()) => {
                address
                    .parse::<u8>()
                    .ok()
                    .map(|station| Binding::Mac(MacAddr::from_slice(&[station])))
            }
            Self::Mstp(_) => None,
        }
    }

    /// Whether `mac` is another node's unicast MAC on an SC or MS/TP link,
    /// by the Rust session's rule: the link's MAC length, not this node's
    /// own, and not SC's broadcast or all-zero VMAC or MS/TP's station 255.
    fn unicast(self, mac: &[u8]) -> bool {
        match self {
            Self::Sc(own) => mac.len() == 6 && mac != own && mac != [0xFF; 6] && mac != [0; 6],
            Self::Mstp(own) => mac.len() == 1 && mac != [own] && mac != [0xFF],
            Self::Bip(_) => false,
        }
    }

    /// Why `binding` can't be a unicast recipient on this link, if it can't:
    /// B/IP's recipient rule (#2348), or [`Self::unicast`] on SC and MS/TP.
    fn refusal(self, binding: &Binding, device_instance: u32, name: &str) -> PyResult<()> {
        let mac = match (self, binding) {
            (Self::Bip(broadcast), Binding::Bip(address)) => {
                let mut mac = address.ip().octets().to_vec();
                mac.extend_from_slice(&address.port().to_be_bytes());
                return crate::server::validate_audit_recipient(
                    &BACnetRecipient::Address(bacnet_types::constructed::BACnetAddress {
                        network_number: 0,
                        mac_address: mac.as_slice().into(),
                    }),
                    device_instance,
                    Some(broadcast),
                    name,
                );
            }
            (_, Binding::Mac(mac)) => mac.as_slice(),
            (_, Binding::Bip(_)) => unreachable!("only B/IP parses an IPv4 binding"),
        };
        if !self.unicast(mac) {
            return Err(PyValueError::new_err(format!(
                "{name} must be another node's unicast address on this link"
            )));
        }
        Ok(())
    }

    /// The binding address format, for messages.
    fn format(self) -> &'static str {
        match self {
            Self::Bip(_) => "'ipv4:port'",
            Self::Sc(_) => "a VMAC, 'xx:xx:xx:xx:xx:xx'",
            Self::Mstp(_) => "a station, 0 to 254",
        }
    }
}

/// One Device binding's address.
#[derive(Clone, Debug)]
enum Binding {
    Bip(SocketAddrV4),
    Mac(MacAddr),
}

/// A validated source Reporter selection: the Reporter, the Device's Audit
/// recipient and the routes to recipient Devices.
#[derive(Clone, Debug)]
pub(super) struct SourceAuditKeywords {
    reporter: ObjectIdentifier,
    recipient: BACnetRecipient,
    bindings: Vec<(ObjectIdentifier, Binding)>,
}

impl SourceAuditKeywords {
    /// Parse `source_audit_reporter`, `audit_recipient` and
    /// `audit_device_bindings`. What construction can check is refused here
    /// with ValueError or TypeError: a Reporter identifier that names no
    /// concrete Audit Reporter, a recipient or binding without a Reporter, a
    /// Reporter without a recipient or without `device_write_policy`, a
    /// recipient the session can never deliver to (this device, the global
    /// network, or an address that isn't another node's unicast MAC by
    /// `link`'s rule), and bad or duplicate bindings, by the same rule. Whether the
    /// Reporter is in the database is checked at start, once the `add_*`
    /// calls are in.
    pub(super) fn parse(
        reporter: Option<PyObjectIdentifier>,
        recipient: Option<&Bound<'_, PyAny>>,
        bindings: Option<Vec<(PyObjectIdentifier, String)>>,
        device_instance: u32,
        link: AuditLink,
        device_writes: bool,
    ) -> PyResult<Option<Self>> {
        let Some(reporter) = reporter.map(|oid| oid.to_rust()) else {
            if recipient.is_some() || bindings.is_some() {
                return Err(PyValueError::new_err(
                    "audit_recipient and audit_device_bindings need source_audit_reporter",
                ));
            }
            return Ok(None);
        };
        if reporter.object_type() != ObjectType::AUDIT_REPORTER
            || reporter.instance_number() == ObjectIdentifier::MAX_INSTANCE
        {
            return Err(PyValueError::new_err(
                "source_audit_reporter must name a concrete Audit Reporter object",
            ));
        }
        if !device_writes {
            return Err(PyValueError::new_err(
                "source_audit_reporter needs device_write_policy: the Audit recipient \
                 it reports to is writable on the local Device",
            ));
        }
        let recipient = recipient
            .ok_or_else(|| PyValueError::new_err("source_audit_reporter needs audit_recipient"))?;
        let recipient = crate::types::audit_recipient_from_py(recipient, "audit_recipient")?;
        match (link, &recipient) {
            // An address on this network, or on one the session's number
            // may yet name, as the Rust session resolves it on any link
            // (#2357): another node's unicast MAC there, by the link's rule.
            (AuditLink::Sc(_) | AuditLink::Mstp(_), BACnetRecipient::Address(address)) => {
                if address.network_number == u16::MAX
                    || !link.unicast(address.mac_address.as_slice())
                {
                    return Err(PyValueError::new_err(
                        "audit_recipient must be a concrete remote Device or another node's \
                         unicast address on this link",
                    ));
                }
            }
            (AuditLink::Bip(broadcast), _) => crate::server::validate_audit_recipient(
                &recipient,
                device_instance,
                Some(broadcast),
                "audit_recipient",
            )?,
            (AuditLink::Sc(_) | AuditLink::Mstp(_), _) => crate::server::validate_audit_recipient(
                &recipient,
                device_instance,
                None,
                "audit_recipient",
            )?,
        }
        let mut parsed: Vec<(ObjectIdentifier, Binding)> = Vec::new();
        for (index, (device, address)) in bindings.into_iter().flatten().enumerate() {
            let device = device.to_rust();
            if device.object_type() != ObjectType::DEVICE
                || device.instance_number() == ObjectIdentifier::MAX_INSTANCE
            {
                return Err(PyValueError::new_err(format!(
                    "audit_device_bindings[{index}] must name a concrete Device"
                )));
            }
            let address = link.parse(&address).ok_or_else(|| {
                PyValueError::new_err(format!(
                    "audit_device_bindings[{index}] address must be {}",
                    link.format()
                ))
            })?;
            // The link's unicast rule, which the session applies to its
            // routes at start (#2348, #2357).
            link.refusal(
                &address,
                device_instance,
                &format!("audit_device_bindings[{index}]"),
            )?;
            if parsed.iter().any(|(known, _)| *known == device) {
                return Err(PyValueError::new_err(format!(
                    "audit_device_bindings[{index}] repeats a Device"
                )));
            }
            parsed.push((device, address));
        }
        Ok(Some(Self {
            reporter,
            recipient,
            bindings: parsed,
        }))
    }

    /// Provision the Device's Audit recipient in the database built for this
    /// start.
    pub(super) fn provision(
        &self,
        identity: &DeviceIdentity,
        db: &mut ObjectDatabase,
    ) -> PyResult<()> {
        db.get_mut(&identity.device_oid())
            .and_then(|device| device.device_authority_internal())
            .ok_or_else(|| {
                PyRuntimeError::new_err("the endpoint Device lacks its Audit authority")
            })?
            .provision_audit_recipient(self.recipient.clone())
            .map_err(to_py_err)
    }

    /// Add the B/IP Device routes to the B/IP builder.
    pub(super) fn bind(&self, builder: BipEndpointBuilder) -> BipEndpointBuilder {
        self.bindings
            .iter()
            .fold(builder, |builder, (device, address)| match address {
                Binding::Bip(address) => builder.source_audit_device_binding(*device, *address),
                Binding::Mac(_) => builder,
            })
    }

    /// Select the Reporter on `session`, with the routes of a link other
    /// than B/IP. Its start refuses a Reporter that isn't an Audit Reporter
    /// in the database, with a BacnetError naming it.
    pub(super) fn select<T: TransportPort + 'static>(
        &self,
        session: EndpointSession<T>,
    ) -> EndpointSession<T> {
        self.bindings
            .iter()
            .fold(session, |session, (device, address)| match address {
                Binding::Mac(mac) => session.with_source_audit_device_binding(*device, mac.clone()),
                Binding::Bip(_) => session,
            })
            .with_source_audit_reporter(self.reporter)
    }
}
