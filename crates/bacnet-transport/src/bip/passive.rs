//! A socket owner that can only receive host-delivered B/IP datagrams.
use super::socket::{SocketRole, udp_socket};
use crate::udp_metadata::{DestinationReceiver, IpVersion};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddrV4};
use tokio::net::UdpSocket;

/// A receive-only B/IP socket. No node, responder, registration or send worker
/// is created. Dropping this value closes the socket; receiving is cancel-safe.
/// This is host UDP delivery, not promiscuous capture or a multicast listener.
pub struct PassiveBip {
    socket: UdpSocket,
    receiver: DestinationReceiver,
    interface: Ipv4Addr,
    broadcast: Ipv4Addr,
    index: Option<u32>,
}
impl PassiveBip {
    /// Bind the selected interface and port (zero requests an exclusive ephemeral
    /// port). Unix uses a wildcard socket with destination/interface filtering so
    /// broadcasts arrive too. Windows can receive broadcasts on the address bind.
    pub async fn bind(interface: Ipv4Addr, port: u16) -> io::Result<Self> {
        let local = if interface.is_unspecified() {
            None
        } else {
            super::socket::probe_interface(interface)
                .map_err(|e| io::Error::other(e.to_string()))?;
            Some(
                tokio::task::spawn_blocking(move || {
                    crate::local_addresses::interface_of(interface)
                })
                .await
                .map_err(io::Error::other)??
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::AddrNotAvailable,
                        "passive B/IP interface is not configured",
                    )
                })?,
            )
        };
        #[cfg(unix)]
        if local.is_some_and(|local| local.index.is_none()) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "passive B/IP needs the selected interface's receive index",
            ));
        }
        let role = if port == 0 {
            SocketRole::PrivateWildcard
        } else {
            SocketRole::SharedWildcard
        };
        let socket = udp_socket(role)?;
        let bind_ip = if cfg!(windows) {
            interface
        } else {
            Ipv4Addr::UNSPECIFIED
        };
        socket.bind(&SocketAddrV4::new(bind_ip, port).into())?;
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V4)?;
        Ok(Self {
            socket: UdpSocket::from_std(socket.into())?,
            receiver,
            interface,
            broadcast: local
                .and_then(|local| local.subnet_broadcast(interface))
                .unwrap_or(Ipv4Addr::BROADCAST),
            index: local.and_then(|local| local.index),
        })
    }
    /// The selected address and actual bound port; wildcard means every interface.
    pub fn local_addr(&self) -> io::Result<SocketAddrV4> {
        Ok(SocketAddrV4::new(
            self.interface,
            self.socket.local_addr()?.port(),
        ))
    }
    /// Pure native address rules using the selected interface's known subnet.
    /// Wildcard binds have no selected subnet, so only universal groups are
    /// known; a remote directed broadcast is never guessed from its suffix.
    pub fn groups(&self) -> io::Result<bacnet_bip_core::node::Groups> {
        Ok(bacnet_bip_core::node::Groups::new(
            SocketAddrV4::new(self.broadcast, self.socket.local_addr()?.port()),
            self.interface,
        ))
    }
    /// Receive one datagram and its UDP peer into `buffer`, ignoring traffic for
    /// other selected interfaces. Supply 65,535 bytes to avoid UDP truncation.
    pub async fn recv_from(&self, buffer: &mut [u8]) -> io::Result<(usize, SocketAddrV4)> {
        loop {
            let datagram = self.receiver.recv_from(&self.socket, buffer).await?;
            let std::net::SocketAddr::V4(peer) = datagram.peer else {
                continue;
            };
            if !self.interface.is_unspecified() {
                let IpAddr::V4(destination) = datagram.destination else {
                    continue;
                };
                let group = destination == self.broadcast || destination.is_broadcast();
                if destination != self.interface && !group {
                    continue;
                }
                if group && self.index.is_some() && datagram.arrival_index != self.index {
                    continue;
                }
            }
            return Ok((datagram.len, peer));
        }
    }
}
