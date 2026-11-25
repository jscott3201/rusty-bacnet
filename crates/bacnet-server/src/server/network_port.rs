//! Explicit single B/IP or B/IPv6 port admission and cancellation-owned
//! startup cleanup.
//!
//! The registered port may run in NORMAL, FOREIGN or BBMD mode. Startup
//! publishes the mode with the actual bind; in BBMD mode the Network Port
//! object then reads the transport's own tables on each property read. A
//! B/IPv6 port registers the same way, through
//! [`TransportPort::bip6_port`], and publishes its VMAC and multicast group
//! too.
use super::*;
use crate::network_port_registration as registration;

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Select the concrete built-in Network Port for this owned B/IP or
    /// B/IPv6 link, in whichever mode the transport runs. The object must
    /// already describe the concrete interface and configured UDP port
    /// (zero is allowed): an IPV4 object for B/IP, an IPV6 one for B/IPv6.
    /// A B/IPv6 transport must be built with the served Device's instance,
    /// which is its VMAC (#2057). Startup validates, then publishes the
    /// actual bind and mode.
    pub fn registered_network_port(mut self, oid: ObjectIdentifier) -> Self {
        self.config.registered_network_port = Some(oid);
        self
    }
}
impl BipServerBuilder {
    /// Select the built-in Network Port that represents this owned B/IP link.
    pub fn registered_network_port(mut self, oid: ObjectIdentifier) -> Self {
        self.config.registered_network_port = Some(oid);
        self
    }
}

/// Validate and reserve the registered port `oid`, if any, before the bind.
/// `device_instance` is the served Device's, which a B/IPv6 VMAC must match.
pub(super) fn prepare<T: TransportPort>(
    db: &mut ObjectDatabase,
    transport: &mut T,
    oid: Option<ObjectIdentifier>,
    device_instance: u32,
) -> Result<(), Error> {
    let Some(oid) = oid else {
        return Ok(());
    };
    let address = match transport.bip_port() {
        Some(port) => registration::bip_address(&port, transport.bip_broadcast_endpoint())?,
        None if transport.bip6_port().is_some() => {
            return prepare_bip6(db, transport, oid, device_instance);
        }
        // A B/IP link that reports no mode is configured both ways.
        None if transport.bip_broadcast_endpoint().is_some() => {
            return Err(Error::invalid_config(
                "registered Network Port refused: a BBMD that also registers as a foreign \
                 device has no single B/IP mode",
            ));
        }
        None => {
            return Err(Error::unsupported(NOT_BIP));
        }
    };
    let (_, lease) = db.reserve_bip_port_internal(oid, address.ip().octets(), address.port())?;
    transport.retain_network_port_lease_internal(lease)
}

/// A registered port's link that is neither B/IP nor B/IPv6.
const NOT_BIP: &str = "registered port requires B/IP or B/IPv6";

/// [`prepare`] for a B/IPv6 link: its configured address must be one
/// concrete unicast address, as a B/IP link's must, and its VMAC the served
/// Device's (#2057).
fn prepare_bip6<T: TransportPort>(
    db: &mut ObjectDatabase,
    transport: &mut T,
    oid: ObjectIdentifier,
    device_instance: u32,
) -> Result<(), Error> {
    let port = transport.bip6_port().ok_or(Error::unsupported(NOT_BIP))?;
    let address = registration::bip6_address(&port, Some(device_instance))?;
    let (_, lease) = db.reserve_bip6_port_internal(oid, address.ip().octets(), address.port())?;
    transport.retain_network_port_lease_internal(lease)
}

pub(super) fn publish<T: TransportPort + 'static>(
    db: &mut ObjectDatabase,
    network: &NetworkLayer<T>,
    oid: Option<ObjectIdentifier>,
    activation: bool,
) -> Result<(), Error> {
    let Some(oid) = oid else {
        return Ok(());
    };
    let transport = network.transport();
    let capacity = transport.local_receive_apdu_capacity() as u32;
    match transport.bip_port() {
        Some(port) => {
            registration::publish_bip(db, oid, port, capacity)?;
        }
        _ => {
            // The link reports no single B/IP mode now, as `prepare`
            // refuses one configured both ways (#1833).
            let port = transport.bip6_port().ok_or(Error::invalid_config(
                "registered B/IP mode changed during bind",
            ))?;
            registration::publish_bip6(db, oid, port, capacity)?;
        }
    }
    // Written values activate only through a ReinitializeDevice handler.
    db.set_network_port_activation_internal(oid, activation)?;
    // The port's configured number is the local network number from the
    // start; the control worker copies every later change from the port.
    if let Some(state) = db.network_number_internal(oid, None) {
        network.local_network_number().publish(state);
    }
    Ok(())
}

