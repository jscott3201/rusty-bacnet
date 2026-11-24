//! The checks of `registered_network_port` that `start()` makes before it
//! takes the registrations (#2748).
//!
//! The Rust server refuses a registered port inside its builder, after the
//! objects have gone into its database, and a refusal there cannot give them
//! back. Everything it checks that the pending objects and the configured
//! transport already tell is checked here first, so a fault the application
//! can correct (a port never added, one of the wrong family, an address or
//! UDP port that differs from the transport's) leaves the registrations for
//! the next start. A transport address that only the bind settles is not
//! known here.

use std::net::{Ipv4Addr, Ipv6Addr};

use bacnet_objects::traits::BACnetObject;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use super::BACnetServer;

fn refused(instance: u32, what: &str) -> PyErr {
    PyValueError::new_err(format!("registered_network_port {instance} {what}"))
}

fn octets(object: &dyn BACnetObject, property: PropertyIdentifier) -> Option<Vec<u8>> {
    match object.read_property(property, None) {
        Ok(PropertyValue::OctetString(bytes)) => Some(bytes),
        _ => None,
    }
}

fn unsigned(object: &dyn BACnetObject, property: PropertyIdentifier) -> Option<u64> {
    match object.read_property(property, None) {
        Ok(PropertyValue::Unsigned(value)) => Some(value),
        _ => None,
    }
}

impl BACnetServer {
    /// Refuse a `registered_network_port` the server's builder would refuse,
    /// as far as the pending objects and the configured transport show.
    pub(super) fn check_registered_port(
        &self,
        pending: &[Box<dyn BACnetObject + Send>],
    ) -> PyResult<()> {
        let Some(instance) = self.registered_network_port else {
            return Ok(());
        };
        let port = pending
            .iter()
            .find(|object| {
                let id = object.object_identifier();
                id.object_type() == ObjectType::NETWORK_PORT && id.instance_number() == instance
            })
            .ok_or_else(|| {
                refused(
                    instance,
                    "names no Network Port: add it with add_bip_network_port or \
                     add_bip6_network_port before start()",
                )
            })?;
        let port = port.as_ref();
        if self.transport_type == "ipv6" {
            let configured = octets(port, PropertyIdentifier::IPV6_ADDRESS)
                .and_then(|bytes| <[u8; 16]>::try_from(bytes).ok())
                .map(Ipv6Addr::from)
                .ok_or_else(|| refused(instance, "is not a B/IPv6 port (add_bip6_network_port)"))?;
            let wanted: Ipv6Addr = self
                .ipv6_interface
                .as_deref()
                .unwrap_or("::")
                .parse()
                .map_err(|_| refused(instance, "needs a valid ipv6_interface"))?;
            if wanted.is_unspecified() || wanted.is_multicast() {
                return Err(refused(
                    instance,
                    "needs ipv6_interface to be one concrete unicast address",
                ));
            }
            if configured != wanted
                || unsigned(port, PropertyIdentifier::BACNET_IPV6_UDP_PORT)
                    != Some(u64::from(self.port))
            {
                return Err(refused(
                    instance,
                    "is configured with an address or UDP port that differs from the \
                     transport's ipv6_interface and port",
                ));
            }
            return Ok(());
        }
        let configured = octets(port, PropertyIdentifier::IP_ADDRESS)
            .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            .map(Ipv4Addr::from)
            .ok_or_else(|| refused(instance, "is not a B/IP port (add_bip_network_port)"))?;
        let wanted = self.interface;
        if wanted.is_unspecified() || wanted.is_multicast() || wanted.is_broadcast() {
            return Err(refused(
                instance,
                "needs interface to be one concrete unicast address",
            ));
        }
        if configured != wanted
            || unsigned(port, PropertyIdentifier::BACNET_IP_UDP_PORT) != Some(u64::from(self.port))
        {
            return Err(refused(
                instance,
                "is configured with an address or UDP port that differs from the \
                 transport's interface and port",
            ));
        }
        Ok(())
    }
}
