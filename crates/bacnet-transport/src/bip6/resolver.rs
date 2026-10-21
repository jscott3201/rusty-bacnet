//! The B/IPv6 resolver: the [`Neighbours`] state behind one lock, the task
//! that sends the Address-Resolutions and Virtual-Address-Resolutions it
//! schedules, and the unicast send path that consults it (U.5).
//!
//! The lock is a `std::sync::Mutex` and is never held across an await: each
//! caller takes it, decides, and lets go before any I/O. A send that has to
//! wait parks on a oneshot that a learn or the driver completes.

use std::future::Future;
use std::io;
use std::net::SocketAddrV6;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bacnet_types::error::Error;
use tokio::sync::{oneshot, Notify};
use tokio::time::Instant;
use tracing::debug;

use super::frame::encoded;
use super::resolve::{
    Failure, Neighbours, Route, VirtualRoute, RESOLUTION_INTERVAL, RESOLUTION_TIMEOUT,
};
use super::resolve_counters::{Bip6ResolutionCounters, ResolutionCounts};
use super::socket::Bip6Socket;
use super::vmac_table::Heard;
use super::{Bip6Vmac, Bvll6};

/// What [`send_unicast`](crate::port::TransportPort::send_unicast) does with
/// an NPDU for a VMAC the transport has no address for yet. Either way the
/// transport starts resolving the VMAC (U.5); the two differ in when the
/// call returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnresolvedSend {
    /// Wait for the resolution, then send. The call returns once the NPDU
    /// is sent, or fails after about three seconds with
    /// [`TimedOut`](std::io::ErrorKind::TimedOut). The default, for
    /// applications, which then see a clear error for a peer that isn't
    /// there.
    #[default]
    Wait,
    /// Copy the NPDU and return at once; it goes out when the VMAC
    /// resolves, and is dropped and counted if it doesn't. For a router
    /// port, whose sends must not stall behind one unknown destination:
    /// `BACnetRouter` selects it through
    /// [`prepare_for_routing`](crate::port::TransportPort::prepare_for_routing).
    Queue,
}

/// How the resolver reaches the network; a test captures what it sends.
pub(super) trait DatagramSender: Send + Sync + 'static {
    /// Send `bytes` to `peer`.
    fn send_to(
        &self,
        bytes: &[u8],
        peer: SocketAddrV6,
    ) -> impl Future<Output = io::Result<usize>> + Send;
}

impl DatagramSender for Bip6Socket {
    fn send_to(
        &self,
        bytes: &[u8],
        peer: SocketAddrV6,
    ) -> impl Future<Output = io::Result<usize>> + Send {
        Bip6Socket::send_to(self, bytes, peer)
    }
}

/// The neighbour table shared by the transport, its receive loop and its
/// driver. It lives as long as the transport, so its counts survive a
/// restart; stop clears the rest.
#[derive(Debug, Default)]
pub(super) struct Resolver {
    state: Mutex<Neighbours>,
    /// Wakes the driver for a new lookup or for queued NPDUs to send.
    wake: Notify,
    counts: ResolutionCounts,
}

fn failed(kind: io::ErrorKind, message: String) -> Error {
    Error::Transport(io::Error::new(kind, message))
}

/// A VMAC as six hex digits, as in `0x0004D2`.
pub(super) fn vmac_text(vmac: Bip6Vmac) -> String {
    let [high, middle, low] = vmac.octets();
    format!("0x{high:02X}{middle:02X}{low:02X}")
}

/// The VMAC a network-layer MAC names: exactly three octets (U.1.3).
pub(super) fn vmac_of(mac: &[u8]) -> Result<Bip6Vmac, Error> {
    <[u8; 3]>::try_from(mac).map(Bip6Vmac::new).map_err(|_| {
        failed(
            io::ErrorKind::InvalidInput,
            format!(
                "a B/IPv6 MAC is the peer's 3-octet VMAC, not {} octets",
                mac.len()
            ),
        )
    })
}

/// Send `npdu` to `destination` at `addr` in an Original-Unicast-NPDU.
async fn send_original_unicast(
    socket: &impl DatagramSender,
    source: Bip6Vmac,
    destination: Bip6Vmac,
    npdu: &[u8],
    addr: SocketAddrV6,
) -> Result<(), Error> {
    let frame = encoded(&Bvll6::OriginalUnicastNpdu {
        source,
        destination,
        npdu,
    })?;
    socket
        .send_to(&frame, addr)
        .await
        .map_err(Error::Transport)?;
    Ok(())
}

impl Resolver {
    fn lock(&self) -> MutexGuard<'_, Neighbours> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Set this node's own VMAC, which the table never learns.
    pub(super) fn set_own(&self, own: Option<Bip6Vmac>) {
        self.lock().table.set_own(own);
    }

    /// Forget every peer and fail every waiting send and lookup, counting
    /// the queued NPDUs dropped.
    pub(super) fn clear(&self) {
        self.lock().clear(&self.counts);
    }

