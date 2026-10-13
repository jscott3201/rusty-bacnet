//! How a B/IPv6 transport names this node and its peers: by VMAC, the
//! three octets the network layer knows a B/IPv6 node by (U.1.3), with
//! the B/IPv6 address (IPv6 address and UDP port) underneath.

use std::net::SocketAddrV6;

use bacnet_types::error::Error;

use super::resolve_counters::Bip6ResolutionCounters;
use super::{Bip6Transport, Bip6Vmac, UnresolvedSend};

impl Bip6Transport {
    /// This node's VMAC while the transport runs: the one its device
    /// instance gives (H.7.2), or the random one it chose at start. `None`
    /// before start and after stop. [`local_mac`](crate::port::TransportPort::local_mac)
    /// holds the same three octets.
    pub fn local_vmac(&self) -> Option<Bip6Vmac> {
        self.local_address.map(|_| self.source_vmac)
    }

    /// This node's B/IPv6 address while the transport runs: the selected
    /// IPv6 address and the bound UDP port, with the link's scope ID for a
    /// link-local address. `None` before start and after stop. A peer that
    /// knows this address can find this node's VMAC with
    /// [`resolve_virtual_address`](Self::resolve_virtual_address).
    pub fn local_address(&self) -> Option<SocketAddrV6> {
        self.local_address
    }

    /// The B/IPv6 address the transport has learned for `vmac`, if any. The
    /// table follows the frames this node receives (U.5) and holds up to
    /// 4096 peers, dropping the one heard from least recently for a new one.
    pub fn lookup_vmac(&self, vmac: Bip6Vmac) -> Option<SocketAddrV6> {
        self.resolver.lookup(vmac)
    }

    /// Find the VMAC of the B/IPv6 node at `peer` (U.5).
    ///
    /// When the table already maps that address to one VMAC this returns it
    /// at once. Otherwise it sends the node a Virtual-Address-Resolution by
    /// unicast, up to three times a second apart, and returns the VMAC its
    /// Virtual-Address-Resolution-ACK names; the answer also enters the
    /// table, so a send to that VMAC goes straight out. Because the request
    /// is unicast it works for a foreign device too, and wherever multicast
    /// doesn't reach.
    ///
    /// Fails with [`TimedOut`](std::io::ErrorKind::TimedOut) when nothing
    /// answers within three seconds, with
    /// [`ConnectionRefused`](std::io::ErrorKind::ConnectionRefused) when the
    /// node answers with a NAK, with
    /// [`NotConnected`](std::io::ErrorKind::NotConnected) when the transport
    /// isn't running, and with
    /// [`QuotaExceeded`](std::io::ErrorKind::QuotaExceeded) when sixteen
    /// lookups are already running. This is the only place the transport
    /// turns an IPv6 address into a VMAC: `send_unicast` never does.
    pub async fn resolve_virtual_address(&self, peer: SocketAddrV6) -> Result<Bip6Vmac, Error> {
        let Some(local) = self.local_address else {
            return Err(Error::Transport(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "Transport not started",
            )));
        };
        if peer.ip() == local.ip() && peer.port() == local.port() {
            return Ok(self.source_vmac);
        }
        self.resolver.resolve_virtual(peer).await
    }

    /// What the send side has done to find peers since the transport was
    /// created: resolutions run, requests sent, and sends refused or dropped
    /// while resolving. The totals survive a restart.
    pub fn resolution_counters(&self) -> Bip6ResolutionCounters {
        self.resolver.counters()
    }

    /// Choose what [`send_unicast`](crate::port::TransportPort::send_unicast)
    /// does with an NPDU for a VMAC it has no address for yet: wait for the
    /// resolution (the default), or queue the NPDU and return at once, as
    /// a router port does. Takes effect from the next send.
    pub fn set_unresolved_send(&mut self, hold: UnresolvedSend) {
        self.unresolved = hold;
    }

    /// What `send_unicast` does with an NPDU for an unresolved VMAC.
    pub fn unresolved_send(&self) -> UnresolvedSend {
        self.unresolved
    }
}
