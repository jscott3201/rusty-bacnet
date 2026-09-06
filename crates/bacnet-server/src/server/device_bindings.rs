use super::*;

/// Maximum number of configured and observed device bindings held by a server.
pub(super) const MAX_DEVICE_BINDINGS: usize = 4096;

/// Freshness window for passively observed I-Am bindings.
pub(super) const OBSERVED_BINDING_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceBindingTarget {
    Local {
        peer_mac: MacAddr,
    },
    Routed {
        network: u16,
        final_mac: MacAddr,
        router_mac: MacAddr,
    },
}

/// One explicitly configured unicast route to a Device object.
///
/// Constructed values are transport-neutral. A builder performs the remaining
/// concrete data-link broadcast check before the transport is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBinding {
    device: ObjectIdentifier,
    target: DeviceBindingTarget,
}

impl DeviceBinding {
    /// Create a binding to a peer on the server's local network.
    pub fn local(device: ObjectIdentifier, peer_mac: impl AsRef<[u8]>) -> Result<Self, Error> {
        validate_device_identifier(device)?;
        let peer_mac = MacAddr::from_slice(peer_mac.as_ref());
        if peer_mac.is_empty() {
            return Err(binding_error("local peer MAC must be non-empty"));
        }
        Ok(Self {
            device,
            target: DeviceBindingTarget::Local { peer_mac },
        })
    }

    /// Create a routed binding with a final network/address and local router.
    pub fn routed(
        device: ObjectIdentifier,
        network: u16,
        final_mac: impl AsRef<[u8]>,
        router_mac: impl AsRef<[u8]>,
    ) -> Result<Self, Error> {
        validate_device_identifier(device)?;
        if !(1..=0xFFFE).contains(&network) {
            return Err(binding_error("routed network must be in 1..=65534"));
        }
        let final_mac = MacAddr::from_slice(final_mac.as_ref());
        if final_mac.is_empty() {
            return Err(binding_error("routed final MAC must be non-empty"));
        }
        let router_mac = MacAddr::from_slice(router_mac.as_ref());
        if router_mac.is_empty() {
            return Err(binding_error("routed router MAC must be non-empty"));
        }
        Ok(Self {
            device,
            target: DeviceBindingTarget::Routed {
                network,
                final_mac,
                router_mac,
            },
        })
    }
}

fn binding_error(message: &str) -> Error {
    Error::Encoding(format!("invalid device binding: {message}"))
}

fn validate_device_identifier(device: ObjectIdentifier) -> Result<(), Error> {
    if device.object_type() != ObjectType::DEVICE {
        return Err(binding_error("identifier must name a Device object"));
    }
    Ok(())
}