    /// Learn that `vmac` is at `addr`, completing any resolution of it.
    pub(super) fn learn(&self, vmac: Bip6Vmac, addr: SocketAddrV6, heard: Heard) {
        let ready = self
            .lock()
            .learn(vmac, addr, heard, Instant::now(), &self.counts);
        if ready {
            self.wake.notify_one();
        }
    }

    /// Where `vmac` is, if the table knows.
    pub(super) fn lookup(&self, vmac: Bip6Vmac) -> Option<SocketAddrV6> {
        self.lock().table.lookup(vmac)
    }

    /// The configured BBMD sent an Address-Resolution NAK.
    pub(super) fn refuse_resolutions(&self) {
        self.lock().refuse_resolutions();
    }

    /// `peer` answered a Virtual-Address-Resolution with `vmac`.
    pub(super) fn virtual_answered(&self, peer: SocketAddrV6, vmac: Bip6Vmac) {
        self.lock().virtual_answered(peer, vmac, &self.counts);
    }

    /// `peer` sent a Virtual-Address-Resolution NAK.
    pub(super) fn virtual_refused(&self, peer: SocketAddrV6) {
        self.lock().virtual_refused(peer, &self.counts);
    }

    /// The counts so far.
    pub(super) fn counters(&self) -> Bip6ResolutionCounters {
        self.counts.snapshot()
    }

    /// Send `npdu` from `source` to the node whose VMAC `mac` holds,
    /// resolving it first as `hold` says when the table lacks it.
    pub(super) async fn send_unicast(
        &self,
        socket: &impl DatagramSender,
        source: Bip6Vmac,
        hold: super::UnresolvedSend,
        npdu: &[u8],
        mac: &[u8],
    ) -> Result<(), Error> {
        let destination = vmac_of(mac)?;
        if destination == source {
            return Err(failed(
                io::ErrorKind::InvalidInput,
                format!("{} is this node's own VMAC", vmac_text(destination)),
            ));
        }
        let route = self
            .lock()
            .route(destination, npdu, hold, Instant::now(), &self.counts);
        let addr = match route {
            Route::Send { addr, wake } => {
                if wake {
                    self.wake.notify_one();
                }
                addr
            }
            Route::Queued { wake } => {
                if wake {
                    self.wake.notify_one();
                }
                return Ok(());
            }
            Route::Wait { rx, wake } => {
                if wake {
                    self.wake.notify_one();
                }
                wait(rx, destination).await?
            }
            Route::Unreachable => {
                return Err(failed(
                    io::ErrorKind::HostUnreachable,
                    format!(
                        "B/IPv6 VMAC {} did not resolve a moment ago",
                        vmac_text(destination)
                    ),
                ))
            }
            Route::Full(limit) => {
                return Err(failed(
                    io::ErrorKind::QuotaExceeded,
                    format!(
                        "too many B/IPv6 sends waiting on resolution ({limit:?} bound), refusing one to VMAC {}",
                        vmac_text(destination)
                    ),
                ))
            }
        };
        send_original_unicast(socket, source, destination, npdu, addr).await
    }

    /// Find the VMAC of the node at `peer` with a Virtual-Address-Resolution,
    /// unless the table already maps that address to one.
    pub(super) async fn resolve_virtual(&self, peer: SocketAddrV6) -> Result<Bip6Vmac, Error> {
        let ip = peer.ip();
        if peer.port() == 0 || ip.is_unspecified() || ip.is_multicast() {
            return Err(failed(
                io::ErrorKind::InvalidInput,
                format!("{peer} is not one B/IPv6 node's address"),
            ));
        }
        let route = self.lock().resolve_virtual(peer, Instant::now());
        let rx = match route {
            VirtualRoute::Known(vmac) => return Ok(vmac),
            VirtualRoute::Full(limit) => {
                return Err(failed(
                    io::ErrorKind::QuotaExceeded,
                    format!("too many Virtual-Address-Resolutions running ({limit:?} bound)"),
                ))
            }
            VirtualRoute::Wait { rx, wake } => {
                if wake {
                    self.wake.notify_one();
                }
                rx
            }
        };
        match tokio::time::timeout(RESOLUTION_TIMEOUT + RESOLUTION_INTERVAL, rx).await {
            Ok(Ok(Ok(vmac))) => Ok(vmac),
            Ok(Ok(Err(Failure::Refused))) => Err(failed(
                io::ErrorKind::ConnectionRefused,
                format!("{peer} refused Virtual-Address-Resolution"),
            )),
            Ok(Ok(Err(Failure::TimedOut))) | Err(_) => Err(failed(
                io::ErrorKind::TimedOut,
                format!(
                    "no Virtual-Address-Resolution-ACK from {peer} within {} s",
                    RESOLUTION_TIMEOUT.as_secs()
                ),
            )),
            Ok(Err(_)) => Err(stopped()),
        }
    }
}

