//! One socket/port, selected normal-mode source and receive authority.
//!
//! The socket claims its port, whether the OS picked it (#892) or it was
//! requested (#2385): it sets no `SO_REUSEADDR`, and on Windows it sets
//! `SO_EXCLUSIVEADDRUSE` (`port_ownership`, #950). A later bind of the port
//! then fails, where with sharing it could take the unicast sent there.
//! [`set_share_wildcard_port`](super::Bip6Transport::set_share_wildcard_port)
//! opts a requested port back into `SO_REUSEADDR`.
//!
//! A normal node binds `[::]:port` and joins the BACnet groups on that one
//! socket, so the claim costs it nothing: the groups' datagrams, its own
//! looped back among them, arrive as before. What the claim refuses is a
//! second stack on the host at the same port. On macOS and the BSDs a socket
//! that sets `SO_REUSEADDR` may still bind a specific address on the port
//! and then gets the unicast sent to that address; they have no option to
//! stop it.
//!
//! A foreign device binds `address:port`. Its claim refuses another bind of
//! that address and port on every OS, and of `[::]:port` on Linux and
//! Windows. macOS and the BSDs let an `SO_REUSEADDR` socket bind `[::]:port`
//! beside it, which gets none of the unicast sent to the device's address.

use std::{
    io,
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6},
};
use tokio::net::UdpSocket;

use super::{
    BACNET_IPV6_MULTICAST_LINK_LOCAL, BACNET_IPV6_MULTICAST_ORG_LOCAL,
    BACNET_IPV6_MULTICAST_SITE_LOCAL, link::SelectedLink, send::SelectedSender,
};
use crate::tap::{FrameTap, TapDirection, TapPeer};
use crate::udp_metadata::{DestinationReceiver, IpVersion, ReceivedDatagram};
use bacnet_types::data_link::DataLink;

/// Every task that owns the socket owns it through one `Arc`, so the
/// registered Network Port's lease lives as long as the last of them. Rust
/// drops fields in declaration order: the socket closes before the lease.
pub(super) struct Bip6Socket {
    udp: UdpSocket,
    _network_port_lease: Option<std::sync::Arc<()>>,
    receiver: DestinationReceiver,
    pub selection: Option<SelectedLink>,
    sender: Option<SelectedSender>,
    /// The transport's tap: every datagram sent, and every one read that
    /// the transport takes.
    tap: FrameTap,
}

/// How a socket holds the port it binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PortUse {
    /// Claimed where the OS has a way to (`port_ownership`, #950), with no
    /// `SO_REUSEADDR`: the default for a requested port, so a later bind of
    /// it fails (#2385).
    Claimed,
    /// A requested port opted into sharing: `SO_REUSEADDR`. A port the OS
    /// picks is claimed all the same: Linux would otherwise hand an
    /// ephemeral bind a port another `SO_REUSEADDR` socket owns, and unicast
    /// to it would reach only one of them (#892).
    Shared,
}

impl PortUse {
    /// What a transport asks for: sharing only when it opted in.
    pub(super) fn sharing(share: bool) -> Self {
        if share { Self::Shared } else { Self::Claimed }
    }

    /// What a bind of `port` sets: `self`, but a port the OS picks is never
    /// shared.
    fn on(self, port: u16) -> Self {
        if port == 0 { Self::Claimed } else { self }
    }
}

/// The unbound B/IPv6 socket for `port`, held as `port_use` says.
fn udp_socket(port: u16, port_use: PortUse) -> io::Result<socket2::Socket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_only_v6(true)?;
    match port_use.on(port) {
        PortUse::Claimed => crate::port_ownership::claim_exclusive(&socket)?,
        PortUse::Shared => socket.set_reuse_address(true)?,
    }
    socket.set_nonblocking(true)?;
    socket.set_multicast_loop_v6(true)?;
    Ok(socket)
}

/// A bind's error. On a requested port that another socket holds it says
/// so and names the opt-in, as B/IP's does (#2366), and keeps the OS error
/// as its source, so its OS code survives (#2231).
fn held(bound: SocketAddrV6, port_use: PortUse, e: io::Error) -> io::Error {
    // Windows refuses a bind beside a socket that holds the port with
    // `SO_EXCLUSIVEADDRUSE` as WSAEACCES.
    let held = e.kind() == io::ErrorKind::AddrInUse
        || (cfg!(windows) && e.kind() == io::ErrorKind::PermissionDenied);
    let port = bound.port();
    if port == 0 || !held {
        return e;
    }
    // A foreign device binds its own address.
    let place = if bound.ip().is_unspecified() {
        format!("UDP port {port}")
    } else {
        format!("UDP port {port} on {}", bound.ip())
    };
    let context = match port_use {
        PortUse::Shared => {
            format!("{place} is in use on this host by a socket that refuses sharing")
        }
        PortUse::Claimed => format!(
            "{place} is in use on this host, and a B/IPv6 transport doesn't share its port by \
             default; stop the other program, use another port, or share the port \
             (share_wildcard_port, or --share-port in the CLI), after which unicast reaches \
             only one of the sockets on it"
        ),
    };
    crate::bip::listener_error::listener_error(context, e)
}