pub(super) fn register_configured_binding(
    bindings: &mut Vec<DeviceBinding>,
    binding: DeviceBinding,
) -> Result<(), Error> {
    if bindings.len() >= MAX_DEVICE_BINDINGS {
        return Err(binding_error("configured binding capacity exceeded"));
    }
    if bindings
        .iter()
        .any(|configured| configured.device == binding.device)
    {
        return Err(binding_error("duplicate configured Device identifier"));
    }
    bindings.push(binding);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DeviceResolution {
    ResolvedLocal {
        peer_mac: MacAddr,
        freshness: BindingFreshness,
    },
    ResolvedRouted {
        network: u16,
        final_mac: MacAddr,
        router_mac: MacAddr,
        freshness: BindingFreshness,
    },
    Unknown,
    Stale,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BindingFreshness {
    Configured,
    ObservedUntil(tokio::time::Instant),
}

impl BindingFreshness {
    pub(super) fn permits_attempt_at(self, now: tokio::time::Instant) -> bool {
        match self {
            Self::Configured => true,
            Self::ObservedUntil(deadline) => now < deadline,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObservationOutcome {
    Inserted,
    Refreshed,
    ConfiguredPreserved,
    RejectedInvalid,
    RejectedCapacity,
}

#[derive(Debug, Clone)]
enum BindingEntry {
    Configured(DeviceBindingTarget),
    Observed {
        target: DeviceBindingTarget,
        observed_at: Instant,
    },
}

#[derive(Debug, Default)]
pub(super) struct DeviceBindingTable {
    entries: HashMap<ObjectIdentifier, BindingEntry>,
}

impl DeviceBindingTable {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn from_configured(
        bindings: Vec<DeviceBinding>,
        is_broadcast: impl Fn(&[u8]) -> bool,
    ) -> Result<Self, Error> {
        let mut table = Self::new();
        for binding in bindings {
            table.insert_configured(binding, &is_broadcast)?;
        }
        Ok(table)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn insert_configured(
        &mut self,
        binding: DeviceBinding,
        is_broadcast: impl Fn(&[u8]) -> bool,
    ) -> Result<(), Error> {
        validate_device_identifier(binding.device)?;
        if !target_is_usable(&binding.target, &is_broadcast) {
            return Err(binding_error(
                "local peer or next-hop router is a broadcast address",
            ));
        }
        if self.entries.contains_key(&binding.device) {
            return Err(binding_error("duplicate configured Device identifier"));
        }
        if self.entries.len() >= MAX_DEVICE_BINDINGS {
            return Err(binding_error("configured binding capacity exceeded"));
        }
        self.entries
            .insert(binding.device, BindingEntry::Configured(binding.target));
        Ok(())
    }

    pub(super) fn observe_i_am_at(
        &mut self,
        device: ObjectIdentifier,
        source_mac: &[u8],
        source_network: Option<&NpduAddress>,
        now: Instant,
        is_broadcast: impl Fn(&[u8]) -> bool,
    ) -> ObservationOutcome {
        if validate_device_identifier(device).is_err() {
            return ObservationOutcome::RejectedInvalid;
        }
        let target = match source_network {
            None => DeviceBindingTarget::Local {
                peer_mac: MacAddr::from_slice(source_mac),
            },
            Some(source)
                if (1..=0xFFFE).contains(&source.network) && !source.mac_address.is_empty() =>
            {
                DeviceBindingTarget::Routed {
                    network: source.network,
                    final_mac: source.mac_address.clone(),
                    router_mac: MacAddr::from_slice(source_mac),
                }
            }
            Some(_) => return ObservationOutcome::RejectedInvalid,
        };
        if !target_is_usable(&target, &is_broadcast) {
            return ObservationOutcome::RejectedInvalid;
        }

        match self.entries.get_mut(&device) {
            Some(BindingEntry::Configured(_)) => ObservationOutcome::ConfiguredPreserved,
            Some(BindingEntry::Observed {
                target: observed,
                observed_at,
            }) => {
                *observed = target;
                *observed_at = now;
                ObservationOutcome::Refreshed
            }
            None => {
                if self.entries.len() >= MAX_DEVICE_BINDINGS {
                    self.entries.retain(|_, entry| match entry {
                        BindingEntry::Configured(_) => true,
                        BindingEntry::Observed { observed_at, .. } => {
                            now.saturating_duration_since(*observed_at) < OBSERVED_BINDING_TTL
                        }
                    });
                }
                if self.entries.len() >= MAX_DEVICE_BINDINGS {
                    return ObservationOutcome::RejectedCapacity;
                }
                self.entries.insert(
                    device,
                    BindingEntry::Observed {
                        target,
                        observed_at: now,
                    },
                );
                ObservationOutcome::Inserted
            }
        }
    }

    pub(super) fn resolve_at(
        &self,
        device: &ObjectIdentifier,
        now: Instant,
        is_broadcast: impl Fn(&[u8]) -> bool,
    ) -> DeviceResolution {
        if device.object_type() != ObjectType::DEVICE {
            return DeviceResolution::Invalid;
        }
        let (target, freshness) = match self.entries.get(device) {
            Some(BindingEntry::Configured(target)) => (target, BindingFreshness::Configured),
            Some(BindingEntry::Observed {
                target,
                observed_at,
            }) => {
                if now.saturating_duration_since(*observed_at) >= OBSERVED_BINDING_TTL {
                    return DeviceResolution::Stale;
                }
                (
                    target,
                    BindingFreshness::ObservedUntil(tokio::time::Instant::from_std(
                        *observed_at + OBSERVED_BINDING_TTL,
                    )),
                )
            }
            None => return DeviceResolution::Unknown,
        };
        if !target_is_usable(target, is_broadcast) {
            return DeviceResolution::Invalid;
        }
        match target {
            DeviceBindingTarget::Local { peer_mac } => DeviceResolution::ResolvedLocal {
                peer_mac: peer_mac.clone(),
                freshness,
            },
            DeviceBindingTarget::Routed {
                network,
                final_mac,
                router_mac,
            } => DeviceResolution::ResolvedRouted {
                network: *network,
                final_mac: final_mac.clone(),
                router_mac: router_mac.clone(),
                freshness,
            },
        }
    }
}

fn target_is_usable(target: &DeviceBindingTarget, is_broadcast: impl Fn(&[u8]) -> bool) -> bool {
    match target {
        DeviceBindingTarget::Local { peer_mac } => !peer_mac.is_empty() && !is_broadcast(peer_mac),
        DeviceBindingTarget::Routed {
            network,
            final_mac,
            router_mac,
        } => {
            (1..=0xFFFE).contains(network)
                && !final_mac.is_empty()
                && !router_mac.is_empty()
                && !is_broadcast(router_mac)
        }
    }
}
