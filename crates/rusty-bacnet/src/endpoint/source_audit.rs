//! The endpoints' source Audit Reporter keywords (#2348, #2357):
//! `BipEndpoint`, `ScEndpoint` and `MstpEndpoint` take the same ones, each
//! binding address in its link's format, or a Device behind a router as
//! `"network:0xMAC[:one-station]"` or a `RoutedTarget` on any of them (#2371, #2738). They
//! are parsed once at construction and kept for the rebuild at each start,
//! as the object keywords are (#1867).

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
use crate::types::{PyObjectIdentifier, PyRoutedTarget};

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
    /// separators; an MS/TP station in ASCII decimal digits. On any link, a
    /// station behind a router as [`routed`] reads it. `None` when it
    /// doesn't parse.
    fn parse(self, address: &str) -> Option<Binding> {
        if let Some(binding) = routed(address) {
            return binding;
        }
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
            // The Rust session's routed rule: one station on another network.
            // The MAC's format is that network's link's, so only its length
            // is checked.
            (_, Binding::Routed { network, mac, .. }) => {
                if !(1..=0xFFFE).contains(network)
                    || !(1..=bacnet_npdu::MAX_MAC_LEN).contains(&mac.len())
                {
                    return Err(PyValueError::new_err(format!(
                        "{name} must name one station behind a router: a network of \
                         1 to 65534 and a MAC of 1 to {} octets",
                        bacnet_npdu::MAX_MAC_LEN
                    )));
                }
                return Ok(());
            }
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
            Self::Bip(_) => "'ipv4:port' or a routed 'network:0xMAC[:one-station]'",
            Self::Sc(_) => "a VMAC, 'xx:xx:xx:xx:xx:xx', or a routed 'network:0xMAC[:one-station]'",
            Self::Mstp(_) => "a station, 0 to 254, or a routed 'network:0xMAC[:one-station]'",
        }
    }
}

/// A station behind a router, written as the CLI writes a routed target
/// (`bacnet_ops::target::parse_target`'s `dnet:0xMAC`): the network in
/// decimal digits, a colon, `0x` and an even, nonzero number of hex digits,
/// then optionally `:one-station`, which marks the address one station
/// (#2738) and is taken only at a MAC the mark lifts
/// ([`bacnet_ops::target::check_one_station`]).
/// `None` when `address` isn't in that form (decimal digits, then `:0x`);
/// `Some(None)` when it is, but doesn't parse. No other link's form starts
/// that way: a B/IP address has dots, and a VMAC's second octet no `x`.
fn routed(address: &str) -> Option<Option<Binding>> {
    let (network, rest) = address.split_once(':')?;
    let (mac, mark) = match rest.split_once(':') {
        Some((mac, mark)) => (mac, Some(mark)),
        None => (rest, None),
    };
    let hex = mac.strip_prefix("0x").or(mac.strip_prefix("0X"))?;
    if network.is_empty() || !network.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let octets = (!hex.is_empty()
        && hex.len().is_multiple_of(2)
        && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| {
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()
    })
    .flatten();
    let one_station = mark.is_some();
    let marked_ok = match (mark, &octets) {
        (None, _) => true,
        (Some(bacnet_ops::target::ONE_STATION_SUFFIX), Some(octets)) => {
            bacnet_ops::target::check_one_station(octets).is_ok()
        }
        (Some(_), _) => false,
    };
    Some(
        network
            .parse()
            .ok()
            .zip(octets.filter(|_| marked_ok))
            .map(|(network, mac)| Binding::Routed {
                network,
                mac: MacAddr::from_slice(&mac),
                one_station,
            }),
    )
}

/// One Device binding's address.
#[derive(Clone, Debug)]
enum Binding {
    Bip(SocketAddrV4),
    Mac(MacAddr),
    /// A station behind a router, on any link (#2371), its address marked
    /// one station when a `RoutedTarget` says so (#2738).
    Routed {
        network: u16,
        mac: MacAddr,
        one_station: bool,
    },
}