impl Bip6Socket {
    /// Bind `port` for `role`, held as `port_use` says. A requested port
    /// that another socket holds fails with [`held`]'s error.
    pub async fn bind(
        requested: Ipv6Addr,
        port: u16,
        port_use: PortUse,
        role: Role,
    ) -> io::Result<Self> {
        let selection = match role {
            Role::Normal => Some(SelectedLink::resolve(requested).await?),
            Role::Foreign(_) => None,
        };
        let socket = udp_socket(port, port_use)?;
        let bound = match role {
            Role::Foreign(bbmd) => foreign_bind_address(requested, port, bbmd)?,
            Role::Normal => SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, port, 0, 0),
        };
        socket
            .bind(&bound.into())
            .map_err(|e| held(bound, port_use, e))?;
        let receiver = DestinationReceiver::configure(&socket, IpVersion::V6)?;
        let sender = if let Some(link) = selection {
            socket.set_multicast_if_v6(link.index)?;
            for group in [
                BACNET_IPV6_MULTICAST_LINK_LOCAL,
                BACNET_IPV6_MULTICAST_SITE_LOCAL,
                BACNET_IPV6_MULTICAST_ORG_LOCAL,
            ] {
                socket.join_multicast_v6(&group, link.index)?;
            }
            Some(SelectedSender::new(&socket)?)
        } else {
            None
        };
        Ok(Self {
            udp: UdpSocket::from_std(socket.into())?,
            _network_port_lease: None,
            receiver,
            selection,
            sender,
            tap: FrameTap::default(),
        })
    }

    /// This socket, reporting its datagrams to `tap`.
    pub fn tapped(self, tap: FrameTap) -> Self {
        Self { tap, ..self }
    }

    /// This socket, keeping a registered Network Port's lease while it
    /// lives.
    pub fn leased(self, lease: Option<std::sync::Arc<()>>) -> Self {
        Self {
            _network_port_lease: lease,
            ..self
        }
    }

    pub fn local_address(&self) -> io::Result<SocketAddrV6> {
        match self.udp.local_addr()? {
            SocketAddr::V6(address) => Ok(address),
            _ => Err(io::Error::other("IPv6 socket returned a non-IPv6 address")),
        }
    }
    pub fn local_port(&self) -> io::Result<u16> {
        Ok(self.local_address()?.port())
    }

    pub async fn send_to(&self, bytes: &[u8], peer: impl Into<SocketAddr>) -> io::Result<usize> {
        let peer = peer.into();
        let sent = match (self.selection, &self.sender, peer) {
            (Some(link), Some(sender), SocketAddr::V6(peer)) => {
                sender.send(&self.udp, bytes, peer, link).await
            }
            (None, None, peer) => self.udp.send_to(bytes, peer).await,
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid selected IPv6 socket destination",
            )),
        };
        if sent.is_ok() {
            let to = || TapPeer::Udp(peer);
            self.tap
                .record(TapDirection::Outbound, DataLink::Bip6, to, bytes);
        }
        sent
    }

    /// Read the next datagram. In normal mode one the selected link doesn't
    /// take is read and refused ([`Inbound::OffLink`]); the caller reads on.
    pub async fn recv_from(&self, bytes: &mut [u8]) -> io::Result<Inbound> {
        let received = self.receiver.recv_from(&self.udp, bytes).await?;
        let inbound = classify(self.selection, received);
        if let Inbound::Datagram(datagram) = &inbound {
            let from = || TapPeer::Udp(datagram.peer);
            let read = bytes.get(..datagram.len).unwrap_or_default();
            self.tap
                .record(TapDirection::Inbound, DataLink::Bip6, from, read);
        }
        Ok(inbound)
    }
}

/// Whether the transport takes a datagram the socket read: in normal mode,
/// with `selection`, only one [`admits`] lets in, whose link-local peer then
/// gets the link's scope. In foreign mode, with none, every one.
fn classify(selection: Option<SelectedLink>, mut received: ReceivedDatagram) -> Inbound {
    if let Some(link) = selection {
        if !admits(link, &received) {
            return Inbound::OffLink;
        }
        if let SocketAddr::V6(peer) = &mut received.peer
            && peer.ip().is_unicast_link_local()
        {
            peer.set_scope_id(link.index);
        }
    }
    Inbound::Datagram(received)
}

/// What one read of a [`Bip6Socket`] returned.
#[derive(Debug)]
pub(super) enum Inbound {
    /// A datagram for this transport: in normal mode, one that arrived on
    /// the selected link and was sent to its address or a BACnet group.
    Datagram(ReceivedDatagram),
    /// In normal mode, a datagram that arrived on another interface or was
    /// sent to another address. The wildcard socket gets the datagrams for
    /// every address and interface at its port, so this is routine on a
    /// host with more than one, or with another B/IPv6 port at the same
    /// UDP port.
    OffLink,
}