fn stopped() -> Error {
    failed(
        io::ErrorKind::NotConnected,
        "the B/IPv6 transport stopped while resolving".to_string(),
    )
}

/// Wait for the resolution of `vmac` that `rx` follows. The driver ends it
/// at [`RESOLUTION_TIMEOUT`]; the wait gives up one interval later should
/// the driver be gone.
async fn wait(
    rx: oneshot::Receiver<Result<SocketAddrV6, Failure>>,
    vmac: Bip6Vmac,
) -> Result<SocketAddrV6, Error> {
    let secs = RESOLUTION_TIMEOUT.as_secs();
    match tokio::time::timeout(RESOLUTION_TIMEOUT + RESOLUTION_INTERVAL, rx).await {
        Ok(Ok(Ok(addr))) => Ok(addr),
        Ok(Ok(Err(Failure::Refused))) => Err(failed(
            io::ErrorKind::ConnectionRefused,
            format!(
                "the BBMD refused to pass on Address-Resolution for B/IPv6 VMAC {}, and nothing answered within {secs} s",
                vmac_text(vmac)
            ),
        )),
        Ok(Ok(Err(Failure::TimedOut))) | Err(_) => Err(failed(
            io::ErrorKind::TimedOut,
            format!(
                "no Address-Resolution-ACK for B/IPv6 VMAC {} within {secs} s",
                vmac_text(vmac)
            ),
        )),
        Ok(Err(_)) => Err(stopped()),
    }
}

/// The task that sends what the resolver schedules: Address-Resolutions to
/// `resolution_to`, Virtual-Address-Resolutions to their peers and to quiet
/// peers being checked, and the queued NPDUs whose VMAC resolved. It sleeps until the next deadline or
/// until woken, and the transport aborts it on stop and on drop.
pub(super) struct Driver<S> {
    pub(super) socket: Arc<S>,
    pub(super) resolver: Arc<Resolver>,
    /// This node's VMAC.
    pub(super) source: Bip6Vmac,
    /// Where Address-Resolutions go for a node that isn't a foreign
    /// device: the scope's BACnet group at this node's port.
    pub(super) resolution_to: SocketAddrV6,
    /// A foreign device's registration: its Address-Resolutions go to the
    /// BBMD it names at the time (U.4.4).
    pub(super) foreign: Option<Arc<super::foreign_target::ForeignTarget>>,
}

impl<S: DatagramSender> Driver<S> {
    /// Run until aborted.
    pub(super) async fn run(self) {
        loop {
            let due = self
                .resolver
                .lock()
                .poll(Instant::now(), &self.resolver.counts);
            for target in due.resolutions {
                let request = Bvll6::AddressResolution {
                    source: self.source,
                    target,
                };
                let to = self
                    .foreign
                    .as_ref()
                    .map_or(self.resolution_to, |target| target.bbmd_addr());
                self.send(&request, to).await;
                self.resolver.counts.address_resolution_sent();
            }
            let request = Bvll6::VirtualAddressResolution {
                source: self.source,
            };
            for peer in due.virtual_resolutions {
                self.send(&request, peer).await;
                self.resolver.counts.virtual_sent();
            }
            // A quiet peer is asked at the address on file, by unicast, so
            // the check reaches it however it was learned.
            for peer in due.revalidations {
                self.send(&request, peer).await;
                self.resolver.counts.revalidation_sent();
            }
            for (addr, destination, npdu) in due.flush {
                let sent =
                    send_original_unicast(&*self.socket, self.source, destination, &npdu, addr)
                        .await;
                if let Err(error) = sent {
                    debug!(%error, to = %addr, "BIP6: failed to send a queued NPDU");
                }
            }
            match due.next {
                Some(at) => {
                    let _ = tokio::time::timeout_at(at, self.resolver.wake.notified()).await;
                }
                None => self.resolver.wake.notified().await,
            }
        }
    }

    async fn send(&self, message: &Bvll6<'_>, to: SocketAddrV6) {
        match encoded(message) {
            Ok(frame) => {
                if let Err(error) = self.socket.send_to(&frame, to).await {
                    debug!(%error, %to, "BIP6: failed to send a resolution request");
                }
            }
            Err(error) => debug!(%error, "BIP6: failed to encode a resolution request"),
        }
    }
}

#[cfg(test)]
impl Resolver {
    /// A resolver for a node whose VMAC is `own`, as a running transport's.
    pub(super) fn with_own(own: Bip6Vmac) -> Arc<Self> {
        let resolver = Arc::new(Self::default());
        resolver.set_own(Some(own));
        resolver
    }

    /// Run `f` on the state, under the lock.
    pub(super) fn with_state<R>(
        &self,
        f: impl FnOnce(&mut Neighbours, &ResolutionCounts) -> R,
    ) -> R {
        f(&mut self.lock(), &self.counts)
    }
}
