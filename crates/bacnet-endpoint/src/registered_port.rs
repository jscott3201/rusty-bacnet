//! One explicit registered B/IP or B/IPv6 association; declaration is not
//! registration. The port may run in NORMAL, FOREIGN or BBMD mode; startup
//! publishes it. A B/IPv6 port registers through
//! [`TransportPort::bip6_port`] and publishes its VMAC and multicast group
//! too, as the full server's does (#1579, #1940).
use super::*;
use bacnet_server::network_port_registration as registration;
use bacnet_transport::port::{Bip6Port, BipPort};

/// What the started link reports for its registered port, with the port
/// capacity: B/IP's or B/IPv6's.
pub(super) type BoundPorts = (Option<(BipPort, u16)>, Option<(Bip6Port, u16)>);

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Select one existing built-in Network Port for the owned B/IP or
    /// B/IPv6 link, in whichever mode the transport runs: an IPV4 port for
    /// B/IP, an IPV6 one for B/IPv6.
    /// Startup requires a matching concrete interface, identity entry and database
    /// snapshot, and a B/IPv6 transport built with the served Device's
    /// instance (the identity's, else the database's Device), which is its
    /// VMAC (#2057). It publishes the actual address, UDP port and MAC (a
    /// B/IPv6 link's VMAC) before exposing responder roles.
    ///
    /// There is no B/IPv6 endpoint builder, as there is no B/IPv6 server
    /// builder: compose the session over a `Bip6Transport` (the
    /// `bacnet-transport/ipv6` feature) and declare the port with
    /// [`DeviceIdentity::with_bip6_port`](crate::DeviceIdentity::with_bip6_port).
    ///
    /// ```no_run
    /// use std::net::Ipv6Addr;
    /// use bacnet_endpoint::{DeviceIdentity, EndpointSession, SessionConfig, SessionRole};
    /// use bacnet_transport::bip6::Bip6Transport;
    /// use bacnet_types::{enums::ObjectType, primitives::ObjectIdentifier};
    ///
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let identity = DeviceIdentity::new(1234, 42)?.with_bip6_port(1, 6, Ipv6Addr::LOCALHOST, 0)?;
    /// let port = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1)?;
    /// // The VMAC comes from the Device instance (Clause H.7.2).
    /// let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(identity.instance()));
    /// let mut session =
    ///     EndpointSession::new(transport, SessionRole::ServerOnly, SessionConfig::default())?
    ///         .device(identity)
    ///         .with_registered_network_port(port);
    /// session.start().await?; // Port 1 now serves the bound address and VMAC.
    /// session.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_registered_network_port(mut self, oid: ObjectIdentifier) -> Self {
        self.assert_configurable();
        self.registered_network_port = Some(oid);
        self
    }

    pub(super) async fn prepare_registered_port(&mut self) -> Result<(), Error> {
        let Some(oid) = self.registered_network_port else {
            return Ok(());
        };
        let ingress = self
            .ingress
            .as_mut()
            .ok_or_else(|| invalid("missing ingress"))?;
        let address = match ingress.bip_port() {
            Some(port) => registration::bip_address(&port, ingress.bip_broadcast_endpoint())?,
            None if ingress.bip6_port().is_some() => return self.prepare_bip6(oid).await,
            // A B/IP link that reports no mode is configured both ways.
            None if ingress.bip_broadcast_endpoint().is_some() => {
                return Err(invalid(
                    "registered Network Port refused: a BBMD that also registers as a \
                     foreign device has no single B/IP mode",
                ));
            }
            None => return Err(invalid("registration requires B/IP or B/IPv6")),
        };
        let db = self
            .database
            .as_ref()
            .ok_or_else(|| invalid("registered port requires database"))?;
        let mut db = db.write().await;
        // Validate identity before reservation so a rejected configuration has no effects.
        let configured = db
            .configured_bip_port_internal(&oid)
            .ok_or_else(|| invalid("selected built-in B/IP port missing"))?;
        if let Some(identity) = &self.identity {
            identity.validate_registered_bip_internal(oid, &configured)?;
        }
        let (_, lease) =
            db.reserve_bip_port_internal(oid, address.ip().octets(), address.port())?;
        self.registered_port_lease = Arc::downgrade(&lease);
        ingress.retain_network_port_lease_internal(lease)
    }

    /// [`prepare_registered_port`](Self::prepare_registered_port) for a
    /// B/IPv6 link: one concrete unicast address, a VMAC from the served
    /// Device's instance (#2057) and the identity entry, all checked before
    /// the port is reserved.
    async fn prepare_bip6(&mut self, oid: ObjectIdentifier) -> Result<(), Error> {
        let ingress = self
            .ingress
            .as_mut()
            .ok_or_else(|| invalid("missing ingress"))?;
        let port = ingress
            .bip6_port()
            .ok_or_else(|| invalid("registration requires B/IP or B/IPv6"))?;
        let db = self
            .database
            .as_ref()
            .ok_or_else(|| invalid("registered port requires database"))?;
        let mut db = db.write().await;
        // The served Device: the identity's, else the database's own.
        let served = self
            .identity
            .as_ref()
            .map(|identity| identity.instance())
            .or_else(|| {
                db.local_device()
                    .identifier()
                    .map(|device| device.instance_number())
            });
        let address = registration::bip6_address(&port, served)?;
        // Validate identity before reservation so a rejected configuration has no effects.
        let configured = db
            .configured_bip6_port_internal(&oid)
            .ok_or_else(|| invalid("selected built-in B/IPv6 port missing"))?;
        if let Some(identity) = &self.identity {
            identity.validate_registered_bip6_internal(oid, &configured)?;
        }
        let (_, lease) =
            db.reserve_bip6_port_internal(oid, address.ip().octets(), address.port())?;
        self.registered_port_lease = Arc::downgrade(&lease);
        ingress.retain_network_port_lease_internal(lease)
    }

    pub(super) async fn publish_registered_port(
        &mut self,
        actual: BoundPorts,
        local_network: &bacnet_network::network_number::LocalNetworkNumber,
    ) -> Result<(), Error> {
        let Some(oid) = self.registered_network_port else {
            return Ok(());
        };
        let mut db = self
            .database
            .as_ref()
            .expect("validated database")
            .write()
            .await;
        // The identity entry follows only once the database took the bind.
        let bound = match actual {
            (Some((port, capacity)), _) => Bound::V4(registration::publish_bip(
                &mut db,
                oid,
                port,
                capacity.into(),
            )?),
            (None, Some((port, capacity))) => {
                let (address, vmac) =
                    registration::publish_bip6(&mut db, oid, port, capacity.into())?;
                Bound::V6(address, vmac)
            }
            (None, None) => return Err(invalid("registered B/IP mode changed during bind")),
        };
        // Written values activate only through a ReinitializeDevice handler.
        db.set_network_port_activation_internal(oid, self.reinitialize.is_some())?;
        // The port's configured number is the local network number from the
        // start (#1403); the Number owner copies every later change from the
        // port, as the full server's does.
        if let Some(state) = db.network_number_internal(oid, None) {
            local_network.publish(state);
        }
        if let Some(identity) = &mut self.identity {
            match bound {
                Bound::V4(address) => identity.publish_registered_bip_internal(oid, address),
                Bound::V6(address, vmac) => {
                    identity.publish_registered_bip6_internal(oid, address, vmac)
                }
            }
        }
        Ok(())
    }
}

/// The bind a started link published for its registered port.
enum Bound {
    V4(std::net::SocketAddrV4),
    V6(std::net::SocketAddrV6, [u8; 3]),
}
fn invalid(message: &str) -> Error {
    Error::Encoding(message.into())
}