fn foreign_bind_address(
    requested: Ipv6Addr,
    port: u16,
    bbmd: Option<SocketAddrV6>,
) -> io::Result<SocketAddrV6> {
    if !requested.is_unspecified() {
        return Ok(SocketAddrV6::new(requested, port, 0, 0));
    }
    // A start on `::` looks a BBMD's host name up first (#2365).
    let bbmd = bbmd.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "a foreign device bound to '::' needs its BBMD's address",
        )
    })?;
    // Bind the production socket to the route's exact source, so subsequent
    // registration and DBTN cannot silently advertise one address while
    // using another.
    route_source(bbmd).map(|source| SocketAddrV6::new(source, port, 0, 0))
}

/// How a socket is bound.
#[derive(Clone, Copy, Debug)]
pub(super) enum Role {
    /// A normal node: bound to `::` and joined to the BACnet groups on its
    /// selected link.
    Normal,
    /// A foreign device, which joins no group: bound to the interface
    /// given, or on `::` to the source of the route to its BBMD, whose
    /// address it then needs.
    Foreign(Option<SocketAddrV6>),
}

/// The source address this host's routes would send from to reach `bbmd`.
/// A UDP connect selects a route and source without sending a packet, so
/// this is a local, synchronous probe.
pub(super) fn route_source(bbmd: SocketAddrV6) -> io::Result<Ipv6Addr> {
    let probe = std::net::UdpSocket::bind(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))?;
    probe.connect(bbmd)?;
    match probe.local_addr()? {
        SocketAddr::V6(address)
            if !address.ip().is_unspecified() && !address.ip().is_multicast() =>
        {
            Ok(*address.ip())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "no concrete IPv6 source for configured BBMD",
        )),
    }
}

fn admits(link: SelectedLink, datagram: &ReceivedDatagram) -> bool {
    datagram.arrival_index == Some(link.index)
        && link.index != 0
        && matches!(datagram.destination, IpAddr::V6(ip) if ip == link.address
            || [BACNET_IPV6_MULTICAST_LINK_LOCAL, BACNET_IPV6_MULTICAST_SITE_LOCAL,
                BACNET_IPV6_MULTICAST_ORG_LOCAL].contains(&ip))
}

#[cfg(test)]
mod claim_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_interface_and_selected_destination_are_required_before_learning() {
        let link = SelectedLink {
            address: "fd12::1".parse().unwrap(),
            index: 3,
        };
        let mut datagram = ReceivedDatagram {
            len: 11,
            peer: "[fd12::2]:47808".parse().unwrap(),
            destination: IpAddr::V6(BACNET_IPV6_MULTICAST_SITE_LOCAL),
            arrival_index: Some(3),
            os_group_delivery: None,
            arrival_stamp: None,
        };
        assert!(admits(link, &datagram));
        for index in [None, Some(0), Some(4)] {
            datagram.arrival_index = index;
            assert!(!admits(link, &datagram));
        }
        datagram.arrival_index = Some(3);
        datagram.destination = IpAddr::V6(link.address);
        assert!(admits(link, &datagram));
        for destination in ["fd12::9", "::1", "::", "ff05::1"] {
            datagram.destination = IpAddr::V6(destination.parse().unwrap());
            assert!(!admits(link, &datagram));
        }
    }

    /// In normal mode a datagram the selected link doesn't admit is read as
    /// off-link, and one it admits is kept, a link-local peer taking the
    /// link's scope. In foreign mode every datagram is kept as it came.
    #[test]
    fn a_datagram_for_another_link_or_address_is_off_link() {
        let link = SelectedLink {
            address: "fd12::1".parse().unwrap(),
            index: 3,
        };
        let datagram = |peer: &str, destination: &str, arrival_index| ReceivedDatagram {
            len: 11,
            peer: peer.parse().unwrap(),
            destination: destination.parse().unwrap(),
            arrival_index,
            os_group_delivery: None,
            arrival_stamp: None,
        };
        let elsewhere = [
            datagram("[fd12::2]:47808", "fd12::9", Some(3)),
            datagram("[fd12::2]:47808", "fd12::1", Some(4)),
            datagram("[fe80::2]:47808", "ff05::bac0", None),
        ];
        for received in elsewhere {
            assert!(
                matches!(classify(Some(link), received), Inbound::OffLink),
                "{received:?}"
            );
            assert!(matches!(
                classify(None, received),
                Inbound::Datagram(kept) if kept == received
            ));
        }

        let on_link = datagram("[fe80::2]:47808", "ff05::bac0", Some(3));
        let Inbound::Datagram(kept) = classify(Some(link), on_link) else {
            panic!("an admitted datagram is kept");
        };
        assert_eq!(kept.peer, "[fe80::2%3]:47808".parse().unwrap());
        assert_eq!((kept.destination, kept.len), (on_link.destination, 11));
        let unicast = datagram("[fd12::2]:47808", "fd12::1", Some(3));
        assert!(matches!(
            classify(Some(link), unicast),
            Inbound::Datagram(kept) if kept == unicast
        ));
    }
}
