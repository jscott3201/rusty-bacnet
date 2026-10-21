//! One window of the VMAC collision probe a normal-mode B/IPv6 transport
//! runs at start (U.5): after it sends an Address-Resolution for its own
//! VMAC, it reads until the window closes, answers another node probing for
//! the same VMAC, and watches for an answer from a node that already uses
//! it. The reads go through [`ProbeSocket`], so a test can script them.

use std::io;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use bacnet_types::error::Error;
use tokio::time::Instant;
use tracing::debug;

use super::frame::encoded;
use super::ingress::{is_local_unicast_delivery, LocalBinding};
use super::socket::{Bip6Socket, Inbound};
use super::{Bip6Vmac, Bvll6};

/// The socket operations a probe window uses.
pub(super) trait ProbeSocket {
    /// The next read, as [`Bip6Socket::recv_from`] returns it.
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<Inbound>;
    /// Send `bytes` to `peer`.
    async fn send_to(&self, bytes: &[u8], peer: SocketAddr) -> io::Result<usize>;
}

impl ProbeSocket for Bip6Socket {
    async fn recv_from(&self, buf: &mut [u8]) -> io::Result<Inbound> {
        Bip6Socket::recv_from(self, buf).await
    }

    async fn send_to(&self, bytes: &[u8], peer: SocketAddr) -> io::Result<usize> {
        Bip6Socket::send_to(self, bytes, peer).await
    }
}

/// What a probe window judges its reads against.
pub(super) struct Probe<'a> {
    /// The VMAC being probed for.
    pub(super) vmac: Bip6Vmac,
    /// The selected local address.
    pub(super) local_ip: Ipv6Addr,
    /// The socket's UDP port.
    pub(super) local_port: u16,
    /// The local unicast addresses.
    pub(super) local_unicast_ips: &'a [Ipv6Addr],
    /// Whether the socket is bound to the wildcard address.
    pub(super) wildcard_bind: bool,
    /// The BACnet group the probe went to.
    pub(super) group: Ipv6Addr,
    /// Whether a failed send or read fails the start, as for a random VMAC,
    /// rather than ending the window.
    pub(super) required: bool,
}

