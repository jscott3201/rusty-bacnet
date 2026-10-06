//! Explicit single B/IP port admission and cancellation-owned startup cleanup.
//!
//! The registered port may run in NORMAL, FOREIGN or BBMD mode. Startup
//! publishes the mode with the actual bind; in BBMD mode the Network Port
//! object then reads the transport's own tables on each property read.
use super::*;
use bacnet_network::layer::ReceivedApdu;

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Select the concrete built-in Network Port for this owned B/IP link, in
    /// whichever B/IP mode the transport runs. The object must already
    /// describe the concrete interface and configured UDP port (zero is
    /// allowed). Startup validates, then publishes the actual bind and mode.
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

pub(super) fn prepare<T: TransportPort>(
    db: &mut ObjectDatabase,
    transport: &mut T,
    oid: Option<ObjectIdentifier>,
) -> Result<(), Error> {
    let Some(oid) = oid else {
        return Ok(());
    };
    let address = transport
        .bip_port()
        .ok_or_else(|| Error::Encoding("registered port requires B/IP in one mode".into()))?
        .endpoint;
    let ip = *address.ip();
    if ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_broadcast()
        || transport
            .bip_broadcast_endpoint()
            .is_some_and(|broadcast| *broadcast.ip() == ip)
    {
        return Err(Error::Encoding(
            "registered B/IP requires a concrete unicast interface".into(),
        ));
    }
    let (_, lease) = db.reserve_bip_port_internal(oid, ip.octets(), address.port())?;
    transport.retain_network_port_lease_internal(lease)
}

pub(super) fn publish<T: TransportPort + 'static>(
    db: &mut ObjectDatabase,
    network: &NetworkLayer<T>,
    oid: Option<ObjectIdentifier>,
) -> Result<(), Error> {
    let Some(oid) = oid else {
        return Ok(());
    };
    let port = network
        .transport()
        .bip_port()
        .ok_or_else(|| Error::Encoding("registered B/IP mode changed during bind".into()))?;
    db.publish_bip_port_internal(
        oid,
        port.endpoint.ip().octets(),
        port.endpoint.port(),
        network.transport().local_receive_apdu_capacity() as u32,
        port.mode,
    )?;
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
        let mut network = self.network.take().expect("startup network owned");
        self.runtime
            .spawn(async move { network.stop().await })
            .await
            .map_err(|error| Error::Encoding(format!("startup cleanup failed: {error}")))?
    }
}
impl<T: TransportPort + 'static> Drop for StartingNetwork<T> {
    fn drop(&mut self) {
        if let Some(mut network) = self.network.take() {
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
        mpsc::Receiver<ReceivedApdu>,
        Arc<super::audit_recipient_routes::AuditRoutes>,
        Option<mpsc::Receiver<bacnet_network::layer::ReceivedNetworkControl>>,
    ),
    Error,
> {
    prepare(db, &mut transport, config.registered_network_port)?;
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
        publish(db, starting.network(), config.registered_network_port)?;
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
        Err(error) => {
            starting.cleanup().await?;
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
        while let Some(control) = controls.recv().await {
            let before = network.local_network_number().get();
            let reply = owner.handle(control).await;
            if network.local_network_number().get() != before {
                if let Some(audit) = audit.as_ref().and_then(std::sync::Weak::upgrade) {
                    audit.number_changed();
                }
            }
            if let Some(npdu) = reply {
                if let Err(error) = network.transport().send_broadcast(&npdu).await {
                    tracing::debug!(%error, "Network-Number-Is broadcast failed");
                }
            }
        }
    })
}

pub(super) fn validate_apdu_capacity<T: TransportPort>(
    config: &mut ServerConfig,
    transport: &T,
) -> Result<(), Error> {
    let transport_max = transport.local_receive_apdu_capacity() as u32;
    config.max_apdu_length = config.max_apdu_length.min(transport_max);
    if config.max_apdu_length < u32::from(apdu::MINIMUM_MESSAGE_SIZE) {
        return Err(Error::Encoding(format!(
            "local max_apdu_length {} is below 50",
            config.max_apdu_length
        )));
    }
    Ok(())
}
