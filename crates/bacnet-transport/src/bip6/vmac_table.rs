//! BACnet/IPv6 VMAC generation and scoped-endpoint mappings.

use std::collections::HashMap;
use std::net::{Ipv6Addr, SocketAddrV6};
use std::sync::Arc;

use bacnet_types::error::Error;

use super::Bip6Vmac;

/// Defensive cap for learned on-link VMAC mappings.
pub(super) const MAX_VMAC_TABLE_ENTRIES: usize = 4096;

pub(super) fn derive_vmac_from_device_instance(device_instance: u32) -> Bip6Vmac {
    debug_assert!(device_instance <= 0x3F_FFFF);
    let bytes = device_instance.to_be_bytes();
    [bytes[1], bytes[2], bytes[3]]
}

/// Generate a Random Device Instance VMAC for collision resolution (Clause H.7.2).
pub fn generate_random_vmac() -> Result<Bip6Vmac, Error> {
    let mut vmac = [0u8; 3];
    getrandom::fill(&mut vmac).map_err(|error| {
        Error::Encoding(format!("failed to generate Random Instance VMAC: {error}"))
    })?;
    vmac[0] = (vmac[0] & 0x3F) | 0x40;
    Ok(vmac)
}

/// VMAC-to-address mapping table per Clause U.5.
/// Updated from incoming frames and address-resolution exchanges.
#[derive(Debug, Clone)]
pub(super) struct VmacTable {
    mappings: Arc<tokio::sync::RwLock<VmacMappings>>,
}

#[derive(Debug, Default)]
struct VmacMappings {
    by_vmac: HashMap<Bip6Vmac, SocketAddrV6>,
}

impl VmacTable {
    pub(super) fn new() -> Self {
        Self {
            mappings: Arc::new(tokio::sync::RwLock::new(VmacMappings::default())),
        }
    }

    /// Learn the latest VMAC and exact scoped endpoint from an incoming frame.
    pub(super) async fn learn(&self, vmac: Bip6Vmac, addr: SocketAddrV6) {
        let mut mappings = self.mappings.write().await;

        let updates_known_vmac = mappings.by_vmac.contains_key(&vmac);
        // A fully scoped endpoint has one current owner. Replacing it removes
        // the stale VMAC while still allowing the same public IP+port on
        // different interfaces to remain ambiguous at lookup time.
        let old_owner = mappings.by_vmac.iter().find_map(|(mapped_vmac, endpoint)| {
            (*mapped_vmac != vmac && *endpoint == addr).then_some(*mapped_vmac)
        });
        if !updates_known_vmac
            && old_owner.is_none()
            && mappings.by_vmac.len() >= MAX_VMAC_TABLE_ENTRIES
        {
            return;
        }
        if let Some(old_vmac) = old_owner {
            mappings.by_vmac.remove(&old_vmac);
        }

        mappings.by_vmac.insert(vmac, addr);
    }

    /// Look up the B/IPv6 address for a VMAC.
    #[allow(dead_code)] // used by address-resolution handling and tests
    pub(super) async fn lookup(&self, vmac: &Bip6Vmac) -> Option<SocketAddrV6> {
        self.mappings.read().await.by_vmac.get(vmac).copied()
    }

    /// Resolve the public IPv6+port MAC to the latest VMAC and scoped endpoint.
    ///
    /// The 18-byte public MAC omits IPv6 scope ID. Returning the exact learned
    /// endpoint lets link-local replies preserve the ingress interface.
    pub(super) async fn resolve_by_addr(
        &self,
        ip: Ipv6Addr,
        port: u16,
    ) -> Option<(Bip6Vmac, SocketAddrV6)> {
        let mappings = self.mappings.read().await;
        let mut matches = mappings
            .by_vmac
            .iter()
            .filter(|(_, endpoint)| *endpoint.ip() == ip && endpoint.port() == port);
        let (&vmac, &endpoint) = matches.next()?;
        if matches.next().is_some() {
            None
        } else {
            Some((vmac, endpoint))
        }
    }

    #[cfg(test)]
    pub(super) async fn len(&self) -> usize {
        self.mappings.read().await.by_vmac.len()
    }
}