/// One `audit_device_bindings` address as Python gives it: a `str` in the
/// link's format or a routed `"network:0xMAC[:one-station]"`, or a
/// `RoutedTarget`, which also marks an address one station (#2738). A
/// `RoutedTarget`'s network and MAC were checked when it was made; a router
/// on it is refused when the bindings are parsed.
pub(crate) enum BindingAddress {
    /// A `str`.
    Text(String),
    /// A `RoutedTarget`: its network, its MAC there, whether it names a
    /// router, and its one-station mark.
    Routed {
        network: u16,
        mac: MacAddr,
        router: bool,
        one_station: bool,
    },
}

impl<'a, 'py> FromPyObject<'a, 'py> for BindingAddress {
    type Error = PyErr;

    fn extract(address: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        if let Ok(target) = address.cast::<PyRoutedTarget>() {
            let target = target.get();
            let (network, mac, router) = target.parts();
            return Ok(Self::Routed {
                network,
                mac: MacAddr::from_slice(mac),
                router: router.is_some(),
                one_station: target.is_one_station(),
            });
        }
        match address.cast::<pyo3::types::PyString>() {
            Ok(text) => Ok(Self::Text(text.to_cow()?.into_owned())),
            Err(_) => Err(pyo3::exceptions::PyTypeError::new_err(format!(
                "an audit_device_bindings address must be a str or a RoutedTarget, not {}",
                address.get_type().name()?
            ))),
        }
    }
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
    /// Reporter without a recipient, or beside a server whose policy leaves
    /// mutations undecided (`decides_mutations` false: `mutation_policy=
    /// "permissive"`), a recipient the session can never deliver to (this
    /// device, the global network, or an address that isn't another node's
    /// unicast MAC by `link`'s rule), and bad or duplicate bindings, by the
    /// same rule. Whether the Reporter is in the database is checked at
    /// start, once the `add_*` calls are in.
    pub(super) fn parse(
        reporter: Option<PyObjectIdentifier>,
        recipient: Option<&Bound<'_, PyAny>>,
        bindings: Option<Vec<(PyObjectIdentifier, BindingAddress)>>,
        device_instance: u32,
        link: AuditLink,
        decides_mutations: bool,
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
        if !decides_mutations {
            return Err(PyValueError::new_err(
                "source_audit_reporter needs a mutation_policy that decides writes, since \
                 the Device's Audit recipient is writable: leave mutation_policy out (the \
                 read-only server) or give 'deny_all' or a callable; 'permissive' lets any \
                 peer redirect the audit trail",
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
            let address = match address {
                BindingAddress::Text(address) => link.parse(&address).ok_or_else(|| {
                    PyValueError::new_err(format!(
                        "audit_device_bindings[{index}] address must be {}",
                        link.format()
                    ))
                })?,
                // The session learns a routed binding's router (#2371).
                BindingAddress::Routed { router: true, .. } => {
                    return Err(PyValueError::new_err(format!(
                        "audit_device_bindings[{index}] is a RoutedTarget with a router: the \
                         session learns the router to a binding's network, so leave router out"
                    )));
                }
                BindingAddress::Routed {
                    network,
                    mac,
                    one_station,
                    ..
                } => Binding::Routed {
                    network,
                    mac,
                    one_station,
                },
            };
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
                Binding::Mac(_) | Binding::Routed { .. } => builder,
            })
    }

    /// Select the Reporter on `session`, with the routes of a link other
    /// than B/IP and the routed ones of any link. Its start refuses a
    /// Reporter that isn't an Audit Reporter in the database, with a
    /// BacnetError naming it.
    pub(super) fn select<T: TransportPort + 'static>(
        &self,
        session: EndpointSession<T>,
    ) -> EndpointSession<T> {
        self.bindings
            .iter()
            .fold(session, |session, (device, address)| match address {
                Binding::Mac(mac) => session.with_source_audit_device_binding(*device, mac.clone()),
                Binding::Routed {
                    network,
                    mac,
                    one_station: false,
                } => {
                    session.with_source_audit_routed_device_binding(*device, *network, mac.clone())
                }
                Binding::Routed {
                    network,
                    mac,
                    one_station: true,
                } => session.with_source_audit_routed_one_station_binding(
                    *device,
                    *network,
                    mac.clone(),
                ),
                Binding::Bip(_) => session,
            })
            .with_source_audit_reporter(self.reporter)
    }
}