impl Probe<'_> {
    /// Read `socket` into `buf` until `deadline`, and say whether another
    /// node uses this VMAC. A read the selected link doesn't take, or one
    /// with unusable metadata, is skipped and the window goes on.
    pub(super) async fn window(
        &self,
        socket: &impl ProbeSocket,
        buf: &mut [u8],
        deadline: Instant,
    ) -> Result<bool, Error> {
        loop {
            let Ok(read) = tokio::time::timeout_at(deadline, socket.recv_from(buf)).await else {
                // The window closed with no collision.
                return Ok(false);
            };
            let received = match read {
                Ok(Inbound::Datagram(received)) => received,
                Ok(Inbound::OffLink) => continue,
                Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                    debug!(error = %e, "Error during VMAC collision check");
                    continue;
                }
                Err(e) => {
                    if self.required {
                        return Err(Error::Transport(e));
                    }
                    debug!(error = %e, "Configured-VMAC collision probe receive failed");
                    return Ok(false);
                }
            };
            let Ok(message) = bacnet_bip_codec::ipv6::decode(&buf[..received.len]) else {
                continue;
            };
            let is_self_loop = matches!(
                received.peer,
                SocketAddr::V6(peer)
                    if peer.port() == self.local_port
                        && (*peer.ip() == self.local_ip
                            || self.local_unicast_ips.contains(peer.ip()))
            );
            if let Bvll6::AddressResolution { source, target } = message {
                if target == self.vmac
                    && matches!(received.destination, IpAddr::V6(ip) if ip == self.group)
                    && received.os_group_delivery != Some(false)
                    && !is_self_loop
                {
                    let ack = encoded(&Bvll6::AddressResolutionAck {
                        source: self.vmac,
                        destination: source,
                    })?;
                    if let Err(error) = socket.send_to(&ack, received.peer).await {
                        if self.required {
                            return Err(Error::Transport(error));
                        }
                        debug!(error = %error, "Configured-VMAC AR-Ack send failed");
                    }
                    if source == self.vmac {
                        return Ok(true);
                    }
                }
            }
            // AR-ACK from another node using our VMAC.
            if let Bvll6::AddressResolutionAck {
                source,
                destination,
            } = message
            {
                let local = LocalBinding {
                    ip: self.local_ip,
                    vmac: self.vmac,
                    unicast_ips: self.local_unicast_ips,
                    wildcard_bind: self.wildcard_bind,
                };
                if source == self.vmac
                    && is_local_unicast_delivery(
                        received.destination,
                        Some(destination),
                        &local,
                        received.os_group_delivery,
                    )
                {
                    return Ok(true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::net::SocketAddrV6;
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use crate::bip6::BACNET_IPV6_MULTICAST_SITE_LOCAL;
    use crate::udp_metadata::ReceivedDatagram;

    const VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x07, 0x10]);
    const OTHER_VMAC: Bip6Vmac = Bip6Vmac::new([0x00, 0x07, 0x11]);
    const GROUP: Ipv6Addr = BACNET_IPV6_MULTICAST_SITE_LOCAL;
    const PORT: u16 = 0xBAC0;

    fn local_ip() -> Ipv6Addr {
        Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x10)
    }

    fn peer() -> SocketAddr {
        SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0x30),
            PORT,
            0,
            0,
        ))
    }

    /// One scripted read.
    enum Read {
        OffLink,
        Datagram(Bvll6<'static>, Ipv6Addr),
        Error(io::ErrorKind),
    }

    /// A socket whose reads are scripted, and which records what it sends.
    /// Once the script runs out, a read waits for ever.
    #[derive(Default)]
    struct Scripted {
        reads: Mutex<VecDeque<Read>>,
        sent: Mutex<Vec<(Bvll6<'static>, SocketAddr)>>,
    }

    impl Scripted {
        fn new(reads: impl IntoIterator<Item = Read>) -> Self {
            Self {
                reads: Mutex::new(reads.into_iter().collect()),
                ..Self::default()
            }
        }
    }

    impl ProbeSocket for Scripted {
        async fn recv_from(&self, buf: &mut [u8]) -> io::Result<Inbound> {
            let next = self.reads.lock().unwrap().pop_front();
            match next {
                Some(Read::OffLink) => Ok(Inbound::OffLink),
                Some(Read::Datagram(message, destination)) => {
                    let frame = encoded(&message).unwrap();
                    buf[..frame.len()].copy_from_slice(&frame);
                    Ok(Inbound::Datagram(ReceivedDatagram {
                        len: frame.len(),
                        peer: peer(),
                        destination: IpAddr::V6(destination),
                        arrival_index: Some(1),
                        os_group_delivery: None,
                        arrival_stamp: None,
                    }))
                }
                Some(Read::Error(kind)) => Err(io::Error::from(kind)),
                None => std::future::pending().await,
            }
        }

        async fn send_to(&self, bytes: &[u8], peer: SocketAddr) -> io::Result<usize> {
            let Ok(Bvll6::AddressResolutionAck {
                source,
                destination,
            }) = bacnet_bip_codec::ipv6::decode(bytes)
            else {
                panic!("the probe sends only Address-Resolution-ACKs");
            };
            let ack = Bvll6::AddressResolutionAck {
                source,
                destination,
            };
            self.sent.lock().unwrap().push((ack, peer));
            Ok(bytes.len())
        }
    }

    fn probe(local_unicast_ips: &[Ipv6Addr], required: bool) -> Probe<'_> {
        Probe {
            vmac: VMAC,
            local_ip: local_ip(),
            local_port: PORT,
            local_unicast_ips,
            wildcard_bind: false,
            group: GROUP,
            required,
        }
    }

    /// An Address-Resolution-ACK from a node that uses this VMAC, sent to
    /// this node: a collision.
    fn collision() -> Read {
        let ack = Bvll6::AddressResolutionAck {
            source: VMAC,
            destination: VMAC,
        };
        Read::Datagram(ack, local_ip())
    }

    async fn window(socket: &Scripted, required: bool) -> Result<bool, Error> {
        let ips = [local_ip()];
        let mut buf = [0u8; 64];
        let deadline = Instant::now() + Duration::from_millis(200);
        probe(&ips, required)
            .window(socket, &mut buf, deadline)
            .await
    }

    /// A read the selected link doesn't take, or one with unusable metadata,
    /// doesn't end the window: the collision after it is still seen.
    #[tokio::test(start_paused = true)]
    async fn a_skipped_read_does_not_end_the_window() {
        for skipped in [Read::OffLink, Read::Error(io::ErrorKind::InvalidData)] {
            let socket = Scripted::new([skipped, Read::OffLink, collision()]);
            assert!(window(&socket, true).await.unwrap());
        }
        let quiet = Scripted::new([Read::OffLink]);
        assert!(!window(&quiet, true).await.unwrap(), "the window closes");
    }

    /// Another node probing for this VMAC is answered, and the window goes
    /// on; a probe from this VMAC elsewhere is answered and is a collision.
    #[tokio::test(start_paused = true)]
    async fn another_nodes_probe_for_this_vmac_is_answered() {
        let probing = |source| Bvll6::AddressResolution {
            source,
            target: VMAC,
        };
        let socket = Scripted::new([Read::Datagram(probing(OTHER_VMAC), GROUP)]);
        assert!(!window(&socket, true).await.unwrap());
        let answer = Bvll6::AddressResolutionAck {
            source: VMAC,
            destination: OTHER_VMAC,
        };
        assert_eq!(*socket.sent.lock().unwrap(), [(answer, peer())]);

        let socket = Scripted::new([Read::Datagram(probing(VMAC), GROUP)]);
        assert!(window(&socket, true).await.unwrap());
        assert_eq!(socket.sent.lock().unwrap().len(), 1);
    }

    /// Any other failed read fails the start when the probe is required,
    /// and otherwise just ends the window.
    #[tokio::test(start_paused = true)]
    async fn a_failed_read_ends_the_window() {
        let failed = || Scripted::new([Read::Error(io::ErrorKind::ConnectionReset), collision()]);
        assert!(matches!(
            window(&failed(), true).await,
            Err(Error::Transport(_))
        ));
        assert!(!window(&failed(), false).await.unwrap());
    }
}
