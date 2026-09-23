//! Immutable target routing facts captured before shared runtime ownership.
use super::event_recipient_route::{ConfirmedRecipientRoute, RecipientRoute};
use super::*;
use bacnet_types::constructed::BACnetRecipient;

#[derive(Default)]
pub(super) struct AuditRoutes {
    devices: HashMap<ObjectIdentifier, Arc<ConfirmedRecipientRoute>>,
    bip_broadcast: Option<std::net::SocketAddrV4>,
    configured_broadcasts: std::collections::HashSet<MacAddr>,
}

impl AuditRoutes {
    fn capture<T: TransportPort>(bindings: &DeviceBindingTable, transport: &T) -> Self {
        Self {
            devices: bindings
                .configured_resolutions()
                .filter_map(|(device, resolution)| {
                    RecipientRoute::from_device_resolution(resolution)
                        .into_confirmed()
                        .map(|route| (device, Arc::new(route)))
                })
                .collect(),
            bip_broadcast: transport.bip_broadcast_endpoint(),
            configured_broadcasts: Default::default(),
        }
    }

    pub(super) fn prepare<T: TransportPort>(
        db: &mut ObjectDatabase,
        config: &ServerConfig,
        bindings: &DeviceBindingTable,
        transport: &T,
    ) -> Result<Self, Error> {
        let routes = if config.audit_reporter.is_some() {
            Self::capture(bindings, transport)
        } else {
            Self::default()
        };
        super::audit_recipient::validate(db, config, &routes)?;
        Ok(routes)
    }

    /// Finalize link facts and configured next hops after start, before shared ownership.
    pub(super) async fn finish<T: TransportPort + 'static>(
        mut self,
        db: &mut ObjectDatabase,
        config: &ServerConfig,
        network: &mut NetworkLayer<T>,
    ) -> Result<Arc<Self>, Error> {
        if config.audit_reporter.is_some() {
            self.bip_broadcast = network.transport().bip_broadcast_endpoint();
            // A generic link can learn its broadcast identity during start.
            // Invoke caller code once here, never during target production or
            // mutation under shared DB/admission locks. Invalid Device routes
            // become unresolved rather than rejecting the whole server.
            self.devices.retain(|_, route| {
                local_next_hop(route).is_some_and(|mac| {
                    if network.transport().is_broadcast_mac(mac) {
                        // Retain the fact as well as pruning delivery: source
                        // correlation uses the same post-start eligibility.
                        self.configured_broadcasts.insert(mac.clone());
                        false
                    } else {
                        true
                    }
                })
            });
            if let Err(error) = super::audit_recipient::validate(db, config, &self) {
                let _ = network.stop().await;
                return Err(error);
            }
        }
        Ok(Arc::new(self))
    }

    pub(super) fn is_broadcast(&self, mac: &[u8]) -> bool {
        self.configured_broadcasts.contains(mac)
            || self.bip_broadcast.is_some_and(|broadcast| {
                mac.len() == 6
                    && mac[..4] == broadcast.ip().octets()
                    && mac[4..] == broadcast.port().to_be_bytes()
            })
    }

    /// Pure lookup/byte validation: no transport, caller code, locks or clocks.
    pub(super) fn resolve(
        &self,
        recipient: &BACnetRecipient,
    ) -> Option<Arc<ConfirmedRecipientRoute>> {
        match recipient {
            BACnetRecipient::Device(device) => {
                let route = self.devices.get(device)?;
                let next_hop = local_next_hop(route)?;
                (!self.is_broadcast(next_hop)).then(|| Arc::clone(route))
            }
            BACnetRecipient::Address(address) => {
                self.bip_broadcast?;
                if !valid_bip_audit_address(address) || self.is_broadcast(&address.mac_address) {
                    return None;
                }
                RecipientRoute::LocalUnicast(address.mac_address.clone())
                    .into_confirmed()
                    .map(Arc::new)
            }
        }
    }
}

fn local_next_hop(route: &ConfirmedRecipientRoute) -> Option<&MacAddr> {
    route.local_target.as_ref().or_else(|| {
        route
            .remote
            .as_ref()
            .and_then(|(_, _, router)| router.as_ref())
    })
}
