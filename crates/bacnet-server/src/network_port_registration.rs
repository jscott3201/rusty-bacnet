//! The registered B/IP or B/IPv6 Network Port's address checks and post-bind
//! publication, shared by the full server and the shared endpoint (#1940),
//! so both owners refuse the same addresses and publish the same fields.
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::port::{Bip6Port, BipPort};
use bacnet_types::bip6_vmac::{VmacHex, device_instance_vmac};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use std::net::{SocketAddrV4, SocketAddrV6};

/// The configured B/IP address a port registers, refused unless it names
/// one concrete unicast interface that is not the link's broadcast address.
pub fn bip_address(port: &BipPort, broadcast: Option<SocketAddrV4>) -> Result<SocketAddrV4, Error> {
    let address = port.endpoint;
    let ip = *address.ip();
    if ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || broadcast.is_some_and(|broadcast| *broadcast.ip() == ip)
    {
        return Err(invalid(
            "registered B/IP requires a concrete unicast interface",
        ));
    }
    Ok(address)
}

/// The configured B/IPv6 address a port registers: one concrete unicast
/// address, as a B/IP link's must be, on a link whose VMAC is the one
/// `served_device` gives (#2057).
///
/// `served_device` is the instance of the Device the owner serves. A B/IPv6
/// node with a Device instance uses it as its VMAC (Annex U.1.3, Clause
/// H.7.2), and peers find it by that instance, so a transport built for
/// another instance, or with none (a random VMAC), would publish a VMAC that
/// isn't this Device's. `None`, no served Device, is refused too: there is
/// no instance to check the VMAC against.
pub fn bip6_address(port: &Bip6Port, served_device: Option<u32>) -> Result<SocketAddrV6, Error> {
    let address = port.endpoint;
    let ip = *address.ip();
    let refused = if ip.is_unspecified() || ip.is_multicast() {
        Some("registered B/IPv6 requires a concrete unicast interface")
    } else if ip.to_ipv4_mapped().is_some() {
        Some("registered B/IPv6 refuses an IPv4-mapped address: register IPv4 as B/IP")
    } else if ip.is_unicast_link_local() {
        // A link-local address needs IPv6_Zone_Index, which isn't modeled.
        Some("registered B/IPv6 refuses a link-local address: no IPv6_Zone_Index is modeled")
    } else {
        None
    };
    if let Some(message) = refused {
        return Err(invalid(message));
    }
    check_vmac(port, served_device)?;
    Ok(address)
}

/// [`bip6_address`]'s VMAC check.
fn check_vmac(port: &Bip6Port, served_device: Option<u32>) -> Result<(), Error> {
    let Some(instance) = served_device else {
        return Err(invalid(
            "registered B/IPv6 requires a served Device: the port's VMAC is its instance",
        ));
    };
    let expected = device_instance_vmac(instance)
        .ok_or_else(|| invalid("registered B/IPv6 requires a Device instance in 0..=4194303"))?;
    match port.vmac {
        Some(vmac) if vmac == expected => Ok(()),
        Some(vmac) => Err(Error::Encoding(format!(
            "registered B/IPv6 refused: the transport's VMAC {} is not Device {instance}'s \
             VMAC {}; build the transport with the served Device instance",
            VmacHex(vmac),
            VmacHex(expected),
        ))),
        // Before start a transport reports a VMAC only when a valid Device
        // instance gave it one.
        None => Err(Error::Encoding(format!(
            "registered B/IPv6 refused: the transport has no Device instance in 0..=4194303, \
             so its VMAC would be random; build it with Device instance {instance}"
        ))),
    }
}

/// Publish a started B/IP port's bind, mode and multicast group.
pub fn publish_bip(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    port: BipPort,
    capacity: u32,
) -> Result<SocketAddrV4, Error> {
    let address = port.endpoint;
    db.publish_bip_port_internal(
        oid,
        address.ip().octets(),
        address.port(),
        capacity,
        port.mode,
    )?;
    // A port that can use B/IP multicast serves its group (#941).
    db.set_network_port_multicast_internal(oid, port.multicast)?;
    Ok(address)
}

/// Publish a started B/IPv6 port's bind, VMAC, group and mode; the bound
/// address and VMAC it published.
pub fn publish_bip6(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    port: Bip6Port,
    capacity: u32,
) -> Result<(SocketAddrV6, [u8; 3]), Error> {
    let vmac = port
        .vmac
        .ok_or_else(|| invalid("started B/IPv6 port reports no VMAC"))?;
    let address = port.endpoint;
    db.publish_bip6_port_internal(
        oid,
        address.ip().octets(),
        address.port(),
        vmac,
        port.multicast_group.octets(),
        capacity,
        port.mode,
    )?;
    Ok((address, vmac))
}

fn invalid(message: &str) -> Error {
    Error::Encoding(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::bip_port::BipPortMode;
    use std::net::Ipv6Addr;

    fn port(vmac: Option<[u8; 3]>) -> Bip6Port {
        Bip6Port {
            endpoint: SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 5), 0, 0, 0),
            vmac,
            multicast_group: Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0),
            mode: BipPortMode::Normal,
        }
    }

    /// The VMAC is the served instance's 22 bits, high octet first (#2057).
    #[test]
    fn bip6_vmac_is_the_served_device_instance() {
        for (instance, vmac) in [
            (0, [0, 0, 0]),
            (1234, [0x00, 0x04, 0xD2]),
            (0x3F_FFFE, [0x3F, 0xFF, 0xFE]),
        ] {
            check_vmac(&port(Some(vmac)), Some(instance)).unwrap();
            // One bit off in each octet is another Device's.
            for octet in 0..3 {
                let mut other = vmac;
                other[octet] ^= 1;
                assert!(check_vmac(&port(Some(other)), Some(instance)).is_err());
            }
            assert!(check_vmac(&port(None), Some(instance)).is_err());
        }
        // A served instance past 4194303 has no VMAC to match.
        assert!(check_vmac(&port(Some([0x40, 0, 0])), Some(0x40_0000)).is_err());
        // No served Device: nothing to check the VMAC against.
        let error = bip6_address(&port(Some([0, 0, 1])), None).unwrap_err();
        assert_eq!(
            error.to_string(),
            invalid("registered B/IPv6 requires a served Device: the port's VMAC is its instance")
                .to_string()
        );
    }
}