/// Failed/cancelled startup still owns cleanup. Capturing the runtime at creation
/// lets Drop transfer the network even when its caller drops the future elsewhere.
pub(super) struct StartingNetwork<T: TransportPort + 'static> {
    network: Option<NetworkLayer<T>>,
    runtime: tokio::runtime::Handle,
}
impl<T: TransportPort + 'static> StartingNetwork<T> {
    pub(super) fn new(transport: T) -> Self {
        Self {
            network: Some(NetworkLayer::new(transport)),
            runtime: tokio::runtime::Handle::current(),
        }
    }
    pub(super) fn from_network(network: NetworkLayer<T>) -> Self {
        Self {
            network: Some(network),
            runtime: tokio::runtime::Handle::current(),
        }
    }
    pub(super) fn network(&mut self) -> &mut NetworkLayer<T> {
        self.network.as_mut().expect("startup network owned")
    }
    pub(super) fn finish(mut self) -> NetworkLayer<T> {
        self.network.take().expect("startup network owned")
    }
    pub(super) async fn cleanup(mut self) -> Result<(), Error> {
        let network = self.network.take().expect("startup network owned");
        self.runtime
            .spawn(async move { network.stop().await })
            .await
            .map_err(|error| {
                // The cleanup task panicked or was aborted (#1833).
                Error::State(
                    bacnet_types::error::Lifecycle::Closed,
                    bacnet_types::error::Refusal::text(format!("startup cleanup failed: {error}")),
                )
            })?
    }
}
impl<T: TransportPort + 'static> Drop for StartingNetwork<T> {
    fn drop(&mut self) {
        if let Some(network) = self.network.take() {
            self.runtime.spawn(async move {
                if let Err(error) = network.stop().await {
                    tracing::warn!(%error, "cancelled startup cleanup failed");
                }
            });
        }
    }
}

/// Bind, validate the selected port and finalize Audit routes under one cleanup owner.
pub(super) async fn start<T: TransportPort + 'static>(
    db: &mut ObjectDatabase,
    config: &ServerConfig,
    mut transport: T,
    audit_routes: super::audit_recipient_routes::AuditRoutes,
) -> Result<
    (
        NetworkLayer<T>,
        bacnet_network::layer::ApduReceiver,
        Arc<super::audit_recipient_routes::AuditRoutes>,
        Option<mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>>,
    ),
    Error,
> {
    prepare(
        db,
        &mut transport,
        config.registered_network_port,
        config.device.instance(),
    )?;
    let mut starting = StartingNetwork::new(transport);
    let started = async {
        let controls = if starting
            .network()
            .transport()
            .supports_local_nonrouter_number_controls()
        {
            Some(starting.network().enable_network_control_receiver()?)
        } else {
            None
        };
        let apdu_rx = starting.network().start().await?;
        publish(
            db,
            starting.network(),
            config.registered_network_port,
            config.on_reinitialize.is_some(),
        )?;
        let routes = audit_routes.finish(db, config, starting.network()).await?;
        let controls = starting
            .network()
            .transport()
            .supports_local_nonrouter_number_controls()
            .then_some(controls)
            .flatten();
        Ok::<_, Error>((apdu_rx, routes, controls))
    }
    .await;
    match started {
        Ok((apdu_rx, routes, controls)) => Ok((starting.finish(), apdu_rx, routes, controls)),
        // The refusal stays the start's own, a port in use among them; a
        // cleanup that fails too is logged beside it (#2752).
        Err(error) => {
            if let Err(cleanup) = starting.cleanup().await {
                tracing::warn!(%cleanup, "startup cleanup after a refused start failed");
            }
            Err(error)
        }
    }
}

/// Own the Number controls. When one changes the published number, target
/// Audit re-evaluates its recipient's route (#1460).
pub(super) fn spawn_number_worker<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
    db: &Arc<RwLock<ObjectDatabase>>,
    selected: Option<ObjectIdentifier>,
    mut controls: mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>,
    audit: Option<std::sync::Weak<super::audit_recipient::TargetAudit<T>>>,
) -> JoinHandle<()> {
    let network = Arc::clone(network);
    let mut owner =
        crate::network_number::NetworkNumberOwner::new(selected.map(|oid| (Arc::clone(db), oid)))
            .publishing_to(network.local_network_number().clone());
    super::heap_futures::spawn_boxed(move || async move {
        use crate::network_number::NumberTurn;
        loop {
            // A control, or the answer a broadcast question left waiting
            // (#2597).
            let reply = match owner.next(&mut controls).await {
                NumberTurn::Closed => break,
                NumberTurn::Control(control) => {
                    let before = network.local_network_number().get();
                    let reply = owner.handle(control).await;
                    if network.local_network_number().get() != before
                        && let Some(audit) = audit.as_ref().and_then(std::sync::Weak::upgrade)
                    {
                        audit.number_changed();
                    }
                    reply
                }
                NumberTurn::Answer(answer) => answer,
            };
            if let Some(npdu) = reply
                && let Err(error) = network.transport().send_broadcast(&npdu).await
            {
                tracing::debug!(%error, "Network-Number-Is broadcast failed");
            }
        }
    })
}

/// Resolve the Device's Max_APDU_Length_Accepted against the transport's
/// receive capacity, once, at start.
///
/// The result stays fixed while the server runs, even if the transport's
/// capacity later falls (a BACnet/SC hub failover, a router port whose path
/// shrinks; #2093). Peers read the value once, from I-Am or their address
/// binding, and cache it, so changing the property afterwards would not
/// reach them. Replies stay safe without it: each response is sized to its
/// own route's budget (`ResponseRoute::max_apdu_length`, the requester's
/// declared limit included), so a smaller path still gets replies that fit.
pub(super) fn validate_apdu_capacity<T: TransportPort>(
    config: &mut ServerConfig,
    transport: &T,
) -> Result<(), Error> {
    config.device = config
        .device
        .resolve(transport.local_receive_apdu_capacity())?;
    Ok(())
}
